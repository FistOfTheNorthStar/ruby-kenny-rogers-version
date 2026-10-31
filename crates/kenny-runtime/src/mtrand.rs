//! Random, Kernel#rand/srand, Array#shuffle/sample on CRuby's generator (MT19937, random.c)
//! so that a seed gives the same numbers as in CRuby: tests and fixtures rely on it.
//!
//! A Random keeps its state in a binary String ivar (`@__mt`); Kernel#rand and friends use
//! the default generator, seeded from the OS at startup.

use crate::core::*;
use crate::tri;
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};
use std::sync::Mutex;

const N: usize = 624;
const M: usize = 397;

#[derive(Clone)]
pub struct Mt {
    s: [u32; N],
    i: usize,
}

impl Mt {
    fn genrand(seed: u32) -> Mt {
        let mut s = [0u32; N];
        s[0] = seed;
        for j in 1..N {
            s[j] = 1812433253u32.wrapping_mul(s[j - 1] ^ (s[j - 1] >> 30)).wrapping_add(j as u32);
        }
        Mt { s, i: N }
    }
    fn by_array(key: &[u32]) -> Mt {
        let mut mt = Mt::genrand(19650218);
        let s = &mut mt.s;
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..N.max(key.len()) {
            s[i] = (s[i] ^ (s[i - 1] ^ (s[i - 1] >> 30)).wrapping_mul(1664525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                s[0] = s[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            s[i] = (s[i] ^ (s[i - 1] ^ (s[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                s[0] = s[N - 1];
                i = 1;
            }
        }
        s[0] = 0x8000_0000;
        mt
    }
    /// CRuby's rand_init: the seed's absolute value as 32-bit words, least significant first.
    pub fn from_seed(seed: &BigInt) -> Mt {
        let (_, mut words) = seed.abs().to_u32_digits();
        if words.is_empty() {
            words.push(0);
        }
        if words.len() <= 1 {
            return Mt::genrand(words[0]);
        }
        // a top word of 1 is taken for CRuby's leading-zero guard and dropped
        if words.last() == Some(&1) {
            words.pop();
        }
        Mt::by_array(&words)
    }
    fn next_state(&mut self) {
        let s = &mut self.s;
        let mix = |u: u32, v: u32| ((u & 0x8000_0000) | (v & 0x7fff_ffff)) >> 1 ^ if v & 1 != 0 { 0x9908_b0df } else { 0 };
        for k in 0..N - M {
            s[k] = s[k + M] ^ mix(s[k], s[k + 1]);
        }
        for k in N - M..N - 1 {
            s[k] = s[k + M - N] ^ mix(s[k], s[k + 1]);
        }
        s[N - 1] = s[M - 1] ^ mix(s[N - 1], s[0]);
        self.i = 0;
    }
    pub fn int32(&mut self) -> u32 {
        if self.i >= N {
            self.next_state();
        }
        let mut y = self.s[self.i];
        self.i += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }
    /// [0, 1) with 53 bits (genrand_res53)
    pub fn real(&mut self) -> f64 {
        let a = (self.int32() >> 5) as f64;
        let b = (self.int32() >> 6) as f64;
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }
    /// [0, 1] (int_pair_to_real_inclusive): the top 53 of the pair's 64 bits over 2^53 - 1
    fn real_incl(&mut self) -> f64 {
        let a = self.int32() as u64;
        let b = self.int32() as u64;
        ((a << 32 | b) >> 11) as f64 / ((1u64 << 53) - 1) as f64
    }
    /// [0, limit] by masking and retrying (limited_rand)
    pub fn limited(&mut self, limit: u64) -> u64 {
        if limit == 0 {
            return 0;
        }
        let mask = u64::MAX >> limit.leading_zeros();
        'retry: loop {
            let mut val: u64 = 0;
            for i in (0..2).rev() {
                if (mask >> (i * 32)) & 0xffff_ffff != 0 {
                    val |= (self.int32() as u64) << (i * 32);
                    val &= mask;
                    if limit < val {
                        continue 'retry;
                    }
                }
            }
            return val;
        }
    }
    /// [0, limit] for a big limit (limited_big_rand)
    fn limited_big(&mut self, limit: &BigInt) -> BigInt {
        let (_, lim) = limit.to_u32_digits();
        let len = lim.len();
        let mut out = vec![0u32; len];
        'retry: loop {
            let mut mask: u32 = 0;
            let mut boundary = true;
            for i in (0..len).rev() {
                let l = lim[i];
                mask = if mask != 0 { 0xffff_ffff } else if l == 0 { 0 } else { u32::MAX >> l.leading_zeros() };
                let mut r = 0;
                if mask != 0 {
                    r = self.int32() & mask;
                    if boundary {
                        if l < r {
                            continue 'retry;
                        }
                        if r < l {
                            boundary = false;
                        }
                    }
                }
                out[i] = r;
            }
            return BigInt::from_slice(Sign::Plus, &out);
        }
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n + 4);
        while out.len() + 4 <= n {
            out.extend_from_slice(&self.int32().to_le_bytes());
        }
        if out.len() < n {
            let x = self.int32().to_le_bytes();
            let k = n - out.len();
            out.extend_from_slice(&x[..k]);
        }
        out
    }
    fn save(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(4 * N + 4);
        for w in &self.s {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b.extend_from_slice(&(self.i as u32).to_le_bytes());
        b
    }
    fn load(b: &[u8]) -> Mt {
        let mut s = [0u32; N];
        for (k, w) in s.iter_mut().enumerate() {
            *w = u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap());
        }
        let i = u32::from_le_bytes(b[4 * N..4 * N + 4].try_into().unwrap()) as usize;
        Mt { s, i }
    }
}

/// The default generator and its seed.
static DEFAULT: Mutex<Option<(Mt, BigInt)>> = Mutex::new(None);

fn new_seed() -> BigInt {
    BigInt::from_bytes_le(Sign::Plus, &crate::openssl::urandom(16))
}

fn with_default<R>(f: impl FnOnce(&mut Mt) -> R) -> R {
    let mut d = DEFAULT.lock().unwrap();
    let (mt, _) = d.get_or_insert_with(|| {
        let s = new_seed();
        (Mt::from_seed(&s), s)
    });
    f(mt)
}

fn int_big(v: V) -> Option<BigInt> {
    if is_fix(v) {
        Some(BigInt::from(fix(v)))
    } else if is_big(v) {
        Some(rbig(v).n.clone())
    } else {
        None
    }
}

/// A generator: a Random's own state, or the default one.
enum Gen {
    Obj(V),
    Default,
}

impl Gen {
    fn with<R>(&self, f: impl FnOnce(&mut Mt) -> R) -> R {
        match self {
            Gen::Default => with_default(f),
            Gen::Obj(o) => {
                let st = ivar_get(*o, sym("@__mt"));
                let mut mt = if is_str(st) && rstr(st).b.len() == 4 * N + 4 { Mt::load(&rstr(st).b) } else { Mt::from_seed(&BigInt::zero()) };
                let r = f(&mut mt);
                if is_str(st) && rstr(st).b.len() == 4 * N + 4 {
                    rstr(st).b.copy_from_slice(&mt.save());
                }
                r
            }
        }
    }
}

fn invalid(arg: V) -> V {
    arg_error(&format!("invalid argument - {}", crate::fmt::inspect_lossy(arg)))
}

/// rand(arg): nil → [0,1); Integer n → [0,n); Float f → [0,f); a Range. `strict`
/// (Random#rand): invalid limits raise; else (Kernel#rand) a limit of 0 or below takes its
/// absolute value or gives a Float.
fn rand_value(g: &Gen, arg: V, strict: bool) -> V {
    if arg == QNIL {
        return float_new(g.with(|m| m.real()));
    }
    if let Some(n) = int_big(arg) {
        let n = if strict { n } else { n.abs() };
        if n.is_zero() {
            return if strict { invalid(arg) } else { float_new(g.with(|m| m.real())) };
        }
        if n.is_negative() {
            return invalid(arg);
        }
        let lim: BigInt = n - 1;
        return match lim.to_u64() {
            Some(l) => int_from_big(BigInt::from(g.with(|m| m.limited(l)))),
            None => int_from_big(g.with(|m| m.limited_big(&lim))),
        };
    }
    if is_float(arg) && !strict {
        // Kernel#rand(1.5) is rand(1): the limit truncated
        let f = float_val(arg);
        if !f.is_finite() {
            return float_new(g.with(|m| m.real()));
        }
        return rand_value(g, int_from_big(BigInt::from(f.abs().trunc() as i64)), false);
    }
    if is_float(arg) {
        let f = float_val(arg);
        if !f.is_finite() || (strict && f <= 0.0) {
            return if strict { invalid(arg) } else { float_new(g.with(|m| m.real())) };
        }
        if f == 0.0 {
            return float_new(g.with(|m| m.real()));
        }
        return float_new(g.with(|m| m.real()) * f);
    }
    if is_range(arg) {
        let rg = rrange(arg);
        let (lo, hi, excl) = (rg.lo, rg.hi, rg.excl);
        if let (Some(l), Some(h)) = (int_big(lo), int_big(hi)) {
            let span: BigInt = &h - &l - BigInt::from(if excl { 1 } else { 0 });
            if span.is_negative() {
                return QNIL;
            }
            let r = match span.to_u64() {
                Some(s) => BigInt::from(g.with(|m| m.limited(s))),
                None => g.with(|m| m.limited_big(&span)),
            };
            return int_from_big(l + r);
        }
        let (Some(l), Some(h)) = (crate::fmt::num_to_f(lo), crate::fmt::num_to_f(hi)) else {
            return rand_other_range(g, lo, hi, excl, strict, arg);
        };
        if h < l || (excl && h == l) {
            return QNIL;
        }
        let r = g.with(|m| if excl { m.real() } else { m.real_incl() });
        return float_new(l + r * (h - l));
    }
    if strict {
        return invalid(arg);
    }
    // Kernel#rand(x): x.to_i
    let i = tri!(send(arg, sym("to_i"), &[], QNIL));
    rand_value(g, i, false)
}

/// rand(a..b) of other objects (Date..Date, Rational ends), as CRuby's rand_range: the span
/// is `b - a`; a Float span picks a Float offset in [0, span], any other that converts with
/// `to_int` (Integer, Rational, ...) an Integer one, another Numeric a Float one; the result
/// is `a + offset`. Ends without `-` raise TypeError as CRuby's fallback to `to_int` does.
fn rand_other_range(g: &Gen, lo: V, hi: V, excl: bool, strict: bool, arg: V) -> V {
    if lo == QNIL || hi == QNIL {
        return if strict { invalid(arg) } else { QNIL };
    }
    if !respond_to(hi, sym("-")) {
        return type_error("no implicit conversion of Range into Integer");
    }
    let span = tri!(send(hi, sym("-"), &[lo], QNIL));
    let float_off = |f: f64| -> Option<V> {
        if f > 0.0 {
            Some(float_new(g.with(|m| if excl { m.real() } else { m.real_incl() }) * f))
        } else if f == 0.0 && !excl {
            Some(float_new(0.0))
        } else {
            None
        }
    };
    let off = if is_float(span) {
        float_off(float_val(span))
    } else if int_big(span).is_some() || respond_to(span, sym("to_int")) {
        let n = if int_big(span).is_some() { span } else { tri!(send(span, sym("to_int"), &[], QNIL)) };
        let Some(n) = int_big(n) else { return type_error("can't convert to Integer") };
        let n: BigInt = n - BigInt::from(if excl { 1 } else { 0 });
        if n.is_negative() {
            None
        } else {
            Some(match n.to_u64() {
                Some(s) => int_from_big(BigInt::from(g.with(|m| m.limited(s)))),
                None => int_from_big(g.with(|m| m.limited_big(&n))),
            })
        }
    } else if is_a(span, rt().c.numeric) {
        let f = tri!(send(span, sym("to_f"), &[], QNIL));
        float_off(float_val(f))
    } else {
        return type_error("no implicit conversion of Range into Integer");
    };
    match off {
        Some(o) => send(lo, sym("+"), &[o], QNIL),
        None => QNIL,
    }
}

/// [0, limit] from `random:` (a Random, or any object with `rand`), or the default generator.
fn upto(rnd: Option<V>, limit: u64) -> Result<u64, V> {
    match rnd {
        None => Ok(with_default(|m| m.limited(limit))),
        Some(r) if is_random(r) => Ok(Gen::Obj(r).with(|m| m.limited(limit))),
        Some(r) if is_class(r) && r == random_class() as V => Ok(with_default(|m| m.limited(limit))),
        Some(r) => {
            if limit == 0 {
                return Ok(0);
            }
            let v = send(r, sym("rand"), &[int_from_big(BigInt::from(limit) + 1)], QNIL);
            if v == UNDEF {
                return Err(UNDEF);
            }
            let Some(n) = int_big(v) else { return Err(type_error("random number is not an Integer")) };
            if n.is_negative() {
                return Err(raise(rt().c.range_error, &format!("random number too small {n}")));
            }
            if n > BigInt::from(limit) {
                return Err(raise(rt().c.range_error, &format!("random number too big {n}")));
            }
            Ok(n.to_u64().unwrap_or(0))
        }
    }
}

fn random_class() -> *mut Class {
    const_lookup(rt().c.object, sym("Random")).unwrap() as *mut Class
}
fn is_random(v: V) -> bool {
    is_heap(v) && !is_class(v) && is_str(ivar_get(v, sym("@__mt")))
}

/// The `random:` keyword of shuffle/sample (a trailing Hash).
fn random_kw(a: &[V]) -> (Vec<V>, Option<V>) {
    let mut a = a.to_vec();
    if let Some(&h) = a.last().filter(|h| is_hash(**h)) {
        a.pop();
        let r = crate::hash::hget(h, mksym(sym("random")));
        return (a, (r != QNIL).then_some(r));
    }
    (a, None)
}

fn shuffle_in_place(v: &mut [V], rnd: Option<V>) -> Result<(), V> {
    let mut i = v.len();
    while i > 0 {
        let j = upto(rnd, (i - 1) as u64)? as usize;
        i -= 1;
        v.swap(i, j);
    }
    Ok(())
}

pub fn init() {
    let r = random_class();
    defn(r, "initialize", 0, 1, |s, a, _| {
        let seed = match a.first() {
            Some(&x) if x != QNIL => x,
            _ => int_from_big(new_seed()),
        };
        let Some(n) = int_big(seed) else {
            let i = tri!(send(seed, sym("to_int"), &[], QNIL));
            return send(s, sym("initialize"), &[i], QNIL);
        };
        let mt = Mt::from_seed(&n);
        ivar_set(s, sym("@__seed"), seed);
        ivar_set(s, sym("@__mt"), crate::pack::bin_str(mt.save()));
        QNIL
    });
    defn(r, "initialize_copy", 1, 1, |s, a, _| {
        ivar_set(s, sym("@__seed"), ivar_get(a[0], sym("@__seed")));
        let st = ivar_get(a[0], sym("@__mt"));
        if is_str(st) {
            ivar_set(s, sym("@__mt"), crate::pack::bin_str(rstr(st).b.clone()));
        }
        s
    });
    defn(r, "seed", 0, 0, |s, _, _| ivar_get(s, sym("@__seed")));
    defn(r, "rand", 0, 1, |s, a, _| rand_value(&Gen::Obj(s), a.first().copied().unwrap_or(QNIL), true));
    defn(r, "random_number", 0, 1, |s, a, _| {
        let arg = a.first().copied().unwrap_or(QNIL);
        let ok = |x: V| (is_fix(x) && fix(x) > 0) || (is_float(x) && float_val(x) > 0.0) || is_big(x) || is_range(x);
        rand_value(&Gen::Obj(s), if ok(arg) { arg } else { QNIL }, true)
    });
    defn(r, "bytes", 1, 1, |s, a, _| {
        let n = if is_fix(a[0]) { fix(a[0]).max(0) as usize } else { 0 };
        crate::pack::bin_str(Gen::Obj(s).with(|m| m.bytes(n)))
    });
    defn(r, "==", 1, 1, |s, a, _| {
        let o = a[0];
        if !is_random(s) || !is_random(o) || real_class(o) != real_class(s) {
            return QFALSE;
        }
        bool_v(eql(ivar_get(s, sym("@__seed")), ivar_get(o, sym("@__seed"))) && rstr(ivar_get(s, sym("@__mt"))).b == rstr(ivar_get(o, sym("@__mt"))).b)
    });
    defs(r, "rand", 0, 1, |_, a, _| rand_value(&Gen::Default, a.first().copied().unwrap_or(QNIL), true));
    defs(r, "random_number", 0, 1, |_, a, _| {
        let arg = a.first().copied().unwrap_or(QNIL);
        let ok = |x: V| (is_fix(x) && fix(x) > 0) || (is_float(x) && float_val(x) > 0.0) || is_big(x) || is_range(x);
        rand_value(&Gen::Default, if ok(arg) { arg } else { QNIL }, true)
    });
    defs(r, "bytes", 1, 1, |_, a, _| {
        let n = if is_fix(a[0]) { fix(a[0]).max(0) as usize } else { 0 };
        crate::pack::bin_str(with_default(|m| m.bytes(n)))
    });
    defs(r, "new_seed", 0, 0, |_, _, _| int_from_big(new_seed()));
    defs(r, "seed", 0, 0, |_, _, _| {
        with_default(|_| ());
        int_from_big(DEFAULT.lock().unwrap().as_ref().unwrap().1.clone())
    });
    fn srand(a: &[V]) -> V {
        let seed = match a.first() {
            Some(&x) => match int_big(x) {
                Some(n) => n,
                None if x != QNIL && respond_to(x, sym("to_int")) => {
                    let i = tri!(send(x, sym("to_int"), &[], QNIL));
                    if int_big(i).is_none() {
                        return type_error(&format!("can't convert {0} to Integer ({0}#to_int gives {1})", class_name(real_class(x)), class_name(real_class(i))));
                    }
                    return srand(&[i]);
                }
                // (rb_to_int: nil, Strings ... are no seed)
                None => {
                    let what = if x == QNIL { "nil".to_string() } else if x == QTRUE { "true".into() } else if x == QFALSE { "false".into() } else { class_name(real_class(x)) };
                    return type_error(&format!("no implicit conversion of {what} into Integer"));
                }
            },
            None => new_seed(),
        };
        let mut d = DEFAULT.lock().unwrap();
        let old = d.take().map(|(_, s)| s).unwrap_or_else(BigInt::zero);
        *d = Some((Mt::from_seed(&seed), seed));
        int_from_big(old)
    }
    defs(r, "srand", 0, 1, |_, a, _| srand(a));
    let k = rt().c.kernel;
    defn(k, "srand", 0, 1, |_, a, _| srand(a));
    defn(k, "rand", 0, 1, |_, a, _| rand_value(&Gen::Default, a.first().copied().unwrap_or(QNIL), false));
    // (private: visibility.rs runs after this)

    let ac = rt().c.array;
    defn(ac, "shuffle", 0, 1, |s, a, _| {
        let (_, rnd) = random_kw(a);
        let mut v = crate::gc::Rooted::of(rary(s).v.clone());
        if let Err(e) = shuffle_in_place(&mut v, rnd) {
            return e;
        }
        ary_new(v.into_vec())
    });
    defn(ac, "shuffle!", 0, 1, |s, a, _| {
        if is_frozen(s) {
            return check_frozen(s);
        }
        let (_, rnd) = random_kw(a);
        let mut v = crate::gc::Rooted::of(rary(s).v.clone());
        if let Err(e) = shuffle_in_place(&mut v, rnd) {
            return e;
        }
        rary(s).v.set(v.into_vec());
        s
    });
    // sample / sample(n), as array.c's rb_ary_sample (its choice of method by n decides
    // which random numbers are drawn)
    defn(ac, "sample", 0, 2, |s, a, _| {
        let (pos, rnd) = random_kw(a);
        let v = crate::gc::Rooted::of(rary(s).v.clone());
        let len = v.len();
        let Some(&nv) = pos.first() else {
            if len == 0 {
                return QNIL;
            }
            let i = match upto(rnd, (len - 1) as u64) {
                Ok(i) => i as usize,
                Err(e) => return e,
            };
            return v.get(i).copied().unwrap_or(QNIL);
        };
        if !is_fix(nv) {
            return type_error("no implicit conversion into Integer");
        }
        let n = fix(nv);
        if n < 0 {
            return arg_error("negative sample number");
        }
        let n = (n as usize).min(len);
        let r = |lim: usize| upto(rnd, lim as u64).map(|x| x as usize);
        let out: Vec<V> = match n {
            0 => vec![],
            1 => match r(len - 1) {
                Ok(i) => vec![v[i]],
                Err(e) => return e,
            },
            2 => {
                let (i, j) = match (r(len - 1), r(len - 2)) {
                    (Ok(i), Ok(j)) => (i, j),
                    (Err(e), _) | (_, Err(e)) => return e,
                };
                let j = if j >= i { j + 1 } else { j };
                vec![v[i], v[j]]
            }
            3 => {
                let (i, j, k) = match (r(len - 1), r(len - 2), r(len - 3)) {
                    (Ok(i), Ok(j), Ok(k)) => (i, j, k),
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return e,
                };
                let (mut j, mut k) = (j, k);
                let (mut l, mut g) = (j, i);
                if j >= i {
                    l = i;
                    j += 1;
                    g = j;
                }
                if k >= l {
                    k += 1;
                    if k >= g {
                        k += 1;
                    }
                }
                vec![v[i], v[j], v[k]]
            }
            4..=10 => {
                // indices drawn into a sorted list, each skipping the ones taken
                let mut rnds = Vec::with_capacity(n);
                for i in 0..n {
                    match r(len - i - 1) {
                        Ok(x) => rnds.push(x),
                        Err(e) => return e,
                    }
                }
                let mut sorted: Vec<usize> = vec![rnds[0]];
                let mut idx = vec![rnds[0]];
                for &x in &rnds[1..] {
                    let mut k = x;
                    let mut j = 0;
                    while j < sorted.len() {
                        if k < sorted[j] {
                            break;
                        }
                        k += 1;
                        j += 1;
                    }
                    sorted.insert(j, k);
                    idx.push(k);
                }
                idx.into_iter().map(|i| v[i]).collect()
            }
            _ => {
                // a partial Fisher-Yates from the front (the memo/shuffle paths draw the same)
                let mut w = v.to_vec();
                for i in 0..n {
                    let j = match r(len - i - 1) {
                        Ok(j) => j + i,
                        Err(e) => return e,
                    };
                    w.swap(i, j);
                }
                w.truncate(n);
                w
            }
        };
        ary_new(out)
    });
}
