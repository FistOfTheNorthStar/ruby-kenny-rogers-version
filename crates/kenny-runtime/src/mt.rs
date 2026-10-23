//! Shared state for running Ruby code on several OS threads at once ("Ps").
//!
//! While only one P runs (every program until its second `Thread` starts), nothing here
//! costs anything: buffers are freed at once and writers take no locks.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static MULTI: AtomicBool = AtomicBool::new(false);
/// True during a GC sweep: buffers of dead objects can be freed at once.
static IN_SWEEP: AtomicBool = AtomicBool::new(false);
thread_local! {
    /// This thread is lazily sweeping a page (freeing dead objects' buffers at once).
    pub static LAZY_SWEEP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// More than one OS thread may be running Ruby code.
#[inline(always)]
pub fn multi() -> bool {
    MULTI.load(Ordering::Relaxed)
}
pub fn set_multi() {
    MULTI.store(true, Ordering::SeqCst);
    // direct calls from compiled code check one global stack limit: only for one P
    crate::core::KN_DIRECT.call_limit.store(usize::MAX, Ordering::SeqCst);
    crate::core::KN_DIRECT.entry_limit.store(0, Ordering::SeqCst);
    crate::core::drop_claims();
}
pub fn set_in_sweep(b: bool) {
    IN_SWEEP.store(b, Ordering::Relaxed);
}

/// Buffers replaced while other threads may still be reading them: (address, bytes, align).
static RETIRED: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

pub fn free_buf(p: usize, bytes: usize, align: usize) {
    unsafe { std::alloc::dealloc(p as *mut u8, std::alloc::Layout::from_size_align_unchecked(bytes, align)) };
}

/// A buffer an object no longer uses. Freed now if no other thread can be reading it,
/// otherwise at the next GC (see `gc::free_retired`).
pub fn retire(p: usize, bytes: usize, align: usize) {
    if !multi() || IN_SWEEP.load(Ordering::Relaxed) || LAZY_SWEEP.with(|l| l.get()) {
        free_buf(p, bytes, align);
    } else {
        push_retired(p, bytes, align);
    }
}
/// Into this thread's own list (no lock; the GC gathers every thread's list), or the shared
/// one from a thread that doesn't run Ruby code.
fn push_retired(p: usize, bytes: usize, align: usize) {
    match crate::core::pst_opt() {
        Some(ps) => ps.heap.retired.push((p, bytes, align)),
        None => RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push((p, bytes, align)),
    }
}

/// Like `retire`, but never freed before the next GC, even with one thread: for tables a
/// lookup may still be reading after calling into Ruby (a key's `eql?` can change the hash).
pub fn retire_deferred(p: usize, bytes: usize, align: usize) {
    push_retired(p, bytes, align);
}

/// For the GC (all other threads stopped): the retired buffers.
pub fn take_retired() -> Vec<(usize, usize, usize)> {
    std::mem::take(&mut *RETIRED.lock().unwrap_or_else(|e| e.into_inner()))
}
/// Put back buffers that are still referenced.
pub fn keep_retired(v: Vec<(usize, usize, usize)>) {
    RETIRED.lock().unwrap_or_else(|e| e.into_inner()).extend(v);
}

// ---------- L<T>: a lock for runtime tables changed after boot ----------

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, AtomicU32};

/// What a thread spinning on a runtime lock does between attempts: lets a pending GC stop
/// this thread (the lock holder may be the one that wants to collect). Set by the GC.
pub static SPIN_HOOK: std::sync::atomic::AtomicPtr<()> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Identifies the OS thread (opaque, so it is never merged with a lookup made before a Ruby
/// thread moved to another OS thread).
#[inline(never)]
fn my_token() -> usize {
    thread_local!(static T: u8 = const { 0 });
    std::hint::black_box(T.with(|t| t as *const u8 as usize))
}

pub fn spin_pause(spins: &mut u32) {
    *spins += 1;
    if *spins < 64 {
        std::hint::spin_loop();
    } else {
        let h = SPIN_HOOK.load(Ordering::Acquire);
        if !h.is_null() {
            let f: fn() = unsafe { std::mem::transmute(h) };
            f();
        }
        std::thread::yield_now();
    }
}

/// A global runtime table that can change after boot (features, autoloads, caches...).
/// `get()` locks it for the current statement (or as long as the guard lives); the lock is
/// reentrant per OS thread and free while only one OS thread runs Ruby code.
pub struct L<T> {
    owner: AtomicUsize,
    depth: AtomicU32,
    v: UnsafeCell<T>,
}
unsafe impl<T> Sync for L<T> {}

pub struct LGuard<'a, T> {
    l: &'a L<T>,
    locked: bool,
}

impl<T> L<T> {
    pub const fn new(v: T) -> Self {
        L { owner: AtomicUsize::new(0), depth: AtomicU32::new(0), v: UnsafeCell::new(v) }
    }
    #[inline]
    pub fn get(&self) -> LGuard<'_, T> {
        if !multi() {
            return LGuard { l: self, locked: false };
        }
        let me = my_token();
        if self.owner.load(Ordering::Relaxed) == me {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return LGuard { l: self, locked: true };
        }
        let mut spins = 0;
        while self.owner.compare_exchange_weak(0, me, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spin_pause(&mut spins);
        }
        self.depth.store(1, Ordering::Relaxed);
        LGuard { l: self, locked: true }
    }
    /// Unlocked access, for the GC while every other thread is stopped.
    #[allow(clippy::mut_from_ref)]
    pub fn raw(&self) -> &mut T {
        unsafe { &mut *self.v.get() }
    }
}
impl<T> Drop for LGuard<'_, T> {
    fn drop(&mut self) {
        if self.locked && self.l.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.l.owner.store(0, Ordering::Release);
        }
    }
}
impl<T> std::ops::Deref for LGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.l.v.get() }
    }
}
impl<T> std::ops::DerefMut for LGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.l.v.get() }
    }
}
