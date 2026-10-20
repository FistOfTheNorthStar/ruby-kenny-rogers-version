//! Rational: exact fractions (numerator/denominator kept normalized, denominator > 0).

use crate::core::*;
use crate::kernel::new_str_v;
use crate::tri;
use num_bigint::BigInt;
use num_integer::Integer as _;
use num_traits::{One, Signed, ToPrimitive, Zero};

static RATIONAL: G<*mut Class> = G::new(std::ptr::null_mut());

pub fn is_rational(v: V) -> bool {
    is_kind(v, Kind::Object) && real_class(v) == *RATIONAL.get()
}
fn to_big(v: V) -> BigInt {
    if is_fix(v) { BigInt::from(fix(v)) } else { rbig(v).n.clone() }
}
pub fn parts(v: V) -> (BigInt, BigInt) {
    (to_big(ivar_get(v, sym("@__num"))), to_big(ivar_get(v, sym("@__den"))))
}

pub fn rational_new(n: BigInt, d: BigInt) -> V {
    if d.is_zero() {
        return raise(rt().c.zero_div_error, "divided by 0");
    }
    let g = n.gcd(&d);
    let (mut n, mut d) = if g.is_zero() { (n, d) } else { (n / &g, d / &g) };
    if d.is_negative() {
        n = -n;
        d = -d;
    }
    let r = object_new(*RATIONAL.get());
    ivar_set(r, sym("@__num"), int_from_big(n));
    ivar_set(r, sym("@__den"), int_from_big(d));
    hdr(r).flags |= FL_FROZEN;
    r
}

fn f64_to_rational(f: f64) -> V {
    if !f.is_finite() {
        return raise(rt().c.float_domain_error, &crate::fmt::float_to_s(f));
    }
    // exact binary expansion: f = m * 2^e
    let bits = f.to_bits();
    let sign = if bits >> 63 == 1 { -1 } else { 1 };
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let mant = if exp == 0 { (bits & 0xfffffffffffff) << 1 } else { (bits & 0xfffffffffffff) | 0x10000000000000 };
    let e = exp - 1075;
    let mut n = BigInt::from(mant) * sign;
    let mut d = BigInt::one();
    if e > 0 {
        n <<= e as usize;
    } else {
        d <<= (-e) as usize;
    }
    rational_new(n, d)
}

