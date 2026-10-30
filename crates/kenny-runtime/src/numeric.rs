//! Integer (Fixnum + Bignum) and Float.

use crate::core::*;
use crate::fmt::*;
use crate::kernel::new_str_v;
use crate::tri;
use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{Signed, ToPrimitive, Zero};

pub enum N {
    I(i64),
    B(BigInt),
    F(f64),
}
pub fn num(v: V) -> Option<N> {
    if is_fix(v) {
        Some(N::I(fix(v)))
    } else if is_float(v) {
        Some(N::F(float_val(v)))
    } else if is_big(v) {
        Some(N::B(rbig(v).n.clone()))
    } else {
        None
    }
}
fn big(n: &N) -> BigInt {
    match n {
        N::I(i) => BigInt::from(*i),
        N::B(b) => b.clone(),
        N::F(f) => BigInt::from(*f as i128),
    }
}
fn flt(n: &N) -> f64 {
    match n {
        N::I(i) => *i as f64,
        N::B(b) => b.to_f64().unwrap_or(f64::INFINITY),
        N::F(f) => *f,
    }
}
fn coerce_fail(a: V, b: V) -> V {
    let what = if b == QNIL {
        "nil".to_string()
    } else if is_str(b) || is_ary(b) || is_hash(b) {
        class_name(real_class(b))
    } else {
        inspect_lossy(b)
    };
    type_error(&format!("{} can't be coerced into {}", what, class_name(real_class(a))))
}
fn zerodiv() -> V {
    raise(rt().c.zero_div_error, "divided by 0")
}

#[derive(Clone, Copy, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

