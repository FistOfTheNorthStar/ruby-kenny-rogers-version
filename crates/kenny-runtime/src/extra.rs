//! Core-library methods that the other modules don't define yet (gap analysis against CRuby):
//! Kernel/BasicObject/main extras, Math, Array, Enumerable/Enumerator, Integer/Float/Numeric,
//! Hash, Module, nil, Proc, Range, Regexp, MatchData, String, Encoding, Struct, Symbol,
//! Thread, Fiber and Time.
//!
//! Registered last in `boot()`, so a `defn` here replaces an earlier definition of the same name.

use crate::core::*;
use crate::fmt::*;
use crate::gc::RVec;
use crate::kernel::{clone_body, new_str_v, sym_arg};
use crate::misc::{each_to_vec, enum_for};
use crate::tri;
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};

// ---------- small helpers ----------

fn cname(v: V) -> String {
    match v {
        QNIL => "nil".into(),
        QTRUE => "true".into(),
        QFALSE => "false".into(),
        _ => class_name(real_class(v)),
    }
}
fn no_conv(v: V, into: &str) -> V {
    type_error(&format!("no implicit conversion of {} into {}", cname(v), into))
}
fn kwarg(a: &[V], key: &str) -> Option<V> {
    let h = *a.last()?;
    if !is_hash(h) {
        return None;
    }
    rhash(h).m.get(&HKey(mksym(sym(key)))).copied()
}
/// Drop a trailing keyword hash from the positional arguments.
fn positional(a: &[V]) -> &[V] {
    match a.last() {
        Some(&h) if is_hash(h) && hdr(h).flags & FL_KWHASH != 0 => &a[..a.len() - 1],
        _ => a,
    }
}
fn s_of(v: V) -> String {
    String::from_utf8_lossy(&rstr(v).b).into_owned()
}
fn want_int(v: V) -> Result<i64, V> {
    if is_fix(v) {
        Ok(fix(v))
    } else if is_float(v) {
        Ok(float_val(v).trunc() as i64)
    } else {
        Err(no_conv(v, "Integer"))
    }
}
pub(crate) fn math_domain(name: &str) -> V {
    let m = const_lookup(rt().c.object, sym("Math")).unwrap();
    let d = const_get_under(m as *mut Class, sym("DomainError")).unwrap();
    raise(d as *mut Class, &format!("Numerical argument is out of domain - {name}"))
}
/// A Math function's argument as f64: Integer, Float, BigDecimal, or another Numeric
/// (Rational...) through its `to_f`.
pub(crate) fn float_arg(v: V) -> Result<f64, V> {
    match num_to_f(v).or_else(|| crate::bigdecimal::to_f(v)) {
        Some(f) => Ok(f),
        None if is_a(v, rt().c.numeric) => {
            let r = send0(v, "to_f");
            if r == UNDEF {
                Err(r)
            } else if is_float(r) {
                Ok(float_val(r))
            } else {
                Err(type_error(&format!("can't convert {} into Float", cname(v))))
            }
        }
        None if v == QNIL => Err(type_error("can't convert nil into Float")),
        None if is_str(v) => Err(type_error("can't convert String into Float")),
        None => Err(type_error(&format!("can't convert {} into Float", cname(v)))),
    }
}
macro_rules! tryr {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}
/// Add `name` to `en` as "collect with each, then call Array's method" (like misc.rs does).
fn enum_delegate(en: *mut Class, name: &str) {
    let id = sym(name);
    let p = proc_new_native(
        Box::new(move |s, args, blk| match each_to_vec(s) {
            Ok(v) => {
                // keep the temporary array (and the values only it references) alive
                let tmp = RVec::from(vec![ary_new(v)]);
                send(tmp[0], id, args, blk)
            }
            Err(e) => e,
        }),
        0,
        -1,
    );
    add_method(en, id, Body::Proc(p));
}
fn copy_method(from: *mut Class, to: *mut Class, name: &str) {
    let m = find_method(from, sym(name));
    if !m.is_null() {
        add_method(to, sym(name), clone_body(unsafe { &(*m).body }));
    }
}

// ---------- libm ----------

unsafe extern "C" {
    #[link_name = "erf"]
    fn c_erf(x: f64) -> f64;
    #[link_name = "erfc"]
    fn c_erfc(x: f64) -> f64;
    #[link_name = "tgamma"]
    fn c_tgamma(x: f64) -> f64;
    #[link_name = "lgamma_r"]
    fn c_lgamma_r(x: f64, sign: *mut i32) -> f64;
    #[link_name = "frexp"]
    fn c_frexp(x: f64, e: *mut i32) -> f64;
    #[link_name = "ldexp"]
    fn c_ldexp(x: f64, e: i32) -> f64;
}

// ---------- Array helpers ----------

/// Binary search over 0..len (find-minimum mode for true/false, find-any mode for numbers).
fn bsearch_by(lo0: i64, hi0: i64, mut f: impl FnMut(i64) -> V) -> Result<Option<i64>, V> {
    let (mut lo, mut hi) = (lo0, hi0);
    let mut found = None;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = f(mid);
        if r == UNDEF {
            return Err(UNDEF);
        }
        if r == QTRUE {
            found = Some(mid);
            hi = mid;
        } else if r == QFALSE || r == QNIL {
            lo = mid + 1;
        } else if let Some(x) = num_to_f(r) {
            if x == 0.0 {
                return Ok(Some(mid));
            } else if x < 0.0 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        } else {
            return Err(type_error(&format!("wrong argument type {} (must be numeric, true, false or nil)", cname(r))));
        }
    }
    Ok(found)
}

fn index_tuples(len: usize, n: usize, nondecreasing: bool) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    if len == 0 && n > 0 {
        return out;
    }
    let mut cur = vec![0usize; n];
    loop {
        out.push(cur.clone());
        // advance like an odometer
        let mut i = n;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if cur[i] + 1 < len {
                cur[i] += 1;
                let v = cur[i];
                for c in cur.iter_mut().skip(i + 1) {
                    *c = if nondecreasing { v } else { 0 };
                }
                break;
            }
        }
    }
}

fn yield_tuples(s: V, name: &str, a: &[V], b: V, nondecreasing: bool) -> V {
    if !is_proc(b) {
        return enum_for(s, name, a);
    }
    let n = tryr!(want_int(a[0]));
    if n < 0 {
        return s;
    }
    let items = RVec::from(rary(s).v.clone());
    for t in index_tuples(items.len(), n as usize, nondecreasing) {
        let tuple = ary_new(t.iter().map(|&i| items[i]).collect());
        tri!(call_block(b, &[tuple]));
    }
    s
}

fn grep_impl(s: V, pat: V, b: V, invert: bool) -> V {
    let items = RVec::from(rary(s).v.clone());
    let mut out = RVec::new();
    for i in 0..items.len() {
        let x = items[i];
        let r = tri!(send1(pat, "===", x));
        if truthy(r) != invert {
            out.push(if is_proc(b) { tri!(call_block(b, &[x])) } else { x });
        }
    }
    ary_new(out.take())
}

fn chunk_impl(s: V, b: V) -> V {
    if !is_proc(b) {
        return enum_for(s, "chunk", &[]);
    }
    let items = RVec::from(rary(s).v.clone());
    let mut out = RVec::new();
    let mut cur = RVec::new();
    let mut key = RVec::new(); // 0 or 1 element: the current chunk's key
    let sep = mksym(sym("_separator"));
    let alone = mksym(sym("_alone"));
    for i in 0..items.len() {
        let x = items[i];
        let k = tri!(call_block(b, &[x]));
        let flush = |out: &mut RVec, cur: &mut RVec, key: &mut RVec| {
            if let Some(&kk) = key.first() {
                let chunk = ary_new(cur.take());
                out.push(ary_new(vec![kk, chunk]));
            }
            key.clear();
        };
        if k == QNIL || k == sep {
            flush(&mut out, &mut cur, &mut key);
            continue;
        }
        if k == alone {
            flush(&mut out, &mut cur, &mut key);
            out.push(ary_new(vec![k, ary_new(vec![x])]));
            continue;
        }
        if is_sym(k) && sym_name(sym_id(k)).starts_with(b"_") {
            return raise(rt().c.runtime_error, "symbols beginning with an underscore are reserved");
        }
        let same = match key.first() {
            Some(&kk) => truthy(tri!(equal(kk, k))),
            None => false,
        };
        if !same {
            flush(&mut out, &mut cur, &mut key);
            key.push(k);
        }
        cur.push(x);
    }
    if let Some(&kk) = key.first() {
        let chunk = ary_new(cur.take());
        out.push(ary_new(vec![kk, chunk]));
    }
    enum_for(ary_new(out.take()), "each", &[])
}

fn slice_impl(s: V, a: &[V], b: V, after: bool) -> V {
    let pat = a.first().copied();
    if pat.is_none() && !is_proc(b) {
        return arg_error("wrong number of arguments (given 0, expected 1)");
    }
    let items = RVec::from(rary(s).v.clone());
    let mut out = RVec::new();
    let mut cur = RVec::new();
    for i in 0..items.len() {
        let x = items[i];
        let hit = match pat {
            Some(p) => tri!(send1(p, "===", x)),
            None => tri!(call_block(b, &[x])),
        };
        if after {
            cur.push(x);
            if truthy(hit) {
                out.push(ary_new(cur.take()));
            }
        } else {
            if truthy(hit) && !cur.is_empty() {
                out.push(ary_new(cur.take()));
            }
            cur.push(x);
        }
    }
    if !cur.is_empty() {
        out.push(ary_new(cur.take()));
    }
    enum_for(ary_new(out.take()), "each", &[])
}

fn chain_impl(s: V, a: &[V]) -> V {
    let mut all = RVec::new();
    for &e in std::iter::once(&s).chain(a.iter()) {
        match each_to_vec(e) {
            Ok(v) => all.extend(v),
            Err(e) => return e,
        }
    }
    enum_for(ary_new(all.take()), "each", &[])
}

// ---------- String helpers ----------

/// Byte offset where each character starts (UTF-8, or one per byte for invalid/binary data), plus the end.
fn char_starts(b: &[u8], binary: bool) -> Vec<usize> {
    let mut out = Vec::with_capacity(b.len() + 1);
    let mut i = 0;
    while i < b.len() {
        out.push(i);
        let c = b[i];
        let len = if binary || c < 0x80 {
            1
        } else {
            match c {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            }
        };
        if len > 1 && (i + len > b.len() || std::str::from_utf8(&b[i..i + len]).is_err()) {
            i += 1;
        } else {
            i += len;
        }
    }
    out.push(b.len());
    out
}
fn is_bin(v: V) -> bool {
    hdr(v).flags & FL_BINARY != 0
}
fn new_like(src: V, b: Vec<u8>) -> V {
    let r = str_new(b);
    if is_bin(src) {
        hdr(r).flags |= FL_BINARY;
    }
    r
}
fn str_mutate(s: V) -> Result<&'static mut SVec<u8>, V> {
    if is_frozen(s) {
        return Err(check_frozen(s));
    }
    unchill(s);
    Ok(&mut rstr(s).b)
}
/// `name!` in terms of `name`: replace the receiver's bytes, nil when nothing changed.
fn bang(s: V, name: &str, a: &[V], b: V, always_self: bool) -> V {
    crate::tri!(check_frozen(s));
    let r = tri!(send(s, sym(name), a, b));
    if !is_str(r) {
        return r;
    }
    let nb = rstr(r).b.clone();
    let changed = nb != rstr(s).b;
    rstr(s).b.set(nb);
    if changed || always_self { s } else { QNIL }
}

fn expand_set(set: &str) -> (bool, Vec<char>) {
    let (neg, set) = match set.strip_prefix('^') {
        Some(r) if !r.is_empty() => (true, r),
        _ => (false, set),
    };
    let cs: Vec<char> = set.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '\\' && i + 1 < cs.len() {
            out.push(cs[i + 1]);
            i += 2;
        } else if i + 2 < cs.len() && cs[i + 1] == '-' {
            for c in cs[i]..=cs[i + 2] {
                out.push(c);
            }
            i += 3;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    (neg, out)
}
fn tr_squeeze(s: &str, from: &str, to: &str) -> String {
    let (neg, f) = expand_set(from);
    let (_, t) = expand_set(to);
    let mut out = String::new();
    let mut last: Option<char> = None;
    for c in s.chars() {
        let pos = f.iter().position(|x| *x == c);
        let mapped = match (neg, pos) {
            (false, Some(i)) => Some(if t.is_empty() { None } else { Some(*t.get(i).unwrap_or(t.last().unwrap())) }),
            (true, None) => Some(t.last().copied()),
            _ => None,
        };
        match mapped {
            Some(Some(m)) => {
                if last != Some(m) {
                    out.push(m);
                }
                last = Some(m);
            }
            Some(None) => {}
            None => {
                out.push(c);
                last = None;
            }
        }
    }
    out
}

fn is_extend(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A | 0x064B..=0x065F
        | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C | 0x20D0..=0x20FF | 0x302A..=0x302F
        | 0x3099..=0x309A | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F
        | 0xE0100..=0xE01EF)
}
fn is_regional(c: char) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&(c as u32))
}
/// Extended grapheme clusters (approximation: combining marks, ZWJ sequences, flags, CRLF).
fn graphemes(s: &str) -> Vec<String> {
    let cs: Vec<char> = s.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let mut g = String::new();
        g.push(cs[i]);
        if cs[i] == '\r' && cs.get(i + 1) == Some(&'\n') {
            g.push('\n');
            out.push(g);
            i += 2;
            continue;
        }
        let first_regional = is_regional(cs[i]);
        i += 1;
        if first_regional && i < cs.len() && is_regional(cs[i]) {
            g.push(cs[i]);
            i += 1;
        }
        while i < cs.len() {
            if is_extend(cs[i]) {
                g.push(cs[i]);
                i += 1;
            } else if cs[i] == '\u{200D}' {
                g.push(cs[i]);
                i += 1;
                if i < cs.len() {
                    g.push(cs[i]);
                    i += 1;
                }
            } else {
                break;
            }
        }
        out.push(g);
    }
    out
}

