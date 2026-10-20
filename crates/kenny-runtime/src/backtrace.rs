//! Backtraces without runtime cost: compiled code keeps frame pointers, and the compiler
//! emits a table of every compiled function (address, size, label) with the source line of
//! each of its calls, keyed by the call's return-address offset. Taking a backtrace walks the
//! frame-pointer chain of the current stack and maps each return address that lies in
//! compiled code to "file:line:in 'label'". Runtime (Rust) frames in between are skipped.

use crate::core::*;
use std::sync::RwLock;

struct Func {
    start: usize,
    end: usize,
    label: *const u8,
    file: *const u8,
    /// (return-address offset, line), sorted by offset; points into the compiled tables
    /// (or, for eval units, a boxed copy: `owned`, freed by `unregister_jit`)
    calls: &'static [[u32; 2]],
    owned: bool,
}
unsafe impl Send for Func {}
unsafe impl Sync for Func {}

struct Table {
    funcs: Vec<Func>,
    sorted: bool,
}

static TABLE: RwLock<Table> = RwLock::new(Table { funcs: Vec::new(), sorted: true });

/// Raw table entry written by the compiler (see kenny-compiler `finish`).
#[repr(C)]
struct RawFn {
    addr: usize,
    size: u32,
    label: u32,
    file: u32,
    _pad: u32,
    calls_start: u32,
    calls_len: u32,
}

/// Called by the program's `main` before the runtime starts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_debug(fns: *const u8, n: u64, calls: *const u8, strs: *const u8) {
    let fns = fns as *const RawFn;
    let calls = calls as *const [u32; 2];
    let mut t = TABLE.write().unwrap();
    t.funcs.reserve(n as usize);
    for i in 0..n as usize {
        let f = unsafe { &*fns.add(i) };
        let cs = unsafe { std::slice::from_raw_parts(calls.add(f.calls_start as usize), f.calls_len as usize) };
        t.funcs.push(Func {
            start: f.addr,
            end: f.addr + f.size as usize,
            label: unsafe { strs.add(f.label as usize) },
            file: unsafe { strs.add(f.file as usize) },
            calls: cs,
            owned: false,
        });
    }
    t.sorted = false;
}

/// Register functions compiled at runtime (eval units): (address, size, label offset,
/// [(return offset, loc offset)]) with offsets into `strs`, which must live forever.
pub fn register_jit(funcs: &[(usize, u32, u32, u32, Vec<(u32, u32)>)], strs: *const u8) {
    let mut t = TABLE.write().unwrap();
    for (addr, size, label, file, calls) in funcs {
        // (only read with the table locked; freed when the unit is: `unregister_jit`)
        let cs: &'static [[u32; 2]] = Box::leak(calls.iter().map(|&(r, l)| [r, l]).collect::<Vec<_>>().into_boxed_slice());
        t.funcs.push(Func {
            start: *addr,
            end: addr + *size as usize,
            label: unsafe { strs.add(*label as usize) },
            file: unsafe { strs.add(*file as usize) },
            calls: cs,
            owned: true,
        });
    }
    t.sorted = false;
}

/// Forget eval-unit functions starting at these addresses (their code is being freed).
pub fn unregister_jit(starts: &[usize]) {
    let set: std::collections::HashSet<usize> = starts.iter().copied().collect();
    let mut t = TABLE.write().unwrap();
    t.funcs.retain(|f| {
        if !set.contains(&f.start) {
            return true;
        }
        if f.owned {
            // readers only use a table while holding the lock
            drop(unsafe { Box::from_raw(f.calls as *const [[u32; 2]] as *mut [[u32; 2]]) });
        }
        false
    });
}

fn cstr(p: *const u8) -> String {
    unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_string_lossy().into_owned()
}