/// An Integer that fits in an i128 (Fixnums, small Bignums), without copying a Bignum.
#[inline]
fn int128(v: V) -> Option<i128> {
    if is_fix(v) {
        Some(fix(v) as i128)
    } else if is_big(v) {
        rbig(v).n.to_i128()
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn __cospi(x: f64) -> f64;
    fn __sinpi(x: f64) -> f64;
}
#[cfg(target_os = "macos")]
fn cospi(x: f64) -> f64 {
    unsafe { __cospi(x) }
}
#[cfg(target_os = "macos")]
fn sinpi(x: f64) -> f64 {
    unsafe { __sinpi(x) }
}
#[cfg(not(target_os = "macos"))]
fn cospi(x: f64) -> f64 {
    (std::f64::consts::PI * x).cos()
}
#[cfg(not(target_os = "macos"))]
fn sinpi(x: f64) -> f64 {
    (std::f64::consts::PI * x).sin()
}
/// `p ** q` for a negative base and a fractional exponent: a Complex, as CRuby's
/// rb_dbl_complex_new_polar_pi(pow(-p, q), q).
fn neg_pow(p: f64, q: f64) -> V {
    let mut abs = (-p).powf(q);
    let fi = q.trunc();
    let fr = q - fi;
    let pos = fr == 0.5;
    if pos || fr == -0.5 {
        let h = fi / 2.0;
        if ((h - h.trunc()) != fr) ^ pos {
            abs = -abs;
        }
        return crate::rational::complex_new(float_new(0.0), float_new(abs));
    }
    crate::rational::complex_new(float_new(abs * cospi(q)), float_new(abs * sinpi(q)))
}

pub fn arith(op: Op, a: V, b: V) -> V {
    // Integers within i128 (the usual Bignums): no BigInt copies unless the result needs one
    if let (Some(p), Some(q)) = (int128(a), int128(b)) {
        let r = match op {
            Op::Add => p.checked_add(q),
            Op::Sub => p.checked_sub(q),
            Op::Mul => p.checked_mul(q),
            Op::Div if q != 0 && !(p == i128::MIN && q == -1) => Some(num_integer::Integer::div_floor(&p, &q)),
            Op::Mod if q != 0 && !(p == i128::MIN && q == -1) => Some(p.mod_floor(&q)),
            _ => None,
        };
        if let Some(r) = r {
            return int_from_i128(r);
        }
    }
    let (Some(x), Some(y)) = (num(a), num(b)) else {
        if num(a).is_some() && is_heap(b) && respond_to(b, sym("coerce")) {
            let pair = tri!(send1(b, "coerce", a));
            if is_ary(pair) && rary(pair).v.len() == 2 {
                let (p, q) = (rary(pair).v[0], rary(pair).v[1]);
                let name = match op {
                    Op::Add => "+",
                    Op::Sub => "-",
                    Op::Mul => "*",
                    Op::Div => "/",
                    Op::Mod => "%",
                    Op::Pow => "**",
                };
                return send1(p, name, q);
            }
        }
        return coerce_fail(a, b);
    };
    match (&x, &y) {
        (N::F(_), _) | (_, N::F(_)) => {
            let (p, q) = (flt(&x), flt(&y));
            if op == Op::Pow && p < 0.0 && q.is_finite() && q != q.trunc() {
                return neg_pow(p, q);
            }
            float_new(match op {
                Op::Add => p + q,
                Op::Sub => p - q,
                Op::Mul => p * q,
                Op::Div => p / q,
                Op::Mod => {
                    if q == 0.0 {
                        f64::NAN
                    } else {
                        let m = p % q;
                        if m != 0.0 && (m < 0.0) != (q < 0.0) { m + q } else { m }
                    }
                }
                Op::Pow => p.powf(q),
            })
        }
        (N::I(p), N::I(q)) => {
            let (p, q) = (*p as i128, *q as i128);
            match op {
                Op::Add => int_from_i128(p + q),
                Op::Sub => int_from_i128(p - q),
                Op::Mul => match p.checked_mul(q) {
                    Some(r) => int_from_i128(r),
                    None => int_from_big(BigInt::from(p) * BigInt::from(q)),
                },
                Op::Div => {
                    if q == 0 {
                        return zerodiv();
                    }
                    int_from_i128(num_integer::Integer::div_floor(&p, &q))
                }
                Op::Mod => {
                    if q == 0 {
                        return zerodiv();
                    }
                    int_from_i128(p.mod_floor(&q))
                }
                Op::Pow => int_pow(BigInt::from(p), q),
            }
        }
        _ => {
            let (p, q) = (big(&x), big(&y));
            match op {
                Op::Add => int_from_big(p + q),
                Op::Sub => int_from_big(p - q),
                Op::Mul => int_from_big(p * q),
                Op::Div => {
                    if q.is_zero() {
                        return zerodiv();
                    }
                    int_from_big(p.div_floor(&q))
                }
                Op::Mod => {
                    if q.is_zero() {
                        return zerodiv();
                    }
                    int_from_big(p.mod_floor(&q))
                }
                Op::Pow => match q.to_i128() {
                    Some(e) => int_pow(p, e),
                    None => float_new(f64::INFINITY),
                },
            }
        }
    }
}
fn int_pow(base: BigInt, e: i128) -> V {
    if e < 0 {
        return float_new(base.to_f64().unwrap_or(0.0).powf(e as f64));
    }
    if e > u32::MAX as i128 {
        return arg_error("exponent too large");
    }
    int_from_big(num_traits::pow(base, e as usize))
}

/// <=> for numbers; None if not comparable.
pub fn num_cmp(a: V, b: V) -> Option<std::cmp::Ordering> {
    if let (Some(p), Some(q)) = (int128(a), int128(b)) {
        return Some(p.cmp(&q));
    }
    let (x, y) = (num(a)?, num(b)?);
    match (&x, &y) {
        (N::I(p), N::I(q)) => Some(p.cmp(q)),
        (N::F(p), N::F(q)) => p.partial_cmp(q),
        (N::F(f), _) => int_flt_cmp(&y, *f).map(|o| o.reverse()),
        (_, N::F(f)) => int_flt_cmp(&x, *f),
        _ => Some(big(&x).cmp(&big(&y))),
    }
}
/// Exact Integer <=> Float (like CRuby's rb_integer_float_cmp: no rounding of large integers).
fn int_flt_cmp(i: &N, f: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 { Ordering::Less } else { Ordering::Greater });
    }
    if let N::I(x) = i {
        if x.unsigned_abs() < (1u64 << 53) {
            return (*x as f64).partial_cmp(&f);
        }
    }
    let t = f.trunc();
    let bt = <BigInt as num_traits::FromPrimitive>::from_f64(t)?;
    match big(i).cmp(&bt) {
        Ordering::Equal => 0.0f64.partial_cmp(&(f - t)),
        o => Some(o),
    }
}
fn ord_v(o: std::cmp::Ordering) -> V {
    mkfix(o as i64)
}
/// `a op b` through `b.coerce(a)` when b is a non-builtin numeric (BigDecimal etc.).
fn coerce_op(a: V, b: V, op: &str) -> Option<V> {
    if num(b).is_some() || !is_heap(b) || is_str(b) || !respond_to(b, sym("coerce")) {
        return None;
    }
    let pair = send1(b, "coerce", a);
    if pair == UNDEF {
        return Some(UNDEF);
    }
    if is_ary(pair) && rary(pair).v.len() == 2 {
        let (p, q) = (rary(pair).v[0], rary(pair).v[1]);
        return Some(send1(p, op, q));
    }
    None
}
fn cmp_or_fail(a: V, b: V, f: fn(std::cmp::Ordering) -> bool) -> V {
    match num_cmp(a, b) {
        Some(o) => bool_v(f(o)),
        None => {
            if num(b).is_some() {
                QFALSE // NaN
            } else if let Some(r) = coerce_op(a, b, "<=>") {
                if r == UNDEF {
                    return UNDEF;
                }
                if is_fix(r) { bool_v(f(fix(r).cmp(&0))) } else { arg_error(&format!("comparison of {} with {} failed", class_name(real_class(a)), crate::kernel::cmperr_desc(b))) }
            } else {
                arg_error(&format!("comparison of {} with {} failed", class_name(real_class(a)), crate::kernel::cmperr_desc(b)))
            }
        }
    }
}
fn num_eq(a: V, b: V) -> V {
    match num_cmp(a, b) {
        Some(o) => bool_v(o == std::cmp::Ordering::Equal),
        None => {
            if num(b).is_none() && is_heap(b) && !is_str(b) {
                return send1(b, "==", a);
            }
            QFALSE
        }
    }
}

