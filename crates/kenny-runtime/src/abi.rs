//! The C ABI that compiled Ruby code calls into. Every function returning `V` may return
//! UNDEF, meaning "a throw is pending" (exception, break, return, throw); compiled code
//! checks for it after every call and unwinds.

#![allow(clippy::missing_safety_doc)]

use crate::core::*;
use crate::kernel::flush_out;
use crate::tri;
use std::ffi::{c_char, CStr};

pub static AT_EXIT: crate::mt::L<Vec<V>> = crate::mt::L::new(Vec::new());

pub fn static_roots(out: &mut Vec<V>) {
    out.extend(AT_EXIT.raw().iter().copied());
    out.extend(crate::server::static_roots());
}

pub fn run_at_exit() {
    while let Some(p) = AT_EXIT.get().pop() {
        let r = call_block(p, &[]);
        if r == UNDEF {
            report_uncaught();
        }
    }
}

/// Value used by keyword-argument helpers for "key not given".
pub const KW_MISSING: V = 0x24;

/// An ivar site's cache: the object shape (class) and the slot index packed into one word
/// (`shape | idx << 48`), so a thread never pairs one shape with another shape's index.
#[repr(C)]
pub struct IvCache {
    pub packed: std::sync::atomic::AtomicU64,
    /// For a class/module receiver (its own ivars): class | position in `civars` << 48.
    pub class_pos: std::sync::atomic::AtomicU64,
}
const IV_PTR: u64 = (1 << 48) - 1;
impl IvCache {
    #[inline(always)]
    fn get(&self, shape: *mut Class) -> Option<usize> {
        let w = self.packed.load(std::sync::atomic::Ordering::Relaxed);
        if w != 0 && (w & IV_PTR) == shape as u64 { Some((w >> 48) as usize) } else { None }
    }
    #[inline(always)]
    fn set(&self, shape: *mut Class, idx: u32) {
        if idx < (1 << 16) && (shape as u64) <= IV_PTR {
            self.packed.store(shape as u64 | ((idx as u64) << 48), std::sync::atomic::Ordering::Relaxed);
        }
    }
}

// ---------- program entry ----------

const STACK_SIZE: usize = 256 << 20;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_start(
    argc: i32,
    argv: *const *const c_char,
    symnames: *const u8,
    nsyms: u64,
    symtab: *mut V,
    toplevel: CompiledFn,
    script: *const c_char,
    caches: *mut V,
    cache_words: u64,
) -> i32 {
    let args: Vec<String> = (0..argc as usize)
        .map(|i| unsafe { CStr::from_ptr(*argv.add(i)).to_string_lossy().into_owned() })
        .collect();
    let script = unsafe { CStr::from_ptr(script).to_string_lossy().into_owned() };
    let symnames = symnames as usize;
    let symtab = symtab as usize;
    let caches = caches as usize;
    let handle = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let x = 0u8;
            init_runtime_at((&x as *const u8 as usize) + 256);
            crate::gc::heap().extra_roots.push(static_roots);
            crate::gc::register_region(caches, cache_words as usize);
            let r = rt();
            set_stack_limit((&x as *const u8 as usize).saturating_sub(STACK_SIZE - (4 << 20)));
            r.script_name = script.clone();
            // intern compiled-in symbols in order
            let mut p = symnames as *const u8;
            for i in 0..nsyms as usize {
                // u32 LE length, then the bytes (names may contain NUL)
                let len = u32::from_le_bytes(unsafe { std::ptr::read_unaligned(p as *const [u8; 4]) }) as usize;
                let bytes = unsafe { std::slice::from_raw_parts(p.add(4), len) };
                unsafe { *(symtab as *mut V).add(i) = mksym(intern(bytes)) };
                p = unsafe { p.add(4 + len) };
            }
            let argv_v = ary_new(args.iter().skip(1).map(|a| str_from(a)).collect());
            set_global_const("ARGV", argv_v);
            r.gvars.insert(sym("$0"), str_from(&script));
            r.gvars.insert(sym("$PROGRAM_NAME"), str_from(&script));
            r.gvars.insert(sym("$,"), QNIL);
            r.gvars.insert(sym("$/"), str_from("\n"));
            static CODE: G<i32> = G::new(0);
            let main_obj = r.main_obj;
            crate::thread::run_main(Box::new(move || {
                let res = unsafe { toplevel(main_obj, 0, std::ptr::null(), QNIL) };
                let mut code = 0;
                if res == UNDEF {
                    match pst().pending {
                        // (a top-level `return`, also from a block; one for a method that has
                        // already returned is reported below)
                        Throw::Return { tag: 0, .. } | Throw::Kill => pst().pending = Throw::None,
                        _ => {
                            code = report_uncaught();
                        }
                    }
                }
                if code == 0 {
                    if let Some(c) = crate::server::maybe_serve() {
                        code = c;
                    }
                }
                run_at_exit();
                flush_out();
                *CODE.get() = code;
                QNIL
            }));
            if !matches!(pst().pending, Throw::None) {
                let c = report_uncaught();
                if c != 0 {
                    *CODE.get() = c;
                }
            }
            flush_out();
            *CODE.get()
        })
        .expect("failed to start main thread");
    handle.join().unwrap_or(1)
}

fn report_uncaught() -> i32 {
    let r = rt();
    let pending = std::mem::replace(&mut pst().pending, Throw::None);
    let msg = match pending {
        Throw::Raise(e) => {
            if is_a(e, r.c.exception) && class_name(real_class(e)) == "SystemExit" {
                return 0;
            }
            let m = send0(e, "message");
            let m = if is_str(m) { String::from_utf8_lossy(&rstr(m).b).into_owned() } else { String::new() };
            let bt = crate::backtrace::exc_backtrace(e);
            let lines: Vec<String> = if is_ary(bt) {
                rary(bt).v.iter().filter(|x| is_str(**x)).map(|x| String::from_utf8_lossy(&rstr(*x).b).into_owned()).collect()
            } else {
                vec![]
            };
            let loc = lines.first().cloned().unwrap_or_else(|| r.script_name.clone());
            let mut out = format!("{}: {} ({})\n", loc, m, class_name(real_class(e)));
            for l in lines.iter().skip(1) {
                out.push_str(&format!("\tfrom {l}\n"));
            }
            out
        }
        Throw::Throw { tag, .. } => {
            format!("{}: uncaught throw {} (UncaughtThrowError)\n", r.script_name, crate::fmt::inspect_lossy(tag))
        }
        Throw::Break { .. } => format!("{}: break from proc-closure (LocalJumpError)\n", r.script_name),
        // a proc's `return` for a method that had already returned (nothing caught it)
        Throw::Return { tag, .. } if tag != 0 => format!("{}: unexpected return (LocalJumpError)\n", r.script_name),
        Throw::Return { .. } | Throw::None | Throw::StopLoop(_) | Throw::Kill => return 0,
    };
    crate::kernel::err_out(msg.as_bytes());
    1
}

// ---------- calls ----------

#[inline(always)]
pub(crate) unsafe fn dispatch(cache: *mut CallCache, recv: V, mid: V) -> *const Method {
    let k = class_of(recv);
    let c = unsafe { &mut *cache };
    let serial = serial();
    let e = c.entry.load(std::sync::atomic::Ordering::Acquire);
    if !e.is_null() {
        let e = unsafe { &*e };
        if e.serial == serial {
            if e.klass[0] == k {
                return e.method[0];
            }
            for i in 1..e.n.min(IC_WAYS) {
                if e.klass[i] == k {
                    return e.method[i];
                }
            }
        }
    }
    unsafe { dispatch_miss(c, e, k, serial, mid) }
}
/// `dispatch` for callers that still need `cache` afterwards (`opt::kn_attr_get`): the miss
/// path hands it back, so the common path needn't keep it in a saved register.
#[inline(always)]
pub(crate) unsafe fn dispatch_keep(cache: *mut CallCache, recv: V, mid: V) -> (*const Method, *mut CallCache) {
    let k = class_of(recv);
    let c = unsafe { &mut *cache };
    let serial = serial();
    let e = c.entry.load(std::sync::atomic::Ordering::Acquire);
    if !e.is_null() {
        let e = unsafe { &*e };
        if e.serial == serial {
            if e.klass[0] == k {
                return (e.method[0], cache);
            }
            for i in 1..e.n.min(IC_WAYS) {
                if e.klass[i] == k {
                    return (e.method[i], cache);
                }
            }
        }
    }
    unsafe { dispatch_miss_keep(cache, e, k, serial, mid) }
}
#[inline(never)]
unsafe fn dispatch_miss_keep(cache: *mut CallCache, e: *mut CacheEntry, k: *mut Class, serial: u64, mid: V) -> (*const Method, *mut CallCache) {
    (unsafe { dispatch_miss(&mut *cache, e, k, serial, mid) }, cache)
}

/// Free the entries that an eval unit's call-site and constant caches point to (the unit is
/// being freed: nothing can run its code). `owned` lists (word offset, kind): 1 = CallCache,
/// 2 = ConstCache (kenny_compiler::OWNED_CALL / OWNED_CONST).
pub fn free_owned_caches(caches: usize, owned: &[(u32, u8)]) {
    use std::sync::atomic::Ordering;
    crate::opt::forget_attr_sites(owned.iter().filter(|(_, k)| *k == 1).map(|(off, _)| caches + *off as usize * 8));
    for &(off, kind) in owned {
        let p = caches + off as usize * 8;
        match kind {
            1 => {
                let e = unsafe { &*(p as *const CallCache) }.entry.swap(std::ptr::null_mut(), Ordering::AcqRel);
                if !e.is_null() {
                    drop(unsafe { Box::from_raw(e) });
                }
            }
            2 => {
                let e = unsafe { &*(p as *const ConstCache) }.entry.swap(std::ptr::null_mut(), Ordering::AcqRel);
                if !e.is_null() {
                    drop(unsafe { Box::from_raw(e) });
                }
            }
            _ => {}
        }
    }
}

