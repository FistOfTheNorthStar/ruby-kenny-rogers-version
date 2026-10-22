//! Type-specialised call paths for compiled code.
//!
//! Calls with 0, 1 or 2 arguments and no block go through `kn_send0/1/2` instead of the
//! generic `kn_send`: no argument buffer, no `invoke` dispatch for the common bodies, and for
//! a few hot core methods (`Array#[]`, `Hash#[]=`, `String#==`, `nil?`, ...) the operation is
//! done right here. Whether a core method is "the builtin one" is decided by the method found
//! through the call site's inline cache: builtin methods carry an `opt` tag (set once at boot,
//! see `init`), and a method redefined or overridden at runtime (monkey patch, subclass,
//! prepend, singleton method, `method_missing` instead of a method) is a different `Method`
//! without the tag. So no global "redefined" flag is needed and every way of redefining a
//! method is covered by the same serial-checked cache that generic calls use.
//!
//! The compiler also inlines `times`/`each`/`map`/... blocks as loops; `kn_opt_check` is
//! the guard it uses (the receiver's method must be the builtin one).

use crate::core::*;

pub const OPT_NONE: u8 = 0;
pub const OPT_ARY_AREF: u8 = 1;
pub const OPT_ARY_ASET: u8 = 2;
pub const OPT_ARY_PUSH: u8 = 3;
pub const OPT_ARY_SIZE: u8 = 4;
pub const OPT_ARY_EMPTY: u8 = 5;
pub const OPT_ARY_FIRST: u8 = 6;
pub const OPT_ARY_LAST: u8 = 7;
pub const OPT_HASH_AREF: u8 = 8;
pub const OPT_HASH_ASET: u8 = 9;
pub const OPT_HASH_SIZE: u8 = 10;
pub const OPT_HASH_EMPTY: u8 = 11;
pub const OPT_HASH_KEY: u8 = 12;
pub const OPT_STR_EQ: u8 = 13;
pub const OPT_STR_EMPTY: u8 = 14;
pub const OPT_IDENTITY_EQ: u8 = 15;
pub const OPT_NIL_P: u8 = 16;
pub const OPT_INT_TIMES: u8 = 17;
pub const OPT_INT_UPTO: u8 = 18;
pub const OPT_INT_DOWNTO: u8 = 19;
pub const OPT_ARY_EACH: u8 = 20;
pub const OPT_ARY_EACH_WITH_INDEX: u8 = 21;
pub const OPT_ARY_MAP: u8 = 22;
pub const OPT_RANGE_EACH: u8 = 23;
pub const OPT_ARY_SELECT: u8 = 24;
pub const OPT_ARY_REJECT: u8 = 25;
pub const OPT_ARY_EACH_INDEX: u8 = 26;
pub const OPT_ARY_ANY: u8 = 27;
pub const OPT_ARY_ALL: u8 = 28;
pub const OPT_STR_APPEND: u8 = 29;
pub const OPT_IS_A: u8 = 30;
/// Module#=== (`case x when SomeClass`)
pub const OPT_MOD_EQQ: u8 = 31;
pub const OPT_STR_GETBYTE: u8 = 32;
pub const OPT_STR_SETBYTE: u8 = 33;
pub const OPT_STR_BYTESIZE: u8 = 34;
pub const OPT_INT_SUCC: u8 = 35;
pub const OPT_ARY_COUNT: u8 = 36;
pub const OPT_ARY_SUM: u8 = 37;
/// The builtin Math functions compiled code computes inline (`MATH_FNS`).
pub const OPT_MATH: u8 = 38;
/// Integer#to_f, Float#to_f, Integer#-@, Float#-@, Float#** (`DIRECT_NUM`).
pub const OPT_NUM_INLINE: u8 = 39;
/// Class#new (kenny-runtime abi.rs `kn_new_fill`).
pub const OPT_CLASS_NEW: u8 = 40;
/// The Math functions compiled code may compute inline, in the order of `kn_math1`'s
/// `which` (kenny-compiler fastpath.rs `MATH_FNS` must match).
pub const MATH_FNS: [&str; 15] = ["sqrt", "sin", "cos", "tan", "atan", "exp", "log", "log2", "log10", "cbrt", "sinh", "cosh", "tanh", "asin", "acos"];

