//! BigDecimal: arbitrary-precision decimal numbers (the bigdecimal gem's API, 3.1.x behaviour).
//!
//! Values are `m × 10^e` with an unsigned BigInt mantissa `m` (no trailing zeros), plus
//! sign and NaN/±Infinity flags, stored in hidden ivars of a frozen object. `+ - *` are exact;
//! `/` ports the gem's word-based (base 10^9) long division and rounding so that the number
//! of digits produced matches CRuby (e.g. `BigDecimal(1)/3` has 36 digits).

use crate::core::*;
use crate::tri;
use num_bigint::BigInt;
use num_integer::Integer as _;
use num_traits::{One, Signed, Zero};
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kd {
    Fin,
    NaN,
    Inf,
}

#[derive(Clone, Debug)]
pub struct D {
    pub k: Kd,
    pub neg: bool,
    /// non-negative mantissa, no trailing decimal zeros (0 for zero)
    pub m: BigInt,
    pub e: i64,
}

const BASE: i64 = 1_000_000_000;
const BASE1: i64 = 100_000_000;

// rounding modes (BigDecimal::ROUND_*)
const ROUND_UP: i64 = 1;
const ROUND_DOWN: i64 = 2;
const ROUND_HALF_UP: i64 = 3;
const ROUND_HALF_DOWN: i64 = 4;
const ROUND_CEILING: i64 = 5;
const ROUND_FLOOR: i64 = 6;
const ROUND_HALF_EVEN: i64 = 7;

static MODE: G<i64> = G::new(ROUND_HALF_UP);
static CLASS: G<usize> = G::new(0);

fn bd_class() -> *mut Class {
    *CLASS.get() as *mut Class
}