fn undump(b: &[u8]) -> Result<Vec<u8>, V> {
    let bad = || raise(rt().c.runtime_error, "invalid dumped string; not wrapped with '\"' nor '\"...\".force_encoding(\"...\")' form");
    let inner = if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' { &b[1..b.len() - 1] } else { return Err(bad()) };
    let mut out = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&e) = inner.get(i) else { return Err(raise(rt().c.runtime_error, "invalid escape")) };
        i += 1;
        match e {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'e' => out.push(0x1b),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'v' => out.push(0x0b),
            b'f' => out.push(0x0c),
            b's' => out.push(b' '),
            b'x' => {
                let mut v = 0u32;
                let mut k = 0;
                while k < 2 && i < inner.len() && inner[i].is_ascii_hexdigit() {
                    v = v * 16 + (inner[i] as char).to_digit(16).unwrap();
                    i += 1;
                    k += 1;
                }
                out.push(v as u8);
            }
            b'u' => {
                let mut cps = Vec::new();
                if inner.get(i) == Some(&b'{') {
                    i += 1;
                    let end = inner[i..].iter().position(|&c| c == b'}').map(|p| i + p).ok_or_else(|| raise(rt().c.runtime_error, "unterminated Unicode escape"))?;
                    for part in String::from_utf8_lossy(&inner[i..end]).split_whitespace() {
                        cps.push(u32::from_str_radix(part, 16).unwrap_or(0xFFFD));
                    }
                    i = end + 1;
                } else {
                    let hex = String::from_utf8_lossy(&inner[i..(i + 4).min(inner.len())]).into_owned();
                    cps.push(u32::from_str_radix(&hex, 16).map_err(|_| raise(rt().c.runtime_error, "invalid Unicode escape"))?);
                    i += 4;
                }
                for cp in cps {
                    let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
            }
            b'"' | b'\\' | b'#' => out.push(e),
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

// ---------- Encoding ----------

fn encoding(name: &str) -> V {
    let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class;
    let cname = match name {
        "ASCII-8BIT" | "BINARY" => "BINARY",
        "US-ASCII" | "ASCII" => "US_ASCII",
        _ => "UTF_8",
    };
    const_get_under(enc, sym(cname)).unwrap_or(QNIL)
}
fn enc_name_of(v: V) -> Option<String> {
    if is_str(v) {
        return Some(s_of(v).to_uppercase());
    }
    if is_heap(v) && ivar_defined(v, sym("@__name")) {
        let n = ivar_get(v, sym("@__name"));
        if is_str(n) {
            return Some(s_of(n));
        }
    }
    None
}

// ---------- Time ----------

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Local,
    Utc,
    Off(i64),
}

#[repr(C)]
struct CTm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}
unsafe extern "C" {
    fn mktime(tm: *mut CTm) -> i64;
}

const NS: i128 = 1_000_000_000;

fn is_time(v: V) -> bool {
    is_heap(v) && is_a(v, rt().c.time)
}
fn time_ns(t: V) -> i128 {
    crate::misc::time_ns(t)
}
fn zone_of(t: V) -> Zone {
    if truthy(ivar_get(t, sym("@__utc"))) {
        return Zone::Utc;
    }
    let off = ivar_get(t, sym("@__off"));
    if is_fix(off) { Zone::Off(fix(off)) } else { Zone::Local }
}
fn set_time(t: V, ns: i128, z: Zone) {
    ivar_set(t, sym("@__ns"), int_from_i128(ns));
    ivar_set(t, sym("@__t"), float_new(ns.div_euclid(NS) as f64 + ns.rem_euclid(NS) as f64 / 1e9));
    ivar_set(t, sym("@__utc"), bool_v(z == Zone::Utc));
    ivar_set(t, sym("@__off"), if let Zone::Off(o) = z { mkfix(o) } else { QNIL });
}
fn time_make(cls: *mut Class, ns: i128, z: Zone) -> V {
    let t = object_new(cls);
    set_time(t, ns, z);
    t
}
fn time_cls(s: V) -> *mut Class {
    if is_class(s) { s as *mut Class } else { real_class(s) }
}
fn now_ns() -> i128 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    d.as_nanos() as i128
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn zone_err(v: V) -> V {
    arg_error(&format!("\"+HH:MM\", \"-HH:MM\", \"UTC\" or \"A\"..\"I\",\"K\"..\"Z\" expected for utc_offset: {}", inspect_lossy(v)))
}
fn parse_zone(v: V) -> Result<Zone, V> {
    if v == QNIL {
        return Ok(Zone::Local);
    }
    if is_fix(v) {
        let o = fix(v);
        if o.abs() >= 86400 {
            return Err(arg_error("utc_offset out of range"));
        }
        return Ok(Zone::Off(o));
    }
    if is_time(v) {
        return Ok(zone_of(v));
    }
    if !is_str(v) {
        return Err(zone_err(v));
    }
    let s = s_of(v);
    match s.as_str() {
        "UTC" | "Z" | "-00:00" => return Ok(Zone::Utc),
        _ => {}
    }
    let b = s.as_bytes();
    if b.len() == 1 && b[0].is_ascii_uppercase() && b[0] != b'J' {
        let c = b[0];
        let h: i64 = match c {
            b'A'..=b'I' => (c - b'A' + 1) as i64,
            b'K'..=b'M' => (c - b'K' + 10) as i64,
            b'N'..=b'Y' => -((c - b'N' + 1) as i64),
            _ => 0,
        };
        return Ok(if h == 0 { Zone::Utc } else { Zone::Off(h * 3600) });
    }
    if b.len() >= 3 && (b[0] == b'+' || b[0] == b'-') {
        let sign = if b[0] == b'-' { -1 } else { 1 };
        let digits: String = s[1..].chars().filter(|c| *c != ':').collect();
        let colon_ok = match s.len() {
            3 | 5 | 7 => !s.contains(':'),
            6 => &s[3..4] == ":",
            9 => &s[3..4] == ":" && &s[6..7] == ":",
            _ => false,
        };
        if colon_ok && digits.chars().all(|c| c.is_ascii_digit()) && matches!(digits.len(), 2 | 4 | 6) {
            let h: i64 = digits[0..2].parse().unwrap();
            let m: i64 = if digits.len() >= 4 { digits[2..4].parse().unwrap() } else { 0 };
            let sec: i64 = if digits.len() == 6 { digits[4..6].parse().unwrap() } else { 0 };
            if h < 24 && m < 60 && sec < 60 {
                return Ok(Zone::Off(sign * (h * 3600 + m * 60 + sec)));
            }
        }
    }
    Err(zone_err(v))
}

const MON_ABBR: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn int_field(v: V, what: &str) -> Result<i64, V> {
    if v == QNIL {
        return Err(type_error(&format!("can't convert nil into Integer ({what})")));
    }
    if is_fix(v) {
        return Ok(fix(v));
    }
    if is_float(v) {
        return Ok(float_val(v).floor() as i64);
    }
    if is_str(v) {
        let s = s_of(v);
        if what == "mon" {
            if let Some(i) = MON_ABBR.iter().position(|m| s.eq_ignore_ascii_case(m)) {
                return Ok(i as i64 + 1);
            }
        }
        return s.trim().parse::<i64>().map_err(|_| arg_error(&format!("can't parse: {}", inspect_lossy(v))));
    }
    let r = send0(v, "to_int");
    if r == UNDEF {
        return Err(UNDEF);
    }
    if is_fix(r) { Ok(fix(r)) } else { Err(no_conv(v, "Integer")) }
}
/// Seconds argument: whole seconds and extra nanoseconds.
fn sec_field(v: V) -> Result<(i64, i128), V> {
    if is_float(v) {
        let f = float_val(v);
        let whole = f.floor();
        return Ok((whole as i64, ((f - whole) * 1e9).round() as i128));
    }
    if is_str(v) {
        let s = s_of(v);
        if let Ok(f) = s.trim().parse::<f64>() {
            if s.contains('.') {
                let whole = f.floor();
                return Ok((whole as i64, ((f - whole) * 1e9).round() as i128));
            }
        }
    }
    Ok((int_field(v, "sec")?, 0))
}

/// Build a time from civil fields in the given zone.
fn time_from_fields(cls: *mut Class, f: [i64; 6], extra_ns: i128, z: Zone) -> Result<V, V> {
    let [year, mon, day, hour, min, sec] = f;
    if !(1..=12).contains(&mon) {
        return Err(arg_error("mon out of range"));
    }
    if !(1..=31).contains(&day) {
        return Err(arg_error("mday out of range"));
    }
    if !(0..=24).contains(&hour) {
        return Err(arg_error("hour out of range"));
    }
    if !(0..=59).contains(&min) {
        return Err(arg_error("min out of range"));
    }
    if !(0..=60).contains(&sec) {
        return Err(arg_error("sec out of range"));
    }
    let wall = days_from_civil(year, mon, day) * 86400 + hour * 3600 + min * 60 + sec;
    let secs = match z {
        Zone::Utc => wall,
        Zone::Off(o) => wall - o,
        Zone::Local => {
            let mut tm: CTm = unsafe { std::mem::zeroed() };
            tm.tm_year = (year - 1900) as i32;
            tm.tm_mon = (mon - 1) as i32;
            tm.tm_mday = day as i32;
            tm.tm_hour = hour as i32;
            tm.tm_min = min as i32;
            tm.tm_sec = sec as i32;
            tm.tm_isdst = -1;
            unsafe { mktime(&mut tm) }
        }
    };
    Ok(time_make(cls, secs as i128 * NS + extra_ns, z))
}

/// `Time.new("2000-01-02 03:04:05.5 +09:00")`
fn parse_time_string(cls: *mut Class, v: V, zone_kw: Option<V>) -> Result<V, V> {
    let s = s_of(v);
    let bad = || arg_error(&format!("can't parse: {}", inspect_lossy(v)));
    let t = s.trim();
    let (date, rest) = t.split_once([' ', 'T']).unwrap_or((t, ""));
    let dp: Vec<&str> = date.split('-').collect();
    if dp.len() != 3 || dp[0].len() < 4 {
        return Err(bad());
    }
    let year: i64 = dp[0].parse().map_err(|_| bad())?;
    let mon: i64 = dp[1].parse().map_err(|_| bad())?;
    let day: i64 = dp[2].parse().map_err(|_| bad())?;
    let rest = rest.trim();
    let (clock, zone) = match rest.find(['+', '-', ' ', 'Z']) {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    };
    let (mut h, mut mi, mut sec, mut ns) = (0, 0, 0, 0i128);
    if !clock.is_empty() {
        let cp: Vec<&str> = clock.split(':').collect();
        if cp.len() < 2 {
            return Err(bad());
        }
        h = cp[0].parse().map_err(|_| bad())?;
        mi = cp[1].parse().map_err(|_| bad())?;
        if let Some(sp) = cp.get(2) {
            let (whole, frac) = sp.split_once('.').unwrap_or((sp, ""));
            sec = whole.parse().map_err(|_| bad())?;
            if !frac.is_empty() {
                let digits: String = frac.chars().take(9).collect();
                ns = format!("{digits:0<9}").parse().map_err(|_| bad())?;
            }
        }
    }
    let z = if !zone.is_empty() {
        parse_zone(str_from(zone))?
    } else {
        match zone_kw {
            Some(z) => parse_zone(z)?,
            None => Zone::Local,
        }
    };
    time_from_fields(cls, [year, mon, day, h, mi, sec], ns, z)
}