fn tag(c: *mut Class, name: &str, t: u8) {
    let m = unsafe { (*c).methods.get(&sym(name)).copied() };
    if let Some(m) = m {
        unsafe { (*(m as *mut Method)).opt = t };
    }
}

/// Mark the builtin core methods that have a specialised path (after the core library is
/// defined, before any Ruby code runs).
pub fn init() {
    let c = rt().c;
    tag(c.array, "[]", OPT_ARY_AREF);
    tag(c.array, "slice", OPT_ARY_AREF);
    tag(c.array, "[]=", OPT_ARY_ASET);
    tag(c.array, "<<", OPT_ARY_PUSH);
    tag(c.array, "push", OPT_ARY_PUSH);
    tag(c.array, "append", OPT_ARY_PUSH);
    tag(c.array, "size", OPT_ARY_SIZE);
    tag(c.array, "length", OPT_ARY_SIZE);
    tag(c.array, "empty?", OPT_ARY_EMPTY);
    tag(c.array, "first", OPT_ARY_FIRST);
    tag(c.array, "last", OPT_ARY_LAST);
    tag(c.array, "each", OPT_ARY_EACH);
    tag(c.array, "each_with_index", OPT_ARY_EACH_WITH_INDEX);
    tag(c.array, "each_index", OPT_ARY_EACH_INDEX);
    tag(c.array, "map", OPT_ARY_MAP);
    tag(c.array, "collect", OPT_ARY_MAP);
    tag(c.array, "select", OPT_ARY_SELECT);
    tag(c.array, "filter", OPT_ARY_SELECT);
    tag(c.array, "reject", OPT_ARY_REJECT);
    tag(c.array, "any?", OPT_ARY_ANY);
    tag(c.array, "all?", OPT_ARY_ALL);
    tag(c.hash, "[]", OPT_HASH_AREF);
    tag(c.hash, "[]=", OPT_HASH_ASET);
    tag(c.hash, "store", OPT_HASH_ASET);
    tag(c.hash, "size", OPT_HASH_SIZE);
    tag(c.hash, "length", OPT_HASH_SIZE);
    tag(c.hash, "empty?", OPT_HASH_EMPTY);
    for n in ["key?", "has_key?", "include?", "member?"] {
        tag(c.hash, n, OPT_HASH_KEY);
    }
    tag(c.string, "==", OPT_STR_EQ);
    tag(c.string, "empty?", OPT_STR_EMPTY);
    tag(c.string, "<<", OPT_STR_APPEND);
    tag(c.string, "concat", OPT_STR_APPEND);
    tag(c.basic_object, "==", OPT_IDENTITY_EQ);
    tag(c.symbol, "==", OPT_IDENTITY_EQ);
    tag(c.kernel, "nil?", OPT_NIL_P);
    tag(c.kernel, "is_a?", OPT_IS_A);
    tag(c.kernel, "kind_of?", OPT_IS_A);
    tag(c.module, "===", OPT_MOD_EQQ);
    tag(c.string, "getbyte", OPT_STR_GETBYTE);
    tag(c.string, "setbyte", OPT_STR_SETBYTE);
    tag(c.string, "bytesize", OPT_STR_BYTESIZE);
    tag(c.integer, "succ", OPT_INT_SUCC);
    tag(c.array, "count", OPT_ARY_COUNT);
    tag(c.array, "sum", OPT_ARY_SUM);
    if let Some(math) = crate::core::const_lookup(c.object, sym("Math")) {
        let sc = crate::core::singleton_class(math);
        for n in MATH_FNS {
            tag(sc, n, OPT_MATH);
        }
        crate::core::KN_DIRECT.math_module.store(math as usize, std::sync::atomic::Ordering::Relaxed);
    }
    for k in [c.integer, c.float] {
        tag(k, "to_f", OPT_NUM_INLINE);
        tag(k, "-@", OPT_NUM_INLINE);
    }
    tag(c.float, "**", OPT_NUM_INLINE);
    tag(c.integer, "<<", OPT_NUM_INLINE);
    tag(c.integer, ">>", OPT_NUM_INLINE);
    tag(c.class, "new", OPT_CLASS_NEW);
    if let Some(m) = unsafe { (*c.module).methods.get(&sym("===")).copied() } {
        crate::core::KN_DIRECT.mod_eqq.store(m as usize, std::sync::atomic::Ordering::Relaxed);
    }
    tag(c.integer, "times", OPT_INT_TIMES);
    tag(c.integer, "upto", OPT_INT_UPTO);
    tag(c.integer, "downto", OPT_INT_DOWNTO);
    tag(c.range, "each", OPT_RANGE_EACH);
    crate::core::KN_DIRECT.array_class.store(c.array as usize, std::sync::atomic::Ordering::Relaxed);
    crate::core::DIRECT_READY.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::core::update_direct_flags();
}