/// Convert an Integer/Rational to (num, den); None for anything else.
fn as_frac(v: V) -> Option<(BigInt, BigInt)> {
    if is_int(v) {
        Some((to_big(v), BigInt::one()))
    } else if is_rational(v) {
        Some(parts(v))
    } else {
        None
    }
}
fn to_f(v: V) -> f64 {
    let (n, d) = parts(v);
    n.to_f64().unwrap_or(f64::NAN) / d.to_f64().unwrap_or(f64::NAN)
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

fn arith(op: Op, a: V, b: V) -> V {
    if is_float(b) {
        let (x, y) = (to_f(a), float_val(b));
        return float_new(match op {
            Op::Add => x + y,
            Op::Sub => x - y,
            Op::Mul => x * y,
            Op::Div => x / y,
        });
    }
    let Some((bn, bd)) = as_frac(b) else {
        if is_heap(b) && respond_to(b, sym("coerce")) {
            let pair = tri!(send1(b, "coerce", a));
            if is_ary(pair) && rary(pair).v.len() == 2 {
                let name = match op {
                    Op::Add => "+",
                    Op::Sub => "-",
                    Op::Mul => "*",
                    Op::Div => "/",
                };
                return send1(rary(pair).v[0], name, rary(pair).v[1]);
            }
        }
        return type_error(&format!("{} can't be coerced into Rational", class_name(real_class(b))));
    };
    let (an, ad) = parts(a);
    match op {
        Op::Add => rational_new(&an * &bd + &bn * &ad, &ad * &bd),
        Op::Sub => rational_new(&an * &bd - &bn * &ad, &ad * &bd),
        Op::Mul => rational_new(&an * &bn, &ad * &bd),
        Op::Div => {
            if bn.is_zero() {
                return raise(rt().c.zero_div_error, "divided by 0");
            }
            rational_new(&an * &bd, &ad * &bn)
        }
    }
}

fn cmp(a: V, b: V) -> Option<std::cmp::Ordering> {
    if is_float(b) {
        return to_f(a).partial_cmp(&float_val(b));
    }
    let (bn, bd) = as_frac(b)?;
    let (an, ad) = parts(a);
    Some((an * bd).cmp(&(bn * ad)))
}

fn round_with(v: V, digits: i64, mode: fn(&BigInt, &BigInt) -> BigInt) -> V {
    let (n, d) = parts(v);
    if digits <= 0 {
        let scale = BigInt::from(10).pow((-digits) as u32);
        let q = mode(&n, &(&d * &scale));
        return int_from_big(q * scale);
    }
    let scale = BigInt::from(10).pow(digits as u32);
    let q = mode(&(&n * &scale), &d);
    rational_new(q, scale)
}
fn floor_div(n: &BigInt, d: &BigInt) -> BigInt {
    n.div_floor(d)
}
fn ceil_div(n: &BigInt, d: &BigInt) -> BigInt {
    -((-n).div_floor(d))
}
fn trunc_div(n: &BigInt, d: &BigInt) -> BigInt {
    n / d
}
fn round_half_up(n: &BigInt, d: &BigInt) -> BigInt {
    let two = BigInt::from(2);
    if n.is_negative() {
        -((-n * &two + d).div_floor(&(d * &two)))
    } else {
        (n * &two + d).div_floor(&(d * &two))
    }
}

/// Kernel#Rational(a, b = 1)
fn make(a: V, b: V) -> V {
    let conv = |x: V| -> Result<(BigInt, BigInt), V> {
        if let Some(f) = as_frac(x) {
            return Ok(f);
        }
        if is_float(x) {
            let r = f64_to_rational(float_val(x));
            if r == UNDEF {
                return Err(UNDEF);
            }
            return Ok(parts(r));
        }
        if is_str(x) {
            let s = String::from_utf8_lossy(&rstr(x).b).trim().replace('_', "");
            let (num_s, den_s) = s.split_once('/').unwrap_or((&s, "1"));
            let parse_dec = |t: &str| -> Option<(BigInt, BigInt)> {
                let t = t.trim();
                let (neg, t) = match t.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, t.strip_prefix('+').unwrap_or(t)),
                };
                let (ip, fp) = t.split_once('.').unwrap_or((t, ""));
                let digits = format!("{ip}{fp}");
                let n = BigInt::parse_bytes(if digits.is_empty() { b"0" } else { digits.as_bytes() }, 10)?;
                let d = BigInt::from(10).pow(fp.len() as u32);
                Some((if neg { -n } else { n }, d))
            };
            match (parse_dec(num_s), parse_dec(den_s)) {
                (Some((an, ad)), Some((bn, bd))) => return Ok((an * bd, ad * bn)),
                _ => return Err(arg_error(&format!("invalid value for convert(): {:?}", s))),
            }
        }
        Err(type_error(&format!("can't convert {} into Rational", class_name(real_class(x)))))
    };
    let (an, ad) = match conv(a) {
        Ok(x) => x,
        Err(e) => return e,
    };
    let (bn, bd) = match conv(b) {
        Ok(x) => x,
        Err(e) => return e,
    };
    rational_new(an * bd, ad * bn)
}

