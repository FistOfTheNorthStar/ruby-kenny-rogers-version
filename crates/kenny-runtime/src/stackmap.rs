//! Stack frames for the collector: walking the frame-pointer chains of the stopped threads,
//! and the stack maps of compiled code (kenny-compiler gcmap.rs).
//!
//! At each call, a compiled function keeps the Ruby objects it still needs after the call in
//! stack slots that the compiler lists for the call's return address (Cranelift's user stack
//! maps): [return offset, frame size, slots]. A compacting collection walks every stopped
//! stack's frame-pointer chain; a frame record whose return address has a map belongs to a
//! callee of a compiled frame stopped at that call, and the caller's slots are at its stack
//! pointer (its frame pointer minus the frame size) plus each offset. Those slots are
//! *precise*: the collector marks their objects without pinning them and writes the new
//! addresses back after moving (gc.rs `PRECISE`). Every other stack word (native frames,
//! the rest of compiled frames, registers) is still scanned conservatively and pins what it
//! points to, so a value the compiler didn't declare is safe, only not movable.
//!
//! A frame is taken as precise only when the chain and the map agree exactly: the callee's
//! frame record sits right below the caller's stack pointer, and the caller's frame lies
//! inside the stack. Otherwise it is scanned conservatively like a native frame.

use std::sync::atomic::Ordering;
use std::sync::RwLock;

struct MapFn {
    start: usize,
    end: usize,
    /// [sites, (ret, span, first, count) * sites, slots...]
    table: *const u32,
    /// eval units: the table is a boxed copy (freed by `unregister_range`)
    owned: bool,
}
unsafe impl Send for MapFn {}
unsafe impl Sync for MapFn {}
struct Maps {
    fns: Vec<MapFn>,
    sorted: bool,
}
static MAPS: RwLock<Maps> = RwLock::new(Maps { fns: Vec::new(), sorted: true });

/// Called by the program's `main` (kenny-compiler gcmap.rs `emit_stack_maps`): `n` entries
/// [address: u64, code size: u32, table start: u32] and the tables (u32 words).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_stackmaps(fns: *const u8, n: u64, words: *const u8) {
    let mut m = MAPS.write().unwrap();
    m.fns.reserve(n as usize);
    for i in 0..n as usize {
        let e = unsafe { fns.add(i * 16) };
        let start = unsafe { *(e as *const usize) };
        let size = unsafe { *(e.add(8) as *const u32) } as usize;
        let at = unsafe { *(e.add(12) as *const u32) } as usize;
        m.fns.push(MapFn { start, end: start + size, table: unsafe { (words as *const u32).add(at) }, owned: false });
    }
    m.sorted = false;
}

/// Stack maps of an eval unit's functions: (address, code size, table).
pub fn register_jit(maps: &[(usize, u32, Vec<u32>)]) {
    if maps.is_empty() {
        return;
    }
    let mut m = MAPS.write().unwrap();
    for (start, size, t) in maps {
        let table = Box::leak(t.clone().into_boxed_slice()).as_ptr();
        m.fns.push(MapFn { start: *start, end: start + *size as usize, table, owned: true });
    }
    m.sorted = false;
}

/// Forget the maps of functions in [start, end) (an eval unit's code being freed).
pub fn unregister_range(start: usize, end: usize) {
    let mut m = MAPS.write().unwrap();
    m.fns.retain(|f| {
        if f.start < start || f.start >= end {
            return true;
        }
        if f.owned {
            let n = table_len(f.table);
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(f.table as *mut u32, n)) });
        }
        false
    });
}

fn table_len(t: *const u32) -> usize {
    let sites = unsafe { *t } as usize;
    let mut n = 1 + 4 * sites;
    for s in 0..sites {
        let e = unsafe { t.add(1 + 4 * s) };
        n = n.max(unsafe { *e.add(2) + *e.add(3) } as usize);
    }
    n
}

