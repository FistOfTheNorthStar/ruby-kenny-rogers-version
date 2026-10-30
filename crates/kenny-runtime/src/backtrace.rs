//! Backtraces without runtime cost: compiled code keeps frame pointers, and the compiler
//! emits a table of every compiled function (address, size, label) with the source line of
//! each of its calls, keyed by the call's return-address offset. Taking a backtrace walks the
//! frame-pointer chain of the current stack and maps each return address that lies in
//! compiled code to "file:line:in 'label'". Runtime (Rust) frames in between are skipped.
//!
//! A call inside a method body the compiler inlined has a location with `INLINE` set: the
//! rest is the offset (in the unit's strings) of a record [line, label, file, outer
//! location] for the inlined method's frame, and the outer location is the call site's (a
//! line, or another record when inlined code was itself inlined). `capture` lists such a
//! return address once per frame, the frame index in the top byte.

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
    /// Only return addresses of `calls` are frames (a fast entry's wrapper: a frame for
    /// its arity error, not for the call into the method it wraps).
    exact: bool,
    /// The unit's strings (inline records).
    strs: *const u8,
    /// The line of its `def` or block, 0 if none (Method/Proc#source_location).
    def_line: u32,
}
/// A call location that is an inline record (see the module docs).
const INLINE: u32 = 1 << 31;
/// Frame index of a captured address (frames of inlined methods at one return address).
const FRAME_SHIFT: u32 = 56;
const ADDR_MASK: usize = (1 << FRAME_SHIFT) - 1;
/// Top bit of a table entry's size: `Func::exact`.
const EXACT: u32 = 1 << 31;
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
    def_line: u32,
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
            end: f.addr + (f.size & !EXACT) as usize,
            label: unsafe { strs.add(f.label as usize) },
            file: unsafe { strs.add(f.file as usize) },
            calls: cs,
            owned: false,
            exact: f.size & EXACT != 0,
            strs,
            def_line: f.def_line,
        });
    }
    t.sorted = false;
}

/// Register functions compiled at runtime (eval units): (address, size, label offset,
/// [(return offset, loc offset)]) with offsets into `strs`, which must live forever.
pub fn register_jit(funcs: &[(usize, u32, u32, u32, Vec<(u32, u32)>, u32)], strs: *const u8) {
    let mut t = TABLE.write().unwrap();
    for (addr, size, label, file, calls, def_line) in funcs {
        // (only read with the table locked; freed when the unit is: `unregister_jit`)
        let cs: &'static [[u32; 2]] = Box::leak(calls.iter().map(|&(r, l)| [r, l]).collect::<Vec<_>>().into_boxed_slice());
        t.funcs.push(Func {
            start: *addr,
            end: addr + (*size & !EXACT) as usize,
            label: unsafe { strs.add(*label as usize) },
            file: unsafe { strs.add(*file as usize) },
            calls: cs,
            owned: true,
            exact: *size & EXACT != 0,
            strs,
            def_line: *def_line,
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
        if let Some(f) = find(&t.funcs, ret) {
            if !f.exact || f.calls.iter().any(|c| f.start + c[0] as usize == ret) {
                // one entry per frame: the inlined methods' ones first
                let n = call_loc(f, ret).map(|l| frames(f, l).len()).unwrap_or(1).min(255);
                for j in 0..n {
                    out.push(ret | (j << FRAME_SHIFT));
                }
            }
        }
        if next <= fp {
            break;
        }
        fp = next;
        n += 1;
    }
    out
}

/// The label of the innermost compiled-code frame on the stack ("A#foo", "block in A#foo",
/// "<main>" ...), cached by return address.
#[inline(never)]
pub fn caller_label() -> Option<String> {
    static CACHE: std::sync::Mutex<Option<rustc_hash::FxHashMap<usize, Option<String>>>> = std::sync::Mutex::new(None);
    let (lo, hi) = crate::thread::current_stack()?;
    let mut fp = frame_pointer();
    let mut n = 0;
    let mut ret_found = None;
    {
        let t = TABLE.read().unwrap();
        if !t.sorted {
            drop(t);
            let mut w = TABLE.write().unwrap();
            if !w.sorted {
                w.funcs.sort_by_key(|f| f.start);
                w.sorted = true;
            }
        }
    }
    let t = TABLE.read().unwrap();
    while fp >= lo && fp + 16 <= hi && fp % 8 == 0 && n < 64 {
        let next = unsafe { *(fp as *const usize) };
        let ret = unsafe { *((fp + 8) as *const usize) } & 0x0000_ffff_ffff_ffff;
        if find(&t.funcs, ret).is_some() {
            ret_found = Some(ret);
            break;
        }
        if next <= fp {
            break;
        }
        fp = next;
        n += 1;
    }
    drop(t);
    let ret = ret_found?;
    if let Some(l) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&ret)) {
        return l.clone();
    }
    let l = symbolize(ret).map(|x| x.2);
    CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(ret, l.clone());
    l
}

fn find(funcs: &[Func], addr: usize) -> Option<&Func> {
    let i = funcs.partition_point(|f| f.start < addr);
    let f = funcs.get(i.checked_sub(1)?)?;
    (addr > f.start && addr <= f.end).then_some(f)
}

/// The location recorded for the call returning to `ret`.
fn call_loc(f: &Func, ret: usize) -> Option<u32> {
    let off = (ret - f.start) as u32;
    let i = f.calls.partition_point(|c| c[0] <= off);
    Some(f.calls.get(i.checked_sub(1)?).or(f.calls.first())?[1])
}

/// The frames (file, line, label) of one call location, innermost first.
fn frames(f: &Func, mut loc: u32) -> Vec<(*const u8, u32, *const u8)> {
    let mut out = Vec::new();
    while loc & INLINE != 0 && out.len() < 64 {
        let p = unsafe { f.strs.add((loc & !INLINE) as usize) } as *const [u32; 4];
        let [line, label, file, outer] = unsafe { std::ptr::read_unaligned(p) };
        out.push((unsafe { f.strs.add(file as usize) }, line, unsafe { f.strs.add(label as usize) }));
        loc = outer;
    }
    out.push((f.file, loc & !INLINE, f.label));
    out
}

/// [file, line] of the `def` or block whose compiled function starts at `addr`
/// (Method/Proc#source_location).
pub fn def_location(addr: usize) -> Option<(String, i64)> {
    if !TABLE.read().unwrap().sorted {
        let mut t = TABLE.write().unwrap();
        if !t.sorted {
            t.funcs.sort_by_key(|f| f.start);
            t.sorted = true;
        }
    }
    let t = TABLE.read().unwrap();
    let i = t.funcs.partition_point(|f| f.start < addr);
    let f = t.funcs.get(i).filter(|f| f.start == addr && f.def_line != 0)?;
    Some((cstr(f.file), f.def_line as i64))
}

/// (path, line, label) of one captured return address.
pub fn symbolize(ret: usize) -> Option<(String, i64, String)> {
    let t = TABLE.read().unwrap();
    let j = ret >> FRAME_SHIFT;
    let ret = ret & ADDR_MASK;
    let f = find(&t.funcs, ret)?;
    let loc = call_loc(f, ret)?;
    let fr = frames(f, loc);
    let (file, line, label) = *fr.get(j).or(fr.last())?;
    Some((cstr(file), line as i64, cstr(label)))
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