use crate::abi::{dispatch, dispatch_keep};

#[inline(always)]
fn approx_sp() -> usize {
    let x = 0u8;
    &x as *const u8 as usize
}

/// The general case of `kn_sendN`: same semantics as `kn_send` / `kn_send_pub`, with the
/// common method bodies called directly.
#[inline(always)]
pub(crate) unsafe fn call_found(m: *const Method, recv: V, mid: V, args: &[V], explicit: u64) -> V {
    if m.is_null() {
        return method_missing(recv, sym_id(mid), args, QNIL, false);
    }
    let mm = unsafe { &*m };
    if explicit != 0 && mm.vis == VIS_PRIVATE {
        return crate::visibility::private_call_error(recv, sym_id(mid), args, QNIL);
    }
    match &mm.body {
        Body::Compiled(f) => {
            if crate::gc::safepoint() {
                return UNDEF;
            }
            if approx_sp() < pst().stack_limit {
                return raise(rt().c.system_stack_error, "stack level too deep");
            }
            unsafe { f(recv, args.len() as u64, args.as_ptr(), QNIL) }
        }
        Body::Native(f, min, max) => {
            let n = args.len() as i32;
            if n < *min || (*max >= 0 && n > *max) {
                return arity_error(n, *min, *max);
            }
            if mm.gc_transparent {
                return f(recv, args, QNIL);
            }
            call_native(*f, recv, args, QNIL)
        }
        Body::AttrReader(iv) if args.is_empty() => attr_get(m, recv, *iv),
        Body::AttrWriter(iv) if args.len() == 1 => attr_set(m, recv, *iv, args[0]),
        _ => invoke(m, recv, args, QNIL),
    }
}

/// The slot of attribute method `m`'s ivar `iv` in plain Object `recv`, through the
/// method's one-class cache (`Method::attr_slot`); None while the class has no such ivar.
/// An object's slot table is its class's (fixed at allocation, only appended to), and
/// classes other than singleton classes are never freed, so a cached index stays valid.
#[inline(always)]
fn attr_index(m: *const Method, recv: V, iv: u32) -> Option<usize> {
    use std::sync::atomic::Ordering::Relaxed;
    let shape = obj(recv).shape;
    let cache = unsafe { &(*m).attr_slot };
    let w = cache[0].load(Relaxed);
    if w != 0 && w >> 16 == shape as u64 {
        return Some((w & 0xFFFF) as usize);
    }
    let w1 = cache[1].load(Relaxed);
    if w1 != 0 && w1 >> 16 == shape as u64 {
        return Some((w1 & 0xFFFF) as usize);
    }
    let i = shape_index(unsafe { &*shape }, iv)?;
    if i <= 0xFFFF && (shape as u64) >> 48 == 0 {
        // the newest first (each word on its own is consistent)
        cache[1].store(w, Relaxed);
        cache[0].store(((shape as u64) << 16) | i as u64, Relaxed);
    }
    Some(i as usize)
}