pub fn init() {
    let numc = rt().c.numeric;
    let rc = new_class("Rational", numc);
    set_global_const("Rational", rc as V);
    *RATIONAL.get() = rc;
    defs(rc, "new", 0, -1, |_, _, _| raise(rt().c.no_method_error, "undefined method 'new' for class Rational"));
    defn(rt().c.kernel, "Rational", 1, 3, |_, a, _| {
        let b = a.get(1).copied().filter(|x| !is_hash(*x)).unwrap_or(mkfix(1));
        make(a[0], b)
    });
    defn(rc, "numerator", 0, 0, |s, _, _| ivar_get(s, sym("@__num")));
    defn(rc, "denominator", 0, 0, |s, _, _| ivar_get(s, sym("@__den")));
    defn(rc, "+", 1, 1, |s, a, _| arith(Op::Add, s, a[0]));
    defn(rc, "-", 1, 1, |s, a, _| arith(Op::Sub, s, a[0]));
    defn(rc, "*", 1, 1, |s, a, _| arith(Op::Mul, s, a[0]));
    defn(rc, "/", 1, 1, |s, a, _| arith(Op::Div, s, a[0]));
    defn(rc, "quo", 1, 1, |s, a, _| arith(Op::Div, s, a[0]));
    defn(rc, "fdiv", 1, 1, |s, a, _| float_new(to_f(s) / crate::fmt::num_to_f(a[0]).unwrap_or(f64::NAN)));
    defn(rc, "**", 1, 1, |s, a, _| {
        if is_fix(a[0]) {
            let e = fix(a[0]);
            let (n, d) = parts(s);
            let (n, d) = if e >= 0 { (n.pow(e as u32), d.pow(e as u32)) } else { (d.pow((-e) as u32), n.pow((-e) as u32)) };
            return rational_new(n, d);
        }
        float_new(to_f(s).powf(crate::fmt::num_to_f(a[0]).unwrap_or(f64::NAN)))
    });
    defn(rc, "-@", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        rational_new(-n, d)
    });
    defn(rc, "+@", 0, 0, |s, _, _| s);
    defn(rc, "abs", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        rational_new(n.abs(), d)
    });
    defn(rc, "<=>", 1, 1, |s, a, _| cmp(s, a[0]).map(|o| mkfix(o as i64)).unwrap_or(QNIL));
    defn(rc, "==", 1, 1, |s, a, _| bool_v(cmp(s, a[0]) == Some(std::cmp::Ordering::Equal)));
    defn(rc, "<", 1, 1, |s, a, _| bool_v(cmp(s, a[0]).is_some_and(|o| o.is_lt())));
    defn(rc, "<=", 1, 1, |s, a, _| bool_v(cmp(s, a[0]).is_some_and(|o| o.is_le())));
    defn(rc, ">", 1, 1, |s, a, _| bool_v(cmp(s, a[0]).is_some_and(|o| o.is_gt())));
    defn(rc, ">=", 1, 1, |s, a, _| bool_v(cmp(s, a[0]).is_some_and(|o| o.is_ge())));
    defn(rc, "eql?", 1, 1, |s, a, _| bool_v(is_rational(a[0]) && parts(s) == parts(a[0])));
    defn(rc, "hash", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        n.hash(&mut h);
        d.hash(&mut h);
        mkfix((h.finish() >> 2) as i64)
    });
    defn(rc, "coerce", 1, 1, |s, a, _| {
        if is_float(a[0]) {
            return ary_new(vec![a[0], float_new(to_f(s))]);
        }
        match as_frac(a[0]) {
            Some((n, d)) => ary_new(vec![rational_new(n, d), s]),
            None => type_error("can't coerce into Rational"),
        }
    });
    defn(rc, "zero?", 0, 0, |s, _, _| bool_v(parts(s).0.is_zero()));
    defn(rc, "positive?", 0, 0, |s, _, _| bool_v(parts(s).0.is_positive()));
    defn(rc, "negative?", 0, 0, |s, _, _| bool_v(parts(s).0.is_negative()));
    defn(rc, "integer?", 0, 0, |_, _, _| QFALSE);
    defn(rc, "finite?", 0, 0, |_, _, _| QTRUE);
    defn(rc, "infinite?", 0, 0, |_, _, _| QNIL);
    defn(rc, "to_f", 0, 0, |s, _, _| float_new(to_f(s)));
    defn(rc, "to_i", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        int_from_big(trunc_div(&n, &d))
    });
    defn(rc, "to_r", 0, 0, |s, _, _| s);
    // Numeric#to_int: to_i (Rational, BigDecimal, user Numeric subclasses)
    if let Some(num) = const_lookup(rt().c.object, sym("Numeric")) {
        if unsafe { !(*(num as *mut Class)).methods.contains_key(&sym("to_int")) } {
            defn(num as *mut Class, "to_int", 0, 0, |s, _, _| send0(s, "to_i"));
        }
    }
    defn(rc, "rationalize", 0, 1, |s, _, _| s);
    fn dg(a: &[V]) -> i64 {
        a.first().filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0)
    }
    defn(rc, "floor", 0, 1, |s, a, _| round_with(s, dg(a), floor_div));
    defn(rc, "ceil", 0, 1, |s, a, _| round_with(s, dg(a), ceil_div));
    defn(rc, "truncate", 0, 1, |s, a, _| round_with(s, dg(a), trunc_div));
    defn(rc, "round", 0, 2, |s, a, _| round_with(s, dg(a), round_half_up));
    defn(rc, "to_s", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        new_str_v(format!("{n}/{d}"))
    });
    defn(rc, "inspect", 0, 0, |s, _, _| {
        let (n, d) = parts(s);
        new_str_v(format!("({n}/{d})"))
    });

    // Integer / Float interplay
    let (int, flo) = (rt().c.integer, rt().c.float);
    // Integer ** negative Integer is a Rational in Ruby
    defn(int, "**", 1, 1, |s, a, _| {
        if is_fix(a[0]) && fix(a[0]) < 0 {
            let e = (-fix(a[0])) as u32;
            return rational_new(BigInt::one(), to_big(s).pow(e));
        }
        crate::numeric::arith(crate::numeric::Op::Pow, s, a[0])
    });
    defn(int, "to_r", 0, 0, |s, _, _| rational_new(to_big(s), BigInt::one()));
    defn(int, "rationalize", 0, 1, |s, _, _| rational_new(to_big(s), BigInt::one()));
    defn(int, "numerator", 0, 0, |s, _, _| s);
    defn(int, "denominator", 0, 0, |_, _, _| mkfix(1));
    defn(int, "quo", 1, 1, |s, a, _| {
        if is_float(a[0]) {
            return float_new(crate::fmt::num_to_f(s).unwrap_or(0.0) / float_val(a[0]));
        }
        make(s, a[0])
    });
    defn(flo, "to_r", 0, 0, |s, _, _| f64_to_rational(float_val(s)));
    defn(flo, "rationalize", 0, 1, |s, _, _| {
        // simplest rational within float precision (via the shortest decimal repr)
        let txt = crate::fmt::float_to_s(float_val(s));
        if txt.contains('e') {
            return f64_to_rational(float_val(s));
        }
        make(str_from(&txt), mkfix(1))
    });
    // String#to_r reads the longest rational prefix ("0.4.25" -> 2/5, "1/3x" -> 1/3)
    fn rational_prefix(b: &[u8]) -> String {
        let mut i = 0;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        let digits = |i: &mut usize| {
            let s = *i;
            while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'_' && *i > s && *i + 1 < b.len() && b[*i + 1].is_ascii_digit())) {
                *i += 1;
            }
            *i > s
        };
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if !digits(&mut i) {
            return String::new();
        }
        if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
            i += 1;
            digits(&mut i);
        }
        if i + 1 < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if digits(&mut j) {
                i = j;
            }
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1].is_ascii_digit() {
            i += 1;
            digits(&mut i);
        }
        String::from_utf8_lossy(&b[start..i]).into_owned()
    }
    defn(rt().c.string, "to_r", 0, 0, |s, _, _| {
        let p = rational_prefix(&rstr(s).b);
        if p.is_empty() {
            return rational_new(BigInt::zero(), BigInt::one());
        }
        let r = make(str_from(&p), mkfix(1));
        if r == UNDEF {
            pst().pending = Throw::None;
            return rational_new(BigInt::zero(), BigInt::one());
        }
        r
    });
    defn(rt().c.nil, "to_r", 0, 0, |_, _, _| rational_new(BigInt::zero(), BigInt::one()));
}