/// The map for return address `ret`: (frame size, slot offsets).
fn lookup(fns: &[MapFn], ret: usize) -> Option<(usize, &'static [u32])> {
    let i = fns.partition_point(|f| f.start < ret);
    let f = fns.get(i.checked_sub(1)?)?;
    if ret <= f.start || ret > f.end {
        return None;
    }
    let off = (ret - f.start) as u32;
    let t = f.table;
    let sites = unsafe { *t } as usize;
    let site = |s: usize| unsafe { std::slice::from_raw_parts(t.add(1 + 4 * s), 4) };
    // sites are sorted by return offset
    let (mut lo, mut hi) = (0, sites);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let r = site(mid)[0];
        if r == off {
            let e = site(mid);
            let slots = unsafe { std::slice::from_raw_parts(t.add(e[2] as usize), e[3] as usize) };
            return Some((e[1] as usize, slots));
        }
        if r < off { lo = mid + 1 } else { hi = mid }
    }
    None
}

/// Precise scanning of compiled frames: on where frame pointers are always kept (Apple
/// platforms); KENNY_GC_PRECISE=0 / =1 overrides.
pub fn precise_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("KENNY_GC_PRECISE").as_deref() {
        Ok("0") => false,
        Ok("1") => true,
        _ => cfg!(target_vendor = "apple"),
    })
}

/// The stack slots of compiled frames that hold objects at their current calls, on the
/// given stacks (frame pointer, lowest, highest address): sorted, without duplicates.
pub fn precise_slots(stacks: &[(usize, usize, usize)]) -> Vec<usize> {
    let mut out = Vec::new();
    {
        let m = MAPS.read().unwrap();
        if m.fns.is_empty() {
            return out;
        }
        if !m.sorted {
            drop(m);
            let mut w = MAPS.write().unwrap();
            if !w.sorted {
                w.fns.sort_by_key(|f| f.start);
                w.sorted = true;
            }
        }
    }
    let m = MAPS.read().unwrap();
    for &(fp, lo, hi) in stacks {
        walk(fp, lo, hi, |rec, ret| {
            let Some((span, slots)) = lookup(&m.fns, ret) else { return };
            // the caller's frame: its frame pointer is the record's link, its stack pointer
            // at the call is right above the callee's record
            let caller_fp = unsafe { *(rec as *const usize) };
            let sp = caller_fp.wrapping_sub(span);
            if caller_fp <= rec || sp != rec + 16 || sp < lo || caller_fp + 16 > hi {
                return;
            }
            for &o in slots {
                let a = sp + o as usize;
                if a % 8 == 0 && a + 8 <= caller_fp {
                    out.push(a);
                }
            }
        });
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The frame pointer of the calling function (it must have a frame).
#[inline(always)]
pub fn frame_pointer() -> usize {
    let fp: usize;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        std::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack, preserves_flags));
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        fp = 0;
    }
    fp
}

/// Index of the frame pointer in `PHeap::regs` (see gc::spill_registers).
#[cfg(target_arch = "aarch64")]
pub const FP_REG: usize = 10;
#[cfg(not(target_arch = "aarch64"))]
pub const FP_REG: usize = 1;

/// Walk a frame-pointer chain from `fp` while it stays inside [lo, hi): calls `f(record,
/// ret)` for each frame record (at `record`: the caller's frame pointer, then `ret`, the
/// return address into the caller). Stops at the first record that isn't above the last.
pub fn walk(mut fp: usize, lo: usize, hi: usize, mut f: impl FnMut(usize, usize)) {
    let mut n = 0;
    while fp >= lo && fp + 16 <= hi && fp % 16 == 0 && n < 1_000_000 {
        let next = unsafe { *(fp as *const usize) };
        let ret = unsafe { *((fp + 8) as *const usize) } & 0x0000_ffff_ffff_ffff;
        f(fp, ret);
        if next <= fp {
            break;
        }
        fp = next;
        n += 1;
    }
}

