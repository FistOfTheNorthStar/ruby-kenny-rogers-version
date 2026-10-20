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
    tag(c.integer, "times", OPT_INT_TIMES);
    tag(c.integer, "upto", OPT_INT_UPTO);
    tag(c.integer, "downto", OPT_INT_DOWNTO);
    tag(c.range, "each", OPT_RANGE_EACH);
}

use crate::abi::dispatch;

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
            pst().native_depth += 1;
            let r = f(recv, args, QNIL);
            pst().native_depth -= 1;
            r
        }
        Body::AttrReader(iv) if args.is_empty() => attr_get(recv, *iv),
        Body::AttrWriter(iv) if args.len() == 1 => ivar_set(recv, *iv, args[0]),
        _ => invoke(m, recv, args, QNIL),
    }
}

/// `@x` read for an attr_reader (shape lookup; no per-site cache needed).
#[inline(always)]
fn attr_get(recv: V, iv: u32) -> V {
    if is_kind(recv, Kind::Object) {
        let o = obj(recv);
        let shape = unsafe { &*o.shape };
        return match shape_index(shape, iv) {
            Some(i) => o.ivars.get(i as usize).copied().filter(|x| *x != UNDEF).unwrap_or(QNIL),
            None => QNIL,
        };
    }
    ivar_get(recv, iv)
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
    match opt_of(m, explicit) {
        OPT_ARY_SIZE => return mkfix(rary(recv).v.len() as i64),
        OPT_ARY_EMPTY => return bool_v(rary(recv).v.is_empty()),
        OPT_ARY_FIRST => return rary(recv).v.first().copied().unwrap_or(QNIL),
        OPT_ARY_LAST => return rary(recv).v.last().copied().unwrap_or(QNIL),
        OPT_HASH_SIZE => return mkfix(rhash(recv).m.len() as i64),
        OPT_HASH_EMPTY => return bool_v(rhash(recv).m.is_empty()),
        OPT_STR_EMPTY => return bool_v(rstr(recv).b.is_empty()),
        OPT_NIL_P => return bool_v(recv == QNIL),
        _ => {}
    }
    unsafe { call_found(m, recv, mid, &[], explicit) }
}

/// `recv.mid(a)` with no block (also the slow path of inline binary operators).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send1(cache: *mut CallCache, recv: V, mid: V, a: V, explicit: u64) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
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
        _ => {}
    }
    unsafe { call_found(m, recv, mid, &[a], explicit) }
}

/// `recv.mid(a, b)` with no block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_send2(cache: *mut CallCache, recv: V, mid: V, a: V, b: V, explicit: u64) -> V {
    let m = unsafe { dispatch(cache, recv, mid) };
    match opt_of(m, explicit) {
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
