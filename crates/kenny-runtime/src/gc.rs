//! Garbage collector: non-moving mark & sweep.
//!
//! * Heap objects are traced precisely (we know every object layout).
//! * Machine stacks (compiled Ruby frames and Rust frames) and the compiled program's
//!   data section are scanned conservatively: any word equal to a live object address
//!   keeps that object alive. CRuby does the same for its C stack.
//! * Collection only happens at safepoints: method invocation, block calls and loop
//!   back-edges in compiled code. Allocation itself never collects, so Rust code may build
//!   objects freely; a Rust heap buffer of values that is held *across a call into Ruby*
//!   must be an [`RVec`] (registered root).

use crate::core::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

pub const FL_MARK: u32 = 4;
/// Class header flag: traced by an earlier collection. A minor collection doesn't trace
/// such a class again: its tables are covered by the class-table write barrier, and the
/// classes it links to are never freed.
const FL_CLASS_TRACED: u32 = 1 << 20;

/// Set when a collection is due; checked at safepoints (also by compiled loop back-edges).
/// Bit 1: collection due. PREEMPT_BIT (2): thread timer. STW_BIT: a collector is waiting for
/// every other OS thread to stop.
#[unsafe(no_mangle)]
pub static KN_GC_PENDING: AtomicU8 = AtomicU8::new(0);
pub const STW_BIT: u8 = 8;

const MIN_THRESHOLD: usize = 100_000;
const DEFAULT_GROWTH: f64 = 2.0;

const PAGE: usize = 1 << 16;
const SIZE_CLASSES: [usize; 4] = [32, 48, 64, 96];
/// The largest object a slot holds.
pub const MAX_SLOT: usize = 96;

pub struct Page {
    pub base: usize,
    pub slot: usize,
    pub nslots: usize,
    pub alloc: Vec<u64>,
    pub class_idx: usize,
    /// Objects were allocated here since the last collection (a minor collection sweeps
    /// only these pages: every other page holds old objects only).
    pub young: bool,
    /// SWEPT, UNSWEPT (holds dead objects not freed yet: a minor collection leaves sweeping
    /// to the background sweeper or the thread that next allocates from the page, outside
    /// the pause) or SWEEPING (one of them is at it).
    pub unswept: AtomicU8,
}
pub const SWEPT: u8 = 0;
pub const UNSWEPT: u8 = 1;
pub const SWEEPING: u8 = 2;

impl Page {
    fn is_unswept(&self) -> bool {
        self.unswept.load(Ordering::Acquire) != SWEPT
    }
    /// Sweep the page if it is unswept and nobody else is; wait if someone is.
    fn ensure_swept(&mut self) {
        loop {
            match self.unswept.compare_exchange(UNSWEPT, SWEEPING, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    lazy_sweep(self);
                    self.unswept.store(SWEPT, Ordering::Release);
                    return;
                }
                Err(SWEEPING) => std::hint::spin_loop(),
                Err(_) => return,
            }
        }
    }
}

/// The shared heap. Mutators change it only under `heap_lock()` (taking a page, registering
/// a class or a code region); the collector changes it while every other thread is stopped.
pub struct Heap {
    /// Boxed: a thread keeps a page's address after releasing the heap lock (`refill`), while
    /// others may add pages.
    pub pages: Vec<Box<Page>>,
    pub page_index: FxHashMap<usize, usize>,
    /// Lowest page base and highest page end: a quick filter for `is_object` (most words a
    /// stack scan sees are not heap pointers).
    pub page_lo: usize,
    pub page_hi: usize,
    /// Page index + 1 for each 64 KB block from `page_lo` (0: not a page), while that span is
    /// small enough; otherwise empty and `page_index` is used.
    pub page_table: Vec<u32>,
    /// Pages with free slots that no thread is allocating from, fullest last (popped first).
    pub pool: [Vec<usize>; 4],
    pub live: usize,
    pub class_set: FxHashSet<usize>,
    /// Closure environments of all threads (merged from each thread's list at a collection).
    pub envs: FxHashMap<usize, u32>,
    pub classes: Vec<*mut Class>,
    /// Classes created since the last collection (a minor one traces these fully; older
    /// ones only through what the class-table barrier recorded).
    pub new_classes: Vec<*mut Class>,
    pub regions: Vec<(usize, usize)>,
    pub collections: usize,
    /// Generational GC: minor collections since the last major one, old objects (alive
    /// after the last major one plus those promoted since), and whether the next collection
    /// must be major.
    pub minors: usize,
    pub major_live: usize,
    pub force_major: bool,
    pub majors: usize,
    pub disabled: u32,
    pub extra_roots: Vec<fn(&mut Vec<V>)>,
    pub stack_provider: Option<fn() -> Vec<(usize, usize)>>,
    /// KENNY_GC_STATS=1: time spent collecting, and the most pages ever held
    pub gc_nanos: u128,
    pub peak_pages: usize,
}

/// Per-OS-thread allocation and root state (part of `PState`).
pub struct PHeap {
    /// Free slots of the page this thread allocates from, per size class.
    pub free: [Vec<usize>; 4],
    pub bitmap: [*mut u64; 4],
    pub page_base: [usize; 4],
    /// Allocations not yet added to the global count.
    pub since: usize,
    /// Rust buffers of values held across calls into Ruby (`RVec`).
    pub roots: Vec<*const Vec<V>>,
    /// Closure environments created since the last collection.
    pub envs: FxHashMap<usize, u32>,
    /// Throws saved while `ensure` clauses run (the running thread's; moves with it).
    pub saved_throws: SavedThrows,
    /// Base (highest address) of the stack this thread is running on.
    pub stack_base: usize,
    /// RUNNING, PARKED (stopped for a collection) or IDLE (waiting, not touching the heap).
    pub status: AtomicU8,
    /// While parked or idle: where its stack ends, and its callee-saved registers.
    pub saved_sp: usize,
    pub regs: [usize; 16],
    /// Buffers this thread replaced (see `mt::retire`), freed by the next collection.
    pub retired: Vec<(usize, usize, usize)>,
    /// Old containers/objects written since the last collection (the write barrier).
    pub remembered: Vec<(u8, usize)>,
}
pub const RUNNING: u8 = 0;
pub const PARKED: u8 = 1;
pub const IDLE: u8 = 2;

impl PHeap {
    pub fn new() -> Self {
        PHeap {
            free: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            bitmap: [std::ptr::null_mut(); 4],
            page_base: [0; 4],
            since: 0,
            roots: Vec::new(),
            envs: FxHashMap::default(),
            saved_throws: SavedThrows::default(),
            stack_base: 0,
            status: AtomicU8::new(RUNNING),
            saved_sp: 0,
            regs: [0; 16],
            retired: Vec::new(),
            remembered: Vec::new(),
        }
    }
}
impl Default for PHeap {
    fn default() -> Self {
        Self::new()
    }
}

static HEAP: G<Option<Heap>> = G::new(None);
static HEAP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Allocations since the last collection (all threads), and the count that triggers one.
static SINCE: AtomicUsize = AtomicUsize::new(0);
static THRESHOLD: AtomicUsize = AtomicUsize::new(MIN_THRESHOLD);

pub fn heap() -> &'static mut Heap {
    HEAP.get().get_or_insert_with(|| {
        THRESHOLD.store(min_threshold(), Ordering::Relaxed);
        Heap {
            pages: Vec::new(),
            page_index: FxHashMap::default(),
            page_lo: usize::MAX,
            page_hi: 0,
            page_table: Vec::new(),
            pool: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            live: 0,
            class_set: FxHashSet::default(),
            envs: FxHashMap::default(),
            classes: Vec::new(),
            new_classes: Vec::new(),
            regions: Vec::new(),
            collections: 0,
            minors: 0,
            major_live: 0,
            force_major: true,
            majors: 0,
            disabled: 0,
            extra_roots: Vec::new(),
            stack_provider: None,
            gc_nanos: 0,
            peak_pages: 0,
        }
    })
}
/// Lock for changing the shared heap from a running thread (None with one thread).
pub fn heap_lock() -> Option<std::sync::MutexGuard<'static, ()>> {
    if crate::mt::multi() { Some(HEAP_LOCK.lock().unwrap_or_else(|e| e.into_inner())) } else { None }
}

unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}
extern "C" fn print_stats() {
    let h = heap();
    eprintln!(
        "[gc] collections={} gc_time={:.1}ms stop_wait={:.1}ms live={} pages={} peak_pages={} ({} KB peak) rust_heap={} KB",
        h.collections,
        h.gc_nanos as f64 / 1e6,
        STOP_NANOS.load(Ordering::Relaxed) as f64 / 1e6,
        h.live + h.envs.len(),
        h.pages.len(),
        h.peak_pages,
        h.peak_pages * PAGE / 1024,
        MALLOC_LIVE.load(Ordering::Relaxed) >> 10
    );
}
static STATS_EVERY: G<bool> = G::new(false);
/// Called once at startup: KENNY_GC_STATS=1 prints a GC summary when the process exits,
/// KENNY_GC_STATS=2 also a line after every collection.
pub fn init_stats() {
    if let Some(v) = std::env::var_os("KENNY_GC_STATS") {
        unsafe { atexit(print_stats) };
        *STATS_EVERY.get() = v == "2";
    }
    if std::env::var_os("KENNY_CRASH_TRACE").is_some() {
        install_crash_trace();
    }
}

/// KENNY_CRASH_TRACE=1 (debugging the runtime): on SIGSEGV/SIGBUS print the faulting address
/// and the native backtrace of the crashing thread.
fn install_crash_trace() {
    extern "C" fn on_crash(sig: i32, info: *mut libc::siginfo_t, _ctx: *mut std::ffi::c_void) {
        let addr = unsafe { (*info).si_addr } as usize;
        let name = std::thread::current().name().map(|s| s.to_string()).unwrap_or_default();
        eprintln!("\n*** signal {sig} at address {addr:#x} on thread {name:?}\n{}", std::backtrace::Backtrace::force_capture());
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }
    unsafe {
        let stack = libc::stack_t { ss_sp: Box::into_raw(vec![0u8; 1 << 20].into_boxed_slice()) as *mut _, ss_size: 1 << 20, ss_flags: 0 };
        libc::sigaltstack(&stack, std::ptr::null_mut());
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_crash as extern "C" fn(i32, *mut libc::siginfo_t, *mut std::ffi::c_void) as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
    }
}

/// A new empty page (heap lock held); returns its index.
fn new_page(h: &mut Heap, ci: usize) -> usize {
    let slot = SIZE_CLASSES[ci];
    let layout = std::alloc::Layout::from_size_align(PAGE, PAGE).unwrap();
    let base = unsafe { std::alloc::alloc_zeroed(layout) } as usize;
    if base == 0 {
        std::alloc::handle_alloc_error(layout);
    }
    let nslots = PAGE / slot;
    let idx = h.pages.len();
    h.pages.push(Box::new(Page { base, slot, nslots, alloc: vec![0; nslots.div_ceil(64)], class_idx: ci, young: true, unswept: AtomicU8::new(SWEPT) }));
    h.peak_pages = h.peak_pages.max(h.pages.len());
    h.page_index.insert(base, idx);
    if base < h.page_lo || base + PAGE > h.page_hi {
        h.page_lo = h.page_lo.min(base);
        h.page_hi = h.page_hi.max(base + PAGE);
        rebuild_page_table(h);
    } else if !h.page_table.is_empty() {
        h.page_table[(base - h.page_lo) / PAGE] = idx as u32 + 1;
    }
    idx
}