/// `@x` read for attr_reader `m`.
#[inline(always)]
fn attr_get(m: *const Method, recv: V, iv: u32) -> V {
    if is_kind(recv, Kind::Object) {
        return match attr_index(m, recv, iv) {
            Some(i) => obj(recv).ivars.get(i).copied().filter(|x| *x != UNDEF).unwrap_or(QNIL),
            None => QNIL,
        };
    }
    ivar_get(recv, iv)
}

/// `@x = a` for attr_writer `m` (a new ivar, a frozen receiver...: `ivar_set`).
#[inline(always)]
fn attr_set(m: *const Method, recv: V, iv: u32, a: V) -> V {
    if is_kind(recv, Kind::Object) && hdr(recv).flags & FL_FROZEN == 0 {
        if let Some(i) = attr_index(m, recv, iv) {
            let o = obj(recv);
            if i < o.ivars.len() {
                o.ivars[i] = a; // (through the write barrier)
                return a;
            }
        }
    }
    ivar_set(recv, iv, a)
}

/// The method's specialisation tag, unless the call must go through the normal path
/// (`obj.foo` reaching a method made private: `call_found` raises).
#[inline(always)]
fn opt_of(m: *const Method, explicit: u64) -> u8 {
    if m.is_null() || (explicit != 0 && unsafe { (*m).vis } == VIS_PRIVATE) { OPT_NONE } else { unsafe { (*m).opt } }
}

/// `recv.mid` with no arguments and no block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send0(cache: *mut CallCache, recv: V, mid: V, explicit: u64) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    unsafe { send0_found(m, recv, mid, explicit) }
}
#[inline(always)]
unsafe fn send0_found(m: *const Method, recv: V, mid: V, explicit: u64) -> V {
    match opt_of(m, explicit) {
        OPT_ARY_SIZE => return mkfix(rary(recv).v.len() as i64),
        OPT_ARY_EMPTY => return bool_v(rary(recv).v.is_empty()),
        OPT_ARY_FIRST => return rary(recv).v.first().copied().unwrap_or(QNIL),
        OPT_ARY_LAST => return rary(recv).v.last().copied().unwrap_or(QNIL),
        OPT_HASH_SIZE => return mkfix(rhash(recv).m.len() as i64),
        OPT_HASH_EMPTY => return bool_v(rhash(recv).m.is_empty()),
        OPT_STR_EMPTY => return bool_v(rstr(recv).b.is_empty()),
        OPT_STR_BYTESIZE => return mkfix(rstr(recv).b.len() as i64),
        // (fixnum + 1 stays a fixnum below the largest one)
        OPT_INT_SUCC if is_fix(recv) && fix(recv) < (1 << 61) => return mkfix(fix(recv) + 1),
        OPT_NIL_P => return bool_v(recv == QNIL),
        _ => {}
    }
    unsafe { call_found(m, recv, mid, &[], explicit) }
}