impl D {
    pub fn fin(neg: bool, m: BigInt, e: i64) -> D {
        let mut d = D { k: Kd::Fin, neg, m, e };
        d.norm();
        d
    }
    pub fn zero(neg: bool) -> D {
        D { k: Kd::Fin, neg, m: BigInt::zero(), e: 0 }
    }
    pub fn nan() -> D {
        D { k: Kd::NaN, neg: false, m: BigInt::zero(), e: 0 }
    }
    pub fn inf(neg: bool) -> D {
        D { k: Kd::Inf, neg, m: BigInt::zero(), e: 0 }
    }
    fn from_int(i: &BigInt) -> D {
        D::fin(i.is_negative(), i.abs(), 0)
    }
    fn norm(&mut self) {
        if self.m.is_zero() {
            self.e = 0;
            return;
        }
        let ten = BigInt::from(10);
        // strip trailing zeros quickly in chunks
        let big = BigInt::from(1_000_000_000u64);
        loop {
            let (q, r) = self.m.div_rem(&big);
            if r.is_zero() {
                self.m = q;
                self.e += 9;
            } else {
                break;
            }
        }
        loop {
            let (q, r) = self.m.div_rem(&ten);
            if r.is_zero() {
                self.m = q;
                self.e += 1;
            } else {
                break;
            }
        }
    }
    fn is_zero(&self) -> bool {
        self.k == Kd::Fin && self.m.is_zero()
    }
    fn signed(&self) -> BigInt {
        if self.neg { -self.m.clone() } else { self.m.clone() }
    }
    fn digits(&self) -> String {
        self.m.to_string()
    }
    /// number of digits before the decimal point in 0.xxxEn form (BigDecimal#exponent)
    fn exponent(&self) -> i64 {
        if self.k != Kd::Fin || self.m.is_zero() { 0 } else { self.digits().len() as i64 + self.e }
    }
    fn neg_(&self) -> D {
        let mut d = self.clone();
        if d.k != Kd::NaN {
            d.neg = !d.neg;
        }
        d
    }
    fn precision_scale(&self) -> (i64, i64) {
        if self.k != Kd::Fin || self.m.is_zero() {
            return (0, 0);
        }
        let n = self.digits().len() as i64;
        let scale = if self.e < 0 { -self.e } else { 0 };
        let int_digits = n + self.e;
        (if int_digits > 0 { int_digits + scale } else { scale }, scale)
    }
    fn to_f64(&self) -> f64 {
        match self.k {
            Kd::NaN => f64::NAN,
            Kd::Inf => {
                if self.neg {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
            Kd::Fin => {
                let s = format!("{}{}e{}", if self.neg { "-" } else { "" }, self.digits(), self.e);
                s.parse().unwrap_or(0.0)
            }
        }
    }
}

fn pow10(n: i64) -> BigInt {
    num_traits::pow(BigInt::from(10), n.max(0) as usize)
}

// ---------------------------------------------------------------------------
// conversion between Ruby values and D
// ---------------------------------------------------------------------------

pub fn is_bd(v: V) -> bool {
    !bd_class().is_null() && is_heap(v) && kind(v) == Some(Kind::Object) && is_a(v, bd_class())
}

pub fn get(v: V) -> D {
    let k = ivar_get(v, sym("@__k"));
    let neg = truthy(ivar_get(v, sym("@__s")));
    match if is_fix(k) { fix(k) } else { 0 } {
        1 => D::nan(),
        2 => D::inf(neg),
        _ => {
            let m = ivar_get(v, sym("@__m"));
            let m = if is_fix(m) { BigInt::from(fix(m)) } else if is_big(m) { rbig(m).n.clone() } else { BigInt::zero() };
            let e = ivar_get(v, sym("@__e"));
            D { k: Kd::Fin, neg, m, e: if is_fix(e) { fix(e) } else { 0 } }
        }
    }
}

fn new_bd_p(d: D, maxprec: Option<i64>) -> V {
    let o = object_new(bd_class());
    ivar_set(o, sym("@__k"), mkfix(match d.k {
        Kd::Fin => 0,
        Kd::NaN => 1,
        Kd::Inf => 2,
    }));
    ivar_set(o, sym("@__s"), bool_v(d.neg));
    ivar_set(o, sym("@__m"), int_from_big(d.m));
    ivar_set(o, sym("@__e"), mkfix(d.e));
    if let Some(p) = maxprec {
        ivar_set(o, sym("@__p"), mkfix(p));
    }
    hdr(o).flags |= FL_FROZEN;
    o
}
/// Float value of a BigDecimal (None for other values).
pub fn to_f(v: V) -> Option<f64> {
    if is_bd(v) { Some(get(v).to_f64()) } else { None }
}
pub fn new_bd(d: D) -> V {
    new_bd_p(d, None)
}

/// Shortest round-trip (prec 0) or `prec`-significant-digit decimal form of a float.
fn from_f64(f: f64, prec: usize) -> D {
    if f.is_nan() {
        return D::nan();
    }
    if f.is_infinite() {
        return D::inf(f < 0.0);
    }
    if f == 0.0 {
        return D::zero(f.is_sign_negative());
    }
    let s = if prec == 0 { format!("{:e}", f.abs()) } else { format!("{:.*e}", prec - 1, f.abs()) };
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i64 = exp.parse().unwrap();
    let mut digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    // the gem keeps at most 16 digits of the shortest representation (truncating)
    digits.truncate(16);
    let n = digits.len() as i64;
    D::fin(f < 0.0, digits.parse().unwrap(), exp + 1 - n)
}

/// Strict (BigDecimal("...")) or loose (String#to_d) parse. Returns D and the digit count.
fn parse_str(s: &str, loose: bool) -> Option<(D, usize)> {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    match t {
        "Infinity" | "+Infinity" => return Some((D::inf(false), 0)),
        "-Infinity" => return Some((D::inf(true), 0)),
        "NaN" => return Some((D::nan(), 0)),
        _ => {}
    }
    let b = t.as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut digits = String::new();
    let mut ndig = 0usize;
    let read_digits = |i: &mut usize, out: &mut String| -> Result<usize, ()> {
        let mut n = 0;
        while *i < b.len() {
            if b[*i].is_ascii_digit() {
                out.push(b[*i] as char);
                n += 1;
                *i += 1;
            } else if b[*i] == b'_' && n > 0 && *i + 1 < b.len() && b[*i + 1].is_ascii_digit() {
                *i += 1;
            } else if b[*i] == b'_' {
                return Err(());
            } else {
                break;
            }
        }
        Ok(n)
    };
    let int_n = match read_digits(&mut i, &mut digits) {
        Ok(n) => n,
        Err(()) if loose => digits.len(),
        Err(()) => return None,
    };
    ndig += int_n;
    let mut frac_n = 0;
    if i < b.len() && b[i] == b'.' {
        let save = (i, digits.len());
        i += 1;
        match read_digits(&mut i, &mut digits) {
            Ok(n) if n > 0 => frac_n = n,
            _ => {
                if !loose {
                    return None;
                }
                i = save.0;
                digits.truncate(save.1);
            }
        }
    }
    ndig += frac_n;
    if int_n + frac_n == 0 {
        return if loose { Some((D::zero(false), 1)) } else { None };
    }
    let mut exp: i64 = 0;
    if i < b.len() && matches!(b[i], b'e' | b'E' | b'd' | b'D') {
        let save = i;
        i += 1;
        let mut eneg = false;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            eneg = b[i] == b'-';
            i += 1;
        }
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if st == i {
            if !loose {
                return None;
            }
            i = save;
        } else {
            exp = t[st..i].parse::<i64>().unwrap_or(i64::MAX / 4);
            if eneg {
                exp = -exp;
            }
        }
    }
    if i != b.len() && !loose {
        return None;
    }
    let m: BigInt = digits.parse().unwrap_or_default();
    Some((D::fin(neg, m, exp - frac_n as i64), ndig))
}

/// Round to `n` significant digits (VpLeftRound).
fn round_sig(d: &D, n: i64, mode: i64) -> D {
    if d.k != Kd::Fin || d.m.is_zero() || n <= 0 {
        return d.clone();
    }
    let ex = d.exponent();
    round_at(d, n - ex, mode)
}

/// Round to `n` digits after the decimal point (negative: to the left of it).
fn round_at(d: &D, n: i64, mode: i64) -> D {
    if d.k != Kd::Fin || d.m.is_zero() {
        return d.clone();
    }
    // keep digits with exponent >= -n
    if d.e >= -n {
        return d.clone();
    }
    let drop = -n - d.e; // number of digits to drop (> 0)
    let p = pow10(drop);
    let (q, r) = d.m.div_rem(&p);
    let up = if r.is_zero() {
        false
    } else {
        let twice: BigInt = &r * 2;
        let cmp_half = twice.cmp(&p);
        match mode {
            ROUND_UP => true,
            ROUND_DOWN => false,
            ROUND_HALF_UP => cmp_half != Ordering::Less,
            ROUND_HALF_DOWN => cmp_half == Ordering::Greater,
            ROUND_HALF_EVEN => cmp_half == Ordering::Greater || (cmp_half == Ordering::Equal && q.is_odd()),
            ROUND_CEILING => !d.neg,
            ROUND_FLOOR => d.neg,
            _ => cmp_half != Ordering::Less,
        }
    };
    let q = if up { q + 1 } else { q };
    let r = D::fin(d.neg, q, -n);
    if r.m.is_zero() { D::zero(d.neg) } else { r }
}

fn mode() -> i64 {
    *MODE.get()
}

fn sym_mode(v: V) -> Result<i64, V> {
    if v == QNIL {
        return Ok(mode());
    }
    if is_sym(v) || is_str(v) {
        let s = if is_sym(v) { sym_str(sym_id(v)) } else { String::from_utf8_lossy(&rstr(v).b).into_owned() };
        return match s.as_str() {
            "up" => Ok(ROUND_UP),
            "down" | "truncate" => Ok(ROUND_DOWN),
            "half_up" | "default" => Ok(ROUND_HALF_UP),
            "half_down" => Ok(ROUND_HALF_DOWN),
            "half_even" | "banker" => Ok(ROUND_HALF_EVEN),
            "ceiling" | "ceil" => Ok(ROUND_CEILING),
            "floor" => Ok(ROUND_FLOOR),
            _ => Err(arg_error("invalid rounding mode")),
        };
    }
    if is_hash(v) {
        let h = crate::hash::hget(v, mksym(sym("half")));
        if h == QNIL {
            return Ok(mode());
        }
        let s = if is_sym(h) { sym_str(sym_id(h)) } else if is_str(h) { String::from_utf8_lossy(&rstr(h).b).into_owned() } else { String::new() };
        return match s.as_str() {
            "up" => Ok(ROUND_HALF_UP),
            "down" => Ok(ROUND_HALF_DOWN),
            "even" => Ok(ROUND_HALF_EVEN),
            _ => Err(arg_error(&format!("invalid rounding mode: {}", crate::fmt::inspect_lossy(h)))),
        };
    }
    if is_fix(v) && (1..=7).contains(&fix(v)) {
        return Ok(fix(v));
    }
    Err(arg_error("invalid rounding mode"))
}

// ---------------------------------------------------------------------------
// arithmetic
// ---------------------------------------------------------------------------

fn add(a: &D, b: &D) -> D {
    match (a.k, b.k) {
        (Kd::NaN, _) | (_, Kd::NaN) => D::nan(),
        (Kd::Inf, Kd::Inf) => {
            if a.neg == b.neg {
                a.clone()
            } else {
                D::nan()
            }
        }
        (Kd::Inf, _) => a.clone(),
        (_, Kd::Inf) => b.clone(),
        _ => {
            if a.m.is_zero() && b.m.is_zero() {
                return D::zero(a.neg && b.neg);
            }
            let e = a.e.min(b.e);
            let x = a.signed() * pow10(a.e - e) + b.signed() * pow10(b.e - e);
            D::fin(x.is_negative(), x.abs(), e)
        }
    }
}
fn mul(a: &D, b: &D) -> D {
    let neg = a.neg != b.neg;
    match (a.k, b.k) {
        (Kd::NaN, _) | (_, Kd::NaN) => D::nan(),
        (Kd::Inf, _) | (_, Kd::Inf) => {
            if a.is_zero() || b.is_zero() {
                D::nan()
            } else {
                D::inf(neg)
            }
        }
        _ => {
            if a.m.is_zero() || b.m.is_zero() {
                return D::zero(neg);
            }
            D::fin(neg, &a.m * &b.m, a.e + b.e)
        }
    }
}

// --- word representation for the division port ---

struct W {
    neg: bool,
    exp: i64,
    frac: Vec<i64>,
}
fn to_words(d: &D) -> W {
    let ds = d.digits();
    let p = ds.len() as i64 + d.e;
    let lead = (9 - p.rem_euclid(9)) % 9;
    let mut s = "0".repeat(lead as usize);
    s.push_str(&ds);
    while s.len() % 9 != 0 {
        s.push('0');
    }
    let mut frac: Vec<i64> = s.as_bytes().chunks(9).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect();
    while frac.len() > 1 && *frac.last().unwrap() == 0 {
        frac.pop();
    }
    W { neg: d.neg, exp: (p + lead) / 9, frac }
}
fn from_words(w: &W) -> D {
    let mut m = BigInt::zero();
    let b = BigInt::from(BASE);
    for &x in &w.frac {
        m = m * &b + BigInt::from(x);
    }
    D::fin(w.neg, m, 9 * (w.exp - w.frac.len() as i64))
}
/// VpNmlz: strip leading/trailing zero words. Returns false if zero.
fn nmlz(w: &mut W) -> bool {
    while w.frac.len() > 1 && *w.frac.last().unwrap() == 0 {
        w.frac.pop();
    }
    let lead = w.frac.iter().take_while(|x| **x == 0).count();
    if lead == w.frac.len() {
        w.frac = vec![0];
        return false;
    }
    if lead > 0 {
        w.frac.drain(..lead);
        w.exp -= lead as i64;
    }
    true
}
/// VpRdup: add one unit at the last word.
fn rdup(w: &mut W) {
    let mut i = w.frac.len();
    let mut carry = 1;
    while carry > 0 && i > 0 {
        i -= 1;
        w.frac[i] += carry;
        if w.frac[i] >= BASE {
            w.frac[i] -= BASE;
        } else {
            carry = 0;
        }
    }
    if carry > 0 {
        w.exp += 1;
        w.frac = vec![1];
    } else {
        nmlz(w);
    }
}

/// Port of VpDivd: c = a / b truncated to `word_c` words, r = remainder.
fn vp_divd(a: &W, b: &W, word_c: usize, word_r: usize) -> (W, W) {
    let word_a = a.frac.len();
    let word_b = b.frac.len();
    let word_r = word_r.max(word_a + 1).max(word_b + 2);
    let mut r = vec![0i64; word_r + 1];
    for i in 1..=word_a {
        r[i] = a.frac[i - 1];
    }
    let mut c = vec![0i64; word_c.max(1) + 1];
    let bf = &b.frac;
    let b1 = bf[0];
    let mut b1p1 = b1;
    let (b1b2, mut b1b2p1);
    if word_b <= 1 {
        b1b2 = b1p1 * BASE;
        b1b2p1 = b1b2;
    } else {
        b1p1 = b1 + 1;
        b1b2 = b1 * BASE + bf[1];
        b1b2p1 = b1b2;
        if word_b > 2 {
            b1b2p1 += 1;
        }
    }
    let n_loop = word_c.min(word_r - 1);
    let mut ind_c = 1usize;
    'outer: while ind_c < n_loop {
        if r[ind_c] == 0 {
            ind_c += 1;
            continue;
        }
        let r1r2 = r[ind_c] * BASE + r[ind_c + 1];
        let q: i64;
        let mut ind_r: usize;
        let mut do_div_b1p1 = false;
        if r1r2 == b1b2 {
            let mut ind_b = 2;
            let mut ind_a = ind_c + 2;
            while ind_b < word_b {
                if r[ind_a] < bf[ind_b] {
                    do_div_b1p1 = true;
                    break;
                }
                if r[ind_a] > bf[ind_b] {
                    break;
                }
                ind_a += 1;
                ind_b += 1;
            }
            if !do_div_b1p1 {
                // quotient digit 1: r -= b
                let mut borrow = 0;
                let mut ib = word_b as i64 - 1;
                let mut ir = (ind_c as i64) + ib;
                while ib >= 0 {
                    let (iru, ibu) = (ir as usize, ib as usize);
                    if r[iru] < bf[ibu] + borrow {
                        r[iru] += BASE - (bf[ibu] + borrow);
                        borrow = 1;
                    } else {
                        r[iru] = r[iru] - bf[ibu] - borrow;
                        borrow = 0;
                    }
                    ir -= 1;
                    ib -= 1;
                }
                c[ind_c] += 1;
                // carry
                let mut k = ind_c;
                while c[k] >= BASE && k > 0 {
                    c[k] -= BASE;
                    k -= 1;
                    c[k] += 1;
                }
                continue 'outer;
            }
        }
        if !do_div_b1p1 && r1r2 >= b1b2p1 {
            q = r1r2 / b1b2p1;
            c[ind_c] += q;
            ind_r = word_b + ind_c - 1;
        } else {
            if ind_c + 1 >= word_c {
                break 'outer;
            }
            q = r1r2 / b1p1;
            c[ind_c + 1] += q;
            ind_r = word_b + ind_c;
        }
        // sub_mult: r -= q * b
        let mut borrow2: i64 = 0;
        let mut ib = word_b as i64 - 1;
        while ib >= 0 {
            let ibu = ib as usize;
            let mut qb = q * bf[ibu];
            let borrow1;
            if qb < BASE {
                borrow1 = 0;
            } else {
                borrow1 = qb / BASE;
                qb -= borrow1 * BASE;
            }
            if r[ind_r] < qb {
                r[ind_r] += BASE - qb;
                borrow2 = borrow2 + borrow1 + 1;
            } else {
                r[ind_r] -= qb;
                borrow2 += borrow1;
            }
            if borrow2 != 0 {
                if r[ind_r - 1] < borrow2 {
                    r[ind_r - 1] += BASE - borrow2;
                    borrow2 = 1;
                } else {
                    r[ind_r - 1] -= borrow2;
                    borrow2 = 0;
                }
            }
            ind_r -= 1;
            ib -= 1;
        }
        r[ind_r] -= borrow2;
        // carry
        let mut k = ind_c;
        while c[k] >= BASE && k > 0 {
            c[k] -= BASE;
            k -= 1;
            c[k] += 1;
        }
    }
    c.truncate(word_c.max(1));
    // c->frac[0] has weight BASE^(a.exp - b.exp + 1)
    let mut cw = W { neg: a.neg != b.neg, exp: a.exp - b.exp + 2, frac: c };
    nmlz(&mut cw);
    r.truncate(word_r);
    let mut rw = W { neg: a.neg, exp: a.exp + 1, frac: r };
    nmlz(&mut rw);
    (cw, rw)
}