#[inline(always)]
fn size_class(size: usize) -> usize {
    if size <= 32 {
        0
    } else if size <= 48 {
        1
    } else if size <= 64 {
        2
    } else if size <= 96 {
        3
    } else {
        panic!("object too large for GC slot")
    }
}

/// Allocate an object slot of at least `size` bytes (16-byte aligned).
#[inline]
pub fn alloc_slot(size: usize) -> usize {
    let ci = size_class(size);
    let ph = &mut crate::core::pst().heap;
    let p = match ph.free[ci].pop() {
        Some(p) => p,
        None => refill(ph, ci),
    };
    // this thread owns the page until the next collection: nobody else writes its bitmap
    let i = (p - ph.page_base[ci]) / SIZE_CLASSES[ci];
    unsafe { *ph.bitmap[ci].add(i / 64) |= 1 << (i % 64) };
    count_alloc(ph);
    p
}
#[inline(always)]
fn count_alloc(ph: &mut PHeap) {
    ph.since += 1;
    if ph.since >= 64 {
        flush_count(ph);
    }
}
#[inline(never)]
fn flush_count(ph: &mut PHeap) {
    let n = std::mem::take(&mut ph.since);
    let t = SINCE.fetch_add(n, Ordering::Relaxed) + n;
    if t >= THRESHOLD.load(Ordering::Relaxed) {
        KN_GC_PENDING.fetch_or(1, Ordering::Relaxed);
    }
}
/// Take a page with free slots for this thread (from the pool, or a new one).
#[inline(never)]
fn refill(ph: &mut PHeap, ci: usize) -> usize {
    loop {
        let g = heap_lock();
        let h = heap();
        let pi = match h.pool[ci].pop() {
            Some(pi) => pi,
            None => new_page(h, ci),
        };
        let pg = &mut *h.pages[pi] as *mut Page;
        drop(g);
        // the page is this thread's now: sweep it here, outside the lock and the pause
        let pg = unsafe { &mut *pg };
        pg.ensure_swept();
        pg.young = true;
        if let Some(p) = take_free_slots(ph, ci, pg) {
            return p;
        }
    }
}
/// Rebuild the flat page table after the heap's span changed (heap lock held, or collecting).
fn rebuild_page_table(h: &mut Heap) {
    // up to 4M entries (16 MB) covers a 256 GB span; beyond that use the hash map
    let n = if h.page_hi > h.page_lo { (h.page_hi - h.page_lo) / PAGE } else { 0 };
    if n == 0 || n > (1 << 22) {
        h.page_table = Vec::new();
        return;
    }
    let mut t = vec![0u32; n];
    for (i, pg) in h.pages.iter().enumerate() {
        t[(pg.base - h.page_lo) / PAGE] = i as u32 + 1;
    }
    h.page_table = t;
}

/// Free the dead objects of a page left unswept by a minor collection (the caller claimed it).
fn lazy_sweep(pg: &mut Page) {
    crate::mt::LAZY_SWEEP.with(|l| l.set(true));
    let mut freed = 0usize;
    for wi in 0..pg.alloc.len() {
        let mut bits = pg.alloc[wi];
        while bits != 0 {
            let b = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let p = pg.base + (wi * 64 + b) * pg.slot;
            let hd = unsafe { &*(p as *const Header) };
            if hd.flags & FL_MARK == 0 {
                pg.alloc[wi] &= !(1u64 << b);
                freed += 1;
                if !matches!(hd.kind, Kind::String | Kind::Array | Kind::Hash | Kind::Object | Kind::Env) {
                    // these keep instance variables in the shared table
                    if let Some(t) = crate::core::GENERIC_IVARS.get().as_mut() {
                        t.remove(&p);
                    }
                }
                unsafe { free_obj(p) };
            }
        }
    }
    crate::mt::LAZY_SWEEP.with(|l| l.set(false));
    LAZY_FREED.fetch_add(freed, Ordering::Relaxed);
}
static LAZY_FREED: AtomicUsize = AtomicUsize::new(0);

// ---------- background sweeping ----------
//
// With several Ps, the pages a minor collection leaves unswept are swept by a background
// thread right after it, so the dead objects' buffers (strings, arrays) are freed within
// milliseconds instead of whenever a P next takes the page (which, with many Ps and many
// pages, could be a whole cycle later, or never once the program goes idle). A P that takes
// an unswept page sweeps it itself; the sweeper and the P claim a page through its state.
// A collection pauses the sweeper (BG_LOCK) and gives it a fresh list at the end.

/// Unswept pages for the sweeper (boxed pages: their addresses are stable between
/// collections, and pages are freed only during one).
static BG_LIST: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
static BG_CV: std::sync::Condvar = std::sync::Condvar::new();
/// Held by the sweeper while it sweeps a page, and by the collector for a whole collection.
static BG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static BG_STARTED: AtomicBool = AtomicBool::new(false);
static BG_FREED: AtomicUsize = AtomicUsize::new(0);

fn bg_sweep_pages(pages: Vec<usize>) {
    if pages.is_empty() {
        return;
    }
    if !BG_STARTED.swap(true, Ordering::AcqRel) {
        std::thread::Builder::new().name("kenny-gc-sweep".into()).spawn(bg_sweep_loop).expect("gc sweep thread");
    }
    *BG_LIST.lock().unwrap_or_else(|e| e.into_inner()) = pages;
    BG_CV.notify_one();
}

fn bg_sweep_loop() {
    loop {
        {
            let mut l = BG_LIST.lock().unwrap_or_else(|e| e.into_inner());
            while l.is_empty() {
                l = BG_CV.wait(l).unwrap_or_else(|e| e.into_inner());
            }
        }
        let g = BG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let next = BG_LIST.lock().unwrap_or_else(|e| e.into_inner()).pop();
        if let Some(p) = next {
            let pg = unsafe { &mut *(p as *mut Page) };
            if pg.unswept.compare_exchange(UNSWEPT, SWEEPING, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let before = LAZY_FREED.load(Ordering::Relaxed);
                lazy_sweep(pg);
                pg.unswept.store(SWEPT, Ordering::Release);
                BG_FREED.fetch_add(LAZY_FREED.load(Ordering::Relaxed).saturating_sub(before), Ordering::Relaxed);
            }
        }
        drop(g);
    }
}
/// The collector stops the sweeper (it holds the returned guard for the collection).
fn bg_sweep_pause() -> std::sync::MutexGuard<'static, ()> {
    let g = BG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    BG_LIST.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}
/// This thread's free list for size class `ci` from page `pg`; None if it is full.
fn take_free_slots(ph: &mut PHeap, ci: usize, pg: &mut Page) -> Option<usize> {
    let fl = &mut ph.free[ci];
    // free slots, highest address first (so the lowest is popped first)
    for wi in (0..pg.alloc.len()).rev() {
        let valid = (pg.nslots - wi * 64).min(64);
        let mask = if valid == 64 { u64::MAX } else { (1u64 << valid) - 1 };
        let mut free = !pg.alloc[wi] & mask;
        while free != 0 {
            let b = 63 - free.leading_zeros() as usize;
            free &= !(1u64 << b);
            fl.push(pg.base + (wi * 64 + b) * pg.slot);
        }
    }
    ph.bitmap[ci] = pg.alloc.as_mut_ptr();
    ph.page_base[ci] = pg.base;
    fl.pop()
}

/// Is `w` the address of a live heap object? (For the collector, or with one thread.)
#[inline]
pub fn is_object(w: usize) -> bool {
    if w & 15 != 0 {
        return false;
    }
    let h = heap();
    if w < h.page_lo || w >= h.page_hi {
        return false;
    }
    let base = w & !(PAGE - 1);
    let found = if h.page_table.is_empty() {
        h.page_index.get(&base).copied()
    } else {
        match h.page_table[(base - h.page_lo) / PAGE] {
            0 => None,
            i => Some(i as usize - 1),
        }
    };
    match found {
        Some(pi) => {
            let pg = &h.pages[pi];
            let off = w - base;
            if off % pg.slot != 0 {
                return false;
            }
            let i = off / pg.slot;
            i < pg.nslots && pg.alloc[i / 64] & (1 << (i % 64)) != 0
        }
        None => false,
    }
}
// ---------- malloc pressure ----------
//
// Collections are triggered by the number of heap objects allocated, but objects own
// malloc'd buffers (string bytes, array and hash storage) that are freed only when the GC
// frees the object. Like CRuby's malloc_increase limit, count the bytes malloc'd since the
// last collection and ask for one when they exceed a limit, so garbage with large buffers
// (rendered pages, request strings) doesn't grow the process between collections.

static MALLOC_SINCE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// Net bytes allocated by Rust code (to within a per-thread batch), for KENNY_GC_STATS.
pub static MALLOC_LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static MALLOC_LIMIT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(isize::MAX);

pub struct CountingAlloc;

thread_local! {
    /// Bytes malloc'd minus freed by this thread, not yet added to MALLOC_SINCE (a shared
    /// counter updated on every malloc would make all threads fight over one cache line).
    static MALLOC_LOCAL: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}
const MALLOC_FLUSH: isize = 256 << 10;

impl CountingAlloc {
    /// Net bytes: frees count too, so short-lived Rust buffers don't trigger collections;
    /// only memory that stays allocated (object contents) does.
    #[inline(always)]
    fn note(n: isize) {
        let v = MALLOC_LOCAL.with(|c| {
            let v = c.get() + n;
            c.set(if v.abs() >= MALLOC_FLUSH { 0 } else { v });
            v
        });
        if v.abs() >= MALLOC_FLUSH {
            Self::flush(v);
        }
    }
    #[inline(never)]
    fn flush(v: isize) {
        MALLOC_LIVE.fetch_add(v, Ordering::Relaxed);
        let since = MALLOC_SINCE.fetch_add(v, Ordering::Relaxed) + v;
        if since > MALLOC_LIMIT.load(Ordering::Relaxed) {
            KN_GC_PENDING.fetch_or(1, Ordering::Relaxed);
        }
    }
}

// Which allocator backs Rust allocations, chosen once at the first allocation (before main)
// by KENNY_MALLOC=system|mimalloc. The default is the system allocator on macOS: serving on
// many cores, objects are freed by other threads than the ones that allocated them (the GC
// sweeps on whichever thread takes a page), which macOS's allocator absorbs well while
// mimalloc's per-thread pages keep much more memory resident (the blog on 12 cores: 141 MB vs
// 184 MB, same throughput). Elsewhere mimalloc (faster than glibc malloc, which also tends to
// bloat with many threads).
static WHICH_ALLOC: AtomicU8 = AtomicU8::new(0);
const USE_SYSTEM: u8 = 1;
const USE_MIMALLOC: u8 = 2;
#[cold]
fn choose_alloc() -> u8 {
    // (getenv doesn't allocate)
    let v = unsafe { libc::getenv(c"KENNY_MALLOC".as_ptr()) };
    let w = if v.is_null() {
        if cfg!(target_os = "macos") { USE_SYSTEM } else { USE_MIMALLOC }
    } else {
        match unsafe { std::ffi::CStr::from_ptr(v) }.to_bytes() {
            b"system" => USE_SYSTEM,
            _ => USE_MIMALLOC,
        }
    };
    match WHICH_ALLOC.compare_exchange(0, w, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => w,
        Err(prev) => prev,
    }
}
#[inline(always)]
fn use_system() -> bool {
    let w = WHICH_ALLOC.load(Ordering::Relaxed);
    (if w == 0 { choose_alloc() } else { w }) == USE_SYSTEM
}

unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    #[inline]
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        Self::note(l.size() as isize);
        unsafe { if use_system() { std::alloc::System.alloc(l) } else { mimalloc::MiMalloc.alloc(l) } }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        Self::note(l.size() as isize);
        unsafe { if use_system() { std::alloc::System.alloc_zeroed(l) } else { mimalloc::MiMalloc.alloc_zeroed(l) } }
    }
    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        Self::note(-(l.size() as isize));
        unsafe { if use_system() { std::alloc::System.dealloc(p, l) } else { mimalloc::MiMalloc.dealloc(p, l) } }
    }
    #[inline]
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, new_size: usize) -> *mut u8 {
        Self::note(new_size as isize - l.size() as isize);
        unsafe { if use_system() { std::alloc::System.realloc(p, l, new_size) } else { mimalloc::MiMalloc.realloc(p, l, new_size) } }
    }
}

/// Bytes that may be malloc'd between collections (KENNY_GC_MALLOC_LIMIT, in MB; default 32).
fn malloc_limit() -> isize {
    static M: G<isize> = G::new(0);
    let m = M.get();
    if *m == 0 {
        *m = std::env::var("KENNY_GC_MALLOC_LIMIT").ok().and_then(|v| v.parse::<isize>().ok()).filter(|x| *x > 0).unwrap_or(32) << 20;
    }
    *m
}

pub fn register_env(p: usize, nslots: u32) {
    let ph = &mut crate::core::pst().heap;
    ph.envs.insert(p, nslots);
    count_alloc(ph);
}
pub fn register_class(c: *mut Class) {
    let _g = heap_lock();
    heap().classes.push(c);
    heap().new_classes.push(c);
    heap().class_set.insert(c as usize);
}
// ---------- generational write barrier ----------

pub const REM_SVEC: u8 = 1;
pub const REM_KMAP: u8 = 2;
pub const REM_OBJ: u8 = 3;
/// A class's (or object's) table of values / methods (see CMap::note_write).
pub const REM_CMAP_VALUES: u8 = 4;
pub const REM_CMAP_METHODS: u8 = 5;
static REMEMBERED: std::sync::Mutex<Vec<(u8, usize)>> = std::sync::Mutex::new(Vec::new());

/// An old object's container got new values (see SVec/KMap `wb`): the next minor collection
/// scans it as a root.
pub fn remember(kind: u8, p: usize) {
    match crate::core::pst_opt() {
        Some(ps) => ps.heap.remembered.push((kind, p)),
        None => REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()).push((kind, p)),
    }
}
/// Write barrier for a field stored directly in an object (a Hash's default, generic
/// instance variables): an old object is rescanned by the next minor collection.
#[inline]
pub fn wb_obj(v: V) {
    if is_heap(v) && hdr(v).flags & FL_MARK != 0 {
        remember(REM_OBJ, v as usize);
    }
}
/// C-extension data objects (their mark functions see values the barrier can't): scanned by
/// every minor collection.
static DATA_MARKED: crate::mt::L<Vec<V>> = crate::mt::L::new(Vec::new());
pub fn register_marked_data(v: V) {
    DATA_MARKED.get().push(v);
}
fn generational() -> bool {
    static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("KENNY_GC_GENERATIONAL").map(|v| v != "0").unwrap_or(true))
}
fn verify_mode() -> bool {
    static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var_os("KENNY_GC_VERIFY").is_some())
}

/// Objects that live for the rest of the program (cached literals of compiled code).
static PINNED: crate::mt::L<Vec<V>> = crate::mt::L::new(Vec::new());
pub fn pin(v: V) {
    if is_heap(v) {
        PINNED.get().push(v);
    }
}
/// Scan compiled code's cache regions conservatively too (KENNY_GC_SCAN_REGIONS=1; for
/// checking that every object in them is pinned).
fn scan_regions() -> bool {
    static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var_os("KENNY_GC_SCAN_REGIONS").is_some())
}
/// A data region of compiled code (caches). Objects stored in it are pinned (`pin`), so it is
/// only scanned with KENNY_GC_SCAN_REGIONS=1.
pub fn register_region(ptr: usize, words: usize) {
    let _g = heap_lock();
    heap().regions.push((ptr, words));
}
pub fn unregister_region(ptr: usize) {
    let _g = heap_lock();
    heap().regions.retain(|r| r.0 != ptr);
}
/// Unpin the literals cached in a region (an eval unit's inline caches) before it is freed.
pub fn unpin_region(ptr: usize, words: usize) {
    let vals: FxHashSet<V> = unsafe { std::slice::from_raw_parts(ptr as *const V, words) }.iter().copied().filter(|&v| is_heap(v)).collect();
    if !vals.is_empty() {
        PINNED.get().retain(|v| !vals.contains(v));
    }
}

// ---------- freeing eval'd code ----------
//
// The eval cache (kenny-runtime-jit) evicts units it hasn't used for a while and hands them
// here. A major collection frees an evicted unit once nothing can run its code: no method in
// any class table and no Proc points into it, no stack holds a return address into it, and
// no Method object was made for one of its methods.

struct OrphanUnit {
    id: u64,
    code: Vec<(usize, usize)>,
}
static ORPHANS: crate::mt::L<Vec<OrphanUnit>> = crate::mt::L::new(Vec::new());
static DEAD_UNITS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
/// An evicted eval unit and its code ranges (start, len).
pub fn orphan_code(id: u64, code: Vec<(usize, usize)>) {
    ORPHANS.get().push(OrphanUnit { id, code });
}
/// Units found unreachable since the last call: their memory can be reused.
pub fn take_dead_units() -> Vec<u64> {
    std::mem::take(&mut *DEAD_UNITS.lock().unwrap_or_else(|e| e.into_inner()))
}
/// The registered classes (a copy: other threads may be adding to the list).
pub fn all_classes() -> Vec<*mut Class> {
    let _g = heap_lock();
    heap().classes.clone()
}

/// Returns true if a Thread#raise / kill was delivered (the caller must unwind).
#[inline(always)]
pub fn safepoint() -> bool {
    let f = KN_GC_PENDING.load(Ordering::Relaxed);
    if f == 0 {
        return false;
    }
    safepoint_slow(f)
}
#[inline(never)]
fn safepoint_slow(f: u8) -> bool {
    if f & (1 | STW_BIT) != 0 {
        if crate::mt::multi() {
            stop_the_world_collect();
        } else if f & 1 != 0 {
            collect();
        }
    }
    if pst().pending_is_set() {
        // mid-unwind (e.g. a rescue clause's `===` is running): switching threads or
        // delivering an interrupt now would clobber the throw in flight; do it later
        return false;
    }
    if f & crate::thread::PREEMPT_BIT != 0 {
        KN_GC_PENDING.fetch_and(!crate::thread::PREEMPT_BIT, Ordering::Relaxed);
        crate::thread::preempt();
        if !matches!(pst().pending, Throw::None) {
            return true;
        }
    }
    crate::thread::take_interrupt()
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_safepoint() -> V {
    if safepoint() { UNDEF } else { QNIL }
}

/// GC.start: collect now (with several threads, stopping the others first).
pub fn collect_now() {
    heap().force_major = true;
    if crate::mt::multi() {
        KN_GC_PENDING.fetch_or(1, Ordering::SeqCst);
        stop_the_world_collect();
    } else {
        collect();
    }
}

// ---------- stopping the world (several OS threads) ----------

static COLLECTING: AtomicBool = AtomicBool::new(false);
/// Total time spent waiting for other threads to stop (KENNY_GC_STATS).
static STOP_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The Ps, while the collector holds the ALL_PS lock (only the collector reads it).
static GC_PS: G<Vec<usize>> = G::new(Vec::new());
fn gc_ps() -> &'static Vec<usize> {
    GC_PS.get()
}
fn lock_ps() -> std::sync::MutexGuard<'static, Vec<usize>> {
    let g = ALL_PS.lock().unwrap_or_else(|e| e.into_inner());
    *GC_PS.get() = g.clone();
    g
}
static STW_EPOCH: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
static STW_CV: std::sync::Condvar = std::sync::Condvar::new();

/// At a safepoint with a collection due: become the collector (stop every other thread,
/// collect, restart them) or, if another thread is collecting, stop until it is done.
fn stop_the_world_collect() {
    if COLLECTING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        park();
        return;
    }
    KN_GC_PENDING.fetch_or(STW_BIT, Ordering::SeqCst);
    let ps_guard = lock_ps();
    let t_stop = std::time::Instant::now();
    let me = &crate::core::pst().heap as *const PHeap;
    // wait until every other thread is parked at a safepoint or idle
    let mut spins = 0u32;
    loop {
        let all = gc_ps().iter().all(|&p| {
            let ph = unsafe { &(*(p as *const PState)).heap };
            std::ptr::eq(ph, me) || ph.status.load(Ordering::SeqCst) != RUNNING
        });
        if all {
            break;
        }
        spins += 1;
        if spins < 100 {
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
    STOP_NANOS.fetch_add(t_stop.elapsed().as_nanos() as u64, Ordering::Relaxed);
    if KN_GC_PENDING.load(Ordering::Relaxed) & 1 != 0 {
        GC_TOP.store(approx_sp_here(), Ordering::Relaxed);
        collect_locked();
    }
    drop(ps_guard);
    let mut e = STW_EPOCH.lock().unwrap_or_else(|e| e.into_inner());
    *e += 1;
    KN_GC_PENDING.fetch_and(!STW_BIT, Ordering::SeqCst);
    COLLECTING.store(false, Ordering::Release);
    STW_CV.notify_all();
}

// ---------- parallel stack scanning ----------
//
// Most words on the stacks of stopped threads point nowhere interesting. The stopped threads
// (waiting in `park`) split the stacks into chunks and keep only the words that might matter
// to the marker (heap objects, retired buffers, evicted code, classes in a major
// collection); the collector then marks from those. The filter only reads collector state.

struct ScanFilter {
    page_lo: usize,
    page_hi: usize,
    retired_lo: usize,
    retired_hi: usize,
    code_lo: usize,
    code_hi: usize,
    envs: bool,
    classes: bool,
}
static SCAN_FILTER: G<ScanFilter> = G::new(ScanFilter { page_lo: 0, page_hi: 0, retired_lo: 1, retired_hi: 0, code_lo: 1, code_hi: 0, envs: false, classes: false });
static SCAN_RANGES: G<Vec<(usize, usize)>> = G::new(Vec::new());
static SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);
static SCAN_NEXT: AtomicUsize = AtomicUsize::new(0);
static SCAN_PENDING: AtomicUsize = AtomicUsize::new(0);
static SCAN_OUT: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
const SCAN_CHUNK_WORDS: usize = 4096;

#[inline]
fn scan_candidate(f: &ScanFilter, w: usize) -> bool {
    if w >= f.retired_lo && w <= f.retired_hi {
        return true;
    }
    if w >= f.code_lo && w < f.code_hi {
        return true;
    }
    if w & 7 != 0 || w < 4096 {
        return false;
    }
    if w >= f.page_lo && w < f.page_hi + 16 && (is_object(w) || is_object(w - 16)) {
        return true;
    }
    (f.envs && heap().envs.contains_key(&w)) || (f.classes && heap().class_set.contains(&w))
}