/// `recv.mid(a)` with no block (also the slow path of inline binary operators).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send1(cache: *mut CallCache, recv: V, mid: V, a: V, explicit: u64) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    unsafe { send1_found(m, recv, mid, a, explicit) }
}
#[inline(always)]
unsafe fn send1_found(m: *const Method, recv: V, mid: V, a: V, explicit: u64) -> V {
    match opt_of(m, explicit) {
        OPT_ARY_AREF if is_fix(a) => {
            let v = &rary(recv).v;
            let i = fix(a);
            let i = if i < 0 { i + v.len() as i64 } else { i };
            return if i < 0 { QNIL } else { v.get(i as usize).copied().unwrap_or(QNIL) };
        }
        OPT_ARY_PUSH => {
            if is_frozen(recv) {
                return check_frozen(recv);
            }
            rary(recv).v.push(a);
            return recv;
        }
        OPT_HASH_AREF => return crate::hash::hget(recv, a),
        OPT_HASH_KEY => return bool_v(rhash(recv).m.contains_key(&HKey(a))),
        OPT_STR_EQ => return bool_v(is_str(a) && rstr(recv).b == rstr(a).b),
        OPT_STR_APPEND if is_str(a) && !is_frozen(recv) => {
            if a == recv {
                let add = rstr(a).b.clone();
                rstr(recv).b.extend_from_slice(&add);
            } else {
                let add: &[u8] = &rstr(a).b;
                rstr(recv).b.extend_from_slice(add);
            }
            return recv;
        }
        OPT_IDENTITY_EQ => return bool_v(recv == a),
        OPT_IS_A => return bool_v(is_class(a) && is_a(recv, a as *mut Class)),
        OPT_MOD_EQQ => return bool_v(is_a(a, recv as *mut Class)),
        OPT_STR_GETBYTE if is_fix(a) => {
            let b = &rstr(recv).b;
            let i = fix(a);
            let i = if i < 0 { i + b.len() as i64 } else { i };
            return if i < 0 { QNIL } else { b.get(i as usize).map(|x| mkfix(*x as i64)).unwrap_or(QNIL) };
        }
        _ => {}
    }
    unsafe { call_found(m, recv, mid, &[a], explicit) }
}

/// `recv.mid(a, b)` with no block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send2(cache: *mut CallCache, recv: V, mid: V, a: V, b: V, explicit: u64) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    match opt_of(m, explicit) {
        OPT_STR_SETBYTE if is_fix(a) && is_fix(b) && !is_frozen(recv) => {
            let s = &mut rstr(recv).b;
            let i = fix(a);
            let j = if i < 0 { i + s.len() as i64 } else { i };
            if j >= 0 && (j as usize) < s.len() {
                s[j as usize] = fix(b) as u8;
                return b;
            }
        }
        OPT_ARY_ASET if is_fix(a) && !is_frozen(recv) => {
            let v = &mut rary(recv).v;
            let i = fix(a);
            let i = if i < 0 { i + v.len() as i64 } else { i };
            if i >= 0 && (i as usize) < v.len() {
                v[i as usize] = b;
                return b;
            }
            if i >= 0 && i as usize == v.len() {
                v.push(b);
                return b;
            }
        }
        OPT_HASH_ASET => return crate::hash::hset(recv, a, b),
        _ => {}
    }
    unsafe { call_found(m, recv, mid, &[a, b], explicit) }
}

/// Guard for a block the compiler inlined as a loop: 1 if `recv.mid` is the builtin method
/// tagged `want` (and the receiver has the representation the loop assumes), else 0 (the
/// caller then makes a normal call with a real block).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_opt_check(cache: *mut CallCache, recv: V, mid: V, want: u64) -> u64 {
    let m = unsafe { dispatch(cache, recv, mid) };
    // inlined loops are only for calls with an explicit receiver
    if opt_of(m, 1) as u64 != want {
        return 0;
    }
    let ok = match want as u8 {
        OPT_INT_TIMES | OPT_INT_UPTO | OPT_INT_DOWNTO => is_fix(recv),
        OPT_RANGE_EACH => {
            is_range(recv) && is_fix(rrange(recv).lo) && is_fix(rrange(recv).hi) && !(rrange(recv).excl && fix(rrange(recv).hi) == FIX_MIN)
        }
        _ => is_ary(recv),
    };
    ok as u64
}

/// Element `i` of an Array, or UNDEF when `i` is past the end (loops re-check the length on
/// every iteration, as Ruby's do, so the block may change the array).
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_at(a: V, i: u64) -> V {
    rary(a).v.get(i as usize).copied().unwrap_or(UNDEF)
}

/// Block-argument autosplat for an inlined block with several parameters: the Array to
/// destructure, or nil when `x` isn't one (then the first parameter gets `x` itself).
#[unsafe(no_mangle)]
pub extern "C" fn kn_autosplat(x: V) -> V {
    if is_ary(x) {
        return x;
    }
    if is_heap(x) && !is_str(x) && respond_to(x, sym("to_ary")) {
        let r = send0(x, "to_ary");
        if r == UNDEF || is_ary(r) {
            return r;
        }
    }
    QNIL
}