/// Time.utc / Time.local argument lists (including the 10-element `to_a` form).
fn time_civil(s: V, a: &[V], z: Zone) -> V {
    let a = positional(a);
    let (fields, usec_arg): (Vec<V>, Option<V>) = if a.len() == 10 {
        (vec![a[5], a[4], a[3], a[2], a[1], a[0]], None)
    } else {
        (a[..a.len().min(6)].to_vec(), a.get(6).copied())
    };
    let mut f = [0i64, 1, 1, 0, 0, 0];
    let mut extra = 0i128;
    let names = ["year", "mon", "mday", "hour", "min", "sec"];
    for (i, &v) in fields.iter().enumerate() {
        if v == QNIL && i > 0 {
            continue;
        }
        if i == 5 {
            let (whole, ns) = tryr!(sec_field(v));
            f[5] = whole;
            extra = ns;
        } else {
            f[i] = tryr!(int_field(v, names[i]));
        }
    }
    if let Some(u) = usec_arg {
        let us = tryr!(float_arg(u));
        extra += (us * 1000.0).round() as i128;
    }
    tryr!(time_from_fields(time_cls(s), f, extra, z))
}

fn subsec_str(ns: i128) -> String {
    let sub = ns.rem_euclid(NS);
    if sub == 0 {
        return String::new();
    }
    let d = format!("{:09}", sub);
    format!(".{}", d.trim_end_matches('0'))
}
fn time_strftime(t: V, f: &str) -> String {
    crate::misc::strftime(t, f)
}
fn time_to_s(t: V, inspect: bool) -> V {
    let sub = if inspect { subsec_str(time_ns(t)) } else { String::new() };
    let base = time_strftime(t, "%Y-%m-%d %H:%M:%S");
    let z = if zone_of(t) == Zone::Utc { "UTC".to_string() } else { time_strftime(t, "%z") };
    new_str_v(format!("{base}{sub} {z}"))
}
fn time_round(t: V, a: &[V], mode: u8) -> V {
    let digits = match a.first() {
        Some(&d) => tryr!(want_int(d)),
        None => 0,
    };
    if digits < 0 {
        return arg_error("negative ndigits given");
    }
    let ns = time_ns(t);
    if digits >= 9 {
        return time_make(real_class(t), ns, zone_of(t));
    }
    let unit = 10i128.pow((9 - digits) as u32);
    let base = ns.div_euclid(unit) * unit;
    let rem = ns - base;
    let r = match mode {
        b'f' => base,
        b'c' => {
            if rem > 0 {
                base + unit
            } else {
                base
            }
        }
        _ => {
            if rem * 2 >= unit {
                base + unit
            } else {
                base
            }
        }
    };
    time_make(real_class(t), r, zone_of(t))
}
fn time_at(s: V, a: &[V]) -> V {
    let zone_kw = kwarg(a, "in");
    let a = positional(a);
    let x = a[0];
    let mut z = Zone::Local;
    let mut ns: i128 = if is_time(x) {
        z = zone_of(x);
        time_ns(x)
    } else if is_fix(x) {
        fix(x) as i128 * NS
    } else if is_big(x) {
        rbig(x).n.to_i128().unwrap_or(0).saturating_mul(NS)
    } else if is_float(x) {
        (float_val(x) * 1e9).round() as i128
    } else {
        return type_error(&format!("can't convert {} into an exact number", cname(x)));
    };
    if let Some(&sub) = a.get(1) {
        let unit = match a.get(2) {
            Some(&u) if is_sym(u) => sym_str(sym_id(u)),
            Some(&u) => return arg_error(&format!("unexpected unit: {}", inspect_lossy(u))),
            None => "microsecond".to_string(),
        };
        let mult = match unit.as_str() {
            "millisecond" => 1e6,
            "usec" | "microsecond" => 1e3,
            "nsec" | "nanosecond" => 1.0,
            _ => return arg_error(&format!("unexpected unit: {}", unit)),
        };
        let f = tryr!(float_arg(sub));
        ns += (f * mult).round() as i128;
    }
    if let Some(zv) = zone_kw {
        z = tryr!(parse_zone(zv));
    }
    time_make(time_cls(s), ns, z)
}


// ---------- Random ----------

fn urandom(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if buf.chunks_mut(256).all(|c| unsafe { libc::getentropy(c.as_mut_ptr() as *mut libc::c_void, c.len()) } == 0) {
        return buf;
    }
    for b in buf.iter_mut() {
        *b = crate::kernel::rand_u64() as u8;
    }
    buf
}
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn rng_set_state(r: V, st: u64) {
    ivar_set(r, sym("@__s0"), mkfix((st >> 32) as i64));
    ivar_set(r, sym("@__s1"), mkfix((st & 0xffff_ffff) as i64));
}
/// 64 random bits from a Random instance, the Random class, or any object with `bytes(n)`.
fn rng_u64(r: V) -> Result<u64, V> {
    let s0 = ivar_get(r, sym("@__s0"));
    if is_fix(s0) {
        let mut st = ((fix(s0) as u64) << 32) | fix(ivar_get(r, sym("@__s1"))) as u64;
        let x = splitmix(&mut st);
        rng_set_state(r, st);
        return Ok(x);
    }
    if is_class(r) {
        return Ok(crate::kernel::rand_u64());
    }
    let b = send(r, sym("bytes"), &[mkfix(8)], QNIL);
    if b == UNDEF {
        return Err(UNDEF);
    }
    if !is_str(b) {
        return Err(no_conv(b, "String"));
    }
    let mut a = [0u8; 8];
    for (i, x) in rstr(b).b.iter().take(8).enumerate() {
        a[i] = *x;
    }
    Ok(u64::from_le_bytes(a))
}
fn rng_float(r: V) -> Result<f64, V> {
    Ok((rng_u64(r)? >> 11) as f64 / (1u64 << 53) as f64)
}
fn rng_below(r: V, n: &BigInt) -> Result<BigInt, V> {
    let bits = n.bits() + 64;
    let mut acc = BigInt::zero();
    let mut have = 0;
    while have < bits {
        acc = (acc << 64) + BigInt::from(rng_u64(r)?);
        have += 64;
    }
    Ok(acc % n)
}
/// Random#rand / Random::Formatter#random_number. `strict` raises on invalid limits.
fn rng_number(r: V, a: &[V], strict: bool) -> V {
    let arg = a.first().copied().unwrap_or(QNIL);
    let invalid = || arg_error(&format!("invalid argument - {}", inspect_lossy(arg)));
    if arg == QNIL || (is_fix(arg) && fix(arg) == 0 && !strict) {
        return float_new(tryr!(rng_float(r)));
    }
    if is_int(arg) {
        let n = if is_fix(arg) { BigInt::from(fix(arg)) } else { rbig(arg).n.clone() };
        if n <= BigInt::zero() {
            return if strict { invalid() } else { float_new(tryr!(rng_float(r))) };
        }
        return int_from_big(tryr!(rng_below(r, &n)));
    }
    if is_float(arg) {
        let f = float_val(arg);
        if !(f > 0.0) || !f.is_finite() {
            return if strict { invalid() } else { float_new(tryr!(rng_float(r))) };
        }
        return float_new(tryr!(rng_float(r)) * f);
    }
    if is_range(arg) {
        let rg = rrange(arg);
        let (lo, hi, excl) = (rg.lo, rg.hi, rg.excl);
        if is_int(lo) && is_int(hi) {
            let l = if is_fix(lo) { BigInt::from(fix(lo)) } else { rbig(lo).n.clone() };
            let h = if is_fix(hi) { BigInt::from(fix(hi)) } else { rbig(hi).n.clone() };
            let span = &h - &l + if excl { 0 } else { 1 };
            if span <= BigInt::zero() {
                return if strict { invalid() } else { QNIL };
            }
            return int_from_big(l + tryr!(rng_below(r, &span)));
        }
        if let (Some(l), Some(h)) = (num_to_f(lo), num_to_f(hi)) {
            if h < l || (excl && h == l) {
                return if strict { invalid() } else { QNIL };
            }
            return float_new(l + tryr!(rng_float(r)) * (h - l));
        }
    }
    if strict { invalid() } else { float_new(tryr!(rng_float(r))) }
}

static THREAD_REPORT: G<V> = G::new(QTRUE);
static THREAD_ABORT: G<V> = G::new(QFALSE);

/// String encodings: UTF-8 unless flagged binary (ASCII-8BIT), plus the Encoding class API.
fn init_encoding(st: *mut Class, enc: *mut Class) {
    defn(st, "b", 0, 0, |s, _, _| crate::pack::bin_str(rstr(s).b.clone()));
    // FL_BINARY marks ASCII-8BIT strings, the next bit US-ASCII ones; others are UTF-8
    const FL_ASCII: u32 = FL_BINARY << 1;
    defn(st, "encoding", 0, 0, |s, _, _| {
        let f = hdr(s).flags;
        if f & FL_BINARY != 0 {
            let e = crate::transcode::enc_of(s);
            if e.codec != crate::transcode::Codec::Binary {
                return crate::transcode::encoding_obj(e);
            }
        }
        encoding(if f & FL_BINARY != 0 { "BINARY" } else if f & FL_ASCII != 0 { "US-ASCII" } else { "UTF-8" })
    });
    defn(st, "force_encoding", 1, 1, |s, a, _| {
        let Some(n) = enc_name_of(a[0]) else { return no_conv(a[0], "String") };
        let Some(e) = crate::transcode::lookup(&n) else { return arg_error(&format!("unknown encoding name - {n}")) };
        // (applies to frozen Strings too, as it always has in Kenny; a chilled literal stops
        // being one, as in CRuby)
        unchill(s);
        crate::transcode::set_enc(s, e);
        s
    });
    defn(st, "valid_encoding?", 0, 0, |s, _, _| {
        if is_bin(s) {
            return bool_v(crate::transcode::valid_foreign(s).unwrap_or(true));
        }
        if hdr(s).flags & FL_ASCII != 0 {
            return bool_v(rstr(s).b.is_ascii());
        }
        bool_v(std::str::from_utf8(&rstr(s).b).is_ok())
    });
    let us = object_new(enc);
    ivar_set(us, sym("@__name"), str_from("US-ASCII"));
    const_set(enc, sym("US_ASCII"), us);
    const_set(enc, sym("ASCII"), us);
    const_set(enc, sym("ANSI_X3_4_1968"), us);
    const_set(enc, sym("UTF8"), encoding("UTF-8"));
    const_set(enc, sym("CP65001"), encoding("UTF-8"));
    // the other encodings: placeholders no string carries, for comparisons and lookups
    let mut list = vec![encoding("BINARY"), encoding("UTF-8"), us];
    for (name, _, consts, dummy, ascii) in crate::enc_table::ENCODINGS {
        let o = object_new(enc);
        ivar_set(o, sym("@__name"), str_from(name));
        if *dummy {
            ivar_set(o, sym("@__dummy"), QTRUE);
        }
        if !*ascii {
            ivar_set(o, sym("@__non_ascii"), QTRUE);
        }
        for c in *consts {
            const_set(enc, sym(c), o);
        }
        list.push(o);
    }
    let list = ary_new(list);
    hdr(list).flags |= FL_FROZEN;
    rt().gvars.insert(sym("$__kenny_encodings"), list);
    defn(enc, "ascii_compatible?", 0, 0, |s, _, _| bool_v(!ivar_defined(s, sym("@__non_ascii"))));
    defn(enc, "==", 1, 1, |s, a, _| bool_v(s == a[0]));
    defn(enc, "dummy?", 0, 0, |s, _, _| bool_v(ivar_defined(s, sym("@__dummy"))));
    fn aliases_of(name: &str) -> &'static [&'static str] {
        match name {
            "ASCII-8BIT" => &["BINARY"],
            "UTF-8" => &["CP65001"],
            "US-ASCII" => &["ASCII", "ANSI_X3.4-1968", "646"],
            _ => crate::enc_table::ENCODINGS.iter().find(|e| e.0 == name).map_or(&[], |e| e.1),
        }
    }
    defn(enc, "names", 0, 0, |s, _, _| {
        let n = ivar_get(s, sym("@__name"));
        let mut v = vec![n];
        v.extend(aliases_of(&s_of(n)).iter().map(|a| str_from(a)));
        ary_new(v)
    });
    defs(enc, "default_external", 0, 0, |_, _, _| encoding("UTF-8"));
    defs(enc, "default_internal", 0, 0, |_, _, _| QNIL);
    defs(enc, "default_external=", 1, 1, |_, a, _| a[0]);
    defs(enc, "default_internal=", 1, 1, |_, a, _| a[0]);
    defs(enc, "find", 1, 1, |_, a, _| {
        if is_heap(a[0]) && real_class(a[0]) == const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class {
            return a[0];
        }
        let Some(n) = enc_name_of(a[0]) else { return no_conv(a[0], "String") };
        match n.as_str() {
            "UTF-8" | "UTF8" | "CP65001" | "ASCII-8BIT" | "BINARY" | "US-ASCII" | "ASCII" | "ANSI_X3.4-1968" | "646" | "LOCALE" | "EXTERNAL" | "FILESYSTEM" => encoding(&n),
            "INTERNAL" => QNIL,
            _ => {
                let hit = crate::enc_table::ENCODINGS
                    .iter()
                    .find(|(name, aliases, ..)| name.eq_ignore_ascii_case(&n) || aliases.iter().any(|x| x.eq_ignore_ascii_case(&n)));
                match hit {
                    Some((_, _, consts, ..)) => {
                        let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class;
                        const_get_under(enc, sym(consts[0])).unwrap_or(QNIL)
                    }
                    None => arg_error(&format!("unknown encoding name - {}", s_of(a[0]))),
                }
            }
        }
    });
    defs(enc, "list", 0, 0, |_, _, _| {
        let l = rt().gvars.get(&sym("$__kenny_encodings")).copied().unwrap_or(QNIL);
        ary_new(rary(l).v.to_vec())
    });
    defs(enc, "name_list", 0, 0, |_, _, _| {
        let mut v: Vec<V> = ["ASCII-8BIT", "UTF-8", "US-ASCII", "BINARY", "ASCII", "CP65001"].iter().map(|n| str_from(n)).collect();
        for (name, aliases, ..) in crate::enc_table::ENCODINGS {
            v.push(str_from(name));
            v.extend(aliases.iter().map(|a| str_from(a)));
        }
        ary_new(v)
    });
    defs(enc, "aliases", 0, 0, |_, _, _| {
        let h = hash_new();
        for (n, a) in [("BINARY", "ASCII-8BIT"), ("CP65001", "UTF-8"), ("ASCII", "US-ASCII"), ("ANSI_X3.4-1968", "US-ASCII"), ("646", "US-ASCII")] {
            crate::hash::hset(h, str_from(n), str_from(a));
        }
        for (name, aliases, ..) in crate::enc_table::ENCODINGS {
            for a in *aliases {
                crate::hash::hset(h, str_from(a), str_from(name));
            }
        }
        h
    });
    defs(enc, "compatible?", 2, 2, |_, _, _| encoding("UTF-8"));
}