/// Filter stack chunks (a stopped thread helping, or the collector). True if it did any.
fn scan_help() -> bool {
    if !SCAN_ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let ranges = SCAN_RANGES.get();
    let f = SCAN_FILTER.get();
    let mut out = Vec::new();
    let mut worked = false;
    loop {
        let i = SCAN_NEXT.fetch_add(1, Ordering::AcqRel);
        if i >= ranges.len() {
            break;
        }
        worked = true;
        let (lo, hi) = ranges[i];
        let mut p = lo;
        while p + 8 <= hi {
            let w = unsafe { *(p as *const usize) };
            if scan_candidate(f, w) {
                out.push(w);
            }
            p += 8;
        }
        // hand in the chunk's words before counting it done (the collector takes them all
        // once nothing is pending)
        if !out.is_empty() {
            SCAN_OUT.lock().unwrap_or_else(|e| e.into_inner()).append(&mut out);
        }
        SCAN_PENDING.fetch_sub(1, Ordering::AcqRel);
    }
    worked
}

/// Scan these stack ranges with the stopped threads' help, marking what they find.
fn scan_ranges_parallel(m: &mut Marker, ranges: &[(usize, usize)]) {
    let h = heap();
    *SCAN_FILTER.get() = ScanFilter {
        page_lo: h.page_lo,
        page_hi: h.page_hi,
        retired_lo: if m.retired.is_empty() { 1 } else { m.retired_lo },
        retired_hi: if m.retired.is_empty() { 0 } else { m.retired_hi },
        code_lo: m.code_lo,
        code_hi: m.code_hi,
        envs: !h.envs.is_empty(),
        classes: m.major,
    };
    let chunks = SCAN_RANGES.get();
    chunks.clear();
    for &(lo, hi) in ranges {
        if hi > lo && hi - lo > (1usize << 31) {
            eprintln!("[gc] bad stack range {lo:#x}..{hi:#x}");
            std::process::abort();
        }
        let mut p = (lo + 7) & !7;
        while p < hi {
            let e = (p + SCAN_CHUNK_WORDS * 8).min(hi);
            chunks.push((p, e));
            p = e;
        }
        SCANNED_WORDS.fetch_add(hi.saturating_sub(lo) / 8, Ordering::Relaxed);
    }
    SCAN_NEXT.store(0, Ordering::Relaxed);
    SCAN_PENDING.store(chunks.len(), Ordering::SeqCst);
    SCAN_ACTIVE.store(true, Ordering::SeqCst);
    STW_CV.notify_all();
    scan_help();
    let mut spins = 0u32;
    while SCAN_PENDING.load(Ordering::Acquire) != 0 {
        spins += 1;
        if spins < 1000 {
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
    SCAN_ACTIVE.store(false, Ordering::SeqCst);
    let found = std::mem::take(&mut *SCAN_OUT.lock().unwrap_or_else(|e| e.into_inner()));
    for w in found {
        m.word(w);
    }
}

// ---------- parallel sweep ----------

struct SweepJob {
    pages: *mut Box<Page>,
    /// indices of the pages to sweep
    idx: *const usize,
    n: usize,
}
static SWEEP_JOB: G<SweepJob> = G::new(SweepJob { pages: std::ptr::null_mut(), idx: std::ptr::null(), n: 0 });
static SWEEP_ACTIVE: AtomicBool = AtomicBool::new(false);
static SWEEP_NEXT: AtomicUsize = AtomicUsize::new(0);
static SWEEP_PENDING: AtomicUsize = AtomicUsize::new(0);
static SWEEP_LIVE: AtomicUsize = AtomicUsize::new(0);
static SWEEP_FREED: AtomicUsize = AtomicUsize::new(0);
static SWEEP_DEFERRED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
/// Chunks swept by threads other than the collector (KENNY_GC_STATS=2).
static SWEEP_HELPED: AtomicUsize = AtomicUsize::new(0);
static SWEEP_COLLECTOR: AtomicUsize = AtomicUsize::new(0);
const SWEEP_CHUNK: usize = 8;

/// Sweep chunks of pages until none are left (the collector, and stopped threads that are
/// waiting for it anyway). Each page's bitmap and objects belong to one sweeper.
/// Returns whether it swept anything.
fn sweep_help() -> bool {
    if !SWEEP_ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let mut worked = false;
    let job = SWEEP_JOB.get();
    let (pages, idx, n) = (job.pages, job.idx, job.n);
    let gen_tbl = crate::core::GENERIC_IVARS.raw().as_ref().filter(|t| !t.is_empty());
    let mut live = 0usize;
    let mut freed = 0usize;
    let mut deferred = Vec::new();
    loop {
        let c = SWEEP_NEXT.fetch_add(1, Ordering::AcqRel);
        let start = c * SWEEP_CHUNK;
        if start >= n {
            break;
        }
        worked = true;
        for k in start..(start + SWEEP_CHUNK).min(n) {
            let pg = unsafe { &mut **pages.add(*idx.add(k)) };
            pg.young = false;
            pg.unswept.store(SWEPT, Ordering::Relaxed);
            for (wi, word) in pg.alloc.iter_mut().enumerate() {
                let mut bits = *word;
                while bits != 0 {
                    let b = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let p = pg.base + (wi * 64 + b) * pg.slot;
                    let hd = unsafe { &mut *(p as *mut Header) };
                    if hd.flags & FL_MARK != 0 {
                        // (the mark stays: survivors are old until the next major collection)
                        live += 1;
                    } else {
                        *word &= !(1u64 << b);
                        freed += 1;
                        match hd.kind {
                            // buffers only: safe to free on any thread
                            Kind::String | Kind::Array | Kind::Hash | Kind::Object | Kind::Env => unsafe { free_obj(p) },
                            // these keep instance variables in the shared table (read-only
                            // during the sweep); with an entry, the collector removes it
                            Kind::Float | Kind::Bignum | Kind::Range | Kind::Proc if !gen_tbl.is_some_and(|t| t.contains_key(&p)) => unsafe { free_obj(p) },
                            _ => deferred.push(p),
                        }
                    }
                }
            }
        }
        // results first, then the chunk counts as done (the collector reads them after)
        SWEEP_LIVE.fetch_add(std::mem::take(&mut live), Ordering::Relaxed);
        SWEEP_FREED.fetch_add(std::mem::take(&mut freed), Ordering::Relaxed);
        if !deferred.is_empty() {
            SWEEP_DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).append(&mut deferred);
        }
        if crate::core::pst() as *mut PState as usize != SWEEP_COLLECTOR.load(Ordering::Relaxed) {
            SWEEP_HELPED.fetch_add(1, Ordering::Relaxed);
        }
        SWEEP_PENDING.fetch_sub(1, Ordering::AcqRel);
    }
    worked
}

/// Stop this thread until the running collection is over.
#[inline(never)]
fn park() {
    let ph = &mut crate::core::pst().heap;
    spill_registers(&mut ph.regs);
    ph.saved_sp = approx_sp_here();
    ph.status.store(PARKED, Ordering::SeqCst);
    {
        let mut e = STW_EPOCH.lock().unwrap_or_else(|e| e.into_inner());
        while KN_GC_PENDING.load(Ordering::SeqCst) & STW_BIT != 0 || COLLECTING.load(Ordering::SeqCst) {
            if SCAN_ACTIVE.load(Ordering::Acquire) {
                drop(e);
                let worked = scan_help();
                e = STW_EPOCH.lock().unwrap_or_else(|e| e.into_inner());
                if worked {
                    continue;
                }
            }
            if SWEEP_ACTIVE.load(Ordering::Acquire) {
                drop(e);
                let worked = sweep_help();
                e = STW_EPOCH.lock().unwrap_or_else(|e| e.into_inner());
                if worked {
                    continue;
                }
            }
            let (g, _) = STW_CV.wait_timeout(e, std::time::Duration::from_millis(1)).unwrap_or_else(|e| e.into_inner());
            e = g;
        }
    }
    ph.status.store(RUNNING, Ordering::SeqCst);
}

/// Called by a thread spinning on a runtime lock: stop here if a collection is waiting.
pub fn park_if_stopping() {
    // (the collector itself never spins on a runtime lock while it waits or collects)
    if crate::mt::multi() && KN_GC_PENDING.load(Ordering::SeqCst) & STW_BIT != 0 {
        park();
    }
}

/// Run `f` (which must not touch Ruby objects) with this thread counted as stopped, so a
/// collection can run meanwhile: blocking waits (IO, sleep, an idle scheduler).
#[inline(never)]
pub fn blocking<R>(f: impl FnOnce() -> R) -> R {
    if !crate::mt::multi() {
        return f();
    }
    crate::thread::before_blocking();
    let ph = &mut crate::core::pst().heap;
    spill_registers(&mut ph.regs);
    ph.saved_sp = approx_sp_here();
    ph.status.store(IDLE, Ordering::SeqCst);
    let r = f();
    let ph = &mut crate::core::pst().heap;
    ph.status.store(RUNNING, Ordering::SeqCst);
    if KN_GC_PENDING.load(Ordering::SeqCst) & STW_BIT != 0 {
        park();
    }
    r
}

#[inline(always)]
fn approx_sp_here() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

/// Save the callee-saved registers (they may hold the only reference to an object).
#[inline(always)]
fn spill_registers(regs: &mut [usize; 16]) {
    unsafe {
        #[cfg(target_arch = "aarch64")]
        std::arch::asm!(
            "stp x19, x20, [{0}, #0]",
            "stp x21, x22, [{0}, #16]",
            "stp x23, x24, [{0}, #32]",
            "stp x25, x26, [{0}, #48]",
            "stp x27, x28, [{0}, #64]",
            "stp x29, x30, [{0}, #80]",
            in(reg) regs.as_mut_ptr(),
            options(nostack)
        );
        #[cfg(target_arch = "x86_64")]
        std::arch::asm!(
            "mov [{0}], rbx",
            "mov [{0} + 8], rbp",
            "mov [{0} + 16], r12",
            "mov [{0} + 24], r13",
            "mov [{0} + 32], r14",
            "mov [{0} + 40], r15",
            in(reg) regs.as_mut_ptr(),
            options(nostack)
        );
    }
    std::hint::black_box(&regs);
}

// ---------- rooted temporary vectors ----------

/// A `Vec<V>` that the GC treats as a root while it lives. It is registered with the P that
/// created it; it may be dropped elsewhere (a native proc's closure freed by a collection
/// running on another P), and then unregisters from its owner.
pub struct RVec(Box<Vec<V>>, *mut PHeap);

/// RVecs dropped on a P other than their owner outside a collection: (owner, root), removed
/// from the owner's list at the start of the next collection.
static ORPHAN_ROOTS: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

impl RVec {
    pub fn new() -> Self {
        Self::from(Vec::new())
    }
    pub fn with_capacity(n: usize) -> Self {
        Self::from(Vec::with_capacity(n))
    }
    pub fn from(v: Vec<V>) -> Self {
        let b = Box::new(v);
        let ph = &mut crate::core::pst().heap;
        ph.roots.push(&*b as *const Vec<V>);
        RVec(b, ph as *mut PHeap)
    }
    pub fn take(&mut self) -> Vec<V> {
        std::mem::take(&mut *self.0)
    }
}
impl From<RVec> for Vec<V> {
    fn from(mut r: RVec) -> Vec<V> {
        r.take()
    }
}
impl Default for RVec {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for RVec {
    fn drop(&mut self) {
        let p = &*self.0 as *const Vec<V>;
        let me = crate::core::pst_opt().map(|ps| &mut ps.heap as *mut PHeap).unwrap_or(std::ptr::null_mut());
        if self.1 == me && !IN_COLLECT.load(Ordering::Relaxed) {
            unregister_root(self.1, p);
        } else {
            // another thread's root, or dropped by a (possibly parallel) sweep: the next
            // collection unregisters and frees it
            ORPHAN_ROOTS.lock().unwrap_or_else(|e| e.into_inner()).push((self.1 as usize, p as usize));
            // the Vec itself stays allocated until the owner forgets it
            let v = std::mem::take(&mut *self.0);
            let b = std::mem::replace(&mut self.0, Box::new(Vec::new()));
            drop(v);
            std::mem::forget(b);
        }
    }
}
fn unregister_root(owner: *mut PHeap, p: *const Vec<V>) {
    let roots = unsafe { &mut (*owner).roots };
    if let Some(i) = roots.iter().rposition(|r| *r == p) {
        roots.swap_remove(i);
    }
}
/// Set while a collection runs (every other P stopped).
static IN_COLLECT: AtomicBool = AtomicBool::new(false);
impl std::ops::Deref for RVec {
    type Target = Vec<V>;
    fn deref(&self) -> &Vec<V> {
        &self.0
    }
}
impl std::ops::DerefMut for RVec {
    fn deref_mut(&mut self) -> &mut Vec<V> {
        &mut self.0
    }
}

/// A copy of values (an Array's elements, a Hash's pairs) that stays a GC root while it, or
/// an iterator over it, lives: `for x in snapshot(a) { call_block(..) }` must not lose `x`
/// when the block removes it from the original.
pub struct Rooted<T> {
    root: RVec,
    items: Vec<T>,
}
impl<T> Rooted<T> {
    pub fn new(items: Vec<T>, values: Vec<V>) -> Self {
        Rooted { root: RVec::from(values), items }
    }
    pub fn into_vec(mut self) -> Vec<T> {
        std::mem::take(&mut self.items)
    }
}
impl Rooted<V> {
    pub fn of(items: Vec<V>) -> Self {
        let r = RVec::from(items.clone());
        Rooted { root: r, items }
    }
}
impl<T> std::ops::Deref for Rooted<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        &self.items
    }
}
impl<T> std::ops::DerefMut for Rooted<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.items
    }
}
pub struct RootedIter<T> {
    _root: RVec,
    it: std::vec::IntoIter<T>,
}
impl<T> Iterator for RootedIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.it.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.it.size_hint()
    }
}
impl<T> DoubleEndedIterator for RootedIter<T> {
    fn next_back(&mut self) -> Option<T> {
        self.it.next_back()
    }
}
impl<T> ExactSizeIterator for RootedIter<T> {}
impl<T> IntoIterator for Rooted<T> {
    type Item = T;
    type IntoIter = RootedIter<T>;
    fn into_iter(mut self) -> RootedIter<T> {
        let items = std::mem::take(&mut self.items);
        let root = std::mem::take(&mut self.root);
        RootedIter { _root: root, it: items.into_iter() }
    }
}
impl<'a, T> IntoIterator for &'a Rooted<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}
impl<T> From<Rooted<T>> for Vec<T> {
    fn from(r: Rooted<T>) -> Vec<T> {
        r.into_vec()
    }
}