/// `a / b` exactly as BigDecimal#/ does it.
fn div(a: &D, b: &D) -> D {
    let neg = a.neg != b.neg;
    match (a.k, b.k) {
        (Kd::NaN, _) | (_, Kd::NaN) => return D::nan(),
        (Kd::Inf, Kd::Inf) => return D::nan(),
        (Kd::Inf, _) => return D::inf(neg),
        (_, Kd::Inf) => return D::zero(neg),
        _ => {}
    }
    if b.m.is_zero() {
        return if a.m.is_zero() { D::nan() } else { D::inf(neg) };
    }
    if a.m.is_zero() {
        return D::zero(neg);
    }
    let (ap, _) = a.precision_scale();
    let (bp, _) = b.precision_scale();
    let mut mx = ap.max(bp) * 2;
    if mx < 32 {
        mx = 32;
    }
    let word_c = ((mx + 18 + 8) / 9) as usize;
    let word_r = (((mx + 1) * 2 + 18 + 8) / 9) as usize;
    let aw = to_words(a);
    let bw = to_words(b);
    if bw.frac == [1] && bw.exp == 1 {
        // divide by one
        let mut c = aw;
        c.neg = neg;
        if c.frac.len() > word_c {
            let next = c.frac[word_c];
            let prev = c.frac[word_c - 1];
            c.frac.truncate(word_c);
            internal_round(&mut c, prev, next);
        }
        return from_words(&c);
    }
    let (mut c, r) = vp_divd(&aw, &bw, word_c, word_r);
    let rf0 = if r.frac.is_empty() { 0 } else { r.frac[0] };
    let v = ((BASE as u64).wrapping_mul(rf0 as u64) / (bw.frac[0] as u64)) as u32 as i64;
    let prev = *c.frac.last().unwrap_or(&0);
    internal_round(&mut c, prev, v);
    let mut d = from_words(&c);
    d.neg = neg;
    d
}
/// The program bundles bigdecimal 4.x (set by lib/kenny/bigdecimal.rb): `/` rounds to
/// max(precisions) + 16 digits (at least 32), divmod's quotient is an Integer, sqrt(n)
/// rounds to n digits.
static V4: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn v4() -> bool {
    V4.load(std::sync::atomic::Ordering::Relaxed)
}
/// 4.x's `a / b`.
fn div_v4(a: &D, b: &D) -> D {
    if a.k != Kd::Fin || b.k != Kd::Fin || a.m.is_zero() || b.m.is_zero() {
        return div(a, b);
    }
    let ix = (a.precision_scale().0.max(b.precision_scale().0) + 16).max(32);
    div_digits(a, b, ix)
}