fn int_round_digits(v: V, digits: i64, mode: fn(f64) -> f64) -> V {
    if digits >= 0 {
        return v;
    }
    let p = BigInt::from(10).pow((-digits) as u32);
    let n = if is_fix(v) { BigInt::from(fix(v)) } else { rbig(v).n.clone() };
    let q = n.to_f64().unwrap() / p.to_f64().unwrap();
    int_from_big(BigInt::from(mode(q) as i128) * p)
}
fn round_half_up(x: f64) -> f64 {
    x.round()
}
fn float_round(f: f64, digits: i64, mode: fn(f64) -> f64) -> V {
    if digits > 0 {
        if !f.is_finite() {
            return float_new(f);
        }
        let m = 10f64.powi(digits as i32);
        let r = mode(f * m) / m;
        return float_new(if r.is_finite() { r } else { f });
    }
    if !f.is_finite() {
        return raise(rt().c.float_domain_error, &float_to_s(f));
    }
    let r = if digits == 0 {
        mode(f)
    } else {
        let m = 10f64.powi((-digits) as i32);
        mode(f / m) * m
    };
    int_from_big(BigInt::from(r as i128))
}
fn digits_arg(a: &[V]) -> i64 {
    a.first().filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0)
}

pub fn int_to_s_radix(v: V, radix: u32) -> String {
    if is_fix(v) && radix == 10 {
        return fix(v).to_string();
    }
    let n = if is_fix(v) { BigInt::from(fix(v)) } else { rbig(v).n.clone() };
    n.to_str_radix(radix)
}