/// A thread's throws in flight while its `ensure` clauses run, by handle.
#[derive(Default)]
pub struct SavedThrows {
    pub map: FxHashMap<u64, Throw>,
    next: u64,
}

pub fn save_throw(t: Throw) -> u64 {
    let st = &mut crate::core::pst().heap.saved_throws;
    st.next += 1;
    st.map.insert(st.next, t);
    st.next
}
pub fn take_throw(id: u64) -> Throw {
    crate::core::pst().heap.saved_throws.map.remove(&id).unwrap_or(Throw::None)
}

/// The values a throw carries (GC roots).
pub fn throw_values(t: Throw, out: &mut Vec<V>) {
    match t {
        Throw::Raise(e) => out.push(e),
        Throw::Break { val, .. } | Throw::Return { val, .. } | Throw::StopLoop(val) => out.push(val),
        Throw::Throw { tag, val } => out.extend([tag, val]),
        Throw::None | Throw::Kill => {}
    }
}

// ---------- marking ----------

struct Marker {
    stack: Vec<V>,
    envs: Vec<usize>,
    marked_envs: FxHashSet<usize>,
    seen_classes: FxHashSet<usize>,
    /// Retired buffers (start, bytes, align), sorted; `keep[i]` once a stack word points into one.
    retired: Vec<(usize, usize, usize)>,
    keep: Vec<bool>,
    retired_lo: usize,
    retired_hi: usize,
    /// retired[buckets[b]..] start at or after retired_lo + (b << retired_shift): narrows the
    /// search for the buffer a stack word points into to one or two buckets
    retired_buckets: Vec<u32>,
    retired_shift: u32,
    /// KENNY_GC_VERIFY: a full traversal (by its own visited set) that reports reachable
    /// objects the minor collection left unmarked.
    verify: Option<FxHashSet<usize>>,
    verify_fail: usize,
    /// (verifier) the object being traced, and who reached each object first
    cur: usize,
    parents: FxHashMap<usize, usize>,
    /// Objects marked by this collection (promoted to old by a minor one).
    newly_marked: usize,
    /// a minor collection (skips classes traced before)
    minor: bool,
    /// a major collection: stack words are also checked against the class set (a singleton
    /// class referenced only from a stack must not be freed)
    major: bool,
    /// (major collections with evicted eval units) their code ranges (start, end, unit index),
    /// sorted, and which units something still points into
    code: Vec<(usize, usize, usize)>,
    code_lo: usize,
    code_hi: usize,
    code_hit: Vec<bool>,
}

impl Marker {
    fn new(retired: Vec<(usize, usize, usize)>, keep: Vec<bool>, retired_lo: usize, retired_hi: usize) -> Self {
        const NB: usize = 1 << 14;
        let span = retired_hi.saturating_sub(retired_lo) + 1;
        let retired_shift = (usize::BITS - (span / NB).leading_zeros()).min(63);
        let mut retired_buckets = Vec::new();
        if !retired.is_empty() {
            let nb = (span >> retired_shift) + 2;
            retired_buckets = vec![0u32; nb];
            let mut i = 0usize;
            for (b, slot) in retired_buckets.iter_mut().enumerate() {
                let start = retired_lo.saturating_add(b << retired_shift);
                while i < retired.len() && retired[i].0 < start {
                    i += 1;
                }
                *slot = i as u32;
            }
        }
        Marker { stack: Vec::new(), envs: Vec::new(), marked_envs: FxHashSet::default(), seen_classes: FxHashSet::default(), retired, keep, retired_lo, retired_hi, retired_buckets, retired_shift, verify: None, verify_fail: 0, newly_marked: 0, cur: 0, parents: FxHashMap::default(), minor: false, major: false, code: Vec::new(), code_lo: usize::MAX, code_hi: 0, code_hit: Vec::new() }
    }
    #[inline]
    fn value(&mut self, v: V) {
        if !is_heap(v) {
            return;
        }
        if is_object(v as usize) {
            if let Some(seen) = &mut self.verify {
                if seen.insert(v as usize) {
                    self.parents.insert(v as usize, self.cur);
                    if hdr(v).flags & FL_MARK == 0 {
                        self.verify_fail += 1;
                        if self.verify_fail <= 5 {
                            eprintln!("[gc verify] reachable object {:#x} ({:?}, class {}) was not marked by the minor collection; reached via:", v, hdr(v).kind, class_name(hdr(v).klass));
                            let mut p = self.cur;
                            let mut n = 0;
                            while p >= 4096 && n < 8 {
                                let marked = hdr(p as V).flags & FL_MARK != 0;
                                let extra = match hdr(p as V).kind {
                                    Kind::Array => format!(" svec_flags={:#x}", rary(p as V).v.capacity()),
                                    _ => String::new(),
                                };
                                eprintln!("    {:#x} {:?} {} marked={}{}", p, hdr(p as V).kind, class_name(hdr(p as V).klass), marked, extra);
                                p = self.parents.get(&p).copied().unwrap_or(0);
                                n += 1;
                            }
                            eprintln!("    root kind {}", p);
                        }
                    }
                    self.stack.push(v);
                }
                return;
            }
            let hd = hdr(v);
            if hd.flags & FL_MARK == 0 {
                hd.flags |= FL_MARK;
                self.newly_marked += 1;
                self.stack.push(v);
            }
        } else if is_kind_unchecked_class(v) {
            self.class(v as *mut Class);
        }
    }
    /// A word that might be a pointer (stack / data section).
    #[inline]
    fn word(&mut self, w: usize) {
        if !self.retired.is_empty() {
            self.retired_word(w);
        }
        if w >= self.code_lo && w < self.code_hi {
            self.code_word(w);
        }
        if w & 7 != 0 || w < 4096 {
            return;
        }
        if is_object(w) {
            self.value(w as V);
        } else if is_object(w - 16) {
            // the data of a GC-heap environment (what compiled code holds)
            if hdr((w - 16) as V).kind == Kind::Env {
                self.value((w - 16) as V);
            }
        } else if !heap().envs.is_empty() && heap().envs.contains_key(&w) {
            self.env(w);
        } else if self.major && heap().class_set.contains(&w) {
            self.class(w as *mut Class);
        }
    }
    /// A stack word pointing into (or just past) a retired buffer keeps it: a Rust frame may
    /// still be reading it.
    #[inline(never)]
    fn retired_word(&mut self, w: usize) {
        if w < self.retired_lo || w > self.retired_hi {
            return;
        }
        // the last buffer starting at or before w starts in w's bucket or an earlier one
        let b = (w - self.retired_lo) >> self.retired_shift;
        let lo = (self.retired_buckets[b] as usize).saturating_sub(1);
        let hi = (self.retired_buckets[b + 1] as usize).max(lo);
        let i = lo + self.retired[lo..hi].partition_point(|r| r.0 <= w);
        if i > 0 {
            let (start, bytes, _) = self.retired[i - 1];
            if w <= start + bytes {
                self.keep[i - 1] = true;
            }
        }
    }
    /// A word pointing into an evicted eval unit's code (a return address, a function pointer).
    #[inline(never)]
    fn code_word(&mut self, w: usize) {
        let i = self.code.partition_point(|r| r.0 <= w);
        if i > 0 {
            let (_, hi, u) = self.code[i - 1];
            if w < hi {
                self.code_hit[u] = true;
            }
        }
    }
    fn env(&mut self, p: usize) {
        if p == 0 {
            return;
        }
        if p >= 4096 + 16 && is_object(p - 16) {
            self.value((p - 16) as V);
            return;
        }
        if !heap().envs.contains_key(&p) {
            return;
        }
        if self.marked_envs.insert(p) {
            self.envs.push(p);
        }
    }
    fn class(&mut self, c: *mut Class) {
        if c.is_null() || !self.seen_classes.insert(c as usize) {
            return;
        }
        let k = unsafe { &mut *c };
        if self.verify.is_none() {
            if self.minor && k.h.flags & FL_CLASS_TRACED != 0 {
                return;
            }
            k.h.flags |= FL_CLASS_TRACED;
        }
        for &v in k.consts.values() {
            self.value(v);
        }
        for &v in k.civars.values() {
            self.value(v);
        }
        for &v in k.cvars.values() {
            self.value(v);
        }
        for &m in k.methods.values() {
            if let Body::Proc(p) = unsafe { &(*m).body } {
                self.value(*p);
            }
        }
        if k.is_singleton {
            self.value(k.attached);
        }
        if !k.h.klass.is_null() {
            self.class(k.h.klass);
        }
        self.class(k.superclass);
        for &m in &k.includes {
            self.class(m);
        }
        self.class(k.lexical_parent);
    }
    fn drain(&mut self) {
        loop {
            if let Some(v) = self.stack.pop() {
                self.cur = v as usize;
                self.trace(v);
                self.cur = 0;
                continue;
            }
            if let Some(e) = self.envs.pop() {
                self.cur = 2; // an environment
                let n = heap().envs[&e] as usize;
                let words = unsafe { std::slice::from_raw_parts(e as *const usize, 2 + n) };
                self.env(words[0]);
                for &w in &words[1..] {
                    self.value(w as V);
                }
                continue;
            }
            break;
        }
    }
    fn trace(&mut self, v: V) {
        if hdr(v).kind == Kind::Env {
            // [header (klass = slot count)][parent, blk, slots...]
            let n = hdr(v).klass as usize;
            let words = unsafe { std::slice::from_raw_parts((v + 16) as *const usize, 2 + n) };
            self.env(words[0]);
            for &w in &words[1..] {
                self.value(w as V);
            }
            return;
        }
        let k = hdr(v).klass;
        if !k.is_null() && unsafe { (*k).is_singleton } {
            self.class(k);
        }
        let promote = self.verify.is_none();
        match hdr(v).kind {
            Kind::Object => {
                if promote {
                    obj(v).ivars.set_old();
                }
                for &x in &obj(v).ivars {
                    self.value(x);
                }
            }
            Kind::Array => {
                if promote {
                    rary(v).v.set_old();
                }
                for &x in &rary(v).v {
                    self.value(x);
                }
            }
            Kind::Hash => {
                let h = rhash(v);
                if promote {
                    h.m.set_old();
                }
                for (k, x) in h.m.iter() {
                    self.value(k.0);
                    self.value(*x);
                }
                self.value(h.default);
                self.value(h.default_proc);
            }
            Kind::Range => {
                let r = rrange(v);
                self.value(r.lo);
                self.value(r.hi);
            }
            Kind::Proc => {
                let p = rproc(v);
                let f = p.func as usize;
                if f >= self.code_lo && f < self.code_hi {
                    self.code_word(f);
                }
                self.env(p.env as usize);
                self.value(p.self_);
                for &x in &p.roots {
                    self.value(x);
                }
            }
            Kind::Data => {
                let d = unsafe { &*(v as *const RData) };
                for &x in &d.roots {
                    self.value(x);
                }
                if let Some(mark) = d.dmark {
                    CURRENT_MARKER.get().replace(self as *mut Marker);
                    unsafe { mark(d.data) };
                    CURRENT_MARKER.get().take();
                }
            }
            Kind::String | Kind::Float | Kind::Bignum | Kind::Class | Kind::Env => {}
        }
        let mut vals = Vec::new();
        gen_ivar_values(v, &mut vals);
        for x in vals {
            self.value(x);
        }
    }
}