fn div_maxprec(a: &D, b: &D) -> i64 {
    let (ap, _) = a.precision_scale();
    let (bp, _) = b.precision_scale();
    let mut mx = ap.max(bp) * 2;
    if mx < 32 {
        mx = 32;
    }
    (mx + 18 + 8) / 9
}

/// VpInternalRound with the "next word" estimate `v`.
fn internal_round(c: &mut W, vprev: i64, v: i64) {
    if v == 0 {
        return;
    }
    let v = v / BASE1;
    let f = match mode() {
        ROUND_DOWN => false,
        ROUND_UP => v != 0,
        ROUND_HALF_UP => v >= 5,
        ROUND_HALF_DOWN => v >= 6,
        ROUND_CEILING => v != 0 && !c.neg,
        ROUND_FLOOR => v != 0 && c.neg,
        ROUND_HALF_EVEN => v > 5 || (v == 5 && vprev % 2 == 1),
        _ => v >= 5,
    };
    if f {
        rdup(c);
        nmlz(c);
    }
}

/// Exact `a / b` to `n` significant digits (BigDecimal#div(b, n)).
fn div_digits(a: &D, b: &D, n: i64) -> D {
    let neg = a.neg != b.neg;
    match (a.k, b.k) {
        (Kd::Fin, Kd::Fin) => {}
        _ => return div(a, b),
    }
    if b.m.is_zero() || a.m.is_zero() {
        return div(a, b);
    }
    // a/b = (ma/mb) * 10^(ea-eb); compute with n + 2 extra significant digits, truncated
    let extra = n + 20;
    let da = a.m.to_string().len() as i64;
    let db = b.m.to_string().len() as i64;
    let shift = (extra - (da - db)).max(0);
    let num = &a.m * pow10(shift);
    let (q, r) = num.div_rem(&b.m);
    // sticky bit for exactness of rounding
    let q = if r.is_zero() { q * 10 } else { q * 10 + 1 };
    let d = D::fin(neg, q, a.e - b.e - shift - 1);
    round_sig(&d, n, mode())
}

/// floor(a / b) as an integer and the modulo (BigDecimal#divmod semantics).
fn divmod(a: &D, b: &D) -> Result<(D, D), V> {
    if a.k == Kd::NaN || b.k == Kd::NaN || a.k == Kd::Inf {
        return Ok((D::nan(), D::nan()));
    }
    if b.is_zero() {
        return Err(raise(rt().c.zero_div_error, "divided by 0"));
    }
    if b.k == Kd::Inf {
        if a.is_zero() || a.neg == b.neg {
            return Ok((D::zero(false), a.clone()));
        }
        return Ok((D::fin(true, BigInt::one(), 0), b.clone()));
    }
    if a.is_zero() {
        return Ok((D::zero(false), D::zero(false)));
    }
    let e = a.e.min(b.e);
    let x = a.signed() * pow10(a.e - e);
    let y = b.signed() * pow10(b.e - e);
    let q = x.div_floor(&y);
    let m = &x - &q * &y;
    Ok((D::from_int(&q), D::fin(m.is_negative(), m.abs(), e)))
}

fn cmp(a: &D, b: &D) -> Option<Ordering> {
    match (a.k, b.k) {
        (Kd::NaN, _) | (_, Kd::NaN) => None,
        (Kd::Inf, Kd::Inf) => Some((!a.neg).cmp(&!b.neg)),
        (Kd::Inf, _) => Some(if a.neg { Ordering::Less } else { Ordering::Greater }),
        (_, Kd::Inf) => Some(if b.neg { Ordering::Greater } else { Ordering::Less }),
        _ => {
            let e = a.e.min(b.e);
            let x = a.signed() * pow10(a.e - e);
            let y = b.signed() * pow10(b.e - e);
            Some(x.cmp(&y))
        }
    }
}

// ---------------------------------------------------------------------------
// formatting
// ---------------------------------------------------------------------------