/// `3r`, `1.5r` literals: Rational(numerator_text, denominator)
#[unsafe(no_mangle)]
pub extern "C" fn kn_rational_lit(numer: V, denom: V) -> V {
    make(numer, denom)
}

// ---------- Complex (parts are any Ruby numbers; arithmetic is delegated) ----------

static COMPLEX: G<*mut Class> = G::new(std::ptr::null_mut());

fn is_complex(v: V) -> bool {
    is_kind(v, Kind::Object) && real_class(v) == *COMPLEX.get()
}
pub fn complex_new(re: V, im: V) -> V {
    let c = object_new(*COMPLEX.get());
    ivar_set(c, sym("@__re"), re);
    ivar_set(c, sym("@__im"), im);
    hdr(c).flags |= FL_FROZEN;
    c
}
fn re_im(v: V) -> (V, V) {
    if is_complex(v) { (ivar_get(v, sym("@__re")), ivar_get(v, sym("@__im"))) } else { (v, mkfix(0)) }
}
fn nsend(a: V, op: &str, b: V) -> V {
    send1(a, op, b)
}

pub fn init_complex() {
    let cc = new_class("Complex", rt().c.numeric);
    set_global_const("Complex", cc as V);
    *COMPLEX.get() = cc;
    defs(cc, "rectangular", 1, 2, |_, a, _| complex_new(a[0], a.get(1).copied().unwrap_or(mkfix(0))));
    defs(cc, "rect", 1, 2, |_, a, _| complex_new(a[0], a.get(1).copied().unwrap_or(mkfix(0))));
    defs(cc, "polar", 1, 2, |_, a, _| {
        let r = crate::fmt::num_to_f(a[0]).unwrap_or(0.0);
        let t = a.get(1).and_then(|x| crate::fmt::num_to_f(*x)).unwrap_or(0.0);
        complex_new(float_new(r * t.cos()), float_new(r * t.sin()))
    });
    const_set(cc, sym("I"), complex_new(mkfix(0), mkfix(1)));
    defn(rt().c.kernel, "Complex", 1, 3, |_, a, _| {
        if is_complex(a[0]) && a.len() == 1 {
            return a[0];
        }
        complex_new(a[0], a.get(1).copied().filter(|x| !is_hash(*x)).unwrap_or(mkfix(0)))
    });
    defn(cc, "real", 0, 0, |s, _, _| re_im(s).0);
    defn(cc, "imaginary", 0, 0, |s, _, _| re_im(s).1);
    defn(cc, "imag", 0, 0, |s, _, _| re_im(s).1);
    defn(cc, "rectangular", 0, 0, |s, _, _| ary_new(vec![re_im(s).0, re_im(s).1]));
    defn(cc, "real?", 0, 0, |_, _, _| QFALSE);
    defn(cc, "+", 1, 1, |s, a, _| {
        let ((ar, ai), (br, bi)) = (re_im(s), re_im(a[0]));
        complex_new(tri!(nsend(ar, "+", br)), tri!(nsend(ai, "+", bi)))
    });
    defn(cc, "-", 1, 1, |s, a, _| {
        let ((ar, ai), (br, bi)) = (re_im(s), re_im(a[0]));
        complex_new(tri!(nsend(ar, "-", br)), tri!(nsend(ai, "-", bi)))
    });
    defn(cc, "*", 1, 1, |s, a, _| {
        let ((ar, ai), (br, bi)) = (re_im(s), re_im(a[0]));
        let re = tri!(nsend(tri!(nsend(ar, "*", br)), "-", tri!(nsend(ai, "*", bi))));
        let im = tri!(nsend(tri!(nsend(ar, "*", bi)), "+", tri!(nsend(ai, "*", br))));
        complex_new(re, im)
    });
    defn(cc, "/", 1, 1, |s, a, _| {
        let ((ar, ai), (br, bi)) = (re_im(s), re_im(a[0]));
        let den = tri!(nsend(tri!(nsend(br, "*", br)), "+", tri!(nsend(bi, "*", bi))));
        let re = tri!(nsend(tri!(nsend(ar, "*", br)), "+", tri!(nsend(ai, "*", bi))));
        let im = tri!(nsend(tri!(nsend(ai, "*", br)), "-", tri!(nsend(ar, "*", bi))));
        complex_new(tri!(send1(re, "quo", den)), tri!(send1(im, "quo", den)))
    });
    defn(cc, "-@", 0, 0, |s, _, _| {
        let (r, i) = re_im(s);
        complex_new(tri!(send0(r, "-@")), tri!(send0(i, "-@")))
    });
    defn(cc, "conjugate", 0, 0, |s, _, _| {
        let (r, i) = re_im(s);
        complex_new(r, tri!(send0(i, "-@")))
    });
    defn(cc, "abs", 0, 0, |s, _, _| {
        let (r, i) = re_im(s);
        float_new(crate::fmt::num_to_f(r).unwrap_or(0.0).hypot(crate::fmt::num_to_f(i).unwrap_or(0.0)))
    });
    defn(cc, "==", 1, 1, |s, a, _| {
        let ((ar, ai), (br, bi)) = (re_im(s), re_im(a[0]));
        let r = tri!(equal(ar, br));
        let i = tri!(equal(ai, bi));
        bool_v(truthy(r) && truthy(i))
    });
    defn(cc, "hash", 0, 0, |s, _, _| {
        let (r, i) = re_im(s);
        mkfix(((hash_of(r) ^ hash_of(i).rotate_left(9)) >> 2) as i64)
    });
    defn(cc, "coerce", 1, 1, |s, a, _| ary_new(vec![complex_new(a[0], mkfix(0)), s]));
    fn fmt(s: V) -> Result<String, ()> {
        let (r, i) = re_im(s);
        let rs = String::from_utf8_lossy(&crate::fmt::to_s(r)?).into_owned();
        let is = String::from_utf8_lossy(&crate::fmt::to_s(i)?).into_owned();
        let (sign, is) = match is.strip_prefix('-') {
            Some(x) => ('-', x.to_string()),
            None => ('+', is),
        };
        Ok(format!("{rs}{sign}{is}i"))
    }
    defn(cc, "to_s", 0, 0, |s, _, _| match fmt(s) {
        Ok(x) => new_str_v(x),
        Err(()) => UNDEF,
    });
    defn(cc, "inspect", 0, 0, |s, _, _| match fmt(s) {
        Ok(x) => new_str_v(format!("({x})")),
        Err(()) => UNDEF,
    });
    defn(cc, "to_c", 0, 0, |s, _, _| s);
    for c in [rt().c.integer, rt().c.float] {
        defn(c, "to_c", 0, 0, |s, _, _| complex_new(s, mkfix(0)));
        defn(c, "real", 0, 0, |s, _, _| s);
        defn(c, "imaginary", 0, 0, |_, _, _| mkfix(0));
        defn(c, "imag", 0, 0, |_, _, _| mkfix(0));
        defn(c, "i", 0, 0, |s, _, _| complex_new(mkfix(0), s));
    }
}