static CURRENT_MARKER: G<Option<*mut Marker>> = G::new(None);

/// Used by C extensions' mark functions (rb_gc_mark).
pub fn mark_from_extension(v: V) {
    if let Some(m) = *CURRENT_MARKER.get() {
        unsafe { (*m).value(v) };
    }
}

fn is_kind_unchecked_class(v: V) -> bool {
    // classes are not in the object set; only trust pointers we know are classes
    heap().class_set.contains(&(v as usize))
}

#[inline(never)]
fn scan_with_registers(m: &mut Marker) {
    let mut regs = [0usize; 32];
    unsafe {
        #[cfg(target_arch = "aarch64")]
        std::arch::asm!(
            "stp x19, x20, [{0}, #0]",
            "stp x21, x22, [{0}, #16]",
            "stp x23, x24, [{0}, #32]",
            "stp x25, x26, [{0}, #48]",
            "stp x27, x28, [{0}, #64]",
            "stp x29, x30, [{0}, #80]",
            in(reg) regs.as_mut_ptr(),
            options(nostack)
        );
        #[cfg(target_arch = "x86_64")]
        std::arch::asm!(
            "mov [{0}], rbx",
            "mov [{0} + 8], rbp",
            "mov [{0} + 16], r12",
            "mov [{0} + 24], r13",
            "mov [{0} + 32], r14",
            "mov [{0} + 40], r15",
            in(reg) regs.as_mut_ptr(),
            options(nostack)
        );
    }
    let regs = std::hint::black_box(regs);
    let mut lo = &regs as *const _ as usize;
    if m.verify.is_some() {
        // the verifier skips the collector's own GC frames (and registers): stale words
        // there differ from what the collection itself scanned
        lo = GC_TOP.load(Ordering::Relaxed);
    } else {
        for w in regs {
            m.word(w);
        }
    }
    let hi = crate::core::pst().heap.stack_base;
    m.cur = 20;
    if hi > lo {
        scan_range(m, lo, hi);
    }
    // the other threads (stopped): their registers and the stacks they are running on
    let me = &crate::core::pst().heap as *const PHeap;
    // (what is scanned is kept for the verifier: a thread that was idle may stop again
    // further down its stack while the collection runs)
    let snap = OTHER_STACKS.get();
    if m.verify.is_none() {
        snap.clear();
        for &p in gc_ps().iter() {
            let ph = unsafe { &(*(p as *const PState)).heap };
            if std::ptr::eq(ph, me) {
                continue;
            }
            // with KENNY_GC_VERIFY, a copy of the words (an idle thread may run a little)
            let copy = if verify_mode() && ph.stack_base > ph.saved_sp && ph.saved_sp != 0 {
                unsafe { std::slice::from_raw_parts(ph.saved_sp as *const usize, (ph.stack_base - ph.saved_sp) / 8) }.to_vec()
            } else {
                Vec::new()
            };
            snap.push((ph.regs, ph.saved_sp, ph.stack_base, copy));
        }
    }
    // with the stopped threads' help when there is enough to scan (not while verifying)
    let parallel = m.verify.is_none() && crate::mt::multi() && gc_ps().len() > 1;
    let mut ranges = Vec::new();
    for (i, (regs, sp, base, copy)) in snap.iter().enumerate() {
        m.cur = 30 + i;
        for &w in regs.iter() {
            m.word(w);
        }
        if m.verify.is_some() && !copy.is_empty() {
            for &w in copy.iter() {
                m.word(w);
            }
        } else if *base > *sp && *sp != 0 {
            if parallel { ranges.push((*sp, *base)) } else { scan_range(m, *sp, *base) }
        }
    }
    // stacks of suspended threads and fibers
    m.cur = 50;
    if let Some(p) = heap().stack_provider {
        for (lo, hi) in p() {
            if parallel { ranges.push((lo, hi)) } else { scan_range(m, lo, hi) }
        }
    }
    let words: usize = ranges.iter().map(|r| r.1.saturating_sub(r.0) / 8).sum();
    if words > 2 * SCAN_CHUNK_WORDS {
        scan_ranges_parallel(m, &ranges);
    } else {
        for (lo, hi) in ranges {
            scan_range(m, lo, hi);
        }
    }
    m.cur = 0;
}

static SCANNED_WORDS: AtomicUsize = AtomicUsize::new(0);
fn scan_range(m: &mut Marker, lo: usize, hi: usize) {
    SCANNED_WORDS.fetch_add(hi.saturating_sub(lo) / 8, Ordering::Relaxed);
    // a stack is never this large: a corrupt bound (scanning it would hang or crash later)
    if hi > lo && hi - lo > (1usize << 31) {
        eprintln!("[gc] bad stack range {lo:#x}..{hi:#x}");
        std::process::abort();
    }
    let lo = (lo + 7) & !7;
    let mut p = lo;
    while p + 8 <= hi {
        let w = unsafe { *(p as *const usize) };
        m.word(w);
        p += 8;
    }
}