/// Range bounds for an inlined `Range#each` (checked to be Fixnums by `kn_opt_check`):
/// the last value visited, as a Fixnum.
#[unsafe(no_mangle)]
pub extern "C" fn kn_range_last(r: V) -> V {
    let rg = rrange(r);
    if rg.excl { mkfix(fix(rg.hi) - 1) } else { rg.hi }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_range_first(r: V) -> V {
    rrange(r).lo
}

/// A Float result that has no flonum encoding (0.0 is handled too): box it.
#[unsafe(no_mangle)]
pub extern "C" fn kn_float_box(bits: u64) -> V {
    float_new(f64::from_bits(bits))
}

/// Is `v` a Float (flonum or heap)? (kenny-compiler spec.rs: a speculated Float local)
#[unsafe(no_mangle)]
pub extern "C" fn kn_is_float(v: V) -> i64 {
    is_float(v) as i64
}

/// The f64 of a heap Float stored into a local that only ever holds Floats (kenny-compiler
/// fastpath.rs `float_unbox`; flonums are decoded inline). Anything else is a compiler bug.
#[unsafe(no_mangle)]
pub extern "C" fn kn_float_unbox(v: V) -> f64 {
    if is_float(v) {
        float_val(v)
    } else {
        eprintln!("kenny: internal error: a Float local got a {}", class_name(class_of(v)));
        std::process::abort()
    }
}

// ----- attribute access inline in compiled loops -----
//
// Inside loops, a call that may reach an attr_reader/attr_writer (`obj.x`, `obj.x = v`, a
// bare `x`) first checks its call cache's `attr` word inline (kenny-compiler fastpath.rs
// `attr_fast`): (receiver class << 16) | ivar index. When that doesn't match it calls
// `kn_attr_get`/`kn_attr_set`, which make the call like `kn_send0`/`kn_send1`/`kn_vcall`
// and, when it reached an attribute method the site may call, record the receiver's class
// in the word. The first class wins until the method serial changes, which clears every
// recorded word: the recorded sites are listed here.

static ATTR_SITES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
static ATTR_SITES_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `mode`: 0 = a call without an explicit receiver, 1 = with one (`obj.x`: no private or
/// protected method), 2 = a bare identifier (a vcall). Shaped like `kn_send0` (the common
/// case at a site that isn't an attribute), the attribute work out of line.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_attr_get(cache: *mut CallCache, recv: V, mid: V, mode: u64) -> V {
    let (m, cache) = unsafe { dispatch_keep(cache, recv, mid) };
    if !m.is_null() && matches!(unsafe { &(*m).body }, Body::AttrReader(_)) {
        return unsafe { attr_get_site(cache, m, recv, mid, mode) };
    }
    unsafe { get_found(m, recv, mid, mode) }
}
#[inline(always)]
unsafe fn get_found(m: *const Method, recv: V, mid: V, mode: u64) -> V {
    if mode == 2 {
        if m.is_null() {
            return method_missing(recv, sym_id(mid), &[], QNIL, true);
        }
        return unsafe { call_found(m, recv, mid, &[], 0) };
    }
    unsafe { send0_found(m, recv, mid, mode) }
}
#[inline(never)]
unsafe fn attr_get_site(cache: *mut CallCache, m: *const Method, recv: V, mid: V, mode: u64) -> V {
    if is_kind(recv, Kind::Object) && (mode != 1 || unsafe { (*m).vis } == VIS_PUBLIC) {
        if let Body::AttrReader(iv) = unsafe { &(*m).body } {
            if let Some(i) = attr_index(m, recv, *iv) {
                if let Some(&v) = obj(recv).ivars.get(i) {
                    if v != UNDEF {
                        record_attr_site(cache, m, recv, i);
                        return v;
                    }
                }
            }
        }
    }
    unsafe { get_found(m, recv, mid, mode) }
}

/// `recv.mid = a` (`explicit` as for `kn_send1`); see `kn_attr_get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_attr_set(cache: *mut CallCache, recv: V, mid: V, a: V, explicit: u64) -> V {
    let (m, cache) = unsafe { dispatch_keep(cache, recv, mid) };
    if !m.is_null() && matches!(unsafe { &(*m).body }, Body::AttrWriter(_)) {
        return unsafe { attr_set_site(cache, m, recv, mid, a, explicit) };
    }
    unsafe { send1_found(m, recv, mid, a, explicit) }
}
#[inline(never)]
unsafe fn attr_set_site(cache: *mut CallCache, m: *const Method, recv: V, mid: V, a: V, explicit: u64) -> V {
    if is_kind(recv, Kind::Object) && hdr(recv).flags & FL_FROZEN == 0 && (explicit == 0 || unsafe { (*m).vis } == VIS_PUBLIC) {
        if let Body::AttrWriter(iv) = unsafe { &(*m).body } {
            if let Some(i) = attr_index(m, recv, *iv) {
                let o = obj(recv);
                if i < o.ivars.len() {
                    o.ivars[i] = a; // (through the write barrier)
                    record_attr_site(cache, m, recv, i);
                    return a;
                }
            }
        }
    }
    unsafe { send1_found(m, recv, mid, a, explicit) }
}