fn group(s: &str, n: usize, from_right: bool) -> String {
    if n == 0 || s.len() <= n {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    if from_right {
        let first = chars.len() % n;
        for (i, c) in chars.iter().enumerate() {
            if i > 0 && (i + n - first) % n == 0 {
                out.push(' ');
            }
            out.push(*c);
        }
    } else {
        for (i, c) in chars.iter().enumerate() {
            if i > 0 && i % n == 0 {
                out.push(' ');
            }
            out.push(*c);
        }
    }
    out
}

pub fn to_s(d: &D, fmt: &str) -> String {
    let mut plus = "";
    let mut fmt_f = false;
    let mut n = 0usize;
    for c in fmt.chars() {
        match c {
            '+' => plus = "+",
            ' ' => plus = " ",
            'F' | 'f' => fmt_f = true,
            'E' | 'e' => fmt_f = false,
            c if c.is_ascii_digit() => n = n * 10 + c.to_digit(10).unwrap() as usize,
            _ => {}
        }
    }
    match d.k {
        Kd::NaN => return "NaN".into(),
        Kd::Inf => return if d.neg { "-Infinity".into() } else { format!("{plus}Infinity") },
        Kd::Fin => {}
    }
    let sign = if d.neg { "-" } else { plus };
    if d.m.is_zero() {
        return if fmt_f { format!("{sign}0.0") } else { format!("{sign}0.0") };
    }
    let ds = d.digits();
    if fmt_f {
        let p = ds.len() as i64 + d.e;
        let (ip, fp) = if p <= 0 {
            ("0".to_string(), format!("{}{}", "0".repeat((-p) as usize), ds))
        } else if d.e >= 0 {
            (format!("{}{}", ds, "0".repeat(d.e as usize)), "0".to_string())
        } else {
            (ds[..p as usize].to_string(), ds[p as usize..].to_string())
        };
        let ip = if ip == "0" { ip } else { group(&ip, n, true) };
        let fp = group(&fp, n, false);
        format!("{sign}{ip}.{fp}")
    } else {
        format!("{sign}0.{}e{}", group(&ds, n, false), d.exponent())
    }
}

fn maxprec_words(v: V, d: &D) -> i64 {
    let p = ivar_get(v, sym("@__p"));
    if is_fix(p) {
        return fix(p);
    }
    match d.k {
        Kd::Fin if !d.m.is_zero() => to_words(d).frac.len() as i64,
        Kd::Inf | Kd::NaN => 1,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------
// Ruby glue
// ---------------------------------------------------------------------------

fn class_of_name(v: V) -> String {
    if v == QNIL {
        "nil".into()
    } else if v == QTRUE {
        "true".into()
    } else if v == QFALSE {
        "false".into()
    } else {
        class_name(real_class(v))
    }
}

/// Convert an operand; Float uses the shortest representation. None = not convertible.
fn operand(v: V) -> Option<D> {
    if is_bd(v) {
        Some(get(v))
    } else if is_fix(v) {
        Some(D::from_int(&BigInt::from(fix(v))))
    } else if is_big(v) {
        Some(D::from_int(&rbig(v).n))
    } else if is_float(v) {
        Some(from_f64(float_val(v), 0))
    } else if is_rational(v) {
        let n = send0(v, "numerator");
        let d = send0(v, "denominator");
        let (Some(n), Some(d)) = (operand(n), operand(d)) else { return None };
        Some(div(&n, &d))
    } else {
        None
    }
}
fn is_rational(v: V) -> bool {
    is_heap(v) && const_lookup(rt().c.object, sym("Rational")).is_some_and(|r| is_class(r) && is_a(v, r as *mut Class))
}

/// Fall back to `other.coerce(self)` (rb_num_coerce_bin).
fn coerce_bin(s: V, o: V, op: &str) -> V {
    if is_heap(o) && !is_str(o) && respond_to(o, sym("coerce")) {
        let pair = tri!(send1(o, "coerce", s));
        if is_ary(pair) && rary(pair).v.len() == 2 {
            return send1(rary(pair).v[0], op, rary(pair).v[1]);
        }
    }
    type_error(&format!("{} can't be coerced into BigDecimal", if o == QNIL { "nil".into() } else { class_of_name(o) }))
}

fn binop(s: V, o: V, op: &str, f: fn(&D, &D) -> D) -> V {
    match operand(o) {
        Some(b) => new_bd(f(&get(s), &b)),
        None => coerce_bin(s, o, op),
    }
}

fn domain_error(d: &D) -> V {
    let msg = match d.k {
        Kd::NaN => "Computation results in 'NaN' (Not a Number)",
        Kd::Inf if d.neg => "Computation results in '-Infinity'",
        _ => "Computation results in 'Infinity'",
    };
    raise(rt().c.float_domain_error, msg)
}
fn to_int(d: &D) -> V {
    if d.k != Kd::Fin {
        return domain_error(d);
    }
    if d.m.is_zero() {
        return mkfix(0);
    }
    let i = if d.e >= 0 { &d.m * pow10(d.e) } else { &d.m / pow10(-d.e) };
    int_from_big(if d.neg { -i } else { i })
}

fn digits_arg(v: V) -> Result<i64, V> {
    if is_fix(v) {
        Ok(fix(v))
    } else {
        Err(type_error(&format!("no implicit conversion of {} into Integer", class_of_name(v))))
    }
}

/// Kernel#BigDecimal(value, prec = omitted, exception: true)
fn kernel_bigdecimal(a: &[V]) -> V {
    let mut args: Vec<V> = a.to_vec();
    let mut exception = true;
    if let Some(&last) = args.last() {
        if is_hash(last) && hdr(last).flags & FL_KWHASH != 0 || (is_hash(last) && args.len() > 1) {
            let e = crate::hash::hget(last, mksym(sym("exception")));
            if e == QFALSE {
                exception = false;
            }
            args.pop();
        }
    }
    if args.is_empty() || args.len() > 2 {
        return arity_error(args.len() as i32, 1, 2);
    }
    let v = args[0];
    let prec: Option<i64> = match args.get(1) {
        None => None,
        Some(&p) => match digits_arg(p) {
            Ok(n) if n < 0 => return arg_error("negative precision"),
            Ok(n) => Some(n),
            Err(e) => return e,
        },
    };
    let r = convert(v, prec);
    if r == UNDEF && !exception {
        if let Throw::Raise(e) = pst().pending {
            if is_a(e, rt().c.argument_error) || is_a(e, rt().c.type_error) || is_a(e, rt().c.float_domain_error) {
                pst().pending = Throw::None;
                return QNIL;
            }
        }
    }
    r
}

fn convert(v: V, prec: Option<i64>) -> V {
    // like the gem (3.1), the precision only affects Float and Rational conversions
    if is_bd(v) {
        return v;
    }
    if is_int(v) {
        return new_bd(operand(v).unwrap());
    }
    if is_float(v) {
        let f = float_val(v);
        let Some(p) = prec else {
            return arg_error("can't omit precision for a Float.");
        };
        if p > 16 {
            return arg_error("precision too large.");
        }
        return new_bd(from_f64(f, p as usize));
    }
    if is_rational(v) {
        let Some(p) = prec else {
            return arg_error("can't omit precision for a Rational.");
        };
        let n = operand(tri!(send0(v, "numerator"))).unwrap_or(D::zero(false));
        let d = operand(tri!(send0(v, "denominator"))).unwrap_or(D::zero(false));
        return new_bd(div_digits(&n, &d, p.max(1)));
    }
    if v == QNIL {
        return type_error("can't convert nil into BigDecimal");
    }
    let sv = if is_str(v) {
        v
    } else if is_heap(v) && respond_to(v, sym("to_str")) {
        tri!(send0(v, "to_str"))
    } else {
        return type_error(&format!("can't convert {} into BigDecimal", class_of_name(v)));
    };
    let s = String::from_utf8_lossy(&rstr(sv).b).into_owned();
    match parse_str(&s, false) {
        Some((d, ndig)) => new_bd_p(d, Some((ndig as i64 + 8) / 9 + 1)),
        None => arg_error(&format!("invalid value for BigDecimal(): {}", crate::fmt::str_inspect(s.as_bytes()))),
    }
}

fn round_impl(s: V, a: &[V], default_mode: Option<i64>, to_int_when: fn(usize, i64) -> bool) -> V {
    let d = get(s);
    let mut n = 0;
    let mut md = default_mode.unwrap_or_else(mode);
    match a.len() {
        0 => {}
        1 => {
            if is_hash(a[0]) {
                md = match sym_mode(a[0]) {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                let r = round_at(&d, 0, md);
                return new_bd(r);
            }
            n = match digits_arg(a[0]) {
                Ok(n) => n,
                Err(e) => return e,
            };
        }
        _ => {
            n = match digits_arg(a[0]) {
                Ok(n) => n,
                Err(e) => return e,
            };
            md = match sym_mode(a[1]) {
                Ok(m) => m,
                Err(e) => return e,
            };
        }
    }
    let r = round_at(&d, n, md);
    if to_int_when(a.len(), n) { to_int(&r) } else { new_bd(r) }
}

/// Registers BigDecimal: the class exists after `require "bigdecimal"` (lib/kenny/bigdecimal.rb
/// requires "bigdecimal.so"; a runtime `require "bigdecimal"` works too).
pub fn init() {
    crate::compat::provide_feature(&["bigdecimal"], Some(init_bigdecimal));
}

fn init_bigdecimal() {
    let c = new_class("BigDecimal", rt().c.numeric);
    set_global_const("BigDecimal", c as V);
    *CLASS.get() = c as usize;
    // BigDecimal.new is undefined
    add_method(singleton_class(c as V), sym("new"), Body::Undefined);
    add_method(singleton_class(c as V), sym("allocate"), Body::Undefined);

    let consts: &[(&str, i64)] = &[
        ("BASE", BASE),
        ("EXCEPTION_ALL", 0xff),
        ("EXCEPTION_NaN", 0x02),
        ("EXCEPTION_INFINITY", 0x01),
        ("EXCEPTION_UNDERFLOW", 0x04),
        ("EXCEPTION_OVERFLOW", 0x01),
        ("EXCEPTION_ZERODIVIDE", 0x10),
        ("ROUND_MODE", 0x100),
        ("ROUND_UP", ROUND_UP),
        ("ROUND_DOWN", ROUND_DOWN),
        ("ROUND_HALF_UP", ROUND_HALF_UP),
        ("ROUND_HALF_DOWN", ROUND_HALF_DOWN),
        ("ROUND_CEILING", ROUND_CEILING),
        ("ROUND_FLOOR", ROUND_FLOOR),
        ("ROUND_HALF_EVEN", ROUND_HALF_EVEN),
        ("SIGN_NaN", 0),
        ("SIGN_POSITIVE_ZERO", 1),
        ("SIGN_NEGATIVE_ZERO", -1),
        ("SIGN_POSITIVE_FINITE", 2),
        ("SIGN_NEGATIVE_FINITE", -2),
        ("SIGN_POSITIVE_INFINITE", 3),
        ("SIGN_NEGATIVE_INFINITE", -3),
    ];
    for (n, v) in consts {
        const_set(c, sym(n), mkfix(*v));
    }
    const_set(c, sym("VERSION"), str_from("3.1.8"));
    const_set(c, sym("INFINITY"), new_bd(D::inf(false)));
    const_set(c, sym("NAN"), new_bd(D::nan()));

    defn(rt().c.kernel, "BigDecimal", 1, -1, |_, a, _| kernel_bigdecimal(a));

    defs(c, "_load", 1, 1, |_, a, _| {
        if !is_str(a[0]) {
            return type_error("expected String");
        }
        let s = String::from_utf8_lossy(&rstr(a[0]).b).into_owned();
        let body = s.split_once(':').map(|x| x.1).unwrap_or(&s);
        match parse_str(body, false) {
            Some((d, _)) => new_bd(d),
            None => arg_error(&format!("invalid value for BigDecimal(): {}", crate::fmt::str_inspect(body.as_bytes()))),
        }
    });
    defs(c, "interpret_loosely", 1, 1, |_, a, _| {
        let s = String::from_utf8_lossy(&rstr(tri!(send0(a[0], "to_str"))).b).into_owned();
        new_bd(parse_str(&s, true).map(|x| x.0).unwrap_or(D::zero(false)))
    });
    defs(c, "mode", 1, 2, |_, a, _| {
        if !is_fix(a[0]) {
            return type_error("wrong argument type (expected Integer)");
        }
        if fix(a[0]) == 0x100 {
            if a.len() == 2 && a[1] != QNIL {
                match sym_mode(a[1]) {
                    Ok(m) => *MODE.get() = m,
                    Err(e) => return e,
                }
            }
            return mkfix(mode());
        }
        mkfix(0)
    });
    defs(c, "limit", 0, 1, |_, _, _| mkfix(0));
    defs(c, "double_fig", 0, 0, |_, _, _| mkfix(16));
    defs(c, "__kenny_v4!", 0, 0, |_, _, _| {
        V4.store(true, std::sync::atomic::Ordering::Relaxed);
        QNIL
    });
    defs(c, "save_rounding_mode", 0, 0, |_, _, b| {
        let m = mode();
        let r = call_block(b, &[]);
        *MODE.get() = m;
        r
    });
    defs(c, "save_exception_mode", 0, 0, |_, _, b| call_block(b, &[]));
    defs(c, "save_limit", 0, 0, |_, _, b| call_block(b, &[]));

    defn(c, "+", 1, 1, |s, a, _| binop(s, a[0], "+", add));
    defn(c, "-", 1, 1, |s, a, _| binop(s, a[0], "-", |x, y| add(x, &y.neg_())));
    defn(c, "*", 1, 1, |s, a, _| binop(s, a[0], "*", mul));
    defn(c, "/", 1, 1, |s, a, _| match operand(a[0]) {
        Some(b) if v4() => new_bd(div_v4(&get(s), &b)),
        Some(b) => {
            let x = get(s);
            let r = div(&x, &b);
            let mp = if x.k == Kd::Fin && b.k == Kd::Fin && !b.m.is_zero() && !x.m.is_zero() { Some(div_maxprec(&x, &b)) } else { None };
            new_bd_p(r, mp)
        }
        None => coerce_bin(s, a[0], "/"),
    });
    defn(c, "quo", 1, 2, |s, a, _| {
        if a.len() == 2 && a[1] != QNIL {
            let n = match digits_arg(a[1]) {
                Ok(n) => n,
                Err(e) => return e,
            };
            if n < 0 {
                return arg_error("negative precision");
            }
            if n > 0 {
                return match operand(a[0]) {
                    Some(b) => new_bd(div_digits(&get(s), &b, n)),
                    None => coerce_bin(s, a[0], "quo"),
                };
            }
        }
        send1(s, "/", a[0])
    });
    defn(c, "add", 2, 2, |s, a, _| {
        let n = match digits_arg(a[1]) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match operand(a[0]) {
            Some(b) => new_bd(round_sig(&add(&get(s), &b), n, mode())),
            None => coerce_bin(s, a[0], "+"),
        }
    });
    defn(c, "sub", 2, 2, |s, a, _| {
        let n = match digits_arg(a[1]) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match operand(a[0]) {
            Some(b) => new_bd(round_sig(&add(&get(s), &b.neg_()), n, mode())),
            None => coerce_bin(s, a[0], "-"),
        }
    });
    defn(c, "mult", 2, 2, |s, a, _| {
        let n = match digits_arg(a[1]) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match operand(a[0]) {
            Some(b) => new_bd(round_sig(&mul(&get(s), &b), n, mode())),
            None => coerce_bin(s, a[0], "*"),
        }
    });
    defn(c, "div", 1, 2, |s, a, _| {
        let Some(b) = operand(a[0]) else { return coerce_bin(s, a[0], "div") };
        let x = get(s);
        if a.len() == 2 && a[1] != QNIL {
            let n = match digits_arg(a[1]) {
                Ok(n) => n,
                Err(e) => return e,
            };
            if n < 0 {
                return arg_error("negative precision");
            }
            if n == 0 {
                return new_bd(if v4() { div_v4(&x, &b) } else { div(&x, &b) });
            }
            return new_bd(div_digits(&x, &b, n));
        }
        if x.k == Kd::NaN || b.k == Kd::NaN {
            return domain_error(&D::nan());
        }
        if x.k == Kd::Inf {
            return domain_error(&x);
        }
        match divmod(&x, &b) {
            Ok((q, _)) => to_int(&q),
            Err(e) => e,
        }
    });
    fn modulo(s: V, a: &[V]) -> V {
        let Some(b) = operand(a[0]) else { return coerce_bin(s, a[0], "%") };
        match divmod(&get(s), &b) {
            Ok((_, m)) => new_bd(m),
            Err(e) => e,
        }
    }
    defn(c, "%", 1, 1, |s, a, _| modulo(s, a));
    defn(c, "modulo", 1, 1, |s, a, _| modulo(s, a));
    defn(c, "divmod", 1, 1, |s, a, _| {
        let Some(b) = operand(a[0]) else { return coerce_bin(s, a[0], "divmod") };
        match divmod(&get(s), &b) {
            Ok((q, m)) if v4() && q.k == Kd::Fin => ary_new(vec![tri!(to_int(&q)), new_bd(m)]),
            Ok((q, m)) => ary_new(vec![new_bd(q), new_bd(m)]),
            Err(e) => e,
        }
    });
    defn(c, "remainder", 1, 1, |s, a, _| {
        let Some(b) = operand(a[0]) else { return coerce_bin(s, a[0], "remainder") };
        let x = get(s);
        if x.k != Kd::Fin || b.k != Kd::Fin {
            return new_bd(D::nan());
        }
        if b.m.is_zero() {
            return raise(rt().c.zero_div_error, "divided by 0");
        }
        let e = x.e.min(b.e);
        let p = x.signed() * pow10(x.e - e);
        let q = b.signed() * pow10(b.e - e);
        let r = &p % &q; // truncated remainder (sign of dividend)
        new_bd(D::fin(r.is_negative() || (r.is_zero() && x.neg), r.abs(), e))
    });
    fn power(s: V, a: &[V]) -> V {
        let x = get(s);
        let p = a[0];
        if is_int(p) {
            let n = if is_fix(p) { fix(p) } else { return arg_error("exponent too large") };
            if x.k == Kd::NaN {
                return new_bd(D::nan());
            }
            if n == 0 {
                return new_bd(D::fin(false, BigInt::one(), 0));
            }
            if x.k == Kd::Inf {
                if n < 0 {
                    return new_bd(D::zero(false));
                }
                return new_bd(D::inf(x.neg && n % 2 == 1));
            }
            if x.m.is_zero() {
                return new_bd(if n < 0 { D::inf(x.neg && n % 2 != 0) } else { D::zero(x.neg && n % 2 != 0) });
            }
            let pos = D::fin(x.neg && n % 2 != 0, num_traits::pow(x.m.clone(), n.unsigned_abs() as usize), x.e * n.abs());
            let r = if n > 0 { pos } else { div(&D::fin(false, BigInt::one(), 0), &pos) };
            let r = if a.len() > 1 && is_fix(a[1]) && fix(a[1]) > 0 { round_sig(&r, fix(a[1]), mode()) } else { r };
            return new_bd(r);
        }
        let Some(pd) = operand(p) else {
            return type_error(&format!("wrong argument type {} (expected scalar Numeric)", class_of_name(p)));
        };
        let f = x.to_f64().powf(pd.to_f64());
        new_bd(from_f64(f, 16))
    }
    defn(c, "**", 1, 1, |s, a, _| power(s, a));
    defn(c, "power", 1, 2, |s, a, _| power(s, a));
    defn(c, "sqrt", 0, 1, |s, a, _| {
        let x = get(s);
        let n = a.first().filter(|v| is_fix(**v)).map(|v| fix(*v)).unwrap_or(0);
        if x.k == Kd::NaN {
            return raise(rt().c.float_domain_error, "sqrt of 'NaN'(Not a Number)");
        }
        if x.neg && !x.is_zero() {
            return raise(rt().c.float_domain_error, "sqrt of negative value");
        }
        if x.k == Kd::Inf || x.m.is_zero() {
            return s;
        }
        // 4.x: n significant digits (n = 0: the value's own + 16), rounded
        let (digits, n4) = if v4() {
            let n = if n == 0 { x.digits().len() as i64 + 16 } else { n };
            (n + 9, Some(n))
        } else {
            ((n.max(16) / 9 + 2) * 9 + 1, None)
        };
        // sqrt(m * 10^e) with `digits` significant digits (truncated)
        let mut e = x.e;
        let mut m = x.m.clone();
        let target = digits * 2 + 2;
        let len = m.to_string().len() as i64;
        let mut shift = (target - len).max(0);
        if (e - shift) % 2 != 0 {
            shift += 1;
        }
        m *= pow10(shift);
        e -= shift;
        let r = m.sqrt();
        let r = round_sig(&D::fin(false, r, e / 2), digits, ROUND_DOWN);
        new_bd(match n4 {
            Some(n) => round_sig(&r, n, mode()),
            None => r,
        })
    });
    defn(c, "-@", 0, 0, |s, _, _| new_bd(get(s).neg_()));
    defn(c, "+@", 0, 0, |s, _, _| s);
    defn(c, "abs", 0, 0, |s, _, _| {
        let mut d = get(s);
        d.neg = false;
        new_bd(d)
    });

    fn cmp_op(s: V, o: V, f: fn(Ordering) -> bool, name: &str) -> V {
        match operand(o) {
            Some(b) => match cmp(&get(s), &b) {
                Some(ord) => bool_v(f(ord)),
                None => QFALSE,
            },
            None => {
                if is_heap(o) && !is_str(o) && respond_to(o, sym("coerce")) {
                    return coerce_bin(s, o, name);
                }
                arg_error(&format!("comparison of BigDecimal with {} failed", if o == QNIL { "nil".into() } else { crate::fmt::inspect_lossy(o) }))
            }
        }
    }
    defn(c, "<", 1, 1, |s, a, _| cmp_op(s, a[0], |o| o.is_lt(), "<"));
    defn(c, "<=", 1, 1, |s, a, _| cmp_op(s, a[0], |o| o.is_le(), "<="));
    defn(c, ">", 1, 1, |s, a, _| cmp_op(s, a[0], |o| o.is_gt(), ">"));
    defn(c, ">=", 1, 1, |s, a, _| cmp_op(s, a[0], |o| o.is_ge(), ">="));
    defn(c, "<=>", 1, 1, |s, a, _| match operand(a[0]) {
        Some(b) => match cmp(&get(s), &b) {
            Some(o) => mkfix(o as i64),
            None => QNIL,
        },
        None => {
            if is_heap(a[0]) && !is_str(a[0]) && respond_to(a[0], sym("coerce")) {
                let r = coerce_bin(s, a[0], "<=>");
                if r == UNDEF {
                    pst().pending = Throw::None;
                    return QNIL;
                }
                return r;
            }
            QNIL
        }
    });
    fn eq(s: V, o: V) -> V {
        match operand(o) {
            Some(b) => bool_v(cmp(&get(s), &b) == Some(Ordering::Equal)),
            None => {
                if s == o {
                    QTRUE
                } else if is_heap(o) && !is_str(o) && kind(o) == Some(Kind::Object) && respond_to(o, sym("coerce")) {
                    send1(o, "==", s)
                } else {
                    QFALSE
                }
            }
        }
    }
    defn(c, "==", 1, 1, |s, a, _| eq(s, a[0]));
    defn(c, "===", 1, 1, |s, a, _| eq(s, a[0]));
    defn(c, "__kn_eql", 1, 1, |s, a, _| eq(s, a[0]));
    defn(c, "__kn_hash", 0, 0, |s, _, _| {
        use std::hash::{Hash, Hasher};
        let d = get(s);
        let mut h = rustc_hash::FxHasher::default();
        (d.k as u8, d.neg && d.k != Kd::Fin, d.e).hash(&mut h);
        d.m.hash(&mut h);
        mkfix((h.finish() >> 2) as i64)
    });
    defn(c, "coerce", 1, 1, |s, a, _| match operand(a[0]) {
        Some(b) => ary_new(vec![new_bd(b), s]),
        None => type_error(&format!("{} can't be coerced into BigDecimal", class_of_name(a[0]))),
    });

    defn(c, "to_s", 0, 1, |s, a, _| {
        let f = match a.first() {
            None | Some(&QNIL) => String::new(),
            Some(&v) if is_fix(v) => fix(v).to_string(),
            Some(&v) if is_str(v) => String::from_utf8_lossy(&rstr(v).b).into_owned(),
            Some(&v) => return type_error(&format!("no implicit conversion of {} into String", class_of_name(v))),
        };
        str_from(&to_s(&get(s), &f))
    });
    defn(c, "inspect", 0, 0, |s, _, _| str_from(&to_s(&get(s), "")));
    defn(c, "_dump", 0, 1, |s, _, _| {
        let d = get(s);
        str_from(&format!("{}:{}", maxprec_words(s, &d) * 9, to_s(&d, "")))
    });
    defn(c, "to_i", 0, 0, |s, _, _| to_int(&get(s)));
    defn(c, "to_int", 0, 0, |s, _, _| to_int(&get(s)));
    defn(c, "to_f", 0, 0, |s, _, _| float_new(get(s).to_f64()));
    defn(c, "to_d", 0, 0, |s, _, _| s);
    defn(c, "to_r", 0, 0, |s, _, _| {
        let d = get(s);
        if d.k != Kd::Fin {
            return domain_error(&d);
        }
        let (n, den) = if d.e >= 0 { (d.signed() * pow10(d.e), BigInt::one()) } else { (d.signed(), pow10(-d.e)) };
        // Kernel#Rational reduces (Rational has no public `new`)
        send(rt().main_obj, sym("Rational"), &[int_from_big(n), int_from_big(den)], QNIL)
    });
    defn(c, "hash", 0, 0, |s, _, _| send0(s, "__kn_hash"));

    defn(c, "round", 0, 2, |s, a, _| round_impl(s, a, None, |argc, n| argc == 0 || (argc == 1 && n < 1)));
    defn(c, "floor", 0, 1, |s, a, _| round_impl(s, a, Some(ROUND_FLOOR), |argc, _| argc == 0));
    defn(c, "ceil", 0, 1, |s, a, _| round_impl(s, a, Some(ROUND_CEILING), |argc, _| argc == 0));
    defn(c, "truncate", 0, 1, |s, a, _| round_impl(s, a, Some(ROUND_DOWN), |argc, _| argc == 0));
    defn(c, "fix", 0, 0, |s, _, _| new_bd(round_at(&get(s), 0, ROUND_DOWN)));
    defn(c, "frac", 0, 0, |s, _, _| {
        let d = get(s);
        if d.k != Kd::Fin {
            return s;
        }
        let i = round_at(&d, 0, ROUND_DOWN);
        new_bd(add(&d, &i.neg_()))
    });

    defn(c, "sign", 0, 0, |s, _, _| {
        let d = get(s);
        mkfix(match d.k {
            Kd::NaN => 0,
            Kd::Inf => if d.neg { -3 } else { 3 },
            Kd::Fin if d.m.is_zero() => if d.neg { -1 } else { 1 },
            Kd::Fin => if d.neg { -2 } else { 2 },
        })
    });
    defn(c, "zero?", 0, 0, |s, _, _| bool_v(get(s).is_zero()));
    defn(c, "nonzero?", 0, 0, |s, _, _| if get(s).is_zero() { QNIL } else { s });
    defn(c, "positive?", 0, 0, |s, _, _| {
        let d = get(s);
        bool_v(d.k != Kd::NaN && !d.neg && !d.is_zero())
    });
    defn(c, "negative?", 0, 0, |s, _, _| {
        let d = get(s);
        bool_v(d.k != Kd::NaN && d.neg && !d.is_zero())
    });
    defn(c, "finite?", 0, 0, |s, _, _| bool_v(get(s).k == Kd::Fin));
    defn(c, "nan?", 0, 0, |s, _, _| bool_v(get(s).k == Kd::NaN));
    defn(c, "infinite?", 0, 0, |s, _, _| {
        let d = get(s);
        if d.k == Kd::Inf { mkfix(if d.neg { -1 } else { 1 }) } else { QNIL }
    });
    defn(c, "integer?", 0, 0, |_, _, _| QFALSE);
    defn(c, "real?", 0, 0, |_, _, _| QTRUE);
    defn(c, "exponent", 0, 0, |s, _, _| mkfix(get(s).exponent()));
    defn(c, "precision", 0, 0, |s, _, _| mkfix(get(s).precision_scale().0));
    defn(c, "scale", 0, 0, |s, _, _| mkfix(get(s).precision_scale().1));
    defn(c, "precision_scale", 0, 0, |s, _, _| {
        let (p, sc) = get(s).precision_scale();
        ary_new(vec![mkfix(p), mkfix(sc)])
    });
    defn(c, "n_significant_digits", 0, 0, |s, _, _| {
        let d = get(s);
        mkfix(if d.k != Kd::Fin || d.m.is_zero() { 0 } else { d.digits().len() as i64 })
    });
    defn(c, "precs", 0, 0, |s, _, _| {
        let d = get(s);
        let w = if d.k == Kd::Fin && !d.m.is_zero() { to_words(&d).frac.len() as i64 } else { 1 };
        ary_new(vec![mkfix(w * 9), mkfix(maxprec_words(s, &d).max(w) * 9 + 9)])
    });
    defn(c, "split", 0, 0, |s, _, _| {
        let d = get(s);
        let (sign, digits) = match d.k {
            Kd::NaN => (0, "NaN".to_string()),
            Kd::Inf => (if d.neg { -1 } else { 1 }, "Infinity".to_string()),
            Kd::Fin => (if d.neg { -1 } else { 1 }, if d.m.is_zero() { "0".into() } else { d.digits() }),
        };
        ary_new(vec![mkfix(sign), str_from(&digits), mkfix(10), mkfix(d.exponent())])
    });
    defn(c, "clone", 0, -1, |s, _, _| s);
    defn(c, "dup", 0, 0, |s, _, _| s);

}