pub fn collect() {
    let _g = lock_ps();
    GC_TOP.store(approx_sp_here(), Ordering::Relaxed);
    collect_locked();
}
/// Collect, with the ALL_PS lock held (and every other P stopped).
#[inline(never)]
fn collect_locked() {
    let h = heap();
    if h.disabled > 0 {
        return;
    }
    IN_COLLECT.store(true, Ordering::Relaxed);
    let bg_guard = bg_sweep_pause();
    // roots dropped on another P since the last collection: forget them (and free them)
    for (owner, p) in std::mem::take(&mut *ORPHAN_ROOTS.lock().unwrap_or_else(|e| e.into_inner())) {
        unregister_root(owner as *mut PHeap, p as *const Vec<V>);
        drop(unsafe { Box::from_raw(p as *mut Vec<V>) });
    }
    KN_GC_PENDING.fetch_and(!1, Ordering::Relaxed);
    let t0 = std::time::Instant::now();
    // every thread's environments created since the last collection join the shared list
    // (they are the young ones)
    let mut young_envs: FxHashSet<usize> = FxHashSet::default();
    for &p in gc_ps().iter() {
        let ph = unsafe { &mut (*(p as *mut PState)).heap };
        if !ph.envs.is_empty() {
            young_envs.extend(ph.envs.keys().copied());
            h.envs.extend(ph.envs.drain());
        }
    }
    // generational: a minor collection unless the old generation has doubled since the last
    // major one (or a major one is due for another reason)
    let n_young_envs = young_envs.len();
    let major = !generational()
        || h.force_major
        || h.minors >= 100
        || h.live > h.major_live.saturating_mul(2).max(min_threshold());
    h.force_major = false;
    if major {
        clear_marks(h);
    }
    // the write barrier's entries since the last collection
    let mut remembered = std::mem::take(&mut *REMEMBERED.lock().unwrap_or_else(|e| e.into_inner()));
    for &p in gc_ps().iter() {
        let ph = unsafe { &mut (*(p as *mut PState)).heap };
        remembered.append(&mut ph.remembered);
    }
    let mut retired = crate::mt::take_retired();
    for &p in gc_ps().iter() {
        let ph = unsafe { &mut (*(p as *mut PState)).heap };
        retired.append(&mut ph.retired);
    }
    retired.sort_unstable();
    let n_retired = (retired.len(), retired.iter().map(|r| r.1).sum::<usize>() >> 10);
    let keep = vec![false; retired.len()];
    let (retired_lo, retired_hi) = match (retired.first(), retired.iter().map(|r| r.0 + r.1).max()) {
        (Some(f), Some(h)) => (f.0, h),
        _ => (usize::MAX, 0),
    };
    let mut m = Marker::new(retired, keep, retired_lo, retired_hi);

    let mut lap = std::time::Instant::now();
    let mut phases: Vec<(&str, f64)> = Vec::new();
    let mut phase = |name: &'static str, lap: &mut std::time::Instant| {
        phases.push((name, lap.elapsed().as_secs_f64() * 1e3));
        *lap = std::time::Instant::now();
    };
    m.minor = !major;
    m.major = major;
    let orphans = if major { std::mem::take(&mut *ORPHANS.raw()) } else { Vec::new() };
    if !orphans.is_empty() {
        for (u, o) in orphans.iter().enumerate() {
            for &(p, len) in &o.code {
                m.code.push((p, p + len, u));
            }
        }
        m.code.sort_unstable();
        m.code_lo = m.code.first().map(|r| r.0).unwrap_or(usize::MAX);
        m.code_hi = m.code.iter().map(|r| r.1).max().unwrap_or(0);
        m.code_hit = vec![false; orphans.len()];
    }
    mark_roots(&mut m, h, major);
    h.new_classes.clear();
    phase("roots", &mut lap);
    let region_words: usize = h.regions.iter().map(|r| r.1).sum();
    if !major {
        // old objects are not traced again: what they reference was marked when they were,
        // except what the write barrier recorded since, old closure environments (compiled
        // code writes them directly) and C-extension data (their mark functions)
        for &(kind, p) in &remembered {
            match kind {
                REM_SVEC => {
                    let sv = unsafe { &*(p as *const SVec<V>) };
                    sv.clear_remembered();
                    for &x in sv.as_slice() {
                        m.value(x);
                    }
                }
                REM_KMAP => {
                    let km = unsafe { &*(p as *const crate::khash::KMap) };
                    km.clear_remembered();
                    for (k, x) in km.iter() {
                        m.value(k.0);
                        m.value(*x);
                    }
                }
                REM_CMAP_VALUES => {
                    let cm = unsafe { &*(p as *const CMap<u32, V>) };
                    cm.clear_remembered();
                    for &x in cm.values() {
                        m.value(x);
                    }
                }
                REM_CMAP_METHODS => {
                    let cm = unsafe { &*(p as *const CMap<u32, *const Method>) };
                    cm.clear_remembered();
                    for &meth in cm.values() {
                        if let Body::Proc(pr) = unsafe { &(*meth).body } {
                            m.value(*pr);
                        }
                    }
                }
                _ => m.trace(p as V),
            }
        }
        phase("remembered", &mut lap);
        let old_envs: Vec<usize> = h.envs.keys().copied().filter(|e| !young_envs.contains(e)).collect();
        for e in old_envs {
            m.env(e);
        }
        phase("old_envs", &mut lap);
        DBG_NDATA.store(DATA_MARKED.raw().len(), std::sync::atomic::Ordering::Relaxed);
        for &d in DATA_MARKED.raw().iter() {
            m.trace(d);
        }
    } else {
        for &(kind, p) in &remembered {
            match kind {
                REM_SVEC => unsafe { &*(p as *const SVec<V>) }.clear_remembered(),
                REM_KMAP => unsafe { &*(p as *const crate::khash::KMap) }.clear_remembered(),
                REM_CMAP_VALUES => unsafe { &*(p as *const CMap<u32, V>) }.clear_remembered(),
                REM_CMAP_METHODS => unsafe { &*(p as *const CMap<u32, *const Method>) }.clear_remembered(),
                _ => {}
            }
        }
    }
    phase("old", &mut lap);
    scan_with_registers(&mut m);
    phase("stacks", &mut lap);
    m.drain();
    phase("trace", &mut lap);
    if !major && verify_mode() {
        verify_minor(h);
    }
    if major {
        // C-extension data that died
        DATA_MARKED.raw().retain(|&d| hdr(d).flags & FL_MARK != 0);
    }
    free_retired(std::mem::take(&mut m.retired), std::mem::take(&mut m.keep));

    // sweep: pages are split among this thread and the stopped ones (`sweep_help`); objects
    // whose freeing isn't safe on any thread (procs, C data, generic ivars) come back here
    crate::mt::set_in_sweep(true);
    let freed_count;
    let deferred;
    // a minor collection only notes which pages need sweeping (`refill` sweeps a page when a
    // thread takes it); a major one sweeps everything now
    let sweep_pages: Vec<usize> = if major { (0..h.pages.len()).collect() } else { Vec::new() };
    if !major {
        for pg in h.pages.iter_mut() {
            if pg.young {
                pg.young = false;
                pg.unswept.store(UNSWEPT, Ordering::Relaxed);
            }
        }
    }
    if sweep_pages.is_empty() {
        // (a minor collection: nothing to sweep now, so don't wake the stopped threads)
        freed_count = 0;
        deferred = Vec::new();
    } else {
        let n = sweep_pages.len();
        *SWEEP_JOB.get() = SweepJob { pages: h.pages.as_mut_ptr(), idx: sweep_pages.as_ptr(), n };
        SWEEP_LIVE.store(0, Ordering::Relaxed);
        SWEEP_FREED.store(0, Ordering::Relaxed);
        SWEEP_NEXT.store(0, Ordering::Relaxed);
        SWEEP_PENDING.store(n.div_ceil(SWEEP_CHUNK), Ordering::SeqCst);
        SWEEP_COLLECTOR.store(crate::core::pst() as *mut PState as usize, Ordering::Relaxed);
        SWEEP_HELPED.store(0, Ordering::Relaxed);
        SWEEP_ACTIVE.store(true, Ordering::SeqCst);
        STW_CV.notify_all();
        let _ = sweep_help();
        let mut spins = 0u32;
        while SWEEP_PENDING.load(Ordering::Acquire) != 0 {
            spins += 1;
            if spins < 1000 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        SWEEP_ACTIVE.store(false, Ordering::SeqCst);
        freed_count = SWEEP_FREED.load(Ordering::Relaxed);
        deferred = std::mem::take(&mut *SWEEP_DEFERRED.lock().unwrap_or_else(|e| e.into_inner()));
    }
    let gen_iv = crate::core::GENERIC_IVARS.raw();
    for &p in &deferred {
        if let Some(t) = gen_iv.as_mut() {
            if !t.is_empty() {
                t.remove(&p);
            }
        }
        unsafe { free_obj(p) };
    }
    let freed_classes = if major { free_dead_singletons(h, &m.seen_classes) } else { 0 };
    if !orphans.is_empty() {
        free_dead_units(h, &mut m, orphans);
    }
    crate::mt::set_in_sweep(false);
    let marked_envs = m.marked_envs;
    let free_env = |p: usize, n: u32| free_malloc_env(p, n);
    if major {
        h.envs.retain(|&p, &mut n| {
            if marked_envs.contains(&p) {
                true
            } else {
                free_env(p, n);
                false
            }
        });
    } else {
        // only young environments can die in a minor collection; they are freed by a
        // background thread, after the pause
        let mut dead = Vec::new();
        for p in young_envs {
            if !marked_envs.contains(&p) {
                if let Some(n) = h.envs.remove(&p) {
                    dead.push((p, n));
                }
            }
        }
        if !dead.is_empty() {
            send_garbage(Garbage::Envs(dead));
        }
    }
    if major {
        h.live = SWEEP_LIVE.load(Ordering::Relaxed);
        h.major_live = h.live;
        h.minors = 0;
        h.majors += 1;
    } else {
        // old objects: what was old before, plus what this collection promoted
        h.live += m.newly_marked;
        h.minors += 1;
    }
    SINCE.store(0, Ordering::Relaxed);
    // with several threads allocating, every collection stops all of them: collect less often
    // (KENNY_GC_YOUNG_SCALE caps the multiplier: 1 keeps the one-thread young generation, for
    // the least memory, at the cost of more frequent pauses)
    let ps = gc_ps().len().clamp(1, env_knob("KENNY_GC_YOUNG_SCALE", 4).max(1));
    let threshold = if generational() {
        // the young generation's size; the old one grows until the next major collection
        min_threshold() * ps
    } else {
        (min_threshold() * ps).max(((h.live + h.envs.len()) as f64 * growth()) as usize)
    };
    THRESHOLD.store(threshold, Ordering::Relaxed);
    // threads take pages again after the collection (their free lists may hold freed pages)
    for &p in gc_ps().iter() {
        let ph = unsafe { &mut (*(p as *mut PState)).heap };
        for f in ph.free.iter_mut() {
            f.clear();
        }
        ph.since = 0;
    }
    MALLOC_SINCE.store(0, Ordering::Relaxed);
    MALLOC_LIMIT.store(malloc_limit(), Ordering::Relaxed);
    phase("sweep", &mut lap);
    release_and_refill(h, threshold);
    if !major && crate::mt::multi() && std::env::var_os("KENNY_GC_LAZY").is_none() {
        let pages: Vec<usize> = h.pages.iter().filter(|pg| pg.unswept.load(Ordering::Relaxed) == UNSWEPT).map(|pg| &**pg as *const Page as usize).collect();
        bg_sweep_pages(pages);
    }
    drop(bg_guard);
    phase("pages", &mut lap);
    if *STATS_EVERY.get() {
        let txt: Vec<String> = phases.iter().map(|(n, t)| format!("{n}={t:.2}")).collect();
        eprintln!(
            "[gc] {} phases ms: {} (scanned_words={} retired={:?} (n, KB) data_marked={} malloc_envs={}/{} classes={} (freed {}) region_words={} remembered={} promoted={} swept_pages={}/{} freed={} lazily_freed={} helped={}/{})",
            if major { "major" } else { "minor" },
            txt.join(" "),
            SCANNED_WORDS.swap(0, Ordering::Relaxed),
            n_retired,
            DBG_NDATA.load(std::sync::atomic::Ordering::Relaxed),
            n_young_envs,
            h.envs.len(),
            h.classes.len(),
            freed_classes,
            region_words,
            remembered.len(),
            m.newly_marked,
            sweep_pages.len(),
            h.pages.len(),
            freed_count,
            LAZY_FREED.swap(0, Ordering::Relaxed),
            SWEEP_HELPED.load(Ordering::Relaxed),
            sweep_pages.len().div_ceil(SWEEP_CHUNK)
        );
    }
    h.collections += 1;
    h.gc_nanos += t0.elapsed().as_nanos();
    IN_COLLECT.store(false, Ordering::Relaxed);
    if *STATS_EVERY.get() {
        print_stats();
    }
}

static OTHER_STACKS: G<Vec<([usize; 16], usize, usize, Vec<usize>)>> = G::new(Vec::new());
/// Memory that became unreachable in a collection, freed by a background thread so that
/// freeing many small blocks doesn't lengthen the pause.
static DBG_NDATA: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
enum Garbage {
    /// malloc'd closure environments (data address, slots)
    Envs(Vec<(usize, u32)>),
    /// retired buffers (address, bytes, align)
    Buffers(Vec<(usize, usize, usize)>),
}
fn send_garbage(g: Garbage) {
    static TX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Garbage>>> = std::sync::OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Garbage>();
        std::thread::Builder::new()
            .name("kenny-gc-free".into())
            .spawn(move || {
                for batch in rx {
                    match batch {
                        Garbage::Envs(v) => {
                            for (p, n) in v {
                                free_malloc_env(p, n);
                            }
                        }
                        Garbage::Buffers(v) => {
                            for (p, bytes, align) in v {
                                crate::mt::free_buf(p, bytes, align);
                            }
                        }
                    }
                }
            })
            .expect("gc free thread");
        std::sync::Mutex::new(tx)
    });
    let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(g);
}
/// Free a malloc'd environment (`kn_env_new`: a header, then [parent, blk, slots...]).
fn free_malloc_env(p: usize, n: u32) {
    let words = 4 + n as usize;
    unsafe { drop(Vec::from_raw_parts((p - 16) as *mut V, words, words)) };
}

/// The collector's stack position when the collection started (for the verifier).
static GC_TOP: AtomicUsize = AtomicUsize::new(0);