#[inline(always)]
fn frame_pointer() -> usize {
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

/// Return addresses of the compiled-code frames on the current stack, innermost first.
#[inline(never)]
pub fn capture() -> Vec<usize> {
    let mut out = Vec::new();
    let Some((lo, hi)) = crate::thread::current_stack() else { return out };
    if !TABLE.read().unwrap().sorted {
        let mut t = TABLE.write().unwrap();
        if !t.sorted {
            t.funcs.sort_by_key(|f| f.start);
            t.sorted = true;
        }
    }
    let t = TABLE.read().unwrap();
    if t.funcs.is_empty() {
        return out;
    }
    let mut fp = frame_pointer();
    let mut n = 0;
    while fp >= lo && fp + 16 <= hi && fp % 8 == 0 && n < 100_000 {
        let next = unsafe { *(fp as *const usize) };
        let ret = unsafe { *((fp + 8) as *const usize) };
        // strip pointer-authentication bits, if any
        let ret = ret & 0x0000_ffff_ffff_ffff;
        if find(&t.funcs, ret).is_some() {
            out.push(ret);
        }
        if next <= fp {
            break;
        }
        fp = next;
        n += 1;
    }
    out
}

fn find(funcs: &[Func], addr: usize) -> Option<&Func> {
    let i = funcs.partition_point(|f| f.start < addr);
    let f = funcs.get(i.checked_sub(1)?)?;
    (addr > f.start && addr <= f.end).then_some(f)
}

/// (path, line, label) of one captured return address.
pub fn symbolize(ret: usize) -> Option<(String, i64, String)> {
    let t = TABLE.read().unwrap();
    let f = find(&t.funcs, ret)?;
    let off = (ret - f.start) as u32;
    let i = f.calls.partition_point(|c| c[0] <= off);
    let line = f.calls.get(i.checked_sub(1)?).or(f.calls.first())?[1];
    Some((cstr(f.file), line as i64, cstr(f.label)))
}

/// "file:line:in 'label'" lines for captured return addresses.
pub fn lines(addrs: &[usize]) -> Vec<String> {
    addrs.iter().filter_map(|&a| symbolize(a)).map(|(p, l, lb)| format!("{p}:{l}:in '{lb}'")).collect()
}

/// Pack return addresses into a binary String (kept on exceptions until `backtrace` is called).
pub fn pack(addrs: &[usize]) -> V {
    let mut b = Vec::with_capacity(addrs.len() * 8);
    for a in addrs {
        b.extend_from_slice(&(*a as u64).to_le_bytes());
    }
    str_new(b)
}

pub fn unpack(s: V) -> Vec<usize> {
    if !is_str(s) {
        return vec![];
    }
    rstr(s).b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap()) as usize).collect()
}

/// Thread::Backtrace::Location objects for captured return addresses.
pub fn locations(addrs: &[usize]) -> V {
    let th = const_lookup(rt().c.object, sym("Thread")).unwrap() as *mut Class;
    let b = const_get_under(th, sym("Backtrace")).unwrap() as *mut Class;
    let lc = const_get_under(b, sym("Location")).unwrap() as *mut Class;
    let mut out = Vec::new();
    for &a in addrs {
        if let Some((p, l, lb)) = symbolize(a) {
            let o = object_new(lc);
            ivar_set(o, sym("@__loc"), str_from(&format!("{p}:{l}")));
            ivar_set(o, sym("@__label"), str_from(&lb));
            out.push(o);
        }
    }
    ary_new(out)
}

/// Exception#backtrace: the strings, made from the captured addresses on first use.
pub fn exc_backtrace(e: V) -> V {
    let b = ivar_get(e, sym("@__backtrace"));
    if b != QNIL || ivar_defined(e, sym("@__backtrace")) {
        return b;
    }
    let raw = ivar_get(e, sym("@__bt_raw"));
    if raw == QNIL {
        return QNIL;
    }
    let lines = ary_new(lines(&unpack(raw)).iter().map(|l| str_from(l)).collect());
    ivar_set(e, sym("@__backtrace"), lines);
    lines
}