/// The entry `old` (if still current) with (k, m) added; a full entry drops its oldest class.
fn ic_with(old: *mut CacheEntry, serial: u64, k: *mut Class, m: *const Method) -> CacheEntry {
    let mut e = CacheEntry { serial, n: 0, klass: [std::ptr::null_mut(); IC_WAYS], method: [std::ptr::null(); IC_WAYS], direct: [0; IC_WAYS], attr: [0; IC_WAYS] };
    if !old.is_null() {
        let o = unsafe { &*old };
        if o.serial == serial {
            let keep = o.n.min(IC_WAYS - 1);
            // newest first; keep the most recent others
            for i in 0..keep {
                e.klass[i + 1] = o.klass[i];
                e.method[i + 1] = o.method[i];
                e.direct[i + 1] = o.direct[i];
                e.attr[i + 1] = o.attr[i];
            }
            e.n = keep + 1;
        }
    }
    if e.n == 0 {
        e.n = 1;
    }
    e.klass[0] = k;
    e.method[0] = m;
    e.direct[0] = direct_of(m);
    e.attr[0] = attr_of(k, m);
    e
}
#[inline(never)]
unsafe fn dispatch_miss(c: &mut CallCache, e: *mut CacheEntry, k: *mut Class, serial: u64, mid: V) -> *const Method {
    if DIRECT_FLAGS_DIRTY.load(std::sync::atomic::Ordering::Relaxed) {
        update_direct_flags();
    }
    let m = find_method(k, sym_id(mid));
    if !crate::mt::multi() {
        let ne = ic_with(e, serial, k, m);
        if e.is_null() {
            c.entry.store(Box::into_raw(Box::new(ne)), std::sync::atomic::Ordering::Release);
        } else {
            unsafe { *e = ne };
        }
    } else if c.misses < 64 {
        c.misses += 1;
        let ne = Box::into_raw(Box::new(ic_with(e, serial, k, m)));
        let old = c.entry.swap(ne, std::sync::atomic::Ordering::AcqRel);
        if !old.is_null() {
            crate::mt::retire_deferred(old as usize, std::mem::size_of::<CacheEntry>(), std::mem::align_of::<CacheEntry>());
        }
    }
    m
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send(cache: *mut CallCache, recv: V, mid: V, argc: u64, argv: *const V, blk: V) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    let args = unsafe { args_slice(argc, argv) };
    if m.is_null() {
        return method_missing(recv, sym_id(mid), args, blk, false);
    }
    invoke(m, recv, args, blk)
}