/// Let compiled code access slot `i` of `recv`'s class inline at this site (it reached
/// attribute method `m` for that class).
#[inline(always)]
fn record_attr_site(cache: *mut CallCache, m: *const Method, recv: V, i: usize) {
    let c = unsafe { &*cache };
    if c.attr.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        record_attr_site_slow(c, m, recv, i);
    }
}
#[inline(never)]
fn record_attr_site_slow(c: &CallCache, m: *const Method, recv: V, i: usize) {
    use std::sync::atomic::Ordering::*;
    let k = hdr(recv).klass;
    if k as u64 >> 48 != 0 || i > 0xFFFF {
        return;
    }
    let mut sites = ATTR_SITES.lock().unwrap_or_else(|e| e.into_inner());
    // count first, then check that the site's cache still maps the class to `m` under the
    // current serial: a concurrent `clear_attr_sites` either sees the count (and clears
    // this site once we unlock) or bumped the serial before our check
    ATTR_SITES_N.fetch_add(1, SeqCst);
    let serial = SERIAL.load(SeqCst);
    let e = c.entry.load(Acquire);
    let current = !e.is_null() && {
        let e = unsafe { &*e };
        e.serial == serial && (0..e.n.min(IC_WAYS)).any(|w| e.klass[w] == k && e.method[w] == m)
    };
    if !current || c.attr.load(Relaxed) != 0 {
        ATTR_SITES_N.fetch_sub(1, SeqCst);
        return;
    }
    c.attr.store(((k as u64) << 16) | i as u64, Release);
    sites.push(c as *const CallCache as usize);
}

/// The method serial changed (a method defined or removed, a module included, a singleton
/// class freed...): no site may keep using a recorded attribute.
pub fn clear_attr_sites() {
    use std::sync::atomic::Ordering::*;
    if ATTR_SITES_N.load(SeqCst) == 0 {
        return;
    }
    let mut sites = ATTR_SITES.lock().unwrap_or_else(|e| e.into_inner());
    for p in sites.drain(..) {
        unsafe { &*(p as *const CallCache) }.attr.store(0, Release);
    }
    ATTR_SITES_N.store(0, SeqCst);
}

/// A freed eval unit's call caches (their memory gets reused).
pub fn forget_attr_sites(caches: impl Iterator<Item = usize>) {
    if ATTR_SITES_N.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        return;
    }
    let gone: std::collections::HashSet<usize> = caches.collect();
    let mut sites = ATTR_SITES.lock().unwrap_or_else(|e| e.into_inner());
    let before = sites.len();
    sites.retain(|p| !gone.contains(p));
    ATTR_SITES_N.fetch_sub(before - sites.len(), std::sync::atomic::Ordering::SeqCst);
}