pub fn init() {
    let c = rt().c;
    // Upstream may provide Rational (rational.rs); then leave its to_r/quo/numerator alone.
    let have_rational = const_lookup(c.object, sym("Rational")).is_some();

    // ===== Kernel =====
    let k = c.kernel;
    defn(k, "<=>", 1, 1, |s, a, _| {
        if s == a[0] {
            return mkfix(0);
        }
        let r = tri!(equal(s, a[0]));
        if truthy(r) { mkfix(0) } else { QNIL }
    });
    defn(k, "Hash", 1, 1, |_, a, _| {
        let v = a[0];
        if v == QNIL || (is_ary(v) && rary(v).v.is_empty()) {
            return hash_new();
        }
        if is_hash(v) {
            return v;
        }
        if respond_to(v, sym("to_hash")) {
            return send0(v, "to_hash");
        }
        type_error(&format!("can't convert {} into Hash", cname(v)))
    });
    defn(k, "global_variables", 0, 0, |_, _, _| {
        let mut names: Vec<u32> = rt().gvars.keys().copied().collect();
        for n in ["$stdout", "$stderr", "$stdin", "$0", "$!", "$~", "$_"] {
            let id = sym(n);
            if !names.contains(&id) {
                names.push(id);
            }
        }
        names.sort_by_key(|id| sym_str(*id));
        ary_new(names.into_iter().map(mksym).collect())
    });
    defn(k, "local_variables", 0, 0, |_, _, _| ary_new(vec![]));
    defn(k, "public_method", 1, 1, |s, a, _| send(s, sym("method"), a, QNIL));
    defn(k, "public_methods", 0, 1, |s, a, _| send(s, sym("methods"), a, QNIL));
    defn(k, "private_methods", 0, 1, |_, _, _| ary_new(vec![]));
    defn(k, "protected_methods", 0, 1, |_, _, _| ary_new(vec![]));
    defn(k, "singleton_method", 1, 1, |s, a, _| {
        let id = tryr!(sym_arg(a[0]));
        let sc = class_of(s);
        let has = unsafe { (*sc).is_singleton && (*sc).methods.get(&id).is_some_and(|m| !matches!((**m).body, Body::Undefined)) };
        if !has {
            return raise(rt().c.name_error, &format!("undefined singleton method '{}' for {}", sym_str(id), inspect_lossy(s)));
        }
        send(s, sym("method"), &[mksym(id)], QNIL)
    });
    defn(k, "remove_instance_variable", 1, 1, |s, a, _| {
        let id = tryr!(sym_arg(a[0]));
        if !ivar_defined(s, id) || ivar_get(s, id) == UNDEF {
            return raise(rt().c.name_error, &format!("instance variable {} not defined", sym_str(id)));
        }
        tri!(check_frozen(s));
        let old = ivar_get(s, id);
        match kind(s) {
            Some(Kind::Object) => {
                let o = obj(s);
                let shape = unsafe { &*o.shape };
                if let Some(&i) = shape.ivar_index.get(&id) {
                    if (i as usize) < o.ivars.len() {
                        o.ivars[i as usize] = UNDEF;
                    }
                }
            }
            Some(Kind::Class) => {
                rclass(s).civars.remove(&id);
            }
            _ => {
                gen_ivar_remove(s, id);
            }
        }
        old
    });
    defn(k, "gem", 1, -1, |_, _, _| QTRUE);
    // the builtin require, as RubyGems keeps it before overriding `require` (an override
    // calls it: a re-dispatch to `require` would loop)
    crate::abi::alias(k, sym("gem_original_require"), sym("require"));
    defn(k, "initialize_copy", 1, 1, |s, _, _| s);
    defn(k, "initialize_dup", 1, 1, |s, a, _| send(s, sym("initialize_copy"), &[a[0]], QNIL));
    defn(k, "initialize_clone", 1, 2, |s, a, _| send(s, sym("initialize_copy"), &[a[0]], QNIL));

    // BasicObject gets __send__, instance_eval and instance_exec too.
    let bo = c.basic_object;
    for name in ["__send__", "instance_eval", "instance_exec"] {
        copy_method(k, bo, name);
    }
    defn(bo, "singleton_method_removed", 1, 1, |_, _, _| QNIL);
    defn(bo, "singleton_method_undefined", 1, 1, |_, _, _| QNIL);

    // The top-level object: include/public/private/define_method act on Object.
    let main_sc = singleton_class(rt().main_obj);
    defn(main_sc, "include", 1, -1, |_, a, _| send(rt().c.object as V, sym("include"), a, QNIL));
    defn(main_sc, "define_method", 1, 2, |_, a, b| send(rt().c.object as V, sym("define_method"), a, b));
    for name in ["public", "private"] {
        defn(main_sc, name, 0, -1, |_, a, _| match a.len() {
            0 => QNIL,
            1 => a[0],
            _ => ary_new(a.to_vec()),
        });
    }

    // ===== Math =====
    let m = const_lookup(c.object, sym("Math")).unwrap() as *mut Class;
    macro_rules! math1 {
        ($name:expr, $body:expr) => {
            defs(m, $name, 1, 1, |_, a, _| {
                let x = tryr!(float_arg(a[0]));
                let f: fn(f64) -> Result<f64, V> = $body;
                match f(x) {
                    Ok(r) => float_new(r),
                    Err(e) => e,
                }
            });
        };
    }
    math1!("acosh", |x| if x < 1.0 { Err(math_domain("acosh")) } else { Ok(x.acosh()) });
    math1!("asinh", |x| Ok(x.asinh()));
    math1!("atanh", |x| {
        if !(-1.0..=1.0).contains(&x) {
            Err(math_domain("atanh"))
        } else if x == 1.0 {
            Ok(f64::INFINITY)
        } else if x == -1.0 {
            Ok(f64::NEG_INFINITY)
        } else {
            Ok(x.atanh())
        }
    });
    math1!("erf", |x| Ok(unsafe { c_erf(x) }));
    math1!("erfc", |x| Ok(unsafe { c_erfc(x) }));
    math1!("gamma", |x| {
        if x == f64::NEG_INFINITY || (x < 0.0 && x.fract() == 0.0) {
            return Err(math_domain("gamma"));
        }
        if x.fract() == 0.0 && (1.0..=23.0).contains(&x) {
            let mut r = 1.0f64;
            for i in 2..(x as u64) {
                r *= i as f64;
            }
            return Ok(r);
        }
        Ok(unsafe { c_tgamma(x) })
    });
    defs(m, "lgamma", 1, 1, |_, a, _| {
        let x = tryr!(float_arg(a[0]));
        if x == f64::NEG_INFINITY {
            return math_domain("lgamma");
        }
        let mut sign = 1i32;
        let r = unsafe { c_lgamma_r(x, &mut sign) };
        let sign = if x == 0.0 && x.is_sign_negative() { -1 } else if sign < 0 { -1 } else { 1 };
        ary_new(vec![float_new(r), mkfix(sign)])
    });
    defs(m, "frexp", 1, 1, |_, a, _| {
        let x = tryr!(float_arg(a[0]));
        let mut e = 0i32;
        let f = unsafe { c_frexp(x, &mut e) };
        ary_new(vec![float_new(f), mkfix(e as i64)])
    });
    defs(m, "ldexp", 2, 2, |_, a, _| {
        let x = tryr!(float_arg(a[0]));
        let e = tryr!(want_int(a[1]));
        float_new(unsafe { c_ldexp(x, e.clamp(i32::MIN as i64, i32::MAX as i64) as i32) })
    });
    // Math functions are module functions: callable as Math.sqrt and, after `include Math`, as sqrt.
    let msc = singleton_class(m as V);
    let names: Vec<u32> = unsafe { (*msc).methods.keys().copied().collect() };
    for id in names {
        let Some(&meth) = (unsafe { &(*msc).methods }).get(&id) else { continue };
        add_method(m, id, clone_body(unsafe { &(*meth).body }));
    }

    // ===== Array =====
    let ar = c.array;
    defn(ar, "bsearch_index", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "bsearch_index", &[]);
        }
        let len = rary(s).v.len() as i64;
        match bsearch_by(0, len, |i| match rary(s).v.get(i as usize) {
            Some(&x) => call_block(b, &[x]),
            None => QNIL,
        }) {
            Ok(Some(i)) => mkfix(i),
            Ok(None) => QNIL,
            Err(e) => e,
        }
    });
    defn(ar, "rassoc", 1, 1, |s, a, _| {
        let items = RVec::from(rary(s).v.clone());
        for i in 0..items.len() {
            let x = items[i];
            if is_ary(x) && rary(x).v.len() >= 2 && truthy(tri!(equal(rary(x).v[1], a[0]))) {
                return x;
            }
        }
        QNIL
    });
    defn(ar, "repeated_combination", 1, 1, |s, a, b| yield_tuples(s, "repeated_combination", a, b, true));
    defn(ar, "repeated_permutation", 1, 1, |s, a, b| yield_tuples(s, "repeated_permutation", a, b, false));
    defn(ar, "rotate!", 0, 1, |s, a, _| {
        tri!(check_frozen(s));
        let n = match a.first() {
            Some(&x) => tryr!(want_int(x)),
            None => 1,
        };
        let v = &mut rary(s).v;
        if !v.is_empty() {
            let k = n.rem_euclid(v.len() as i64) as usize;
            v.rotate_left(k);
        }
        s
    });
    defs(ar, "try_convert", 1, 1, |_, a, _| {
        let v = a[0];
        if is_ary(v) {
            v
        } else if is_heap(v) && respond_to(v, sym("to_ary")) {
            send0(v, "to_ary")
        } else {
            QNIL
        }
    });
    defn(ar, "grep", 1, 1, |s, a, b| grep_impl(s, a[0], b, false));
    defn(ar, "grep_v", 1, 1, |s, a, b| grep_impl(s, a[0], b, true));
    defn(ar, "chunk", 0, 0, |s, _, b| chunk_impl(s, b));
    defn(ar, "slice_before", 0, 1, |s, a, b| slice_impl(s, a, b, false));
    defn(ar, "slice_after", 0, 1, |s, a, b| slice_impl(s, a, b, true));
    defn(ar, "chain", 0, -1, |s, a, _| chain_impl(s, a));

    // ===== Enumerable / Enumerator =====
    let en = c.enumerable;
    for name in ["grep", "grep_v", "chunk", "slice_before", "slice_after"] {
        enum_delegate(en, name);
    }
    defn(en, "chain", 0, -1, |s, a, _| chain_impl(s, a));
    let er = c.enumerator;
    defn(er, "+", 1, 1, |s, a, _| chain_impl(s, a));
    defn(er, "next_values", 0, 0, |s, _, _| {
        let v = tri!(send0(s, "next"));
        ary_new(vec![v])
    });
    defn(er, "peek_values", 0, 0, |s, _, _| {
        let v = tri!(send0(s, "peek"));
        ary_new(vec![v])
    });
    defn(er, "with_object", 1, 1, |s, a, b| {
        if !is_proc(b) {
            return enum_for(s, "with_object", a);
        }
        send(s, sym("each_with_object"), a, b)
    });
    // produce(initial = none, size: Float::INFINITY) { |prev| } (size: since Ruby 4.0)
    defs(er, "produce", 0, -1, |s, a, b| {
        let mut a = a.to_vec();
        let mut size = float_new(f64::INFINITY);
        if crate::misc::ruby4() {
            if let Some(&h) = a.last() {
                if is_hash(h) && hdr(h).flags & FL_KWHASH != 0 {
                    a.pop();
                    for (k, v) in rhash(h).m.iter() {
                        if k.0 != mksym(sym("size")) {
                            return arg_error(&format!("unknown keyword: {}", String::from_utf8_lossy(&rstr(tri!(send(k.0, sym("inspect"), &[], QNIL))).b)));
                        }
                        let v = *v;
                        let ok = v == QNIL || is_int(v) || respond_to(v, sym("call")) || (is_float(v) && float_val(v).is_infinite());
                        if !ok {
                            tri!(send(v, sym("to_int"), &[], QNIL));
                        }
                        size = v;
                    }
                }
            }
        }
        if a.len() > 1 {
            return arg_error(&format!("wrong number of arguments (given {}, expected 0..1)", a.len()));
        }
        if !is_proc(b) {
            return arg_error("no block given");
        }
        let init = a.first().copied();
        let roots = vec![b, init.unwrap_or(QNIL)];
        let generator = proc_new_native_rooted(
            Box::new(move |_, args, _| {
                let y = args[0];
                let mut cur = match init {
                    Some(v) => v,
                    None => call_block(b, &[QNIL]),
                };
                loop {
                    if cur == UNDEF {
                        if let Throw::Raise(e) = pst().pending {
                            if is_a(e, rt().c.stop_iteration) {
                                pst().pending = Throw::None;
                                return QNIL;
                            }
                        }
                        return UNDEF;
                    }
                    tri!(send(y, sym("yield"), &[cur], QNIL));
                    cur = call_block(b, &[cur]);
                }
            }),
            1,
            1,
            roots,
        );
        let e = object_new(s as *mut Class);
        ivar_set(e, sym("@__gen"), generator);
        ivar_set(e, sym("@__size"), size);
        e
    });

    // ===== Numeric / Integer / Float =====
    let nu = c.numeric;
    defn(nu, "abs2", 0, 0, |s, _, _| send(s, sym("*"), &[s], QNIL));
    for name in ["conj", "conjugate", "real"] {
        defn(nu, name, 0, 0, |s, _, _| s);
    }
    for name in ["imag", "imaginary"] {
        defn(nu, name, 0, 0, |_, _, _| mkfix(0));
    }
    defn(nu, "real?", 0, 0, |_, _, _| QTRUE);
    fn arg_of(s: V) -> V {
        if is_float(s) {
            let f = float_val(s);
            if f.is_nan() {
                return s;
            }
            return if f.is_sign_negative() { float_new(std::f64::consts::PI) } else { mkfix(0) };
        }
        let neg = tri!(send1(s, "<", mkfix(0)));
        if truthy(neg) { float_new(std::f64::consts::PI) } else { mkfix(0) }
    }
    for name in ["angle", "arg", "phase"] {
        defn(nu, name, 0, 0, |s, _, _| arg_of(s));
    }
    defn(nu, "polar", 0, 0, |s, _, _| {
        let a = tri!(send0(s, "abs"));
        let g = tri!(arg_of(s));
        ary_new(vec![a, g])
    });
    for name in ["rect", "rectangular"] {
        defn(nu, name, 0, 0, |s, _, _| ary_new(vec![s, mkfix(0)]));
    }
    if !have_rational {
        // without Rational, quo divides as Float
        defn(nu, "quo", 1, 1, |s, a, _| send(s, sym("fdiv"), a, QNIL));
    }
    let int = c.integer;
    defn(int, "allbits?", 1, 1, |s, a, _| {
        let r = tri!(send1(s, "&", a[0]));
        equal(r, a[0])
    });
    defn(int, "anybits?", 1, 1, |s, a, _| {
        let r = tri!(send1(s, "&", a[0]));
        bool_v(!(is_fix(r) && fix(r) == 0))
    });
    defn(int, "nobits?", 1, 1, |s, a, _| {
        let r = tri!(send1(s, "&", a[0]));
        bool_v(is_fix(r) && fix(r) == 0)
    });
    defn(int, "gcdlcm", 1, 1, |s, a, _| {
        let g = tri!(send1(s, "gcd", a[0]));
        let l = tri!(send1(s, "lcm", a[0]));
        ary_new(vec![g, l])
    });
    if !have_rational {
        defn(int, "numerator", 0, 0, |s, _, _| s);
        defn(int, "denominator", 0, 0, |_, _, _| mkfix(1));
        defn(int, "rationalize", 0, 1, |s, _, _| s);
    }
    let fl = c.float;
    defn(fl, "prev_float", 0, 0, |s, _, _| {
        let f = float_val(s);
        let r = if f.is_nan() || f == f64::NEG_INFINITY {
            f
        } else if f == 0.0 {
            -f64::from_bits(1)
        } else if f > 0.0 {
            f64::from_bits(f.to_bits() - 1)
        } else {
            f64::from_bits(f.to_bits() + 1)
        };
        float_new(r)
    });
    if !have_rational {
        defn(fl, "quo", 1, 1, |s, a, _| match num_to_f(a[0]) {
            Some(d) => float_new(float_val(s) / d),
            None => send(s, sym("/"), a, QNIL),
        });
    }
    fn float_ratio(f: f64) -> (BigInt, BigInt) {
        if f == 0.0 {
            return (BigInt::zero(), BigInt::from(1));
        }
        let bits = f.to_bits();
        let neg = bits >> 63 == 1;
        let exp = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        let (mut mant, mut e) = if exp == 0 { (frac, -1074i64) } else { (frac | (1u64 << 52), exp - 1075) };
        while mant & 1 == 0 && e < 0 {
            mant >>= 1;
            e += 1;
        }
        let (num, den) = if e >= 0 { (BigInt::from(mant) << e as usize, BigInt::from(1)) } else { (BigInt::from(mant), BigInt::from(1) << (-e) as usize) };
        (if neg { -num } else { num }, den)
    }
    defn(fl, "numerator", 0, 0, |s, _, _| {
        let f = float_val(s);
        if !f.is_finite() {
            return s;
        }
        int_from_big(float_ratio(f).0)
    });
    defn(fl, "denominator", 0, 0, |s, _, _| {
        let f = float_val(s);
        if !f.is_finite() {
            return mkfix(1);
        }
        int_from_big(float_ratio(f).1)
    });
    if !have_rational {
        for name in ["to_r", "rationalize"] {
            defn(fl, name, 0, 1, |_, _, _| raise(rt().c.not_implemented_error, "Rational is not supported by Kenny"));
        }
    }

    // ===== Hash =====
    let hs = c.hash;
    fn subset(a: V, b: V, strict: bool) -> V {
        if !is_hash(b) {
            return no_conv(b, "Hash");
        }
        let (la, lb) = (rhash(a).m.len(), rhash(b).m.len());
        if la > lb || (strict && la == lb) {
            return QFALSE;
        }
        for (k, v) in crate::hash::pairs(a) {
            match rhash(b).m.get(&HKey(k)).copied() {
                Some(w) => {
                    if !truthy(tri!(equal(v, w))) {
                        return QFALSE;
                    }
                }
                None => return QFALSE,
            }
        }
        QTRUE
    }
    defn(hs, "<=", 1, 1, |s, a, _| subset(s, a[0], false));
    defn(hs, "<", 1, 1, |s, a, _| subset(s, a[0], true));
    defn(hs, ">=", 1, 1, |s, a, _| if is_hash(a[0]) { subset(a[0], s, false) } else { no_conv(a[0], "Hash") });
    defn(hs, ">", 1, 1, |s, a, _| if is_hash(a[0]) { subset(a[0], s, true) } else { no_conv(a[0], "Hash") });
    defn(hs, "compact!", 0, 0, |s, _, _| {
        tri!(check_frozen(s));
        let before = rhash(s).m.len();
        rhash(s).m.retain(|_, v| *v != QNIL);
        if rhash(s).m.len() == before { QNIL } else { s }
    });
    defn(hs, "compare_by_identity", 0, 0, |s, _, _| {
        if !rhash(s).m.is_ident() {
            tri!(check_frozen(s));
            rhash(s).m.set_ident();
        }
        s
    });
    defn(hs, "compare_by_identity?", 0, 0, |s, _, _| bool_v(rhash(s).m.is_ident()));
    defn(hs, "default_proc=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        let p = a[0];
        if p != QNIL && !is_proc(p) {
            return type_error(&format!("wrong default_proc type {} (expected Proc)", cname(p)));
        }
        if is_proc(p) && rproc(p).lambda && rproc(p).max >= 0 && rproc(p).min != 2 {
            return type_error(&format!("default_proc takes two arguments (2 for {})", rproc(p).min));
        }
        crate::gc::wb_obj(s);
        rhash(s).default_proc = p;
        rhash(s).default = QNIL;
        p
    });
    defn(hs, "flatten", 0, 1, |s, a, _| {
        let depth = match a.first() {
            Some(&d) => tryr!(want_int(d)),
            None => 1,
        };
        let mut out = RVec::new();
        for (k, v) in crate::hash::pairs(s) {
            out.push(k);
            out.push(v);
        }
        let arr = ary_new(out.take());
        if depth - 1 > 0 {
            return send(arr, sym("flatten"), &[mkfix(depth - 1)], QNIL);
        }
        if depth < 0 {
            return send0(arr, "flatten");
        }
        arr
    });
    defn(hs, "index", 1, 1, |s, a, _| send(s, sym("key"), a, QNIL));
    defn(hs, "rassoc", 1, 1, |s, a, _| {
        for (k, v) in crate::hash::pairs(s) {
            if truthy(tri!(equal(v, a[0]))) {
                return ary_new(vec![k, v]);
            }
        }
        QNIL
    });
    defn(hs, "rehash", 0, 0, |s, _, _| {
        tri!(check_frozen(s));
        let ps = crate::hash::pairs(s);
        let h = rhash(s);
        h.m.clear();
        for (k, v) in ps {
            h.m.insert(HKey(k), v);
        }
        s
    });
    defn(hs, "to_proc", 0, 0, |s, _, _| {
        let p = proc_new_native_rooted(Box::new(move |_, a, _| if a.len() == 1 { crate::hash::hget(s, a[0]) } else { arity_error(a.len() as i32, 1, 1) }), 1, 1, vec![s]);
        rproc(p).lambda = true;
        p
    });
    defs(hs, "try_convert", 1, 1, |_, a, _| {
        let v = a[0];
        if is_hash(v) {
            v
        } else if is_heap(v) && respond_to(v, sym("to_hash")) {
            send0(v, "to_hash")
        } else {
            QNIL
        }
    });
    defs(hs, "ruby2_keywords_hash?", 1, 1, |_, a, _| if is_hash(a[0]) { bool_v(hdr(a[0]).flags & FL_KWHASH != 0) } else { no_conv(a[0], "Hash") });
    defs(hs, "ruby2_keywords_hash", 1, 1, |_, a, _| {
        if !is_hash(a[0]) {
            return no_conv(a[0], "Hash");
        }
        let h = crate::kernel::dup(a[0], false);
        hdr(h).flags |= FL_KWHASH;
        h
    });

    // ===== Module =====
    let md = c.module;
    defn(md, "<=>", 1, 1, |s, a, _| {
        let o = a[0];
        if s == o {
            return mkfix(0);
        }
        if !is_class(o) {
            return QNIL;
        }
        if ancestors(s as *mut Class).contains(&(o as *mut Class)) {
            mkfix(-1)
        } else if ancestors(o as *mut Class).contains(&(s as *mut Class)) {
            mkfix(1)
        } else {
            QNIL
        }
    });
    defn(md, "class_variables", 0, 1, |s, a, _| {
        let inherit = a.first().is_none_or(|x| truthy(*x));
        let classes = if inherit { ancestors(s as *mut Class) } else { vec![s as *mut Class] };
        let mut ids: Vec<u32> = Vec::new();
        for c in classes {
            let mut own: Vec<u32> = unsafe { (*c).cvars.keys().copied().collect() };
            own.sort_by_key(|id| sym_str(*id));
            for id in own {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ary_new(ids.into_iter().map(mksym).collect())
    });
    defn(md, "remove_class_variable", 1, 1, |s, a, _| {
        let id = tryr!(sym_arg(a[0]));
        match rclass(s).cvars.remove(&id) {
            Some(v) => {
                bump_serial();
                v
            }
            None => raise(rt().c.name_error, &format!("class variable {} not defined for {}", sym_str(id), class_name(s as *mut Class))),
        }
    });
    defn(md, "remove_const", 1, 1, |s, a, _| {
        let id = tryr!(sym_arg(a[0]));
        bump_const_serial();
        crate::abi::cfold_invalidate(id);
        match rclass(s).consts.remove(&id) {
            Some(v) => {
                bump_serial();
                v
            }
            // (a pending autoload: dropped, its file not loaded)
            None if crate::abi::remove_autoload(s as *mut Class, id) => {
                bump_serial();
                QNIL
            }
            None => {
                let prefix = if s as *mut Class == rt().c.object { String::new() } else { format!("{}::", class_name(s as *mut Class)) };
                raise(rt().c.name_error, &format!("constant {}{} not defined", prefix, sym_str(id)))
            }
        }
    });
    defn(md, "const_source_location", 1, 2, |_, _, _| QNIL);
    defn(md, "deprecate_constant", 0, -1, |s, _, _| s);
    defn(md, "protected_method_defined?", 1, 2, |_, _, _| QFALSE);
    defn(md, "public_instance_method", 1, 1, |s, a, _| send(s, sym("instance_method"), a, QNIL));
    defn(md, "ruby2_keywords", 0, -1, |_, _, _| QNIL);
    defn(md, "singleton_class?", 0, 0, |s, _, _| bool_v(rclass(s).is_singleton));
    for name in ["method_removed", "method_undefined"] {
        defn(md, name, 1, 1, |_, _, _| QNIL);
    }
    defs(md, "nesting", 0, 0, |_, _, _| ary_new(vec![]));
    defs(md, "used_modules", 0, 0, |_, _, _| ary_new(vec![]));
    defs(md, "constants", 0, 1, |s, a, _| {
        if s as *mut Class != rt().c.module {
            return send(s, sym("constants"), a, QNIL);
        }
        let mut ids: Vec<u32> = unsafe { (*rt().c.object).consts.keys().copied().collect() };
        ids.sort_by_key(|id| sym_str(*id));
        ary_new(ids.into_iter().map(mksym).collect())
    });

    // ===== nil / true / false =====
    defn(c.nil, "^", 1, 1, |_, a, _| bool_v(truthy(a[0])));
    defn(c.nil, "=~", 1, 1, |_, _, _| QNIL);
    if !have_rational {
        defn(c.nil, "rationalize", 0, 1, |_, _, _| mkfix(0));
    }
    for cls in [c.nil, c.true_, c.false_] {
        defn(cls, "===", 1, 1, |s, a, _| bool_v(s == a[0]));
    }

    // ===== Proc =====
    let pc = c.proc_;
    defn(pc, "to_s", 0, 0, |s, _, _| send0(s, "inspect"));
    defn(pc, "source_location", 0, 0, |s, _, _| crate::methodobj::proc_location(s));
    defn(pc, "ruby2_keywords", 0, 0, |s, _, _| s);

    // ===== Range =====
    defn(c.range, "bsearch", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "bsearch", &[]);
        }
        let r = rrange(s);
        let (lo, hi, excl) = (r.lo, r.hi, r.excl);
        if (is_float(lo) || is_float(hi)) && num_to_f(lo).is_some() && (hi == QNIL || num_to_f(hi).is_some()) {
            // search over the ordered bit patterns of doubles
            let key = |f: f64| -> i64 { if f.is_sign_negative() { -((-f).to_bits() as i64) } else { f.to_bits() as i64 } };
            let unkey = |k: i64| -> f64 { if k < 0 { -f64::from_bits((-k) as u64) } else { f64::from_bits(k as u64) } };
            let a0 = key(num_to_f(lo).unwrap());
            let b0 = if hi == QNIL { key(f64::INFINITY) } else { key(num_to_f(hi).unwrap()) } + if excl { 0 } else { 1 };
            return match bsearch_by(a0, b0, |k| call_block(b, &[float_new(unkey(k))])) {
                Ok(Some(k)) => float_new(unkey(k)),
                Ok(None) => QNIL,
                Err(e) => e,
            };
        }
        if is_fix(lo) && (is_fix(hi) || hi == QNIL) {
            let a0 = fix(lo);
            let b0 = if hi == QNIL {
                // endless: probe for an upper bound
                let mut step = 1i64;
                loop {
                    let probe = a0.saturating_add(step);
                    let r = tri!(call_block(b, &[mkfix(probe)]));
                    let stop = r == QTRUE || num_to_f(r).is_some_and(|x| x <= 0.0);
                    if stop || probe >= FIX_MAX / 2 {
                        break probe + 1;
                    }
                    step = step.saturating_mul(2);
                }
            } else {
                fix(hi) + if excl { 0 } else { 1 }
            };
            return match bsearch_by(a0, b0, |i| call_block(b, &[mkfix(i)])) {
                Ok(Some(i)) => mkfix(i),
                Ok(None) => QNIL,
                Err(e) => e,
            };
        }
        type_error(&format!("can't do binary search for {}", cname(lo)))
    });

    // ===== Regexp / MatchData =====
    let rc = const_lookup(c.object, sym("Regexp")).unwrap() as *mut Class;
    defn(rc, "encoding", 0, 0, |s, _, _| {
        let src = tri!(send0(s, "source"));
        encoding(if is_str(src) && !rstr(src).b.is_ascii() { "UTF-8" } else { "US-ASCII" })
    });
    defn(rc, "fixed_encoding?", 0, 0, |s, _, _| {
        let src = tri!(send0(s, "source"));
        bool_v(is_str(src) && !rstr(src).b.is_ascii())
    });
    defs(rc, "try_convert", 1, 1, |_, a, _| {
        let v = a[0];
        if crate::regex::is_regexp(v) {
            v
        } else if is_heap(v) && respond_to(v, sym("to_regexp")) {
            send0(v, "to_regexp")
        } else {
            QNIL
        }
    });
    let mdc = const_lookup(c.object, sym("MatchData")).unwrap() as *mut Class;
    fn md_eq(s: V, o: V) -> bool {
        if s == o {
            return true;
        }
        if !is_heap(o) || real_class(o) != real_class(s) {
            return false;
        }
        let (sa, sb) = (ivar_get(s, sym("@__str")), ivar_get(o, sym("@__str")));
        let (ra, rb) = (ivar_get(s, sym("@__re")), ivar_get(o, sym("@__re")));
        let (pa, pb) = (ivar_get(s, sym("@__pos")), ivar_get(o, sym("@__pos")));
        is_str(sa) && is_str(sb) && rstr(sa).b == rstr(sb).b && truthy(equal(ra, rb)) && eql(pa, pb)
    }
    defn(mdc, "==", 1, 1, |s, a, _| bool_v(md_eq(s, a[0])));
    defn(mdc, "eql?", 1, 1, |s, a, _| bool_v(md_eq(s, a[0])));
    defn(mdc, "hash", 0, 0, |s, _, _| {
        let src = tri!(send0(ivar_get(s, sym("@__re")), "source"));
        let key = ary_new(vec![ivar_get(s, sym("@__str")), src, ivar_get(s, sym("@__pos"))]);
        mkfix((hash_of(key) >> 2) as i64)
    });

    // ===== String =====
    let st = c.string;
    defn(st, "byteslice", 1, 2, |s, a, _| {
        let n = rstr(s).b.len() as i64;
        let (start, len) = if a.len() == 2 {
            (tryr!(want_int(a[0])), tryr!(want_int(a[1])))
        } else if is_range(a[0]) {
            match crate::string::range_bounds(a[0], n) {
                Some(x) => x,
                None => return QNIL,
            }
        } else {
            let i = tryr!(want_int(a[0]));
            let i = if i < 0 { i + n } else { i };
            if i < 0 || i >= n {
                return QNIL;
            }
            (i, 1)
        };
        let start = if start < 0 { start + n } else { start };
        if start < 0 || start > n || len < 0 {
            return QNIL;
        }
        let end = (start + len).min(n);
        new_like(s, rstr(s).b[start as usize..end as usize].to_vec())
    });
    defn(st, "chr", 0, 0, |s, _, _| {
        let b = &rstr(s).b;
        let cs = char_starts(b, is_bin(s));
        if cs.len() < 2 {
            return new_like(s, vec![]);
        }
        new_like(s, b[cs[0]..cs[1]].to_vec())
    });
    fn codepoints(s: V) -> Result<Vec<V>, V> {
        let b = &rstr(s).b;
        if is_bin(s) {
            return Ok(b.iter().map(|&x| mkfix(x as i64)).collect());
        }
        match std::str::from_utf8(b) {
            Ok(t) => Ok(t.chars().map(|ch| mkfix(ch as i64)).collect()),
            Err(_) => Err(arg_error("invalid byte sequence in UTF-8")),
        }
    }
    defn(st, "codepoints", 0, 0, |s, _, b| {
        let v = tryr!(codepoints(s));
        if is_proc(b) {
            for x in v {
                tri!(call_block(b, &[x]));
            }
            return s;
        }
        ary_new(v)
    });
    defn(st, "each_codepoint", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "each_codepoint", &[]);
        }
        for x in tryr!(codepoints(s)) {
            tri!(call_block(b, &[x]));
        }
        s
    });
    fn gclusters(s: V) -> RVec {
        RVec::from(graphemes(&s_of(s)).into_iter().map(|g| str_new(g.into_bytes())).collect())
    }
    defn(st, "grapheme_clusters", 0, 0, |s, _, b| {
        let v = gclusters(s);
        if is_proc(b) {
            for i in 0..v.len() {
                tri!(call_block(b, &[v[i]]));
            }
            return s;
        }
        ary_new(v.to_vec())
    });
    defn(st, "each_grapheme_cluster", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "each_grapheme_cluster", &[]);
        }
        let v = gclusters(s);
        for i in 0..v.len() {
            tri!(call_block(b, &[v[i]]));
        }
        s
    });
    defn(st, "delete!", 1, -1, |s, a, b| bang(s, "delete", a, b, false));
    defn(st, "delete_prefix!", 1, 1, |s, a, b| bang(s, "delete_prefix", a, b, false));
    defn(st, "delete_suffix!", 1, 1, |s, a, b| bang(s, "delete_suffix", a, b, false));
    defn(st, "tr!", 2, 2, |s, a, b| bang(s, "tr", a, b, false));
    defn(st, "tr_s!", 2, 2, |s, a, b| bang(s, "tr_s", a, b, false));
    defn(st, "scrub!", 0, 1, |s, a, b| bang(s, "scrub", a, b, true));
    defn(st, "unicode_normalize!", 0, 1, |s, a, b| bang(s, "unicode_normalize", a, b, true));
    defn(st, "succ!", 0, 0, |s, a, b| bang(s, "succ", a, b, true));
    defn(st, "next!", 0, 0, |s, a, b| bang(s, "succ", a, b, true));
    defn(st, "tr_s", 2, 2, |s, a, _| {
        if !is_str(a[0]) {
            return no_conv(a[0], "String");
        }
        if !is_str(a[1]) {
            return no_conv(a[1], "String");
        }
        new_str_v(tr_squeeze(&s_of(s), &s_of(a[0]), &s_of(a[1])))
    });
    defn(st, "setbyte", 2, 2, |s, a, _| {
        let i = tryr!(want_int(a[0]));
        let v = tryr!(want_int(a[1]));
        let b = tryr!(str_mutate(s));
        let n = b.len() as i64;
        let j = if i < 0 { i + n } else { i };
        if j < 0 || j >= n {
            return raise(rt().c.index_error, &format!("index {i} out of string"));
        }
        b[j as usize] = v as u8;
        a[1]
    });
    defn(st, "slice!", 1, 2, |s, a, _| {
        tri!(check_frozen(s));
        let bytes = rstr(s).b.clone();
        let cs = char_starts(&bytes, is_bin(s));
        let nchars = cs.len() as i64 - 1;
        // (start char, char count) to remove, or a byte range for string/regexp patterns
        let (bs, be): (usize, usize) = if crate::regex::is_regexp(a[0]) {
            let m = tri!(send(a[0], sym("match"), &[s], QNIL));
            if m == QNIL {
                return QNIL;
            }
            let g = a.get(1).copied().unwrap_or(mkfix(0));
            let b0 = tri!(send1(m, "begin", g));
            let e0 = tri!(send1(m, "end", g));
            if b0 == QNIL {
                return QNIL;
            }
            (cs[fix(b0) as usize], cs[fix(e0) as usize])
        } else if is_str(a[0]) {
            let pat = rstr(a[0]).b.clone();
            match crate::string::find(&bytes, &pat, 0) {
                Some(i) => (i, i + pat.len()),
                None => return QNIL,
            }
        } else {
            let (start, len) = if a.len() == 2 {
                (tryr!(want_int(a[0])), tryr!(want_int(a[1])))
            } else if is_range(a[0]) {
                match crate::string::range_bounds(a[0], nchars) {
                    Some(x) => x,
                    None => return QNIL,
                }
            } else {
                let i = tryr!(want_int(a[0]));
                let i = if i < 0 { i + nchars } else { i };
                if i < 0 || i >= nchars {
                    return QNIL;
                }
                (i, 1)
            };
            let start = if start < 0 { start + nchars } else { start };
            if start < 0 || start > nchars || len < 0 {
                return QNIL;
            }
            let end = (start + len).min(nchars);
            (cs[start as usize], cs[end as usize])
        };
        let removed = new_like(s, bytes[bs..be].to_vec());
        rstr(s).b.drain(bs..be);
        removed
    });
    // unicode_normalize(form = :nfc): :nfc, :nfd, :nfkc, :nfkd
    fn normalize(s: V, a: &[V]) -> Result<String, V> {
        use unicode_normalization::UnicodeNormalization;
        if hdr(s).flags & crate::compat::FL_BINARY != 0 && !rstr(s).b.is_ascii() {
            let c = const_lookup(rt().c.object, sym("Encoding")).and_then(|e| const_lookup(e as *mut Class, sym("CompatibilityError")));
            let c = c.map(|c| c as *mut Class).unwrap_or(rt().c.argument_error);
            return Err(raise(c, "Unicode Normalization not appropriate for ASCII-8BIT"));
        }
        let form = a.first().filter(|f| is_sym(**f)).map(|f| sym_str(sym_id(*f))).unwrap_or_else(|| "nfc".into());
        let t = String::from_utf8_lossy(&rstr(s).b).into_owned();
        Ok(match form.as_str() {
            "nfc" => t.nfc().collect(),
            "nfd" => t.nfd().collect(),
            "nfkc" => t.nfkc().collect(),
            "nfkd" => t.nfkd().collect(),
            f => return Err(arg_error(&format!("Invalid normalization form {f}."))),
        })
    }
    defn(st, "unicode_normalize", 0, 1, |s, a, _| match normalize(s, a) {
        Ok(t) => new_like(s, t.into_bytes()),
        Err(e) => e,
    });
    defn(st, "unicode_normalized?", 0, 1, |s, a, _| match normalize(s, a) {
        Ok(t) => bool_v(t.as_bytes() == rstr(s).b.as_slice()),
        Err(e) => e,
    });
    defn(st, "undump", 0, 0, |s, _, _| match undump(&rstr(s).b.clone()) {
        Ok(b) => str_new(b),
        Err(e) => e,
    });
    defn(st, "dump", 0, 0, |s, _, _| {
        // like inspect, but non-ASCII characters are escaped as \u
        let b = rstr(s).b.clone();
        let mut out = String::from("\"");
        let ins = str_inspect_enc(&b, is_bin(s));
        for ch in ins[1..ins.len() - 1].chars() {
            if (ch as u32) < 0x80 {
                out.push(ch);
            } else if (ch as u32) < 0x10000 {
                out.push_str(&format!("\\u{:04X}", ch as u32));
            } else {
                out.push_str(&format!("\\u{{{:X}}}", ch as u32));
            }
        }
        out.push('"');
        new_str_v(out)
    });
    defs(st, "try_convert", 1, 1, |_, a, _| {
        let v = a[0];
        if is_str(v) {
            v
        } else if is_heap(v) && respond_to(v, sym("to_str")) {
            send0(v, "to_str")
        } else {
            QNIL
        }
    });
    // Encoding and the String encoding methods (String#encode is in compat.rs)
    let enc = const_lookup(c.object, sym("Encoding")).unwrap() as *mut Class;
    init_encoding(st, enc);

    // ===== Struct =====
    let sc = c.struct_;
    defn(sc, "dig", 1, -1, |s, a, _| {
        let ms = unsafe { (*real_class(s)).struct_members.clone() };
        let v = if is_fix(a[0]) {
            let i = fix(a[0]);
            let i = if i < 0 { i + ms.len() as i64 } else { i };
            if i < 0 || i as usize >= ms.len() {
                QNIL
            } else {
                ivar_get(s, sym(&format!("@{}", sym_str(ms[i as usize]))))
            }
        } else {
            match sym_arg(a[0]) {
                Ok(id) if ms.contains(&id) => ivar_get(s, sym(&format!("@{}", sym_str(id)))),
                Ok(_) => QNIL,
                Err(e) => return e,
            }
        };
        if a.len() == 1 || v == QNIL {
            return v;
        }
        send(v, sym("dig"), &a[1..], QNIL)
    });
    defn(sc, "each_pair", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "each_pair", &[]);
        }
        let ms = unsafe { (*real_class(s)).struct_members.clone() };
        for m in ms {
            let v = ivar_get(s, sym(&format!("@{}", sym_str(m))));
            tri!(call_block(b, &[ary_new(vec![mksym(m), v])]));
        }
        s
    });
    for name in ["select", "filter"] {
        defn(sc, name, 0, 0, |s, _, b| {
            if !is_proc(b) {
                return enum_for(s, "select", &[]);
            }
            let vals = tri!(send0(s, "to_a"));
            send(vals, sym("select"), &[], b)
        });
    }
    for name in ["size", "length"] {
        defn(sc, name, 0, 0, |s, _, _| mkfix(unsafe { (*real_class(s)).struct_members.len() } as i64));
    }
    defn(sc, "values_at", 0, -1, |s, a, _| {
        let vals = tri!(send0(s, "to_a"));
        let n = rary(vals).v.len() as i64;
        for &i in a {
            if is_fix(i) && (fix(i) >= n || fix(i) < -n) {
                return raise(rt().c.index_error, &format!("offset {} too large for struct(size:{})", fix(i), n));
            }
        }
        send(vals, sym("values_at"), a, QNIL)
    });

    // ===== Symbol =====
    let sy = c.symbol;
    defn(sy, "casecmp", 1, 1, |s, a, _| {
        if !is_sym(a[0]) {
            return QNIL;
        }
        let x = String::from_utf8_lossy(sym_name(sym_id(s))).to_ascii_lowercase();
        let y = String::from_utf8_lossy(sym_name(sym_id(a[0]))).to_ascii_lowercase();
        mkfix(x.cmp(&y) as i64)
    });
    defn(sy, "casecmp?", 1, 1, |s, a, _| {
        if !is_sym(a[0]) {
            return QNIL;
        }
        let x = String::from_utf8_lossy(sym_name(sym_id(s))).to_lowercase();
        let y = String::from_utf8_lossy(sym_name(sym_id(a[0]))).to_lowercase();
        bool_v(x == y)
    });
    defn(sy, "encoding", 0, 0, |s, _, _| encoding(if sym_name(sym_id(s)).is_ascii() { "US-ASCII" } else { "UTF-8" }));
    defn(sy, "next", 0, 0, |s, _, _| send0(s, "succ"));
    defn(sy, "slice", 1, 2, |s, a, _| send(s, sym("[]"), a, QNIL));
    defn(sy, "swapcase", 0, 0, |s, _, _| {
        let t: String = String::from_utf8_lossy(sym_name(sym_id(s)))
            .chars()
            .flat_map(|ch| if ch.is_uppercase() { ch.to_lowercase().collect::<Vec<_>>() } else { ch.to_uppercase().collect::<Vec<_>>() })
            .collect();
        mksym(sym(&t))
    });
    defn(sy, "===", 1, 1, |s, a, _| bool_v(s == a[0]));
    defs(sy, "all_symbols", 0, 0, |_, _, _| ary_new((0..sym_count()).map(mksym).collect()));

    // ===== Thread =====
    let th = const_lookup(c.object, sym("Thread")).unwrap() as *mut Class;
    defn(th, "report_on_exception", 0, 0, |s, _, _| {
        if ivar_defined(s, sym("@__report")) {
            truthy(ivar_get(s, sym("@__report"))).then_some(QTRUE).unwrap_or(QFALSE)
        } else {
            *THREAD_REPORT.get()
        }
    });
    defn(th, "abort_on_exception", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__abort")))));
    defn(th, "abort_on_exception=", 1, 1, |s, a, _| ivar_set(s, sym("@__abort"), bool_v(truthy(a[0]))));
    defn(th, "backtrace", 0, -1, |_, _, _| QNIL);
    defn(th, "backtrace_locations", 0, -1, |_, _, _| QNIL);
    defn(th, "fetch", 1, 2, |s, a, b| {
        let k = if is_str(a[0]) { mksym(intern(&rstr(a[0]).b.clone())) } else { a[0] };
        let h = ivar_get(s, sym("@__locals"));
        if is_hash(h) {
            if let Some(&v) = rhash(h).m.get(&HKey(k)) {
                return v;
            }
        }
        if is_proc(b) {
            return call_block(b, &[k]);
        }
        if a.len() == 2 {
            return a[1];
        }
        let e = exc_new(rt().c.key_error, &format!("key not found: {}", inspect_lossy(k)));
        ivar_set(e, sym("@key"), k);
        ivar_set(e, sym("@receiver"), s);
        raise_exc(e)
    });
    defn(th, "thread_variables", 0, 0, |s, _, _| {
        let h = ivar_get(s, sym("@__vars"));
        if is_hash(h) { send0(h, "keys") } else { ary_new(vec![]) }
    });
    defn(th, "pending_interrupt?", 0, 1, |_, _, _| QFALSE);
    // ThreadGroup: a thread is in ThreadGroup::Default until added to another group
    // (the timeout gem moves its watcher thread into Default)
    let tg = new_class("ThreadGroup", rt().c.object);
    set_global_const("ThreadGroup", tg as V);
    const_set(tg, sym("Default"), object_new(tg));
    defn(th, "group", 0, 0, |s, _, _| {
        let g = ivar_get(s, sym("@__group"));
        if g != QNIL {
            return g;
        }
        const_lookup(const_lookup(rt().c.object, sym("ThreadGroup")).unwrap() as *mut Class, sym("Default")).unwrap_or(QNIL)
    });
    defn(tg, "add", 1, 1, |s, a, _| {
        if truthy(ivar_get(s, sym("@__enclosed"))) {
            let te = const_lookup(rt().c.object, sym("ThreadError")).unwrap() as *mut Class;
            return raise(te, "can't move to the enclosed thread group");
        }
        ivar_set(a[0], sym("@__group"), s);
        s
    });
    defn(tg, "list", 0, 0, |s, _, _| {
        let all = tri!(send(class_of(rt().main_obj) as V, sym("const_get"), &[mksym(sym("Thread"))], QNIL));
        let all = tri!(send(all, sym("list"), &[], QNIL));
        let mut out = Vec::new();
        for &t in rary(all).v.clone().iter() {
            if tri!(send(t, sym("group"), &[], QNIL)) == s {
                out.push(t);
            }
        }
        ary_new(out)
    });
    defn(tg, "enclose", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__enclosed"), QTRUE);
        s
    });
    defn(tg, "enclosed?", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__enclosed")))));
    defs(th, "handle_interrupt", 1, 1, |_, _, b| call_block(b, &[]));
    defs(th, "pending_interrupt?", 0, 1, |_, _, _| QFALSE);
    defs(th, "kill", 1, 1, |_, a, _| send0(a[0], "kill"));
    defs(th, "exclusive", 0, 0, |_, _, b| call_block(b, &[]));
    defs(th, "report_on_exception", 0, 0, |_, _, _| *THREAD_REPORT.get());
    defs(th, "report_on_exception=", 1, 1, |_, a, _| {
        *THREAD_REPORT.get() = bool_v(truthy(a[0]));
        a[0]
    });
    defs(th, "abort_on_exception", 0, 0, |_, _, _| *THREAD_ABORT.get());
    defs(th, "abort_on_exception=", 1, 1, |_, a, _| {
        *THREAD_ABORT.get() = bool_v(truthy(a[0]));
        a[0]
    });


    // ===== Random (needed by SecureRandom) =====
    if const_lookup(c.object, sym("Random")).is_none() {
        let rn = new_class("Random", c.object);
        set_global_const("Random", rn as V);
        let fmt = new_module("Random::Formatter");
        const_set(rn, sym("Formatter"), fmt as V);
        unsafe { (*rn).includes.push(fmt) };
        defn(fmt, "random_number", 0, 1, |s, a, _| rng_number(s, a, false));
        defn(fmt, "rand", 0, 1, |s, a, _| rng_number(s, a, false));
        defn(rn, "initialize", 0, 1, |s, a, _| {
            let seed = match a.first() {
                Some(&x) if x != QNIL => x,
                _ => {
                    let b = urandom(16);
                    int_from_big(BigInt::from_bytes_le(num_bigint::Sign::Plus, &b))
                }
            };
            let low = if is_fix(seed) {
                fix(seed) as u64
            } else if is_big(seed) {
                let m = BigInt::from(1u128 << 64);
                let r = ((&rbig(seed).n % &m) + &m) % &m;
                r.to_u64().unwrap_or(0)
            } else {
                return no_conv(seed, "Integer");
            };
            ivar_set(s, sym("@__seed"), seed);
            rng_set_state(s, low);
            QNIL
        });
        defn(rn, "seed", 0, 0, |s, _, _| ivar_get(s, sym("@__seed")));
        defn(rn, "rand", 0, 1, |s, a, _| rng_number(s, a, true));
        defn(rn, "bytes", 1, 1, |s, a, _| {
            let n = tryr!(want_int(a[0])).max(0) as usize;
            let mut out = Vec::with_capacity(n + 8);
            while out.len() < n {
                out.extend_from_slice(&tryr!(rng_u64(s)).to_le_bytes());
            }
            out.truncate(n);
            crate::pack::bin_str(out)
        });
        defn(rn, "==", 1, 1, |s, a, _| {
            let o = a[0];
            bool_v(is_heap(o) && real_class(o) == real_class(s) && eql(ivar_get(s, sym("@__seed")), ivar_get(o, sym("@__seed"))) && ivar_get(s, sym("@__s0")) == ivar_get(o, sym("@__s0")) && ivar_get(s, sym("@__s1")) == ivar_get(o, sym("@__s1")))
        });
        defs(rn, "rand", 0, 1, |s, a, _| rng_number(s, a, true));
        defs(rn, "random_number", 0, 1, |s, a, _| rng_number(s, a, false));
        defs(rn, "bytes", 1, 1, |_, a, _| crate::pack::bin_str(urandom(tryr!(want_int(a[0])).max(0) as usize)));
        for name in ["urandom", "raw_seed"] {
            defs(rn, name, 1, 1, |_, a, _| crate::pack::bin_str(urandom(tryr!(want_int(a[0])).max(0) as usize)));
        }
        defs(rn, "new_seed", 0, 0, |_, _, _| int_from_big(BigInt::from_bytes_le(num_bigint::Sign::Plus, &urandom(16))));
        defs(rn, "srand", 0, 1, |_, a, _| send(rt().main_obj, sym("srand"), a, QNIL));
        defs(rn, "seed", 0, 0, |_, _, _| mkfix(0));
    }

    // ===== GC =====
    if let Some(gc) = const_lookup(c.object, sym("GC")) {
        let gc = gc as *mut Class;
        // GC.compact: a full collection that moves objects out of sparsely used pages (when no
        // native frame is active anywhere); returns {moved:, freed_pages:}, or nil if it couldn't
        defs(gc, "compact", 0, 0, |_, _, _| {
            match crate::gc::compact_now() {
                Some((moved, freed)) => {
                    let h = hash_new();
                    rhash(h).m.insert(HKey(mksym(sym("moved"))), mkfix(moved as i64));
                    rhash(h).m.insert(HKey(mksym(sym("freed_pages"))), mkfix(freed as i64));
                    h
                }
                None => QNIL,
            }
        });
        // (these frames hold no object references: they don't stop the GC from moving objects)
        gc_transparent(singleton_class(gc as V), &["start", "compact"]);
        defs(gc, "stress", 0, 0, |_, _, _| QFALSE);
        defs(gc, "stress=", 1, 1, |_, a, _| a[0]);
        defs(gc, "latest_gc_info", 0, 1, |_, _, _| hash_new());
    }

    // ===== Fiber =====
    let fb = const_lookup(c.object, sym("Fiber")).unwrap() as *mut Class;
    defn(fb, "inspect", 0, 0, |s, _, _| new_str_v(format!("#<Fiber:0x{:016x} ({})>", s, crate::thread::fiber_status(s))));
    defn(fb, "to_s", 0, 0, |s, _, _| send0(s, "inspect"));

    // ===== Time =====
    let tm = c.time;
    defs(tm, "now", 0, 1, |s, a, _| {
        let z = match kwarg(a, "in") {
            Some(v) => tryr!(parse_zone(v)),
            None => Zone::Local,
        };
        time_make(time_cls(s), now_ns(), z)
    });
    defs(tm, "at", 1, 4, |s, a, _| time_at(s, a));
    defs(tm, "new", 0, 8, |s, a, _| {
        let zone_kw = kwarg(a, "in");
        let a = positional(a);
        let cls = time_cls(s);
        if a.is_empty() {
            let z = match zone_kw {
                Some(v) => tryr!(parse_zone(v)),
                None => Zone::Local,
            };
            return time_make(cls, now_ns(), z);
        }
        if a.len() == 1 && is_str(a[0]) {
            return tryr!(parse_time_string(cls, a[0], zone_kw));
        }
        let z = match a.get(6).copied().or(zone_kw) {
            Some(v) => tryr!(parse_zone(v)),
            None => Zone::Local,
        };
        time_civil(s, &a[..a.len().min(6)], z)
    });
    defs(tm, "local", 1, 10, |s, a, _| time_civil(s, a, Zone::Local));
    defs(tm, "mktime", 1, 10, |s, a, _| time_civil(s, a, Zone::Local));
    defs(tm, "utc", 1, 10, |s, a, _| time_civil(s, a, Zone::Utc));
    defs(tm, "gm", 1, 10, |s, a, _| time_civil(s, a, Zone::Utc));
    defn(tm, "initialize_copy", 1, 1, |s, a, _| {
        set_time(s, time_ns(a[0]), zone_of(a[0]));
        s
    });
    defn(tm, "to_i", 0, 0, |s, _, _| int_from_i128(time_ns(s).div_euclid(NS)));
    defn(tm, "tv_sec", 0, 0, |s, _, _| int_from_i128(time_ns(s).div_euclid(NS)));
    defn(tm, "to_f", 0, 0, |s, _, _| {
        let ns = time_ns(s);
        float_new(ns.div_euclid(NS) as f64 + ns.rem_euclid(NS) as f64 / 1e9)
    });
    for name in ["usec", "tv_usec"] {
        defn(tm, name, 0, 0, |s, _, _| mkfix((time_ns(s).rem_euclid(NS) / 1000) as i64));
    }
    for name in ["nsec", "tv_nsec"] {
        defn(tm, name, 0, 0, |s, _, _| mkfix(time_ns(s).rem_euclid(NS) as i64));
    }
    // subsec / to_r are Rationals when Rational exists; otherwise 0 or a Float / NotImplementedError.
    defn(tm, "subsec", 0, 0, |s, _, _| {
        let sub = time_ns(s).rem_euclid(NS);
        if const_lookup(rt().c.object, sym("Rational")).is_some() {
            return send(rt().main_obj, sym("Rational"), &[int_from_i128(sub), int_from_i128(NS)], QNIL);
        }
        if sub == 0 { mkfix(0) } else { float_new(sub as f64 / 1e9) }
    });
    defn(tm, "to_r", 0, 0, |s, _, _| {
        if const_lookup(rt().c.object, sym("Rational")).is_some() {
            return send(rt().main_obj, sym("Rational"), &[int_from_i128(time_ns(s)), int_from_i128(NS)], QNIL);
        }
        raise(rt().c.not_implemented_error, "Rational is not supported by Kenny")
    });
    defn(tm, "+", 1, 1, |s, a, _| {
        let x = a[0];
        if is_time(x) {
            return type_error("time + time?");
        }
        let d: i128 = if is_fix(x) {
            fix(x) as i128 * NS
        } else if let Some(f) = num_to_f(x) {
            (f * 1e9).round() as i128
        } else {
            return type_error(&format!("can't convert {} into an exact number", cname(x)));
        };
        time_make(real_class(s), time_ns(s) + d, zone_of(s))
    });
    defn(tm, "-", 1, 1, |s, a, _| {
        let x = a[0];
        if is_time(x) {
            let d = time_ns(s) - time_ns(x);
            return float_new(d.div_euclid(NS) as f64 + d.rem_euclid(NS) as f64 / 1e9);
        }
        let d: i128 = if is_fix(x) {
            fix(x) as i128 * NS
        } else if let Some(f) = num_to_f(x) {
            (f * 1e9).round() as i128
        } else {
            return type_error(&format!("can't convert {} into an exact number", cname(x)));
        };
        time_make(real_class(s), time_ns(s) - d, zone_of(s))
    });
    defn(tm, "succ", 0, 0, |s, _, _| time_make(real_class(s), time_ns(s) + NS, zone_of(s)));
    defn(tm, "<=>", 1, 1, |s, a, _| {
        if !is_time(a[0]) {
            return QNIL;
        }
        mkfix(time_ns(s).cmp(&time_ns(a[0])) as i64)
    });
    defn(tm, "==", 1, 1, |s, a, _| bool_v(is_time(a[0]) && time_ns(s) == time_ns(a[0])));
    defn(tm, "eql?", 1, 1, |s, a, _| bool_v(is_time(a[0]) && time_ns(s) == time_ns(a[0])));
    defn(tm, "hash", 0, 0, |s, _, _| {
        let ns = time_ns(s);
        mkfix((hash_of(int_from_i128(ns)) >> 2) as i64 ^ 0x5a5a)
    });
    defn(tm, "round", 0, 1, |s, a, _| time_round(s, a, b'r'));
    defn(tm, "floor", 0, 1, |s, a, _| time_round(s, a, b'f'));
    defn(tm, "ceil", 0, 1, |s, a, _| time_round(s, a, b'c'));
    for (i, name) in ["sunday?", "monday?", "tuesday?", "wednesday?", "thursday?", "friday?", "saturday?"].iter().enumerate() {
        let f: NativeFn = match i {
            0 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 0),
            1 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 1),
            2 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 2),
            3 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 3),
            4 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 4),
            5 => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 5),
            _ => |s, _, _| bool_v(crate::misc::tm_of(s).tm_wday == 6),
        };
        defn(tm, name, 0, 0, f);
    }
    // utc/gmtime/localtime convert the receiver in place; getutc/getgm/getlocal return a copy.
    for name in ["utc", "gmtime"] {
        defn(tm, name, 0, 0, |s, _, _| {
            set_time(s, time_ns(s), Zone::Utc);
            s
        });
    }
    for name in ["getutc", "getgm"] {
        defn(tm, name, 0, 0, |s, _, _| time_make(real_class(s), time_ns(s), Zone::Utc));
    }
    defn(tm, "localtime", 0, 1, |s, a, _| {
        let z = match a.first() {
            Some(&v) if v != QNIL => tryr!(parse_zone(v)),
            _ => Zone::Local,
        };
        set_time(s, time_ns(s), z);
        s
    });
    defn(tm, "getlocal", 0, 1, |s, a, _| {
        let z = match a.first() {
            Some(&v) if v != QNIL => tryr!(parse_zone(v)),
            _ => Zone::Local,
        };
        time_make(real_class(s), time_ns(s), z)
    });
    for name in ["utc?", "gmt?"] {
        defn(tm, name, 0, 0, |s, _, _| bool_v(zone_of(s) == Zone::Utc));
    }
    for name in ["utc_offset", "gmt_offset", "gmtoff"] {
        defn(tm, name, 0, 0, |s, _, _| match zone_of(s) {
            Zone::Utc => mkfix(0),
            Zone::Off(o) => mkfix(o),
            Zone::Local => mkfix(crate::misc::tm_of(s).tm_gmtoff),
        });
    }
    for name in ["dst?", "isdst"] {
        defn(tm, name, 0, 0, |s, _, _| bool_v(zone_of(s) == Zone::Local && crate::misc::tm_of(s).tm_isdst > 0));
    }
    defn(tm, "zone", 0, 0, |s, _, _| match zone_of(s) {
        Zone::Utc => str_from("UTC"),
        Zone::Off(_) => QNIL,
        Zone::Local => new_str_v(time_strftime(s, "%Z")),
    });
    defn(tm, "to_a", 0, 0, |s, _, _| {
        let t = crate::misc::tm_of(s);
        let zone = tri!(send0(s, "zone"));
        let dst = bool_v(zone_of(s) == Zone::Local && t.tm_isdst > 0);
        ary_new(vec![
            mkfix(t.tm_sec as i64),
            mkfix(t.tm_min as i64),
            mkfix(t.tm_hour as i64),
            mkfix(t.tm_mday as i64),
            mkfix(t.tm_mon as i64 + 1),
            mkfix(t.tm_year as i64 + 1900),
            mkfix(t.tm_wday as i64),
            mkfix(t.tm_yday as i64 + 1),
            dst,
            zone,
        ])
    });
    for name in ["asctime", "ctime"] {
        defn(tm, name, 0, 0, |s, _, _| new_str_v(time_strftime(s, "%a %b %e %H:%M:%S %Y")));
    }
    defn(tm, "to_s", 0, 0, |s, _, _| time_to_s(s, false));
    defn(tm, "inspect", 0, 0, |s, _, _| time_to_s(s, true));
    for name in ["iso8601", "xmlschema"] {
        defn(tm, name, 0, 1, |s, a, _| {
            let n = match a.first() {
                Some(&d) => tryr!(want_int(d)).max(0) as usize,
                None => 0,
            };
            let mut out = time_strftime(s, "%Y-%m-%dT%H:%M:%S");
            if n > 0 {
                out.push('.');
                out.push_str(&time_strftime(s, &format!("%{n}N")));
            }
            out.push_str(&if zone_of(s) == Zone::Utc { "Z".to_string() } else { time_strftime(s, "%:z") });
            new_str_v(out)
        });
    }
}