/// A bare identifier that turned out not to be a local variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_vcall(cache: *mut CallCache, recv: V, mid: V) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    if m.is_null() {
        return method_missing(recv, sym_id(mid), &[], QNIL, true);
    }
    unsafe { crate::opt::call_found(m, recv, mid, &[], 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send_splat(cache: *mut CallCache, recv: V, mid: V, args: V, blk: V) -> V {
    let a = rary(args).v.clone();
    unsafe { kn_send(cache, recv, mid, a.len() as u64, a.as_ptr(), blk) }
}

/// A `def`'s owner slot (where `super` starts looking) once one definition site has defined
/// methods in more than one class (a `def` run in a loop or a `Module.new` block, or a method
/// copied by `Module#dup`): the owner is then found from the running method's code.
pub const SHARED_OWNER: V = 1;

/// The compiled function of each `def` site's owner slot, and back (see `super_owner`).
static SLOT_FNS: crate::mt::L<std::collections::HashMap<usize, usize, FxBuild>> = crate::mt::L::new(std::collections::HashMap::with_hasher(FxBuild::new()));
static FN_SLOTS: crate::mt::L<std::collections::HashMap<usize, usize, FxBuild>> = crate::mt::L::new(std::collections::HashMap::with_hasher(FxBuild::new()));

/// `Module#dup` copied a compiled method: its `super` can no longer use the slot's owner.
pub fn share_owner_of(f: CompiledFn) {
    if let Some(&slot) = FN_SLOTS.get().get(&(f as usize)) {
        unsafe { *(slot as *mut V) = SHARED_OWNER };
    }
}

/// A freed eval unit's functions: drop their owner-slot entries.
pub fn forget_defs(funcs: &[usize]) {
    let mut fs = FN_SLOTS.get();
    let mut ss = SLOT_FNS.get();
    for f in funcs {
        if let Some(slot) = fs.remove(f) {
            if ss.get(&slot) == Some(f) {
                ss.remove(&slot);
            }
        }
    }
}

/// The class whose method is running a `super` (where the lookup continues from).
fn super_owner(owner_slot: *const V, self_: V, mid: u32) -> *mut Class {
    let owner = unsafe { *owner_slot };
    if owner != SHARED_OWNER {
        return owner as *mut Class;
    }
    if let Some(f) = SLOT_FNS.get().get(&(owner_slot as usize)).copied() {
        for &k in ancestors_rc(class_of(self_)).iter() {
            if let Some(&m) = unsafe { (*k).methods.get(&mid) } {
                if matches!(unsafe { &(*m).body }, Body::Compiled(g) if *g as usize == f) {
                    return k;
                }
            }
        }
    }
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_super(owner_slot: *const V, self_: V, mid: V, argc: u64, argv: *const V, blk: V) -> V {
    let owner = super_owner(owner_slot, self_, sym_id(mid));
    let args = unsafe { args_slice(argc, argv) };
    let m = find_super_method(self_, owner, sym_id(mid));
    if m.is_null() {
        if sym_str(sym_id(mid)) == "respond_to_missing?" {
            return QFALSE;
        }
        // like Ruby: a missing superclass method goes to method_missing (SimpleDelegator
        // subclasses rely on it: `def path = super.spec.to_s` in Rails' RouteWrapper)
        let mm = find_method(class_of(self_), sym("method_missing"));
        if !mm.is_null() && !matches!(unsafe { &(*mm).body }, Body::Native(..)) {
            let mut a = Vec::with_capacity(args.len() + 1);
            a.push(mid);
            a.extend_from_slice(args);
            return invoke(mm, self_, &a, blk);
        }
        return raise(
            rt().c.no_method_error,
            &format!("super: no superclass method '{}' for {}", sym_str(sym_id(mid)), describe_recv(self_)),
        );
    }
    invoke(m, self_, args, blk)
}

/// `defined?(super)`: 1 if a superclass method exists.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_super_defined(owner_slot: *const V, self_: V, mid: V) -> u64 {
    let owner = super_owner(owner_slot, self_, sym_id(mid));
    (!find_super_method(self_, owner, sym_id(mid)).is_null()) as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_super_splat(owner_slot: *const V, self_: V, mid: V, args: V, blk: V) -> V {
    let a = rary(args).v.clone();
    unsafe { kn_super(owner_slot, self_, mid, a.len() as u64, a.as_ptr(), blk) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_yield(blk: V, argc: u64, argv: *const V) -> V {
    call_block(blk, unsafe { args_slice(argc, argv) })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_yield_splat(blk: V, args: V) -> V {
    let a = rary(args).v.clone();
    call_block(blk, &a)
}

// ---------- literals ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_str_lit(slot: *mut V, ptr: *const u8, len: u64) -> V {
    let s = unsafe { &mut *slot };
    if *s != 0 {
        return *s;
    }
    let v = str_new(unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec());
    hdr(v).flags |= FL_FROZEN;
    crate::gc::pin(v);
    *s = v;
    v
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_str_new(ptr: *const u8, len: u64) -> V {
    str_new(unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_str_append_raw(s: V, ptr: *const u8, len: u64) -> V {
    rstr(s).b.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len as usize) });
    s
}
/// A String literal with interpolation starts as this: empty, room for `cap` bytes (see
/// `kn_str_interp_raw`, `kn_str_interp`).
#[unsafe(no_mangle)]
pub extern "C" fn kn_str_buf(cap: u64) -> V {
    str_new(Vec::with_capacity(cap as usize))
}
/// A literal part of an interpolation into `s` (from `kn_str_buf`, seen by no other code
/// until done, so `extend_private`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_str_interp_raw(s: V, ptr: *const u8, len: u64) -> V {
    rstr(s).b.extend_private(unsafe { std::slice::from_raw_parts(ptr, len as usize) });
    s
}
/// "#{v}" into `s` (as `kn_str_interp_raw`): appends v.to_s, Strings, Integers, Symbols
/// and nil without a copy in between.
#[unsafe(no_mangle)]
pub extern "C" fn kn_str_interp(s: V, v: V) -> V {
    let b = &mut rstr(s).b;
    if is_fix(v) {
        let mut buf = [0u8; 24];
        let mut i = buf.len();
        let n = fix(v);
        let mut m = n.unsigned_abs();
        loop {
            i -= 1;
            buf[i] = b'0' + (m % 10) as u8;
            m /= 10;
            if m == 0 {
                break;
            }
        }
        if n < 0 {
            i -= 1;
            buf[i] = b'-';
        }
        b.extend_private(&buf[i..]);
        return s;
    }
    if v == QNIL {
        return s;
    }
    if is_sym(v) {
        b.extend_private(sym_name(sym_id(v)));
        return s;
    }
    if kind(v) == Some(Kind::String) {
        b.extend_private(rstr(v).b.as_slice());
        return s;
    }
    match crate::fmt::to_s(v) {
        Ok(x) => {
            rstr(s).b.extend_private(&x);
            s
        }
        Err(()) => UNDEF,
    }
}
/// "#{v}" — appends v.to_s
#[unsafe(no_mangle)]
pub extern "C" fn kn_str_append(s: V, v: V) -> V {
    match crate::fmt::to_s(v) {
        Ok(b) => {
            rstr(s).b.extend_from_slice(&b);
            s
        }
        Err(()) => UNDEF,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_str_to_sym(s: V) -> V {
    mksym(intern(&rstr(s).b.clone()))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_float_lit(slot: *mut V, bits: u64) -> V {
    let s = unsafe { &mut *slot };
    if *s == 0 {
        let v = float_new(f64::from_bits(bits));
        crate::gc::pin(v);
        *s = v;
    }
    *s
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_int_lit(slot: *mut V, ptr: *const u8, len: u64) -> V {
    let s = unsafe { &mut *slot };
    if *s == 0 {
        let txt = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        let v = crate::kernel::parse_int(txt, true).unwrap_or(mkfix(0));
        crate::gc::pin(v);
        *s = v;
    }
    *s
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_ary_new(argc: u64, argv: *const V) -> V {
    ary_new(unsafe { args_slice(argc, argv) }.to_vec())
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_push(a: V, v: V) -> V {
    rary(a).v.push(v);
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_concat_splat(a: V, v: V) -> V {
    let items = tri!(kn_splat_to_ary(v));
    let items = rary(items).v.clone();
    rary(a).v.extend(items);
    a
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_hash_new(n: u64, argv: *const V, kw: u64) -> V {
    let h = hash_new();
    let a = unsafe { args_slice(n, argv) };
    rhash(h).m.reserve_empty(a.len() / 2);
    for ch in a.chunks(2) {
        rhash(h).m.insert(HKey(crate::hash::key_dup(ch[0])), ch[1]);
    }
    if kw != 0 {
        hdr(h).flags |= FL_KWHASH;
    }
    h
}
/// A hash literal whose keys are distinct Symbol literals: no key copies or duplicate checks.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_hash_new_syms(n: u64, argv: *const V, kw: u64) -> V {
    let h = hash_new();
    let a = unsafe { args_slice(n, argv) };
    let m = &mut rhash(h).m;
    m.reserve_empty(a.len() / 2);
    for ch in a.chunks(2) {
        m.push_distinct(ch[0], ch[1]);
    }
    if kw != 0 {
        hdr(h).flags |= FL_KWHASH;
    }
    h
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_hash_merge_splat(h: V, v: V) -> V {
    if v == QNIL {
        return h;
    }
    if !is_hash(v) {
        return type_error(&format!("no implicit conversion of {} into Hash", class_name(real_class(v))));
    }
    for (k, x) in crate::hash::pairs(v) {
        rhash(h).m.insert(HKey(k), x);
    }
    h
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_range_new(lo: V, hi: V, excl: u64) -> V {
    range_new(lo, hi, excl != 0)
}

// ---------- blocks / closures ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_proc_new(func: BlockFn, env: *mut Env, self_: V, min: i64, max: i64, flags: u64) -> V {
    let p = proc_new_compiled(func, env, self_, min as i32, max as i32, flags & 1 != 0, flags & 2 != 0);
    if flags & 4 != 0 {
        hdr(p).flags |= FL_PROC_KW;
    }
    p
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_env_new(parent: *mut Env, blk: V, nslots: u64) -> *mut Env {
    let words = 2 + nslots as usize;
    // a GC object when it fits a slot: [header][parent, blk, slots...]
    if 16 + 8 * words <= crate::gc::MAX_SLOT {
        let p = crate::gc::alloc_slot(16 + 8 * words);
        unsafe {
            std::ptr::write(p as *mut Header, Header { klass: nslots as usize as *mut Class, kind: Kind::Env, flags: 0 });
            let d = (p + 16) as *mut V;
            *d = parent as V;
            *d.add(1) = blk;
            for i in 0..nslots as usize {
                *d.add(2 + i) = QNIL;
            }
            return d as *mut Env;
        }
    }
    // larger: malloc'd, with the same header in front (never old: always scanned as a root)
    let mut v: Vec<V> = vec![QNIL; words + 2];
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    unsafe {
        std::ptr::write(p as *mut Header, Header { klass: nslots as usize as *mut Class, kind: Kind::Env, flags: 0 });
        let d = p.add(2);
        *d = parent as V;
        *d.add(1) = blk;
        crate::gc::register_env(d as usize, nslots as u32);
        d as *mut Env
    }
}
/// `v.is_a?(c)` for the builtin is_a?/kind_of? (compiled code checked that it is).
#[unsafe(no_mangle)]
pub extern "C" fn kn_is_a(v: V, c: V) -> V {
    is_a_v(v, c)
}
/// `recv.is_a?(c)` / `recv.kind_of?(c)` at a call site whose inline cache missed
/// (kenny-compiler fastpath.rs `STUBS`): a heap receiver's is_a? is the builtin one while
/// `IS_A_TOUCHED` is unset, so a site that sees many classes skips the method lookup.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_is_a_send(cache: *mut CallCache, recv: V, mid: V, c: V, explicit: u64) -> V {
    use std::sync::atomic::Ordering::Relaxed;
    if is_heap(recv) && is_class(c) && crate::core::DIRECT_READY.load(Relaxed) && !crate::core::IS_A_TOUCHED.load(Relaxed) {
        return bool_v(is_a(recv, c as *mut Class));
    }
    unsafe { crate::opt::kn_send1(cache, recv, mid, c, explicit) }
}
/// `kn_is_a` for a heap receiver at a call site whose cache entry has the builtin is_a? for
/// the receiver's class: also records the answer for `c` in that way's `attr` word
/// (`c << 16 | answer << 5 | ATTR_IS_A | private`), which the stub then uses. (Only while
/// one OS thread runs Ruby code: entries are changed in place.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_is_a_fill(cache: *mut CallCache, v: V, c: V) -> V {
    if !is_class(c) {
        return is_a_v(v, c);
    }
    let r = is_a(v, c as *mut Class);
    if is_class(c) && (c as u64) < 1 << 47 && !crate::mt::multi() {
        let e = unsafe { (*cache).entry.load(std::sync::atomic::Ordering::Acquire) };
        if !e.is_null() {
            let e = unsafe { &mut *e };
            let k = hdr(v).klass;
            for w in 0..e.n.min(IC_WAYS) {
                if e.klass[w] == k && e.attr[w] & 0x1e == ATTR_IS_A {
                    e.attr[w] = (c as u64) << 16 | (r as u64) << 5 | ATTR_IS_A | (e.attr[w] & 1);
                }
            }
        }
    }
    bool_v(r)
}
/// A static call site's slot ([class, serial], kenny-compiler statics.rs): filled when the
/// receiver's class has the method whose `Method::fast` is `expect` (one OS thread only).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_static_fill(slot: *mut [u64; 2], recv: V, mid: V, expect: usize) -> V {
    if !is_heap(recv) || crate::mt::multi() {
        return QNIL;
    }
    // top bit: a call with a receiver (only a public method will do); bit 62: `expect` is
    // the address of the method's parameter descriptor (a keyword def), else its fast entry
    let public = expect >> 63 != 0;
    let by_params = (expect >> 62) & 1 != 0;
    let expect = expect & !(3 << 62);
    let k = hdr(recv).klass;
    let m = find_method(k, sym_id(mid));
    let have = if m.is_null() { 0 } else if by_params { unsafe { (*m).params as usize } } else { unsafe { (*m).fast } };
    let same = have != 0 && have == expect;
    if same && (!public || unsafe { (*m).vis } == VIS_PUBLIC) {
        unsafe {
            (*slot)[0] = k as u64;
            (*slot)[1] = KN_DIRECT.serial.load(std::sync::atomic::Ordering::Acquire);
        }
    }
    QNIL
}
/// On entry to a compiled method that calls itself (kenny-compiler statics.rs
/// `entry_check`): the stack below `KN_DIRECT.entry_limit` or a collection pending.
#[unsafe(no_mangle)]
pub extern "C" fn kn_entry_slow() -> V {
    if crate::gc::safepoint() {
        return UNDEF;
    }
    if crate::core::approx_sp() < pst().stack_limit {
        return raise(rt().c.system_stack_error, "stack level too deep");
    }
    QNIL
}
/// On entry to an integer copy of a method (kenny-compiler ispec.rs) with the stack below
/// `KN_DIRECT.entry_limit` or a collection pending: 1 to give up on the copy (the normal
/// method then runs and reports a deep stack), UNDEF for an exception delivered, else 0.
#[unsafe(no_mangle)]
pub extern "C" fn kn_ispec_poll() -> V {
    if crate::core::approx_sp() < crate::core::KN_DIRECT.entry_limit.load(std::sync::atomic::Ordering::Relaxed) {
        return 1;
    }
    if crate::gc::safepoint() {
        return UNDEF;
    }
    0
}
/// For an inlined `C.new` (kenny-compiler statics.rs `inline_new`): fill `slot` with
/// [C, method serial] if C makes plain objects, its `new` is the builtin Class#new and its
/// `initialize` has the fast entry `expect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_new_fill(slot: *mut [u64; 2], cls: V, expect: usize) -> V {
    if !is_class(cls) || crate::mt::multi() {
        return QNIL;
    }
    let k = cls as *mut Class;
    if unsafe { (*k).is_module || !matches!((*k).alloc, Kind::Object) } || crate::capi::alloc_func(k).is_some() {
        return QNIL;
    }
    static IDS: std::sync::OnceLock<[u32; 2]> = std::sync::OnceLock::new();
    let [new_id, init_id] = *IDS.get_or_init(|| [sym("new"), sym("initialize")]);
    let nm = find_method(hdr(cls).klass, new_id);
    let im = find_method(k, init_id);
    if nm.is_null() || unsafe { (*nm).opt } != crate::opt::OPT_CLASS_NEW || im.is_null() || unsafe { (*im).fast } != expect {
        return QNIL;
    }
    unsafe {
        (*slot)[0] = cls as u64;
        (*slot)[1] = KN_DIRECT.serial.load(std::sync::atomic::Ordering::Acquire);
    }
    QNIL
}
/// For a virtual object (kenny-compiler vobj.rs): `kn_new_fill`, when also each getter in
/// `desc` (`n` pairs [name, expect]) is a public method of `cls` that is the attr_reader of
/// the instance variable `expect` (a Symbol) or has the fast entry `expect`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_vobj_fill(slot: *mut [u64; 2], cls: V, expect: usize, desc: *const V, n: u64) -> V {
    if !is_class(cls) || crate::mt::multi() {
        return QNIL;
    }
    let k = cls as *mut Class;
    for i in 0..n as usize {
        let (name, e) = unsafe { (*desc.add(2 * i), *desc.add(2 * i + 1)) };
        let m = find_method(k, sym_id(name));
        if m.is_null() || unsafe { (*m).vis } != VIS_PUBLIC {
            return QNIL;
        }
        let ok = if is_sym(e) {
            matches!(unsafe { &(*m).body }, Body::AttrReader(id) if *id == sym_id(e))
        } else {
            unsafe { (*m).fast == e as usize }
        };
        if !ok {
            return QNIL;
        }
    }
    unsafe { kn_new_fill(slot, cls, expect) }
}
/// A plain object of class `cls` (an inlined `C.new`; `kn_new_fill` checked the class).
#[unsafe(no_mangle)]
pub extern "C" fn kn_obj_alloc(cls: V) -> V {
    object_new(cls as *mut Class)
}
/// Write barrier after compiled code stores into an old (GC-heap) environment.
#[unsafe(no_mangle)]
pub extern "C" fn kn_wb_env(env: *mut Env) -> V {
    crate::gc::wb_obj(env as usize as V - 16);
    QNIL
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_to_proc(v: V) -> V {
    if v == QNIL || is_proc(v) {
        return v;
    }
    if is_sym(v) {
        return crate::string::sym_to_proc(sym_id(v));
    }
    let p = tri!(send0(v, "to_proc"));
    if !is_proc(p) {
        return type_error(&format!("wrong argument type {} (expected Proc)", class_name(real_class(v))));
    }
    p
}

// ---------- variables ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_ivar_get(self_: V, id: V, cache: *mut IvCache) -> V {
    if is_kind(self_, Kind::Object) {
        let o = obj(self_);
        let c = unsafe { &*cache };
        if let Some(i) = c.get(o.shape) {
            return o.ivars.get(i).copied().filter(|x| *x != UNDEF).unwrap_or(QNIL);
        }
        let shape = unsafe { &*o.shape };
        if let Some(i) = shape_index(shape, sym_id(id)) {
            c.set(o.shape, i as u32);
            return o.ivars.get(i as usize).copied().filter(|x| *x != UNDEF).unwrap_or(QNIL);
        }
        return QNIL;
    }
    if is_kind(self_, Kind::Class) {
        // a class-level ivar: its position in the class's table, checked against the key
        let c = unsafe { &*cache };
        let iv = &rclass(self_).civars;
        let w = c.class_pos.load(std::sync::atomic::Ordering::Relaxed);
        let key = sym_id(id);
        if w != 0 && (w & IV_PTR) == self_ {
            if let Some(v) = iv.get_at((w >> 48) as usize, &key) {
                return v;
            }
        }
        return match iv.index_of(&key) {
            Some(i) => {
                if i < 1 << 16 && self_ <= IV_PTR {
                    c.class_pos.store(self_ | (i as u64) << 48, std::sync::atomic::Ordering::Relaxed);
                }
                iv.get_at(i, &key).unwrap_or(QNIL)
            }
            None => QNIL,
        };
    }
    ivar_get(self_, sym_id(id))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_ivar_set(self_: V, id: V, val: V, cache: *mut IvCache) -> V {
    if is_kind(self_, Kind::Object) && hdr(self_).flags & FL_FROZEN == 0 {
        let o = obj(self_);
        let c = unsafe { &*cache };
        let i = match c.get(o.shape) {
            Some(i) => i,
            None => {
                let i = ivar_index_for(o.shape, sym_id(id));
                c.set(o.shape, i);
                i as usize
            }
        };
        if o.ivars.len() <= i {
            o.ivars.resize(i + 1, QNIL);
        }
        o.ivars[i] = val;
        return val;
    }
    ivar_set(self_, sym_id(id), val)
}
/// `alias $new $old` (English.rb and friends): new name -> resolved old name.
static GV_ALIASES: std::sync::RwLock<Vec<(u32, u32)>> = std::sync::RwLock::new(Vec::new());
static GV_HAS_ALIASES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn gvar_resolve(id: u32) -> u32 {
    if !GV_HAS_ALIASES.load(std::sync::atomic::Ordering::Acquire) {
        return id;
    }
    let a = GV_ALIASES.read().unwrap_or_else(|e| e.into_inner());
    a.iter().find(|(n, _)| *n == id).map(|(_, o)| *o).unwrap_or(id)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_gvar_alias(new: V, old: V) -> V {
    let (n, o) = (sym_id(new), gvar_resolve(sym_id(old)));
    if n != o {
        let mut a = GV_ALIASES.write().unwrap_or_else(|e| e.into_inner());
        a.retain(|(x, _)| *x != n);
        // names that pointed at `new` follow it to `old`
        for e in a.iter_mut() {
            if e.1 == n {
                e.1 = o;
            }
        }
        a.push((n, o));
        GV_HAS_ALIASES.store(true, std::sync::atomic::Ordering::Release);
    }
    QNIL
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_gvar_get(id: V) -> V {
    let id = gvar_resolve(sym_id(id));
    // `$~` and `$!` belong to the running thread
    match sym_name(id) {
        b"$~" => return pst().last_match,
        b"$!" => return pst().errinfo,
        b"$@" => {
            let e = pst().errinfo;
            return if e == QNIL { QNIL } else { send0(e, "backtrace") };
        }
        b"$&" => return crate::regex::kn_backref(b'&' as u64),
        b"$`" => return crate::regex::kn_backref(b'`' as u64),
        b"$'" => return crate::regex::kn_backref(b'\'' as u64),
        b"$+" => return crate::regex::kn_backref(b'+' as u64),
        _ => {}
    }
    rt().gvars.get(&id).copied().unwrap_or(QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_gvar_set(id: V, v: V) -> V {
    rt().gvars.insert(gvar_resolve(sym_id(id)), v);
    v
}
fn cvar_base(c: *mut Class) -> *mut Class {
    unsafe {
        if (*c).is_singleton && is_class((*c).attached) {
            (*c).attached as *mut Class
        } else {
            c
        }
    }
}
pub fn cvar_get(c: *mut Class, id: u32) -> V {
    let c = cvar_base(c);
    for a in ancestors(c) {
        if let Some(&v) = unsafe { (*a).cvars.get(&id) } {
            return v;
        }
    }
    raise(rt().c.name_error, &format!("uninitialized class variable {} in {}", sym_str(id), class_name(c)))
}
pub fn cvar_set(c: *mut Class, id: u32, v: V) -> V {
    let c = cvar_base(c);
    for a in ancestors(c) {
        if unsafe { (*a).cvars.contains_key(&id) } {
            unsafe { (*a).cvars.insert(id, v) };
            return v;
        }
    }
    unsafe { (*c).cvars.insert(id, v) };
    v
}
fn cref_class(cref: V) -> *mut Class {
    if is_class(cref) { cref as *mut Class } else { rt().c.object }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_cvar_get(cref: V, id: V) -> V {
    cvar_get(cref_class(cref), sym_id(id))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_cvar_set(cref: V, id: V, v: V) -> V {
    cvar_set(cref_class(cref), sym_id(id), v)
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_cvar_defined(cref: V, id: V) -> u64 {
    let c = cvar_base(cref_class(cref));
    ancestors(c).iter().any(|&a| unsafe { (*a).cvars.contains_key(&sym_id(id)) }) as u64
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_cvar_get_soft(cref: V, id: V) -> V {
    let c = cvar_base(cref_class(cref));
    for a in ancestors(c) {
        if let Some(&v) = unsafe { (*a).cvars.get(&sym_id(id)) } {
            return v;
        }
    }
    QNIL
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_object_class() -> V {
    rt().c.object as V
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_main_obj() -> V {
    rt().main_obj
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_len(a: V) -> i64 {
    rary(a).v.len() as i64
}

// ---------- constants & definitions ----------

struct Autoload {
    loader: Option<CompiledFn>,
    flag: *mut V,
    path: String,
}

/// Autoloads running right now (see settle_autoload).
static LOADING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A constant found in `k` while another thread is still running the autoload that defines it
/// (its `class` statement has run, the body maybe not): wait for that load to finish first.
/// CRuby makes other threads wait for an autoload too, so they never see a half-built class
/// (websocket-extensions' Parser without its methods, Action Cable's TaggedLoggerProxy).
pub fn settle_autoload(k: *mut Class, id: u32) {
    if LOADING.load(std::sync::atomic::Ordering::Acquire) == 0 {
        return;
    }
    let entry = autoloads().get(&(k as usize, id)).map(|a| (a.loader, a.flag, a.path.clone()));
    let Some((loader, flag, path)) = entry else { return };
    wait_for_autoload(loader, flag, &path);
}
/// Like settle_autoload, for a constant found in a scope this code didn't track (the
/// compiler's lexical lookup): any autoload of that name another thread is running.
pub fn settle_id(id: u32) {
    if LOADING.load(std::sync::atomic::Ordering::Acquire) == 0 {
        return;
    }
    let entries: Vec<(Option<CompiledFn>, *mut V, String)> =
        autoloads().iter().filter(|((_, i), _)| *i == id).map(|(_, a)| (a.loader, a.flag, a.path.clone())).collect();
    for (loader, flag, path) in entries {
        wait_for_autoload(loader, flag, &path);
    }
}
fn wait_for_autoload(loader: Option<CompiledFn>, flag: *mut V, path: &str) {
    let (f, flag) = match loader {
        Some(f) => (f, flag),
        None => match crate::compat::compiled_entry(path) {
            Some(e) => e,
            None => return,
        },
    };
    let st = unsafe { &*(flag as *const std::sync::atomic::AtomicU64) }.load(std::sync::atomic::Ordering::Acquire);
    if st == 0 || st == 1 || st == crate::thread::current_token() {
        return;
    }
    crate::compat::run_file_once(f, flag); // waits for the loading thread
}
static AUTOLOADS: crate::mt::L<std::collections::HashMap<(usize, u32), Autoload, FxBuild>> = crate::mt::L::new(std::collections::HashMap::with_hasher(FxBuild::new()));

fn autoloads() -> crate::mt::LGuard<'static, std::collections::HashMap<(usize, u32), Autoload, FxBuild>> {
    AUTOLOADS.get()
}
/// A pending autoload for `id` directly in `k`.
pub fn has_autoload(k: *mut Class, id: u32) -> bool {
    let a = autoloads();
    !a.is_empty() && a.contains_key(&(k as usize, id))
}
pub fn register_autoload(owner: *mut Class, id: u32, path: String, loader: Option<CompiledFn>, flag: *mut V) {
    bump_const_serial();
    autoloads().insert((owner as usize, id), Autoload { loader, flag, path });
}
pub fn autoload_path(owner: *mut Class, id: u32) -> Option<String> {
    autoloads().get(&(owner as usize, id)).map(|a| a.path.clone())
}
/// Find a pending autoload for `id` visible from `c` (lexical scope, ancestors, Object).
fn find_autoload(c: *mut Class) -> impl Fn(u32) -> Option<*mut Class> {
    move |id| {
        let tbl = autoloads();
        let mut k = c;
        while !k.is_null() {
            if tbl.contains_key(&(k as usize, id)) {
                return Some(k);
            }
            k = unsafe { (*k).lexical_parent };
        }
        if !c.is_null() {
            for a in ancestors(c) {
                if tbl.contains_key(&(a as usize, id)) {
                    return Some(a);
                }
            }
        }
        let o = rt().c.object;
        if tbl.contains_key(&(o as usize, id)) { Some(o) } else { None }
    }
}

/// Lexical scope chain of a cref slot ([class, parent slot address], see the compiler's
/// FnSpec::cref_slot): the classes of the enclosing class bodies, innermost first. Where the
/// static chain ends (eval'd code, the top level), the classes' own lexical parents follow.
fn lexical_chain(crefp: *const [V; 2]) -> Vec<*mut Class> {
    let mut out = Vec::new();
    let mut p = crefp;
    let object = rt().c.object;
    while !p.is_null() {
        let [v, next] = unsafe { *p };
        if !is_class(v) || v == object as V {
            break;
        }
        out.push(v as *mut Class);
        if next == 0 {
            let mut k = unsafe { (*(v as *mut Class)).lexical_parent };
            while !k.is_null() && k != object {
                out.push(k);
                k = unsafe { (*k).lexical_parent };
            }
            break;
        }
        p = next as *const [V; 2];
    }
    out
}

/// Bare constant read `Foo` with the full lexical scope (per-site cache like kn_const_get_c).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_const_get_l(cache: *mut ConstCache, crefp: *const [V; 2], id: V) -> V {
    let cref = unsafe { (*crefp)[0] };
    if let Some(v) = unsafe { const_cache_get(cache, cref) } {
        return v;
    }
    let id = sym_id(id);
    let chain = lexical_chain(crefp);
    let k = cref_class(cref);
    let mut found = None;
    let mut pending = None;
    for &l in &chain {
        if let Some(&v) = unsafe { (*l).consts.get(&id) } {
            found = Some(v);
            break;
        }
        if has_autoload(l, id) {
            pending = Some(l);
            break;
        }
    }
    let v = match (found, pending) {
        (Some(v), _) => {
            settle_id(id);
            v
        }
        (None, Some(owner)) => const_missing_in(k, id, Some(owner)),
        (None, None) => {
            let mut r = None;
            for &a in ancestors_rc(k).iter() {
                if let Some(&v) = unsafe { (*a).consts.get(&id) } {
                    r = Some(v);
                    break;
                }
                if has_autoload(a, id) {
                    break;
                }
            }
            match r.or_else(|| unsafe { (*rt().c.object).consts.get(&id).copied() }) {
                Some(v) => {
                    settle_id(id);
                    v
                }
                None => const_missing_in(k, id, chain.iter().copied().find(|&l| has_autoload(l, id))),
            }
        }
    };
    if v != UNDEF && !pst().pending_is_set() {
        unsafe { const_cache_set(cache, cref, v) };
    }
    v
}

/// `defined?(Foo)` with the full lexical scope.
#[unsafe(no_mangle)]
pub extern "C" fn kn_const_defined_l(crefp: *const [V; 2], id: V) -> u64 {
    let id = sym_id(id);
    for l in lexical_chain(crefp) {
        if unsafe { (*l).consts.contains_key(&id) } || has_autoload(l, id) {
            return 1;
        }
    }
    let k = cref_class(unsafe { (*crefp)[0] });
    for &a in ancestors_rc(k).iter() {
        if unsafe { (*a).consts.contains_key(&id) } || has_autoload(a, id) {
            return 1;
        }
    }
    let o = rt().c.object;
    (unsafe { (*o).consts.contains_key(&id) } || has_autoload(o, id)) as u64
}

pub fn const_missing(c: *mut Class, id: u32) -> V {
    const_missing_in(c, id, None)
}

/// `owner`: where a pending autoload for `id` was found in the lexical scope, if known.
fn const_missing_in(c: *mut Class, id: u32, owner: Option<*mut Class>) -> V {
    // Constants CRuby has without a require are loaded on first use: `Gem` (RubyGems' API)
    // from the embedded kenny/gem.rb, `Set` from the stdlib's set.rb (Ruby 3.2+ autoloads it)
    let implicit = if id == sym("Gem") {
        Some("kenny/gem")
    } else if id == sym("Set") {
        Some("set")
    } else {
        None
    };
    if let Some(file) = implicit {
        if find_autoload(c)(id).is_none() && crate::compat::require_compiled(file).is_some() {
            if pst().pending_is_set() {
                return UNDEF;
            }
            if let Some(v) = const_get_under(rt().c.object, id) {
                return v;
            }
        }
    }
    if let Some(owner) = owner.or_else(|| find_autoload(c)(id)) {
        // the entry stays until the file has run, so another thread referencing the constant
        // meanwhile finds it and waits for the load instead of raising NameError
        let entry = autoloads().get(&(owner as usize, id)).map(|a| (a.loader, a.flag, a.path.clone()));
        let Some(a) = entry else {
            // another thread finished this autoload just now
            if let Some(v) = after_autoload(owner, c, id) {
                return v;
            }
            return send(c as V, sym("const_missing"), &[mksym(id)], QNIL);
        };
        let a = Autoload { loader: a.0, flag: a.1, path: a.2 };
        match a.loader {
            Some(f) => {
                LOADING.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let r = crate::compat::run_file_once(f, a.flag);
                LOADING.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                autoloads().remove(&(owner as usize, id));
                if r == UNDEF {
                    return UNDEF;
                }
                if let Some(v) = after_autoload(owner, c, id) {
                    return v;
                }
            }
            // CRuby's autoload calls `require` as a method, so an override sees it: Zeitwerk's
            // turns an autoload of a directory into a module and runs callbacks after files
            None if require_overridden() => {
                LOADING.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let r = send(rt().main_obj, sym("require"), &[str_from(&a.path)], QNIL);
                LOADING.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                autoloads().remove(&(owner as usize, id));
                if r == UNDEF {
                    return UNDEF;
                }
                if let Some(v) = after_autoload(owner, c, id) {
                    return v;
                }
            }
            None if {
                LOADING.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let r = crate::compat::require_compiled(&a.path);
                LOADING.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                r.is_some()
            } => {
                autoloads().remove(&(owner as usize, id));
                if pst().pending_is_set() {
                    return UNDEF;
                }
                if let Some(v) = after_autoload(owner, c, id) {
                    return v;
                }
            }
            None => {
                autoloads().remove(&(owner as usize, id));
                let le = const_lookup(rt().c.object, sym("LoadError")).unwrap();
                let e = exc_new(le as *mut Class, &format!("cannot load such file -- {} (it was not compiled into this binary; pass -I so kenny can find it)", a.path));
                ivar_set(e, sym("@path"), str_from(&a.path));
                return raise_exc(e);
            }
        }
    }
    send(c as V, sym("const_missing"), &[mksym(id)], QNIL)
}

/// Whether Ruby code redefined `Kernel#require` (the builtin is native).
fn require_overridden() -> bool {
    let m = find_method(class_of(rt().main_obj), sym("require"));
    !m.is_null() && !matches!(unsafe { &(*m).body }, Body::Native(..))
}

/// After an autoload ran: the constant where it was registered, or else (like CRuby, whose
/// lookup ends with the cref class) wherever the loaded file defined it for `c`.
fn after_autoload(owner: *mut Class, c: *mut Class, id: u32) -> Option<V> {
    const_get_under(owner, id).or_else(|| if c.is_null() { None } else { const_get_under(c, id) })
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_autoload(target: V, id: V, loader: CompiledFn, flag: *mut V) -> V {
    let owner = if is_class(target) { target as *mut Class } else { rt().c.object };
    register_autoload(owner, sym_id(id), sym_str(sym_id(id)), Some(loader), flag);
    if !const_added(owner, sym_id(id)) {
        return UNDEF;
    }
    QNIL
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_const_get(cref: V, id: V) -> V {
    let c = cref_class(cref);
    match const_lookup(c, sym_id(id)) {
        Some(v) => v,
        None => const_missing(c, sym_id(id)),
    }
}
/// Per-site cache for a bare constant read (3 words reserved by the compiler): an immutable
/// [serial, cref, value] entry behind one atomic pointer, updated in place while one OS
/// thread runs. (The value is also held by its class, so the GC needn't see the entry.)
#[repr(C)]
pub struct ConstCache {
    entry: std::sync::atomic::AtomicPtr<ConstEntry>,
    _spare: [u64; 2],
}
/// (compiled code reads it: expr.rs `ConstantReadNode`)
#[repr(C)]
struct ConstEntry {
    serial: u64,
    cref: V,
    val: V,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_const_get_c(cache: *mut ConstCache, cref: V, id: V) -> V {
    if let Some(v) = unsafe { const_cache_get(cache, cref) } {
        return v;
    }
    let v = kn_const_get(cref, id);
    if v != UNDEF && !pst().pending_is_set() {
        unsafe { const_cache_set(cache, cref, v) };
    }
    v
}
unsafe fn const_cache_get(cache: *mut ConstCache, cref: V) -> Option<V> {
    let e = unsafe { (*cache).entry.load(std::sync::atomic::Ordering::Acquire) };
    if !e.is_null() {
        let e = unsafe { &*e };
        if e.serial == const_serial() && e.cref == cref {
            return Some(e.val);
        }
    }
    None
}
unsafe fn const_cache_set(cache: *mut ConstCache, cref: V, v: V) {
    use std::sync::atomic::Ordering;
    let c = unsafe { &*cache };
    let e = c.entry.load(Ordering::Acquire);
    let serial = const_serial();
    {
        if !crate::mt::multi() && !e.is_null() {
            let e = unsafe { &mut *e };
            e.serial = serial;
            e.cref = cref;
            e.val = v;
        } else {
            let ne = Box::into_raw(Box::new(ConstEntry { serial, cref, val: v }));
            let old = c.entry.swap(ne, Ordering::AcqRel);
            if !old.is_null() {
                crate::mt::retire_deferred(old as usize, std::mem::size_of::<ConstEntry>(), std::mem::align_of::<ConstEntry>());
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_const_get_under(parent: V, id: V) -> V {
    if !is_class(parent) {
        return type_error(&format!("{} is not a class/module", crate::fmt::inspect_lossy(parent)));
    }
    match const_get_scoped(parent as *mut Class, sym_id(id)) {
        Some(v) => v,
        None => const_missing(parent as *mut Class, sym_id(id)),
    }
}

/// `Mod::Name`: Mod and its ancestors, but (since Ruby 2.5) not Object's constants unless
/// Mod is Object itself.
pub fn const_get_scoped(c: *mut Class, id: u32) -> Option<V> {
    let object = rt().c.object;
    if c == object {
        return const_get_under(c, id);
    }
    for &a in ancestors_rc(c).iter() {
        if a == object {
            break;
        }
        if let Some(&v) = unsafe { (*a).consts.get(&id) } {
            settle_autoload(a, id);
            return Some(v);
        }
        if has_autoload(a, id) {
            return None;
        }
    }
    None
}

/// `defined?(Mod::Name)`
#[unsafe(no_mangle)]
pub extern "C" fn kn_const_defined_under(parent: V, id: V) -> u64 {
    if !is_class(parent) {
        return 0;
    }
    let c = parent as *mut Class;
    let id = sym_id(id);
    if const_get_scoped(c, id).is_some() {
        return 1;
    }
    let object = rt().c.object;
    ancestors(c).into_iter().take_while(|&a| a != object || c == object).any(|a| has_autoload(a, id)) as u64
}
/// Ruby's `Module#const_added(name)` hook, run after Ruby code defines a constant (class and
/// module definitions, assignments, `const_set`, `autoload`) when a program overrides it, as
/// Zeitwerk does to set up the autoloads of a namespace once it is defined. Returns false if
/// the hook raised.
pub fn const_added(c: *mut Class, id: u32) -> bool {
    if c.is_null() {
        return true;
    }
    let hook = find_method(class_of(c as V), sym("const_added"));
    if hook.is_null() || matches!(unsafe { &(*hook).body }, Body::Native(..)) {
        return true;
    }
    invoke(hook, c as V, &[mksym(id)], QNIL) != UNDEF
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_const_set(cref: V, id: V, v: V) -> V {
    let c = cref_class(cref);
    const_set(c, sym_id(id), v);
    if !const_added(c, sym_id(id)) {
        return UNDEF;
    }
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_const_defined(cref: V, id: V) -> u64 {
    (const_lookup(cref_class(cref), sym_id(id)).is_some() || find_autoload(cref_class(cref))(sym_id(id)).is_some()) as u64
}

/// `class X` / `module X` where X has a pending autoload in `outer` loads that file first, like
/// CRuby (otherwise a file reopening `module ActiveRecord::ConnectionAdapters` would define an
/// empty module and the autoloaded file — with its own autoloads — would never run).
/// Returns false if loading raised.
fn autoload_before_define(outer: *mut Class, id: u32) -> bool {
    let Some(a) = autoloads().get(&(outer as usize, id)).map(|a| Autoload { loader: a.loader, flag: a.flag, path: a.path.clone() }) else { return true };
    // the entry goes once the file has run: while it runs (this `class` statement may be the
    // file's own), other threads that find the constant wait for it (settle_autoload)
    let done = |flag: *mut V| unsafe { &*(flag as *const std::sync::atomic::AtomicU64) }.load(std::sync::atomic::Ordering::Acquire) == 1;
    match a.loader {
        Some(f) => {
            let r = crate::compat::run_file_once(f, a.flag);
            if done(a.flag) {
                autoloads().remove(&(outer as usize, id));
            }
            if r == UNDEF {
                return false;
            }
        }
        None => {
            if crate::compat::require_compiled(&a.path).is_some() && pst().pending_is_set() {
                autoloads().remove(&(outer as usize, id));
                return false;
            }
            if crate::compat::compiled_entry(&a.path).is_none_or(|(_, flag)| done(flag)) {
                autoloads().remove(&(outer as usize, id));
            }
        }
    }
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_define_class(cref: V, id: V, sup: V) -> V {
    let outer = cref_class(cref);
    if !autoload_before_define(outer, sym_id(id)) {
        return UNDEF;
    }
    if let Some(&existing) = unsafe { (*outer).consts.get(&sym_id(id)) } {
        if !is_class(existing) || rclass(existing).is_module {
            return type_error(&format!("{} is not a class", sym_str(sym_id(id))));
        }
        if sup != UNDEF && is_class(sup) && rclass(existing).superclass != sup as *mut Class {
            return type_error(&format!("superclass mismatch for class {}", sym_str(sym_id(id))));
        }
        return existing;
    }
    let sup = if sup == UNDEF { rt().c.object as V } else { sup };
    if !is_class(sup) || rclass(sup).is_module {
        return type_error(&format!("superclass must be an instance of Class (given an instance of {})", class_name(real_class(sup))));
    }
    let c = new_class("", sup as *mut Class);
    unsafe { (*c).lexical_parent = outer };
    const_set(outer, sym_id(id), c as V);
    if !const_added(outer, sym_id(id)) {
        return UNDEF;
    }
    tri!(send(sup, sym("inherited"), &[c as V], QNIL));
    c as V
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_define_module(cref: V, id: V) -> V {
    let outer = cref_class(cref);
    if !autoload_before_define(outer, sym_id(id)) {
        return UNDEF;
    }
    if let Some(&existing) = unsafe { (*outer).consts.get(&sym_id(id)) } {
        if !is_class(existing) || !rclass(existing).is_module {
            return type_error(&format!("{} is not a module", sym_str(sym_id(id))));
        }
        return existing;
    }
    let m = new_module("");
    unsafe { (*m).lexical_parent = outer };
    const_set(outer, sym_id(id), m as V);
    if !const_added(outer, sym_id(id)) {
        return UNDEF;
    }
    m as V
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_singleton_class_for(v: V) -> V {
    tri!(check_singleton_ok(v));
    singleton_class(v) as V
}

/// Integer/Float operators the compiler computes inline (LANGUAGE.md rule 2).
const PROTECTED_OPS: &[&str] = &["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&", "|", "^"];

/// kind: 0 = def in class body (target = self), 1 = top level, 2 = def inside a method/block,
/// 3 = singleton def (`def obj.name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_def(target: V, kind: u64, id: V, f: CompiledFn, owner_slot: *mut V, params: *const u8, fast: usize) -> V {
    unsafe { kn_def_claim(target, kind, id, f, owner_slot, params, fast, std::ptr::null_mut()) }
}
/// `kn_def` of a method that calls itself, with its word for those calls (kenny-compiler
/// statics.rs `self_call`, core.rs `claim`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_def_claim(target: V, kind: u64, id: V, f: CompiledFn, owner_slot: *mut V, params: *const u8, fast: usize, claim: *mut u64) -> V {
    let r = rt();
    let cls: *mut Class = match kind {
        0 => cref_class(target),
        1 => r.c.object,
        // `define_method(:name) { ... }` compiled as a def (the compiler's statics.rs
        // `block_def`): only while `define_method` here is the builtin; 0 tells the caller to
        // make the call as written
        4 => {
            let dm = find_method(class_of(target), sym("define_method"));
            if dm.is_null() || !matches!(unsafe { &(*dm).body }, Body::Native(..)) {
                return QFALSE;
            }
            if is_class(target) { target as *mut Class } else { r.c.object }
        }
        3 => {
            if check_singleton_ok(target) == UNDEF {
                return UNDEF;
            }
            singleton_class(target)
        }
        _ => match crate::kernel::definee_top() {
            Some(c) => c,
            None => {
                if is_class(target) {
                    target as *mut Class
                } else {
                    real_class(target)
                }
            }
        },
    };
    let name = sym_str(sym_id(id));
    if (cls == r.c.integer || cls == r.c.float) && PROTECTED_OPS.contains(&name.as_str()) {
        return raise(
            r.c.not_implemented_error,
            &format!("Kenny does not allow redefining {}#{} (see docs/LANGUAGE.md)", class_name(cls), name),
        );
    }
    unsafe {
        let prev = *owner_slot;
        if prev == 0 {
            *owner_slot = cls as V;
            SLOT_FNS.get().insert(owner_slot as usize, f as usize);
            FN_SLOTS.get().insert(f as usize, owner_slot as usize);
        } else if prev != cls as V {
            *owner_slot = SHARED_OWNER;
        }
    }
    let m = add_method(cls, sym_id(id), Body::Compiled(f));
    if !claim.is_null() {
        crate::core::claim(claim, cls, sym_id(id), fast);
    }
    unsafe {
        let mm = &mut *(m as *mut Method);
        mm.params = params;
        mm.fast = fast;
        if !always_private(sym_id(id)) {
            mm.vis = match kind {
                1 => VIS_PRIVATE, // top-level defs are private methods of Object
                4 => (*cls).default_vis, // (as `define_method` adds it)
                3 => VIS_PUBLIC,
                _ => (*cls).default_vis,
            };
        }
    }
    if unsafe { (*cls).module_function } {
        unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
        let m2 = add_method(singleton_class(cls as V), sym_id(id), Body::Compiled(f));
        unsafe {
            (*(m2 as *mut Method)).params = params;
            (*(m2 as *mut Method)).fast = fast;
            (*(m2 as *mut Method)).vis = VIS_PUBLIC;
        }
    }
    if unsafe { (*cls).is_singleton } {
        let att = unsafe { (*cls).attached };
        let hook = find_method(class_of(att), sym("singleton_method_added"));
        if !hook.is_null() && !matches!(unsafe { &(*hook).body }, Body::Native(..)) {
            tri!(invoke(hook, att, &[id], QNIL));
        }
    } else {
        let hook = find_method(class_of(cls as V), sym("method_added"));
        if !hook.is_null() && !matches!(unsafe { &(*hook).body }, Body::Native(..)) {
            tri!(invoke(hook, cls as V, &[id], QNIL));
        }
    }
    id
}

pub fn alias(c: *mut Class, new: u32, old: u32) -> V {
    let mut m = find_method(c, old);
    if m.is_null() {
        // aliasing inside `refine X do ... end` refers to X's methods
        let refined = unsafe { (*c).civars.get(&sym("@__refined")).copied() };
        if let Some(r) = refined.filter(|r| is_class(*r)) {
            m = find_method(r as *mut Class, old);
        }
    }
    if m.is_null() && unsafe { (*c).is_module } {
        // like CRuby, aliasing in a module falls back to Object's methods
        // (`alias respond_to_without_attributes? respond_to?` in ActiveModel::AttributeMethods)
        m = find_method(rt().c.object, old);
    }
    if m.is_null() {
        return raise(rt().c.name_error, &format!("undefined method '{}' for class '{}'", sym_str(old), class_name(c)));
    }
    let body = crate::kernel::clone_body(unsafe { &(*m).body });
    let nm = Box::into_raw(Box::new(Method { fast: unsafe { (*m).fast }, body, owner: unsafe { (*m).owner }, name: new, params: unsafe { (*m).params }, vis: unsafe { (*m).vis }, opt: unsafe { (*m).opt }, gc_transparent: unsafe { (*m).gc_transparent }, attr_slot: Default::default() }));
    if unsafe { (*c).methods.insert(new, nm) }.is_none() {
        crate::core::method_name_count(new, true);
    }
    crate::core::touch_method_name(c, new);
    bump_serial();
    mksym(new)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_alias(target: V, new: V, old: V) -> V {
    let c = match crate::kernel::definee_top() {
        Some(c) => c,
        None => {
            if is_class(target) {
                target as *mut Class
            } else {
                rt().c.object
            }
        }
    };
    alias(c, sym_id(new), sym_id(old))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_undef(target: V, id: V) -> V {
    let c = cref_class(target);
    add_method(c, sym_id(id), Body::Undefined);
    QNIL
}

// ---------- arguments ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_check_arity(argc: u64, min: i64, max: i64) -> V {
    let n = argc as i64;
    if n < min || (max >= 0 && n > max) {
        return arity_error(n as i32, min as i32, max as i32);
    }
    QNIL
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_args_rest(argc: u64, argv: *const V, start: u64, posts: u64) -> V {
    let a = unsafe { args_slice(argc, argv) };
    let end = (a.len() as u64).saturating_sub(posts) as usize;
    let start = (start as usize).min(end);
    ary_new(a[start..end].to_vec())
}
/// Append a `**h` keyword argument unless it is empty (`f(*a, **{})` passes only `*a`).
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_push_kw(a: V, h: V) -> V {
    if !(is_hash(h) && rhash(h).m.is_empty()) {
        rary(a).v.push(h);
    }
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_is_kwhash(v: V) -> u64 {
    (is_hash(v) && hdr(v).flags & FL_KWHASH != 0) as u64
}
/// The last argument of a call to a method with keyword parameters is its keywords: a keyword
/// hash, or (Ruby 2.x compatibility, for gems written for it) a plain Hash that would otherwise be
/// one positional argument too many (`maxpos` = positional capacity, -1 with a rest param).
/// CRuby 3 raises ArgumentError in that second case, so this only turns errors into 2.7 behavior.
#[unsafe(no_mangle)]
pub extern "C" fn kn_take_kwhash(v: V, argc: u64, maxpos: i64) -> u64 {
    if !is_hash(v) {
        return 0;
    }
    (hdr(v).flags & FL_KWHASH != 0 || (maxpos >= 0 && argc as i64 > maxpos)) as u64
}
/// Ruby 2 keyword semantics, used for methods and blocks defined in dependencies (gems written
/// for Ruby 2): a trailing positional Hash with Symbol keys becomes the
/// keywords when it isn't needed for a required positional (`t.string :key, opts` calling
/// `def string(*args, **options)`).
#[unsafe(no_mangle)]
pub extern "C" fn kn_take_kwhash2(v: V, argc: u64, nreq: u64) -> u64 {
    if !is_hash(v) {
        return 0;
    }
    if hdr(v).flags & FL_KWHASH != 0 {
        return 1;
    }
    (argc > nreq && rhash(v).m.keys().all(|k| is_sym(k.0))) as u64
}
/// Copy of the keyword hash so keys can be consumed.
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_dup(v: V) -> V {
    let h = hash_new();
    if is_hash(v) {
        for (k, x) in crate::hash::pairs(v) {
            rhash(h).m.insert(HKey(k), x);
        }
    }
    h
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_take(h: V, key: V) -> V {
    rhash(h).m.shift_remove(&HKey(key)).unwrap_or(KW_MISSING)
}
/// Keyword `key` of keyword hash `h` (nil: none passed) without removing it, or KW_MISSING.
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_get(h: V, key: V) -> V {
    if h == QNIL {
        return KW_MISSING;
    }
    rhash(h).m.get_ident(key).unwrap_or(KW_MISSING)
}
/// How many keywords `h` holds (nil: none passed).
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_count(h: V) -> u64 {
    if h == QNIL { 0 } else { rhash(h).m.len() as u64 }
}
/// Raise for the keys of `h` that aren't among the `n` method keywords at `keys`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_kw_unknown(h: V, keys: *const V, n: u64) -> V {
    let known = unsafe { std::slice::from_raw_parts(keys, n as usize) };
    let names: Vec<String> = rhash(h).m.keys().map(|k| k.0).filter(|k| !known.contains(k)).map(crate::fmt::inspect_lossy).collect();
    arg_error(&format!("unknown keyword{}: {}", if names.len() > 1 { "s" } else { "" }, names.join(", ")))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_missing(key: V) -> V {
    arg_error(&format!("missing keyword: {}", crate::fmt::inspect_lossy(key)))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_kw_assert_empty(h: V) -> V {
    let keys: Vec<V> = rhash(h).m.keys().map(|k| k.0).collect();
    if keys.is_empty() {
        return QNIL;
    }
    let names: Vec<String> = keys.iter().map(|k| crate::fmt::inspect_lossy(*k)).collect();
    arg_error(&format!("unknown keyword{}: {}", if keys.len() > 1 { "s" } else { "" }, names.join(", ")))
}

// ---------- splats / multiple assignment ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_splat_to_ary(v: V) -> V {
    if is_ary(v) {
        return v;
    }
    if v == QNIL {
        return ary_new(vec![]);
    }
    if is_hash(v) || is_range(v) || respond_to(v, sym("to_a")) && !is_str(v) {
        let r = tri!(send0(v, "to_a"));
        if is_ary(r) {
            return r;
        }
    }
    ary_new(vec![v])
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_masgn_ary(v: V) -> V {
    if is_ary(v) {
        return v;
    }
    if is_heap(v) && !is_str(v) && respond_to(v, sym("to_ary")) {
        let r = tri!(send0(v, "to_ary"));
        if is_ary(r) {
            return r;
        }
    }
    ary_new(vec![v])
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_ref(a: V, i: i64) -> V {
    let v = &rary(a).v;
    let i = if i < 0 { i + v.len() as i64 } else { i };
    if i < 0 { QNIL } else { v.get(i as usize).copied().unwrap_or(QNIL) }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_slice_rest(a: V, start: u64, posts: u64) -> V {
    let v = &rary(a).v;
    let end = (v.len() as u64).saturating_sub(posts) as usize;
    let start = (start as usize).min(end);
    ary_new(v[start..end].to_vec())
}
/// Index of the post-splat element `k` (of `posts`) in a masgn array.
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_post_ref(a: V, pre: u64, posts: u64, k: u64) -> V {
    let n = rary(a).v.len() as u64;
    let idx = if n >= pre + posts { n - posts + k } else { pre + k };
    kn_ary_ref(a, idx as i64)
}

// ---------- exceptions & non-local control flow ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_pending_exc() -> V {
    match pst().pending {
        Throw::Raise(e) => e,
        _ => QNIL,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_exc_match(exc: V, cls: V) -> V {
    if is_ary(cls) {
        for &c in &crate::gc::Rooted::of(rary(cls).v.clone()) {
            let r = kn_exc_match(exc, c);
            if r != QFALSE {
                return r;
            }
        }
        return QFALSE;
    }
    if !is_class(cls) {
        return type_error("class or module required for rescue clause");
    }
    let r = send1(cls, "===", exc);
    if r == UNDEF { UNDEF } else { bool_v(truthy(r)) }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_rescued(exc: V) {
    let p = pst();
    p.pending = Throw::None;
    p.errinfo = exc;
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_errinfo_restore(prev: V) {
    pst().errinfo = prev;
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_errinfo() -> V {
    pst().errinfo
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_save_pending() -> u64 {
    let p = std::mem::replace(&mut pst().pending, Throw::None);
    crate::gc::save_throw(p)
}
/// Returns 1 if a throw was restored (the caller must keep unwinding).
#[unsafe(no_mangle)]
pub extern "C" fn kn_restore_pending(h: u64) -> u64 {
    let p = crate::gc::take_throw(h);
    let pending = !matches!(p, Throw::None);
    pst().pending = p;
    pending as u64
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_throw_break(tag: u64, val: V) -> V {
    pst().pending = Throw::Break { tag, val };
    UNDEF
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_catch_break(tag: u64) -> V {
    if let Throw::Break { tag: t, val } = pst().pending {
        if t == tag {
            pst().pending = Throw::None;
            return val;
        }
    }
    UNDEF
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_throw_return(tag: u64, val: V) -> V {
    pst().pending = Throw::Return { tag, val };
    UNDEF
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_catch_return(tag: u64) -> V {
    if let Throw::Return { tag: t, val } = pst().pending {
        if t == tag {
            pst().pending = Throw::None;
            return val;
        }
    }
    UNDEF
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_raise_msg(cls_name: u64, ptr: *const u8, len: u64) -> V {
    let msg = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len as usize) }).into_owned();
    let c = match cls_name {
        1 => rt().c.local_jump_error,
        2 => rt().c.not_implemented_error,
        _ => rt().c.runtime_error,
    };
    raise(c, &msg)
}

// ---------- defined? ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_respond_to(self_: V, id: V) -> u64 {
    respond_to(self_, sym_id(id)) as u64
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ivar_defined(self_: V, id: V) -> u64 {
    ivar_defined(self_, sym_id(id)) as u64
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_gvar_defined(id: V) -> u64 {
    rt().gvars.contains_key(&gvar_resolve(sym_id(id))) as u64
}

// ---------- misc helpers used by compiled code ----------

/// Slow path for `==` that also handles fixnum/other combos.
#[unsafe(no_mangle)]
pub extern "C" fn kn_eq(a: V, b: V) -> V {
    equal(a, b)
}

// ---------- pattern matching ----------

#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_deconstruct(v: V) -> V {
    if is_ary(v) {
        return v;
    }
    if !respond_to(v, sym("deconstruct")) {
        return QNIL;
    }
    let r = tri!(send0(v, "deconstruct"));
    if !is_ary(r) {
        return type_error("deconstruct must return Array");
    }
    r
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_deconstruct_keys(v: V, keys: V) -> V {
    if is_hash(v) {
        return v;
    }
    if !respond_to(v, sym("deconstruct_keys")) {
        return QNIL;
    }
    let r = tri!(send1(v, "deconstruct_keys", keys));
    if !is_hash(r) {
        return type_error("deconstruct_keys must return Hash");
    }
    r
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_key(h: V, k: V) -> V {
    rhash(h).m.get(&HKey(k)).copied().unwrap_or(KW_MISSING)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_hash_rest(h: V, keys: V) -> V {
    let out = hash_new();
    let ks = rary(keys).v.clone();
    for (k, v) in crate::hash::pairs(h) {
        if !ks.iter().any(|x| eql(*x, k)) {
            rhash(out).m.insert(HKey(k), v);
        }
    }
    out
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_size(h: V) -> i64 {
    rhash(h).m.len() as i64
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_slice(a: V, from: i64, to: i64) -> V {
    let v = &rary(a).v;
    let (f, t) = (from.max(0) as usize, (to.max(0) as usize).min(v.len()));
    ary_new(if f < t { v[f..t].to_vec() } else { vec![] })
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_pm_nomatch(v: V) -> V {
    let c = const_lookup(rt().c.object, sym("NoMatchingPatternError")).unwrap();
    raise(c as *mut Class, &crate::fmt::inspect_lossy(v))
}

// ---------- refinements (only call sites after `using` come here) ----------

static REFINE_SCOPES: crate::mt::L<std::collections::HashMap<u64, Vec<(*mut Class, *mut Class)>, FxBuild>> = crate::mt::L::new(std::collections::HashMap::with_hasher(FxBuild::new()));

fn refine_scopes() -> crate::mt::LGuard<'static, std::collections::HashMap<u64, Vec<(*mut Class, *mut Class)>, FxBuild>> {
    REFINE_SCOPES.get()
}
/// Refinements defined by module `m`: (refined class, refinement module).
pub fn refinements_of(m: V) -> Vec<(*mut Class, *mut Class)> {
    let h = ivar_get(m, sym("@__refinements"));
    if !is_hash(h) {
        return vec![];
    }
    crate::hash::pairs(h).into_iter().map(|(k, v)| (k as *mut Class, v as *mut Class)).collect()
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_using(scope: u64, m: V, parent: u64) -> V {
    if !is_class(m) || !rclass(m).is_module {
        return type_error("wrong argument type (expected Module)");
    }
    let mut list = refine_scopes().get(&parent).cloned().unwrap_or_default();
    let mut add = refinements_of(m);
    add.extend(list);
    list = add;
    refine_scopes().insert(scope, list);
    QNIL
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send_refined(cache: *mut CallCache, scope: u64, recv: V, mid: V, argc: u64, argv: *const V, blk: V) -> V {
    if let Some(list) = refine_scopes().get(&scope) {
        if !list.is_empty() {
            let id = sym_id(mid);
            let k = class_of(recv);
            let anc = ancestors(k);
            for (target, r) in list.clone() {
                if anc.contains(&target) {
                    if let Some(&m) = unsafe { (*r).methods.get(&id) } {
                        return invoke(m, recv, unsafe { args_slice(argc, argv) }, blk);
                    }
                }
            }
        }
    }
    unsafe { kn_send(cache, recv, mid, argc, argv, blk) }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_ptr(a: V) -> *const V {
    rary(a).v.as_ptr()
}

/// `super` inside a block within a method: the method's super, unless this very block is
/// running as a `define_method` body — then that method's super (CRuby resolves it by frame).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_super_blk(procp: V, owner_slot: *const V, self_: V, mid: V, args: V, blk: V) -> V {
    if let Some(&m) = pst().dm_stack.last() {
        if let Body::Proc(p) = unsafe { &(*m).body } {
            if *p == procp {
                return kn_super_dyn(self_, args, blk);
            }
        }
    }
    let a = rary(args).v.clone();
    unsafe { kn_super(owner_slot, self_, mid, a.len() as u64, a.as_ptr(), blk) }
}
/// `super(...)` inside a define_method block.
#[unsafe(no_mangle)]
pub extern "C" fn kn_super_dyn(self_: V, args: V, blk: V) -> V {
    let Some(&m) = pst().dm_stack.last() else {
        return raise(rt().c.runtime_error, "super called outside of method");
    };
    let (owner, name) = unsafe { ((*m).owner, (*m).name) };
    let sm = find_super_method(self_, owner, name);
    if sm.is_null() {
        return raise(rt().c.no_method_error, &format!("super: no superclass method '{}'", sym_str(name)));
    }
    let a = rary(args).v.clone();
    invoke(sm, self_, &a, blk)
}

/// A call with an explicit receiver (`obj.foo`): private methods are not callable this way.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send_pub(cache: *mut CallCache, recv: V, mid: V, argc: u64, argv: *const V, blk: V) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    let args = unsafe { args_slice(argc, argv) };
    if m.is_null() {
        return method_missing(recv, sym_id(mid), args, blk, false);
    }
    if unsafe { (*m).vis } == VIS_PRIVATE {
        return crate::visibility::private_call_error(recv, sym_id(mid), args, blk);
    }
    invoke(m, recv, args, blk)
}

/// Start of a class/module body: it becomes the definee (for `alias`, `define_method`
/// targets etc., even inside an outer `class_eval`), and `private` with no arguments
/// starts out off.
#[unsafe(no_mangle)]
pub extern "C" fn kn_class_enter(c: V) -> V {
    if is_class(c) {
        rclass(c).default_vis = VIS_PUBLIC;
        pst().definee.push(c as *mut Class);
    }
    c
}
/// End of a class/module body (also reached when the body raised).
#[unsafe(no_mangle)]
pub extern "C" fn kn_class_leave(c: V) -> V {
    if is_class(c) {
        rclass(c).default_vis = VIS_PUBLIC;
        pst().definee.pop();
    }
    c
}

/// `Math.<MATH_FNS[which]>(x)` for compiled code (which checked the domain).
#[unsafe(no_mangle)]
pub extern "C" fn kn_math1(which: u64, x: f64) -> f64 {
    match which {
        0 => x.sqrt() + 0.0,
        1 => x.sin(),
        2 => x.cos(),
        3 => x.tan(),
        4 => x.atan(),
        5 => x.exp(),
        6 => x.ln(),
        7 => x.log2(),
        8 => x.log10(),
        9 => x.cbrt(),
        10 => x.sinh(),
        11 => x.cosh(),
        12 => x.tanh(),
        13 => x.asin(),
        _ => x.acos(),
    }
}
/// Float#** for compiled code (x >= 0 or y integral: a Float result).
#[unsafe(no_mangle)]
pub extern "C" fn kn_pow(x: f64, y: f64) -> f64 {
    x.powf(y)
}