pub fn init() {
    let r = rt();
    let (int, flo, numc) = (r.c.integer, r.c.float, r.c.numeric);

    for c in [int, flo] {
        defn(c, "+", 1, 1, |s, a, _| arith(Op::Add, s, a[0]));
        defn(c, "-", 1, 1, |s, a, _| arith(Op::Sub, s, a[0]));
        defn(c, "*", 1, 1, |s, a, _| arith(Op::Mul, s, a[0]));
        defn(c, "/", 1, 1, |s, a, _| arith(Op::Div, s, a[0]));
        defn(c, "%", 1, 1, |s, a, _| arith(Op::Mod, s, a[0]));
        defn(c, "modulo", 1, 1, |s, a, _| arith(Op::Mod, s, a[0]));
        defn(c, "**", 1, 1, |s, a, _| arith(Op::Pow, s, a[0]));
        defn(c, "pow", 1, 2, |s, a, _| {
            if a.len() == 2 {
                let (b, e, m) = (big(&num(s).unwrap()), num(a[0]).map(|n| big(&n)), num(a[1]).map(|n| big(&n)));
                if let (Some(e), Some(m)) = (e, m) {
                    return int_from_big(b.modpow(&e, &m));
                }
            }
            arith(Op::Pow, s, a[0])
        });
        defn(c, "<=>", 1, 1, |s, a, _| match num_cmp(s, a[0]) {
            Some(o) => ord_v(o),
            None => coerce_op(s, a[0], "<=>").unwrap_or(QNIL),
        });
        defn(c, "==", 1, 1, |s, a, _| num_eq(s, a[0]));
        defn(c, "===", 1, 1, |s, a, _| num_eq(s, a[0]));
        defn(c, "<", 1, 1, |s, a, _| cmp_or_fail(s, a[0], |o| o.is_lt()));
        defn(c, "<=", 1, 1, |s, a, _| cmp_or_fail(s, a[0], |o| o.is_le()));
        defn(c, ">", 1, 1, |s, a, _| cmp_or_fail(s, a[0], |o| o.is_gt()));
        defn(c, ">=", 1, 1, |s, a, _| cmp_or_fail(s, a[0], |o| o.is_ge()));
        defn(c, "coerce", 1, 1, |s, a, _| {
            if is_int(s) && is_int(a[0]) {
                ary_new(vec![a[0], s])
            } else {
                match (num(a[0]), num(s)) {
                    (Some(x), Some(y)) => ary_new(vec![float_new(flt(&x)), float_new(flt(&y))]),
                    _ => coerce_fail(s, a[0]),
                }
            }
        });
        // Float#divmod / #div (and Integer's with a Float) by zero: ZeroDivisionError
        fn float_by_zero(s: V, d: V) -> bool {
            (is_float(s) || is_float(d)) && num_cmp(d, mkfix(0)) == Some(std::cmp::Ordering::Equal)
        }
        defn(c, "divmod", 1, 1, |s, a, _| {
            if float_by_zero(s, a[0]) {
                return zerodiv();
            }
            let q = tri!(arith(Op::Div, s, a[0]));
            let q = if is_float(q) { float_round(float_val(q), 0, f64::floor) } else { q };
            if q == UNDEF {
                return UNDEF;
            }
            let m = tri!(arith(Op::Mod, s, a[0]));
            ary_new(vec![q, m])
        });
        defn(c, "div", 1, 1, |s, a, _| {
            if float_by_zero(s, a[0]) {
                return zerodiv();
            }
            let q = tri!(arith(Op::Div, s, a[0]));
            if is_float(q) { float_round(float_val(q), 0, f64::floor) } else { q }
        });
        defn(c, "fdiv", 1, 1, |s, a, _| match (num(s), num(a[0])) {
            (Some(x), Some(y)) => float_new(flt(&x) / flt(&y)),
            _ => coerce_fail(s, a[0]),
        });
        defn(c, "remainder", 1, 1, |s, a, _| {
            let m = tri!(arith(Op::Mod, s, a[0]));
            let sn = num_cmp(s, mkfix(0)) == Some(std::cmp::Ordering::Less);
            let mn = num_cmp(m, mkfix(0)) == Some(std::cmp::Ordering::Less);
            if num_cmp(m, mkfix(0)) != Some(std::cmp::Ordering::Equal) && sn != mn { arith(Op::Sub, m, a[0]) } else { m }
        });
        defn(c, "-@", 0, 0, |s, _, _| if is_float(s) { float_new(-float_val(s)) } else { arith(Op::Sub, mkfix(0), s) });
        defn(c, "+@", 0, 0, |s, _, _| s);
        defn(c, "abs", 0, 0, |s, _, _| match num(s).unwrap() {
            N::I(i) => int_from_i128((i as i128).abs()),
            N::B(b) => int_from_big(b.abs()),
            N::F(f) => float_new(f.abs()),
        });
        defn(c, "magnitude", 0, 0, |s, _, _| send0(s, "abs"));
        defn(c, "zero?", 0, 0, |s, _, _| {
            if is_fix(s) {
                return bool_v(s == mkfix(0));
            }
            bool_v(num_cmp(s, mkfix(0)) == Some(std::cmp::Ordering::Equal))
        });
        defn(c, "nonzero?", 0, 0, |s, _, _| if num_cmp(s, mkfix(0)) == Some(std::cmp::Ordering::Equal) { QNIL } else { s });
        defn(c, "positive?", 0, 0, |s, _, _| bool_v(num_cmp(s, mkfix(0)) == Some(std::cmp::Ordering::Greater)));
        defn(c, "negative?", 0, 0, |s, _, _| bool_v(num_cmp(s, mkfix(0)) == Some(std::cmp::Ordering::Less)));
        defn(c, "to_f", 0, 0, |s, _, _| float_new(flt(&num(s).unwrap())));
        defn(c, "to_c", 0, 0, |s, _, _| s);
        defn(c, "integer?", 0, 0, |s, _, _| bool_v(is_int(s)));
        // step(limit = nil, step = 1), step(by:, to:); no limit: endless
        defn(c, "step", 0, 3, |s, a, b| {
            if !is_proc(b) {
                return crate::misc::enum_for(s, "step", a);
            }
            let (mut pos, mut kw) = (a.to_vec(), None);
            if pos.last().is_some_and(|h| is_hash(*h)) {
                kw = pos.pop();
            }
            let mut limit = pos.first().copied().unwrap_or(QNIL);
            let mut step = pos.get(1).copied().unwrap_or(mkfix(1));
            if let Some(h) = kw {
                for (k, v) in crate::hash::pairs(h) {
                    match sym_str(sym_id(k)).as_str() {
                        "by" => step = v,
                        "to" => limit = v,
                        _ => {}
                    }
                }
            }
            if step == mkfix(0) && !is_float(step) {
                return arg_error("step can't be 0");
            }
            let up = num_cmp(step, mkfix(0)) != Some(std::cmp::Ordering::Less);
            let mut i = s;
            if limit == QNIL {
                loop {
                    tri!(call_block(b, &[i]));
                    i = tri!(arith(Op::Add, i, step));
                }
            }
            if is_float(s) || is_float(step) || is_float(limit) {
                let (st, sp, lm) = (flt(&num(s).unwrap()), flt(&num(step).unwrap()), flt(&num(limit).unwrap()));
                let n = ((lm - st) / sp + 1e-9).floor() as i64;
                for k in 0..=n {
                    tri!(call_block(b, &[float_new(st + k as f64 * sp)]));
                }
                return s;
            }
            loop {
                let c = num_cmp(i, limit).unwrap();
                if (up && c.is_gt()) || (!up && c.is_lt()) {
                    break;
                }
                tri!(call_block(b, &[i]));
                i = tri!(arith(Op::Add, i, step));
            }
            s
        });
        defn(c, "clamp", 1, 2, |s, a, _| crate::kernel::clamp(s, a));
        defn(c, "between?", 2, 2, |s, a, _| {
            bool_v(num_cmp(s, a[0]).is_some_and(|o| o.is_ge()) && num_cmp(s, a[1]).is_some_and(|o| o.is_le()))
        });
    }
    let _ = numc;

    // ---- Integer ----
    defn(int, "to_s", 0, 1, |s, a, _| {
        let radix = a.first().filter(|x| is_fix(**x)).map(|x| fix(*x) as u32).unwrap_or(10);
        new_str_v(int_to_s_radix(s, radix))
    });
    defn(int, "inspect", 0, 0, |s, _, _| new_str_v(int_to_s(s)));
    defn(int, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(int, "eql?", 1, 1, |s, a, _| bool_v(is_int(a[0]) && eql(s, a[0])));
    defn(int, "to_i", 0, 0, |s, _, _| s);
    defn(int, "to_int", 0, 0, |s, _, _| s);
    defn(int, "to_r", 0, 0, |s, _, _| s);
    defn(int, "floor", 0, 1, |s, a, _| int_round_digits(s, digits_arg(a), f64::floor));
    defn(int, "ceil", 0, 1, |s, a, _| int_round_digits(s, digits_arg(a), f64::ceil));
    defn(int, "round", 0, 2, |s, a, _| int_round_digits(s, digits_arg(a), round_half_up));
    defn(int, "truncate", 0, 1, |s, a, _| int_round_digits(s, digits_arg(a), f64::trunc));
    defn(int, "times", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "times", &[]);
        }
        // (a Bignum: more Fixnums than any loop gets through, if positive)
        let n = if is_fix(s) {
            fix(s)
        } else if truthy(tri!(send(s, sym(">"), &[mkfix(0)], QNIL))) {
            FIX_MAX
        } else {
            0
        };
        let mut i = 0;
        while i < n {
            tri!(call_block(b, &[mkfix(i)]));
            i += 1;
        }
        s
    });
    defn(int, "upto", 1, 1, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "upto", a);
        }
        let (lo, hi) = (fix(s), if is_fix(a[0]) { fix(a[0]) } else if is_float(a[0]) { (float_val(a[0]).floor() as i64).clamp(FIX_MIN, FIX_MAX - 1) } else { return type_error("upto: Integer expected") });
        let mut i = lo;
        while i <= hi {
            tri!(call_block(b, &[mkfix(i)]));
            i += 1;
        }
        s
    });
    defn(int, "downto", 1, 1, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "downto", a);
        }
        let (hi, lo) = (fix(s), if is_fix(a[0]) { fix(a[0]) } else if is_float(a[0]) { (float_val(a[0]).ceil() as i64).clamp(FIX_MIN + 1, FIX_MAX) } else { return type_error("downto: Integer expected") });
        let mut i = hi;
        while i >= lo {
            tri!(call_block(b, &[mkfix(i)]));
            i -= 1;
        }
        s
    });
    defn(int, "succ", 0, 0, |s, _, _| arith(Op::Add, s, mkfix(1)));
    defn(int, "next", 0, 0, |s, _, _| arith(Op::Add, s, mkfix(1)));
    defn(int, "pred", 0, 0, |s, _, _| arith(Op::Sub, s, mkfix(1)));
    defn(int, "even?", 0, 0, |s, _, _| bool_v(if is_fix(s) { fix(s) & 1 == 0 } else { rbig(s).n.is_even() }));
    defn(int, "odd?", 0, 0, |s, _, _| bool_v(if is_fix(s) { fix(s) & 1 != 0 } else { rbig(s).n.is_odd() }));
    defn(int, "chr", 0, 1, |s, a, _| {
        let c = fix(s);
        // with an encoding: UTF-8 encodes the code point, ASCII-8BIT/US-ASCII take one byte
        let enc = match a.first() {
            Some(&e) => {
                let n = match crate::fmt::to_s(e) {
                    Ok(n) => n,
                    Err(_) => return UNDEF,
                };
                Some(String::from_utf8_lossy(&n).to_ascii_uppercase())
            }
            None => None,
        };
        let utf8 = matches!(enc.as_deref(), Some("UTF-8" | "UTF8" | "CP65001"));
        if c < 0 || (c > 255 && matches!(enc.as_deref(), Some("ASCII-8BIT" | "BINARY"))) || (c > 127 && matches!(enc.as_deref(), Some("US-ASCII" | "ASCII"))) {
            return raise(rt().c.range_error, &format!("{c} out of char range"));
        }
        if c < 128 || (c < 256 && !utf8) {
            let out = str_new(vec![c as u8]);
            if c >= 128 {
                hdr(out).flags |= crate::compat::FL_BINARY;
            }
            out
        } else {
            match char::from_u32(c as u32) {
                Some(ch) => new_str_v(ch.to_string()),
                None => raise(rt().c.range_error, &format!("{c} out of char range")),
            }
        }
    });
    defn(int, "ord", 0, 0, |s, _, _| s);
    defn(int, "size", 0, 0, |_, _, _| mkfix(8));
    defn(int, "finite?", 0, 0, |_, _, _| QTRUE);
    defn(int, "infinite?", 0, 0, |_, _, _| QNIL);
    defn(int, "nan?", 0, 0, |_, _, _| QFALSE);
    defn(int, "gcd", 1, 1, |s, a, _| int_from_big(big(&num(s).unwrap()).gcd(&big(&num(a[0]).unwrap()))));
    defn(int, "lcm", 1, 1, |s, a, _| int_from_big(big(&num(s).unwrap()).lcm(&big(&num(a[0]).unwrap()))));
    defn(int, "bit_length", 0, 0, |s, _, _| mkfix(big(&num(s).unwrap()).bits() as i64));
    defn(int, "digits", 0, 1, |s, a, _| {
        let base = a.first().map(|x| fix(*x)).unwrap_or(10);
        let mut n = big(&num(s).unwrap());
        if n.is_negative() {
            return raise(rt().c.range_error, "out of domain");
        }
        let b = BigInt::from(base);
        let mut out = vec![];
        loop {
            let (q, r) = n.div_rem(&b);
            out.push(int_from_big(r));
            n = q;
            if n.is_zero() {
                break;
            }
        }
        ary_new(out)
    });
    fn bitop(s: V, o: V, f: fn(BigInt, BigInt) -> BigInt, g: fn(i64, i64) -> i64) -> V {
        if is_fix(s) && is_fix(o) {
            return mkfix(g(fix(s), fix(o)));
        }
        match (num(s), num(o)) {
            (Some(x @ (N::I(_) | N::B(_))), Some(y @ (N::I(_) | N::B(_)))) => int_from_big(f(big(&x), big(&y))),
            _ => coerce_fail(s, o),
        }
    }
    defn(int, "&", 1, 1, |s, a, _| bitop(s, a[0], |x, y| x & y, |x, y| x & y));
    defn(int, "|", 1, 1, |s, a, _| bitop(s, a[0], |x, y| x | y, |x, y| x | y));
    defn(int, "^", 1, 1, |s, a, _| bitop(s, a[0], |x, y| x ^ y, |x, y| x ^ y));
    defn(int, "~", 0, 0, |s, _, _| int_from_big(!big(&num(s).unwrap())));
    /// `s << n` (a negative n shifts right)
    fn shift_left(s: V, n: V) -> V {
        let n = if is_fix(n) {
            fix(n)
        } else if is_float(n) {
            float_val(n) as i64
        } else if is_big(n) {
            // only a right shift by that much is representable: 0 or -1
            return if rbig(n).n.is_negative() { mkfix(if big(&num(s).unwrap()).is_negative() { -1 } else { 0 }) } else { raise(rt().c.range_error, "shift width too big") };
        } else {
            return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(n))));
        };
        if is_fix(s) {
            let x = fix(s);
            if (0..63).contains(&n) {
                let r = x.checked_shl(n as u32).filter(|r| r >> n == x);
                if let Some(r) = r.filter(|r| (FIX_MIN..=FIX_MAX).contains(r)) {
                    return mkfix(r);
                }
            } else if n < 0 {
                return mkfix(if n <= -63 { if x < 0 { -1 } else { 0 } } else { x >> (-n) });
            }
        }
        let b = big(&num(s).unwrap());
        if n > (1 << 32) {
            return raise(rt().c.range_error, "shift width too big");
        }
        int_from_big(if n >= 0 { b << n as usize } else { b >> n.unsigned_abs() as usize })
    }
    defn(int, "<<", 1, 1, |s, a, _| shift_left(s, a[0]));
    defn(int, ">>", 1, 1, |s, a, _| {
        let n = a[0];
        if is_fix(n) {
            return shift_left(s, mkfix(fix(n).checked_neg().unwrap_or(i64::MAX)));
        }
        if is_float(n) {
            return shift_left(s, mkfix(-(float_val(n) as i64)));
        }
        if is_big(n) {
            return if rbig(n).n.is_negative() { raise(rt().c.range_error, "shift width too big") } else { mkfix(if big(&num(s).unwrap()).is_negative() { -1 } else { 0 }) };
        }
        shift_left(s, n)
    });
    // Integer#[](i), [](offset, len), [](range): bits, as CRuby's int_aref1/int_aref2
    defn(int, "[]", 1, 2, |s, a, _| {
        let n = big(&num(s).unwrap());
        // an index: an Integer, else through to_int (a Float truncates)
        let index = |v: V| -> Result<BigInt, V> {
            if is_int(v) {
                return Ok(big(&num(v).unwrap()));
            }
            if is_float(v) {
                return Ok(BigInt::from(float_val(v).trunc() as i64));
            }
            if v == QNIL {
                return Err(type_error(if is_fix(s) { "no implicit conversion of nil into Integer" } else { "no implicit conversion from nil to integer" }));
            }
            if is_heap(v) && respond_to(v, sym("to_int")) {
                let r = send0(v, "to_int");
                if r == UNDEF {
                    return Err(UNDEF);
                }
                if is_int(r) {
                    return Ok(big(&num(r).unwrap()));
                }
            }
            Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v)))))
        };
        // n >> k for any k (a negative one shifts left)
        let shr = |n: &BigInt, k: &BigInt| -> BigInt {
            match k.to_i64() {
                Some(k) if k >= 0 => if k > (1 << 40) { if n.sign() == Sign::Minus { BigInt::from(-1) } else { BigInt::from(0) } } else { n >> (k as usize) },
                Some(k) => n << ((-k).min(1 << 20) as usize),
                None if k.sign() == Sign::Minus => n << (1usize << 20),
                None => if n.sign() == Sign::Minus { BigInt::from(-1) } else { BigInt::from(0) },
            }
        };
        let mask = |len: &BigInt| -> BigInt { shr(&BigInt::from(1), &-len) - BigInt::from(1) };
        let bit = |n: &BigInt, i: &BigInt| -> V {
            if i.sign() == Sign::Minus {
                return mkfix(0);
            }
            mkfix((shr(n, i) & BigInt::from(1)).to_i64().unwrap_or(0))
        };
        if a.len() == 2 {
            let (b, l) = match (index(a[0]), index(a[1])) {
                (Ok(b), Ok(l)) => (b, l),
                (Err(e), _) | (_, Err(e)) => return e,
            };
            return int_from_big(shr(&n, &b) & mask(&l));
        }
        if is_range(a[0]) {
            let r = rrange(a[0]);
            let end = if r.hi == QNIL { None } else { Some(match index(r.hi) { Ok(x) => x, Err(e) => return e }) };
            if r.lo == QNIL {
                let Some(e) = end else { return arg_error("The beginless range for Integer#[] results in infinity") };
                if e.sign() == Sign::Minus {
                    return mkfix(0);
                }
                let e = if r.excl { e } else { e + 1 };
                if (&n & mask(&e)) == BigInt::from(0) {
                    return mkfix(0);
                }
                return arg_error("The beginless range for Integer#[] results in infinity");
            }
            let b = match index(r.lo) {
                Ok(x) => x,
                Err(e) => return e,
            };
            let shifted = shr(&n, &b);
            return match end {
                Some(e) if b < e => {
                    let len = &e - &b + if r.excl { 0 } else { 1 };
                    int_from_big(shifted & mask(&len))
                }
                Some(e) if b == e => if r.excl { mkfix(0) } else { bit(&n, &b) },
                _ => int_from_big(shifted),
            };
        }
        match index(a[0]) {
            Ok(i) => bit(&n, &i),
            Err(e) => e,
        }
    });
    defs(int, "sqrt", 1, 1, |_, a, _| int_from_big(big(&num(a[0]).unwrap()).sqrt()));

    // ---- Float ----
    defn(flo, "to_s", 0, 0, |s, _, _| new_str_v(float_to_s(float_val(s))));
    defn(flo, "inspect", 0, 0, |s, _, _| new_str_v(float_to_s(float_val(s))));
    defn(flo, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(flo, "eql?", 1, 1, |s, a, _| bool_v(is_float(a[0]) && float_val(s) == float_val(a[0])));
    defn(flo, "to_i", 0, 0, |s, _, _| float_round(float_val(s), 0, f64::trunc));
    defn(flo, "to_int", 0, 0, |s, _, _| float_round(float_val(s), 0, f64::trunc));
    defn(flo, "truncate", 0, 1, |s, a, _| float_round(float_val(s), digits_arg(a), f64::trunc));
    defn(flo, "floor", 0, 1, |s, a, _| float_round(float_val(s), digits_arg(a), f64::floor));
    defn(flo, "ceil", 0, 1, |s, a, _| float_round(float_val(s), digits_arg(a), f64::ceil));
    defn(flo, "round", 0, 2, |s, a, _| {
        let half_even = a.iter().any(|h| is_hash(*h) && rhash(*h).m.values().any(|v| *v == mksym(sym("even"))));
        let digits = digits_arg(&a.iter().copied().filter(|x| !is_hash(*x)).collect::<Vec<_>>());
        if half_even { float_round(float_val(s), digits, |x| x.round_ties_even()) } else { float_round(float_val(s), digits, round_half_up) }
    });
    defn(flo, "nan?", 0, 0, |s, _, _| bool_v(float_val(s).is_nan()));
    defn(flo, "finite?", 0, 0, |s, _, _| bool_v(float_val(s).is_finite()));
    defn(flo, "infinite?", 0, 0, |s, _, _| {
        let f = float_val(s);
        if f.is_infinite() { mkfix(if f > 0.0 { 1 } else { -1 }) } else { QNIL }
    });
    defn(flo, "next_float", 0, 0, |s, _, _| float_new(f64::from_bits(float_val(s).to_bits() + 1)));
    let fc = flo as *mut Class;
    const_set(fc, sym("INFINITY"), float_new(f64::INFINITY));
    const_set(fc, sym("NAN"), float_new(f64::NAN));
    const_set(fc, sym("EPSILON"), float_new(f64::EPSILON));
    const_set(fc, sym("MAX"), float_new(f64::MAX));
    const_set(fc, sym("MIN"), float_new(f64::MIN_POSITIVE));
    const_set(fc, sym("DIG"), mkfix(15));
    const_set(fc, sym("MANT_DIG"), mkfix(53));
    const_set(fc, sym("RADIX"), mkfix(2));
    const_set(fc, sym("MAX_EXP"), mkfix(1024));
    const_set(fc, sym("MIN_EXP"), mkfix(-1021));
    const_set(fc, sym("MAX_10_EXP"), mkfix(308));
    const_set(fc, sym("MIN_10_EXP"), mkfix(-307));
}