/// Mark from the roots every collection has (not the stacks: `scan_with_registers`).
fn mark_roots(m: &mut Marker, h: &mut Heap, all_classes: bool) {
    // (m.cur tags where a root came from, for the verifier's reports)
    m.cur = 10;
    // classes (except singleton classes of ordinary objects, traced via their object); a
    // minor collection only needs the new ones
    let classes = if all_classes { h.classes.clone() } else { std::mem::take(&mut h.new_classes) };
    for &c in &classes {
        let k = unsafe { &*c };
        if k.is_singleton && !is_class(k.attached) {
            continue;
        }
        m.class(c);
    }
    let r = rt();
    m.cur = 11;
    m.value(r.main_obj);
    for &v in r.gvars.values() {
        m.value(v);
    }
    m.cur = 12;
    // every P's current exception state
    for &p in gc_ps().iter() {
        let p = unsafe { &*(p as *const PState) };
        m.value(p.errinfo);
        m.value(p.last_match);
        match p.pending {
            Throw::Raise(e) => m.value(e),
            Throw::Break { val, .. } | Throw::Return { val, .. } | Throw::StopLoop(val) => m.value(val),
            Throw::Throw { tag, val } => {
                m.value(tag);
                m.value(val);
            }
            Throw::None | Throw::Kill => {}
        }
        for &b in &p.current_block {
            m.value(b);
        }
        let mut cls = Vec::new();
        definee_values(&p.definee, &p.dm_stack, &mut cls);
        for v in cls {
            m.value(v);
        }
    }
    for &p in gc_ps().iter() {
        let ph = unsafe { &(*(p as *const PState)).heap };
        for t in ph.saved_throws.map.values().copied().collect::<Vec<_>>() {
            match t {
                Throw::Raise(e) => m.value(e),
                Throw::Break { val, .. } | Throw::Return { val, .. } | Throw::StopLoop(val) => m.value(val),
                Throw::Throw { tag, val } => {
                    m.value(tag);
                    m.value(val);
                }
                Throw::None | Throw::Kill => {}
            }
        }
        for &rv in &ph.roots.clone() {
            for &v in unsafe { &*rv }.iter() {
                m.value(v);
            }
        }
    }
    m.cur = 15;
    let mut extra = Vec::new();
    for f in h.extra_roots.clone() {
        f(&mut extra);
    }
    for v in extra {
        m.value(v);
    }
    m.cur = 16;
    for &v in PINNED.raw().iter() {
        m.value(v);
    }
    m.cur = 0;
    if scan_regions() {
        for (p, n) in h.regions.clone() {
            scan_range(m, p, p + n * 8);
        }
    }
}

/// Classes that running class bodies define into and that running `define_method` methods
/// belong to (GC roots; a definee entry with the low bit set is an object whose singleton
/// class is made on demand).
pub fn definee_values(definee: &[*mut Class], dm_stack: &[*const Method], out: &mut Vec<V>) {
    for &c in definee {
        out.push(if c as usize & 1 != 0 { (c as usize & !1) as V } else { c as V });
    }
    for &m in dm_stack {
        if !m.is_null() {
            out.push(unsafe { (*m).owner } as V);
        }
    }
}

/// Code of methods that a Method/UnboundMethod object was made for: never freed (the object
/// can call it after the method is removed from its class).
static METHOD_OBJECT_CODE: crate::mt::L<Option<FxHashSet<usize>>> = crate::mt::L::new(None);
pub fn note_method_object(m: *const Method) {
    if let Body::Compiled(f) = unsafe { &(*m).body } {
        METHOD_OBJECT_CODE.get().get_or_insert_with(FxHashSet::default).insert(*f as usize);
    }
}
pub fn method_object_code() -> Vec<usize> {
    METHOD_OBJECT_CODE.get().as_ref().map(|s| s.iter().copied().collect()).unwrap_or_default()
}

/// Evicted eval units that nothing reached in this major collection: methods of live classes
/// and Method objects count too (procs and stacks were checked while marking).
fn free_dead_units(h: &mut Heap, m: &mut Marker, orphans: Vec<OrphanUnit>) {
    for &c in &h.classes {
        for &meth in unsafe { &*c }.methods.values() {
            if meth.is_null() {
                continue;
            }
            if let Body::Compiled(f) = unsafe { &(*meth).body } {
                let f = *f as usize;
                if f >= m.code_lo && f < m.code_hi {
                    m.code_word(f);
                }
            }
        }
    }
    for f in method_object_code() {
        if f >= m.code_lo && f < m.code_hi {
            m.code_word(f);
        }
    }
    let mut dead = Vec::new();
    let mut keep = Vec::new();
    for (u, o) in orphans.into_iter().enumerate() {
        if m.code_hit[u] { keep.push(o) } else { dead.push(o.id) }
    }
    if std::env::var_os("KENNY_DEBUG_EVAL").is_some() {
        eprintln!("[eval] collection: {} evicted units freed, {} still reachable; Rust heap {} KB", dead.len(), keep.len(), MALLOC_LIVE.load(Ordering::Relaxed) >> 10);
    }
    ORPHANS.raw().extend(keep);
    if !dead.is_empty() {
        // call caches may still hold methods whose code this was
        crate::core::bump_serial();
        DEAD_UNITS.lock().unwrap_or_else(|e| e.into_inner()).extend(dead);
    }
}

/// After a major collection's sweep: free the singleton classes of objects that died (the
/// marker never reached them) with their method tables. Method caches are keyed by class
/// address, so they are invalidated (a new class may get the same address).
fn free_dead_singletons(h: &mut Heap, seen: &FxHashSet<usize>) -> usize {
    let mut dead: FxHashSet<usize> = FxHashSet::default();
    let class_set = &h.class_set;
    h.classes.retain(|&c| {
        let k = unsafe { &*c };
        let attached_is_class = class_set.contains(&(k.attached as usize));
        if k.is_singleton && !attached_is_class && !seen.contains(&(c as usize)) {
            dead.insert(c as usize);
            false
        } else {
            true
        }
    });
    if dead.is_empty() {
        return 0;
    }
    for c in &dead {
        h.class_set.remove(c);
    }
    h.new_classes.retain(|c| !dead.contains(&(*c as usize)));
    crate::core::bump_serial();
    crate::core::bump_const_serial();
    for &c in &dead {
        let k = unsafe { Box::from_raw(c as *mut Class) };
        for &m in k.methods.values() {
            if !m.is_null() && unsafe { (*m).owner } == c as *mut Class {
                drop(unsafe { Box::from_raw(m as *mut Method) });
            }
        }
        drop(k);
    }
    dead.len()
}

/// Before a major collection: every object starts unmarked (young and old alike).
fn clear_marks(h: &mut Heap) {
    for pg in h.pages.iter() {
        for (wi, &word) in pg.alloc.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let p = pg.base + (wi * 64 + b) * pg.slot;
                unsafe { (*(p as *mut Header)).flags &= !FL_MARK };
            }
        }
    }
}

/// KENNY_GC_VERIFY: after a minor collection's marking, traverse everything reachable (like
/// a major collection, with its own visited set) and abort if a reachable object is
/// unmarked, i.e. the write barrier missed a store.
fn verify_minor(h: &mut Heap) {
    let mut v = Marker::new(Vec::new(), Vec::new(), usize::MAX, 0);
    v.verify = Some(FxHashSet::default());
    mark_roots(&mut v, h, true);
    scan_with_registers(&mut v);
    v.drain();
    if v.verify_fail > 0 {
        eprintln!("[gc verify] {} reachable objects were left unmarked by a minor collection", v.verify_fail);
        std::process::abort();
    }
}

/// Free the retired buffers no stack points into; keep the others for the next collection.
fn free_retired(retired: Vec<(usize, usize, usize)>, keep: Vec<bool>) {
    let mut kept = Vec::new();
    let mut dead = Vec::new();
    for (r, k) in retired.into_iter().zip(keep) {
        if k {
            kept.push(r);
        } else {
            dead.push(r);
        }
    }
    if !dead.is_empty() {
        // unreachable from now on: freed after the pause
        send_garbage(Garbage::Buffers(dead));
    }
    if !kept.is_empty() {
        crate::mt::keep_retired(kept);
    }
}

/// Minimum allocations between collections. KENNY_GC_MIN overrides it (small values make
/// a GC stress test).
fn env_knob(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn min_threshold() -> usize {
    static M: G<usize> = G::new(0);
    let m = M.get();
    if *m == 0 {
        *m = std::env::var("KENNY_GC_MIN").ok().and_then(|v| v.parse().ok()).filter(|x: &usize| *x > 0).unwrap_or(MIN_THRESHOLD);
    }
    *m
}

/// Allocations allowed between collections, as a multiple of the live set
/// (peak heap ≈ live × (1 + growth)). KENNY_GC_GROWTH overrides it.
fn growth() -> f64 {
    static G_: G<f64> = G::new(0.0);
    let g = G_.get();
    if *g == 0.0 {
        *g = std::env::var("KENNY_GC_GROWTH").ok().and_then(|v| v.parse().ok()).filter(|x: &f64| *x > 0.0).unwrap_or(DEFAULT_GROWTH);
    }
    *g
}

/// After a sweep: give completely empty pages back to the allocator (keeping enough slots
/// for the live set plus the next cycle's allocations, with some slack), then rebuild the
/// free lists so allocation fills the fullest pages first and emptier pages can drain.
fn release_and_refill(h: &mut Heap, threshold: usize) {
    let used = |pg: &Page| pg.alloc.iter().map(|w| w.count_ones() as usize).sum::<usize>();
    let counts: Vec<usize> = h.pages.iter().map(|p| used(p)).collect();
    let capacity: usize = h.pages.iter().map(|p| p.nslots).sum();
    let needed = ((h.live + threshold) as f64 * 1.25) as usize;
    let mut spare = capacity.saturating_sub(needed);
    let layout = std::alloc::Layout::from_size_align(PAGE, PAGE).unwrap();
    let mut keep = vec![true; h.pages.len()];
    for (i, pg) in h.pages.iter().enumerate() {
        if counts[i] == 0 && pg.nslots <= spare {
            spare -= pg.nslots;
            keep[i] = false;
            unsafe { std::alloc::dealloc(pg.base as *mut u8, layout) };
        }
    }
    if keep.iter().any(|k| !k) {
        let mut i = 0;
        h.pages.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        h.page_index.clear();
        h.page_lo = usize::MAX;
        h.page_hi = 0;
        for (i, pg) in h.pages.iter().enumerate() {
            h.page_index.insert(pg.base, i);
            h.page_lo = h.page_lo.min(pg.base);
            h.page_hi = h.page_hi.max(pg.base + PAGE);
        }
        rebuild_page_table(h);
    }
    // the pool is a stack (pop from the end): emptiest pages first, so allocation fills the
    // fullest pages and emptier ones can drain and be released
    // (unswept pages count as available: their dead objects are freed when taken)
    let mut order: Vec<(usize, usize)> = h.pages.iter().enumerate().filter(|(_, pg)| used(pg) < pg.nslots || pg.is_unswept()).map(|(i, pg)| (used(pg), i)).collect();
    order.sort_unstable();
    for p in h.pool.iter_mut() {
        p.clear();
    }
    for (_, i) in order {
        let ci = h.pages[i].class_idx;
        h.pool[ci].push(i);
    }
}

unsafe fn free_obj(p: usize) {
    use std::ptr::drop_in_place;
    unsafe {
        match (*(p as *mut Header)).kind {
            Kind::Object => drop_in_place(p as *mut RObject),
            Kind::String => drop_in_place(p as *mut RString),
            Kind::Array => drop_in_place(p as *mut RArray),
            Kind::Hash => drop_in_place(p as *mut RHash),
            Kind::Float => {}
            Kind::Bignum => drop_in_place(p as *mut RBignum),
            Kind::Range => {}
            Kind::Proc => {
                let pr = &mut *(p as *mut RProc);
                if !pr.native.is_null() {
                    drop(Box::from_raw(pr.native));
                }
                drop_in_place(&mut pr.roots);
            }
            Kind::Data => {
                let d = &mut *(p as *mut RData);
                if let Some(free) = d.dfree {
                    free(d.data);
                }
                drop_in_place(&mut d.roots);
            }
            Kind::Class | Kind::Env => {}
        }
        // poison the header so stale pointers are obvious
        (*(p as *mut Header)).flags = 0;
    }
}

pub fn stats() -> (usize, usize) {
    let h = heap();
    (h.collections, h.live + h.envs.len() + SINCE.load(Ordering::Relaxed))
}