/// Whether a frame record on the stack returns to one of `marks` (call_native's call of the
/// native function): None when the chain of frame records doesn't reach the top of the stack
/// (a frame without a record somewhere: the caller then scans every word instead).
pub fn chain_has(fp: usize, lo: usize, hi: usize, marks: &[usize]) -> Option<bool> {
    let mut found = false;
    let mut last = 0;
    walk(fp, lo, hi, |rec, ret| {
        found |= marks.contains(&ret);
        last = rec;
    });
    if found {
        return Some(true);
    }
    if last == 0 {
        return None;
    }
    let next = unsafe { *(last as *const usize) };
    // the outermost record ends the chain (0, or a link out of this stack: a coroutine's
    // entry links to the stack that started it) near the top of the stack (above it there's
    // only the entry's own words, no method's frame)
    if (next == 0 || next < lo || next >= hi) && hi - last <= 4096 { Some(false) } else { None }
}

/// Marks the native function's address in `call_native`'s frame.
pub const NATIVE_TAG: usize = 0x6b6e_6e61_7469_7665;

static NATIVES: std::sync::Mutex<Option<rustc_hash::FxHashMap<usize, (usize, u32)>>> = std::sync::Mutex::new(None);
fn diag() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var("KENNY_GC_COMPACT_STATS").is_ok_and(|v| v == "2"))
}
/// (KENNY_GC_COMPACT_STATS=2) Remember which method a native function implements.
pub fn note_native(f: usize, c: *mut crate::core::Class, mid: u32) {
    if diag() {
        NATIVES.lock().unwrap().get_or_insert_with(Default::default).entry(f).or_insert((c as usize, mid));
    }
}

fn symbol(addr: usize) -> String {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    if unsafe { libc::dladdr(addr as *const libc::c_void, &mut info) } != 0 && !info.dli_sname.is_null() {
        if let Some(&(c, mid)) = NATIVES.lock().unwrap().as_ref().and_then(|m| m.get(&(info.dli_saddr as usize))) {
            // (never class_name: a singleton class's name calls `inspect`)
            let k = unsafe { &*(c as *mut crate::core::Class) };
            let (owner, sep) = if k.is_singleton && crate::core::is_class(k.attached) { (unsafe { (*(k.attached as *mut crate::core::Class)).name.clone() }, ".") } else { (k.name.clone(), "#") };
            return format!("{owner}{sep}{}", crate::core::sym_str(mid));
        }
        let s = unsafe { std::ffi::CStr::from_ptr(info.dli_sname) }.to_string_lossy().into_owned();
        return format!("{s}+{:#x}", addr - info.dli_saddr as usize);
    }
    format!("{addr:#x}")
}

/// (KENNY_GC_COMPACT_STATS=2) The native methods that keep a collection from compacting: the
/// functions called by `call_native`/`call_native_block` on each stack.
pub fn native_report(stacks: &[(usize, usize, usize)]) {
    let marks = crate::core::native_return_addresses();
    static REPORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if REPORTS.fetch_add(1, Ordering::Relaxed) >= 200 {
        return;
    }
    for &(fp, lo, hi) in stacks {
        let mut names: Vec<String> = Vec::new();
        let mut prev_ret = 0;
        let mut prev_rec = 0;
        walk(fp, lo, hi, |rec, ret| {
            if ret == marks[0] || ret == marks[1] {
                // call_native's own frame (above the callee's record) holds the function
                let caller_fp = unsafe { *(rec as *const usize) };
                let mut f = 0;
                let mut a = rec + 16;
                while a + 16 <= caller_fp && a < rec + 256 {
                    if unsafe { *(a as *const usize) } == NATIVE_TAG {
                        f = unsafe { *((a + 8) as *const usize) };
                        break;
                    }
                    a += 8;
                }
                names.push(if f != 0 { symbol(f) } else { symbol(prev_ret) });
            }
            prev_ret = ret;
            prev_rec = rec;
        });
        let _ = prev_rec;
        eprintln!("[gc] stack natives: {}", names.join(" | "));
    }
}
