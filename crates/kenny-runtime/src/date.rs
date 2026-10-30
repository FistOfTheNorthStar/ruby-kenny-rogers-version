//! Date and DateTime: a native replacement for CRuby's `date_core` extension
//! (stdlib `date.rb` does `require 'date_core'`, which this module provides).
//!
//! Every Date/DateTime is a plain object with hidden ivars: the local (civil) Julian Day
//! Number `@__jd`, local seconds into the day `@__df`, nanoseconds `@__sf` and the UTC
//! offset in seconds `@__of`. A Date is the same thing at offset 0 with (normally) no time.
//! The calendar follows CRuby's default `Date::ITALY`: Julian before 1582-10-15, Gregorian
//! after (the `start` arguments are accepted and ignored).
//!
//! Values CRuby returns as Rational (Date - Date, offset, sec_fraction, ajd, ...) are an
//! Integer when whole; otherwise a Rational when the runtime defines `Kernel#Rational`,
//! else a Float.
//!
//! `Date._parse`, `_iso8601`, `_strptime`, ... are ports of CRuby's date_parse.c /
//! date_strptime.c (non-TIGHT_PARSER variants) on top of Oniguruma.

use crate::core::*;
use crate::kernel::new_str_v;
use crate::tri;
use onig::{Regex, RegexOptions, Syntax};
use std::collections::HashMap;

static DATE: G<*mut Class> = G::new(std::ptr::null_mut());
static DATETIME: G<*mut Class> = G::new(std::ptr::null_mut());
static DATE_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());

const ITALY: i64 = 2299161;
const ENGLAND: i64 = 2361222;
const DAY: i64 = 86400;
const NS: i128 = 1_000_000_000;
const DAY_NS: i128 = 86400 * NS;
const UNIX_EPOCH_JD: i64 = 2440588;
const MJD_EPOCH: i64 = 2400001;

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

macro_rules! ok {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

// ---------- calendar arithmetic (same algorithms as CRuby's date_core.c) ----------

fn civil_to_jd(y: i64, m: i64, d: i64) -> i64 {
    let (mut y, mut m) = (y, m);
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let mut jd = (365.25 * (y + 4716) as f64).floor() + (30.6001 * (m + 1) as f64).floor() + d as f64 + b - 1524.0;
    if jd < ITALY as f64 {
        jd -= b;
    }
    jd as i64
}

fn jd_to_civil(jd: i64) -> (i64, i64, i64) {
    let a = if jd < ITALY {
        jd as f64
    } else {
        let x = ((jd as f64 - 1867216.25) / 36524.25).floor();
        jd as f64 + 1.0 + x - (x / 4.0).floor()
    };
    let b = a + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();
    let dom = b - d - (30.6001 * e).floor();
    let (y, m) = if e <= 13.0 { (c - 4716.0, e - 1.0) } else { (c - 4715.0, e - 13.0) };
    (y as i64, m as i64, dom as i64)
}

fn valid_civil_pos(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || d < 1 || d > 31 {
        return None;
    }
    let jd = civil_to_jd(y, m, d);
    if jd_to_civil(jd) == (y, m, d) { Some(jd) } else { None }
}

fn last_mday(y: i64, m: i64) -> Option<i64> {
    (1..=31).rev().find(|&d| valid_civil_pos(y, m, d).is_some())
}

/// Validates a civil date; negative month/day count from the end.
fn valid_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    let m = if m < 0 { m + 13 } else { m };
    if !(1..=12).contains(&m) {
        return None;
    }
    let d = if d < 0 { last_mday(y, m)? + d + 1 } else { d };
    valid_civil_pos(y, m, d)
}

fn find_fdoy(y: i64) -> i64 {
    (1..=31).find_map(|d| valid_civil_pos(y, 1, d)).unwrap_or_else(|| civil_to_jd(y, 1, 1))
}
fn find_ldoy(y: i64) -> i64 {
    (1..=31).rev().find_map(|d| valid_civil_pos(y, 12, d)).unwrap_or_else(|| civil_to_jd(y, 12, 31))
}

fn jd_to_ordinal(jd: i64) -> (i64, i64) {
    let (y, _, _) = jd_to_civil(jd);
    (y, jd - find_fdoy(y) + 1)
}

fn valid_ordinal(y: i64, d: i64) -> Option<i64> {
    let d = if d < 0 {
        let (ry, rd) = jd_to_ordinal(find_ldoy(y) + d + 1);
        if ry != y {
            return None;
        }
        rd
    } else {
        d
    };
    let jd = find_fdoy(y) + d - 1;
    if jd_to_ordinal(jd) == (y, d) { Some(jd) } else { None }
}

fn commercial_to_jd(y: i64, w: i64, d: i64) -> i64 {
    let rjd2 = find_fdoy(y) + 3;
    (rjd2 - rjd2.rem_euclid(7)) + 7 * (w - 1) + (d - 1)
}

fn jd_to_commercial(jd: i64) -> (i64, i64, i64) {
    let (a, _, _) = jd_to_civil(jd - 3);
    let mut rjd2 = commercial_to_jd(a + 1, 1, 1);
    let y = if jd >= rjd2 {
        a + 1
    } else {
        rjd2 = commercial_to_jd(a, 1, 1);
        a
    };
    let w = 1 + (jd - rjd2).div_euclid(7);
    let mut d = (jd + 1).rem_euclid(7);
    if d == 0 {
        d = 7;
    }
    (y, w, d)
}

fn valid_commercial(y: i64, w: i64, d: i64) -> Option<i64> {
    let d = if d < 0 { d + 8 } else { d };
    let w = if w < 0 {
        let (ry, rw, _) = jd_to_commercial(commercial_to_jd(y + 1, 1, 1) + w * 7);
        if ry != y {
            return None;
        }
        rw
    } else {
        w
    };
    let jd = commercial_to_jd(y, w, d);
    if jd_to_commercial(jd) == (y, w, d) { Some(jd) } else { None }
}

fn weeknum_to_jd(y: i64, w: i64, d: i64, f: i64) -> i64 {
    let rjd2 = find_fdoy(y) + 6;
    (rjd2 - ((rjd2 - f) + 1).rem_euclid(7) - 7) + 7 * w + d
}

fn jd_to_weeknum(jd: i64, f: i64) -> (i64, i64, i64) {
    let (y, _, _) = jd_to_civil(jd);
    let rjd = find_fdoy(y) + 6;
    let j = jd - (rjd - ((rjd - f) + 1).rem_euclid(7)) + 7;
    (y, j.div_euclid(7), j.rem_euclid(7))
}

fn valid_weeknum(y: i64, w: i64, d: i64, f: i64) -> Option<i64> {
    let d = if d < 0 { d + 7 } else { d };
    let w = if w < 0 {
        let (ry, rw, _) = jd_to_weeknum(weeknum_to_jd(y + 1, 1, f, f) + w * 7, f);
        if ry != y {
            return None;
        }
        rw
    } else {
        w
    };
    let jd = weeknum_to_jd(y, w, d, f);
    if jd_to_weeknum(jd, f) == (y, w, d) { Some(jd) } else { None }
}

fn wday_of(jd: i64) -> i64 {
    (jd + 1).rem_euclid(7)
}
fn gregorian_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn julian_leap(y: i64) -> bool {
    y % 4 == 0
}

/// (hour, min, sec) normalised like CRuby's c_valid_time_p (hour may be 24 for 24:00:00).
fn valid_time(h: i64, mi: i64, s: i64) -> Option<(i64, i64, i64)> {
    let h = if h < 0 { h + 24 } else { h };
    let mi = if mi < 0 { mi + 60 } else { mi };
    let s = if s < 0 { s + 60 } else { s };
    if !(0..=24).contains(&h) || !(0..=59).contains(&mi) || !(0..=59).contains(&s) || (h == 24 && (mi > 0 || s > 0)) {
        return None;
    }
    Some((h, mi, s))
}

// ---------- the value ----------

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct D {
    jd: i64,
    df: i64,
    sf: i64,
    of: i64,
}
impl D {
    fn date(jd: i64) -> D {
        D { jd, ..D::default() }
    }
    /// Nanoseconds since JD 0 00:00 UTC.
    fn abs(&self) -> i128 {
        (self.jd as i128 * DAY as i128 + self.df as i128 - self.of as i128) * NS + self.sf as i128
    }
    fn from_abs(abs: i128, of: i64) -> D {
        let local = abs + of as i128 * NS;
        let jd = local.div_euclid(DAY_NS) as i64;
        let rem = local.rem_euclid(DAY_NS);
        D { jd, df: (rem / NS) as i64, sf: (rem % NS) as i64, of }
    }
    fn civil(&self) -> (i64, i64, i64) {
        jd_to_civil(self.jd)
    }
    fn epoch_secs(&self) -> i64 {
        self.abs().div_euclid(NS) as i64 - UNIX_EPOCH_JD * DAY
    }
}

fn ivids() -> [u32; 4] {
    [sym("@__jd"), sym("@__df"), sym("@__sf"), sym("@__of")]
}
fn dget(v: V) -> D {
    let [a, b, c, e] = ivids();
    let g = |id| {
        let x = ivar_get(v, id);
        if is_fix(x) { fix(x) } else { 0 }
    };
    D { jd: g(a), df: g(b), sf: g(c), of: g(e) }
}
fn dnew(k: *mut Class, d: D) -> V {
    let o = object_new(k);
    let [a, b, c, e] = ivids();
    ivar_set(o, a, mkfix(d.jd));
    ivar_set(o, b, mkfix(d.df));
    ivar_set(o, c, mkfix(d.sf));
    ivar_set(o, e, mkfix(d.of));
    o
}
fn class_le(mut k: *mut Class, base: *mut Class) -> bool {
    unsafe {
        while !k.is_null() {
            if k == base {
                return true;
            }
            k = (*k).superclass;
        }
    }
    false
}
fn is_date(v: V) -> bool {
    is_kind(v, Kind::Object) && class_le(class_of(v), *DATE.get())
}
fn is_dt(v: V) -> bool {
    is_kind(v, Kind::Object) && class_le(class_of(v), *DATETIME.get())
}
fn dt_class(k: V) -> bool {
    class_le(k as *mut Class, *DATETIME.get())
}
fn same(s: V, d: D) -> V {
    dnew(real_class(s), d)
}
fn date_err() -> V {
    raise(*DATE_ERROR.get(), "invalid date")
}

// ---------- numbers ----------

enum Num {
    Int(i128),
    Frac(i128, i128),
    Flt(f64),
}
fn num_of(v: V) -> Option<Num> {
    use num_traits::ToPrimitive;
    if is_fix(v) {
        return Some(Num::Int(fix(v) as i128));
    }
    if is_big(v) {
        return rbig(v).n.to_i128().map(Num::Int);
    }
    if is_float(v) {
        return Some(Num::Flt(float_val(v)));
    }
    if is_heap(v) && is_a(v, rt().c.numeric) && respond_to(v, sym("numerator")) && respond_to(v, sym("denominator")) {
        let n = send0(v, "numerator");
        let d = send0(v, "denominator");
        if is_fix(n) && is_fix(d) && fix(d) != 0 {
            return Some(Num::Frac(fix(n) as i128, fix(d) as i128));
        }
        if n == UNDEF || d == UNDEF {
            pst().pending = Throw::None;
        }
    }
    None
}
/// `n * unit`, rounded to an integer.
fn num_times(n: &Num, unit: i128) -> i128 {
    match *n {
        Num::Int(i) => i * unit,
        Num::Frac(a, b) => (a * unit).div_euclid(b),
        Num::Flt(f) => (f * unit as f64).round() as i128,
    }
}
fn num_is_zero(n: &Num) -> bool {
    match *n {
        Num::Int(i) => i == 0,
        Num::Frac(a, _) => a == 0,
        Num::Flt(f) => f == 0.0,
    }
}
fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.abs()
}
/// n/d as an Integer when whole, else Rational (if available) or Float.
fn mk_ratio(mut n: i128, mut d: i128) -> V {
    if d < 0 {
        n = -n;
        d = -d;
    }
    let g = gcd(n, d);
    if g > 1 {
        n /= g;
        d /= g;
    }
    if d == 1 {
        return int_from_i128(n);
    }
    let main = rt().main_obj;
    if respond_to(main, sym("Rational")) {
        return send(main, sym("Rational"), &[int_from_i128(n), int_from_i128(d)], QNIL);
    }
    float_new(n as f64 / d as f64)
}
fn int_arg(v: V, what: &str) -> Result<i64, V> {
    match num_of(v) {
        Some(Num::Int(i)) if i >= i64::MIN as i128 / 4 && i <= i64::MAX as i128 / 4 => Ok(i as i64),
        Some(Num::Int(_)) => Err(raise(rt().c.range_error, &format!("{what} out of range"))),
        Some(Num::Flt(f)) => Ok(f.floor() as i64),
        Some(Num::Frac(a, b)) => Ok(a.div_euclid(b) as i64),
        None => Err(type_error(&format!("invalid {what} (not numeric)"))),
    }
}
fn opt_int(a: &[V], i: usize, def: i64, what: &str) -> Result<i64, V> {
    match a.get(i) {
        Some(&v) => int_arg(v, what),
        None => Ok(def),
    }
}
/// Drop a trailing keyword hash (e.g. `limit:` for the parse methods).
fn strip_kw(a: &[V]) -> &[V] {
    match a.last() {
        Some(&h) if is_hash(h) && hdr(h).flags & FL_KWHASH != 0 => &a[..a.len() - 1],
        _ => a,
    }
}
fn str_arg(v: V) -> Result<String, V> {
    if is_str(v) {
        Ok(String::from_utf8_lossy(&rstr(v).b).into_owned())
    } else if is_heap(v) && respond_to(v, sym("to_str")) {
        let s = send0(v, "to_str");
        if s == UNDEF {
            return Err(UNDEF);
        }
        str_arg(s)
    } else {
        Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
    }
}
/// A UTC offset argument: "+09:00", a fraction of a day, or seconds-like Integer days.
fn offset_arg(v: V) -> Result<i64, V> {
    let of = if is_str(v) {
        match zone_to_diff(&String::from_utf8_lossy(&rstr(v).b)) {
            Some(o) => o,
            None => 0,
        }
    } else {
        match num_of(v) {
            Some(n) => num_times(&n, DAY as i128) as i64,
            None => return Err(type_error("invalid offset")),
        }
    };
    Ok(if of.abs() > DAY { 0 } else { of })
}

// ---------- clock ----------

fn now() -> (i64, i64) {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_nanos() as i64)
}
fn local_offset(secs: i64) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm.tm_gmtoff as i64
}
fn today_jd() -> i64 {
    let (s, _) = now();
    UNIX_EPOCH_JD + (s + local_offset(s)).div_euclid(DAY)
}
fn time_at(v: V) -> V {
    send(rt().c.time as V, sym("at"), &[v], QNIL)
}

// ---------- strftime ----------

fn fmt_offset(of: i64, colons: usize) -> String {
    let sign = if of < 0 { '-' } else { '+' };
    let a = of.abs();
    let (h, m, s) = (a / 3600, a % 3600 / 60, a % 60);
    match colons {
        0 => format!("{sign}{h:02}{m:02}"),
        1 => format!("{sign}{h:02}:{m:02}"),
        2 => format!("{sign}{h:02}:{m:02}:{s:02}"),
        _ => {
            if s != 0 {
                format!("{sign}{h:02}:{m:02}:{s:02}")
            } else if m != 0 {
                format!("{sign}{h:02}:{m:02}")
            } else {
                format!("{sign}{h:02}")
            }
        }
    }
}

fn strftime(d: &D, fmt: &[u8]) -> Vec<u8> {
    enum It {
        N(i64, usize, u8),
        S(String),
    }
    let (y, mon, mday) = d.civil();
    let wday = wday_of(d.jd);
    let yday = d.jd - find_fdoy(y) + 1;
    let (cwy, cw, cwd) = jd_to_commercial(d.jd);
    let (h, mi, s) = (d.df / 3600, d.df % 3600 / 60, d.df % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' || i + 1 >= fmt.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let (mut nopad, mut padc, mut upper, mut chcase) = (false, None, false, false);
        while i < fmt.len() {
            match fmt[i] {
                b'-' => nopad = true,
                b'_' => padc = Some(b' '),
                b'0' => padc = Some(b'0'),
                b'^' => upper = true,
                b'#' => chcase = true,
                _ => break,
            }
            i += 1;
        }
        let mut width: Option<usize> = None;
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width = Some(width.unwrap_or(0) * 10 + (fmt[i] - b'0') as usize);
            i += 1;
        }
        let mut colons = 0;
        while i < fmt.len() && fmt[i] == b':' {
            colons += 1;
            i += 1;
        }
        if i >= fmt.len() {
            out.extend_from_slice(&fmt[start..]);
            break;
        }
        let dc = fmt[i];
        i += 1;
        let sub = |f: &str| It::S(String::from_utf8_lossy(&strftime(d, f.as_bytes())).into_owned());
        let it = match dc {
            b'Y' => It::N(y, if y >= 0 { 4 } else { 5 }, b'0'),
            b'C' => It::N(y.div_euclid(100), 2, b'0'),
            b'y' => It::N(y.rem_euclid(100), 2, b'0'),
            b'm' => It::N(mon, 2, b'0'),
            b'd' => It::N(mday, 2, b'0'),
            b'e' => It::N(mday, 2, b' '),
            b'j' => It::N(yday, 3, b'0'),
            b'H' => It::N(h, 2, b'0'),
            b'k' => It::N(h, 2, b' '),
            b'I' => It::N(h12, 2, b'0'),
            b'l' => It::N(h12, 2, b' '),
            b'M' => It::N(mi, 2, b'0'),
            b'S' => It::N(s, 2, b'0'),
            b'L' | b'N' => {
                let w = width.unwrap_or(if dc == b'L' { 3 } else { 9 });
                let digits = format!("{:09}", d.sf);
                let s = if w <= 9 { digits[..w].to_string() } else { digits + &"0".repeat(w - 9) };
                out.extend_from_slice(s.as_bytes());
                continue;
            }
            b'z' => It::S(fmt_offset(d.of, colons)),
            b'Z' => It::S(fmt_offset(d.of, 1)),
            b'A' => It::S(DAYS[wday as usize].to_string()),
            b'a' => It::S(DAYS[wday as usize][..3].to_string()),
            b'B' => It::S(MONTHS[mon as usize - 1].to_string()),
            b'b' | b'h' => It::S(MONTHS[mon as usize - 1][..3].to_string()),
            b'p' => It::S((if h < 12 { "AM" } else { "PM" }).to_string()),
            b'P' => It::S((if h < 12 { "am" } else { "pm" }).to_string()),
            b'u' => It::N(cwd, 1, b'0'),
            b'w' => It::N(wday, 1, b'0'),
            b'U' => It::N((yday - 1 + 7 - wday) / 7, 2, b'0'),
            b'W' => It::N((yday - 1 + 7 - (wday + 6) % 7) / 7, 2, b'0'),
            b'G' => It::N(cwy, if cwy >= 0 { 4 } else { 5 }, b'0'),
            b'g' => It::N(cwy.rem_euclid(100), 2, b'0'),
            b'V' => It::N(cw, 2, b'0'),
            b's' => It::N(d.epoch_secs(), 1, b'0'),
            b'Q' => It::N((d.abs().div_euclid(1_000_000) - (UNIX_EPOCH_JD * DAY) as i128 * 1000) as i64, 1, b'0'),
            b'F' => sub("%Y-%m-%d"),
            b'D' | b'x' => sub("%m/%d/%y"),
            b'T' | b'X' => sub("%H:%M:%S"),
            b'R' => sub("%H:%M"),
            b'r' => sub("%I:%M:%S %p"),
            b'c' => sub("%a %b %e %H:%M:%S %Y"),
            b'v' => sub("%e-%^b-%Y"),
            b'+' => sub("%a %b %e %H:%M:%S %Z %Y"),
            b'n' => It::S("\n".into()),
            b't' => It::S("\t".into()),
            b'%' => It::S("%".into()),
            _ => {
                out.extend_from_slice(&fmt[start..i]);
                continue;
            }
        };
        match it {
            It::N(v, defw, defpad) => {
                let s = if nopad {
                    v.to_string()
                } else {
                    let w = width.unwrap_or(defw);
                    if padc.unwrap_or(defpad) == b'0' { format!("{:0w$}", v) } else { format!("{:>w$}", v) }
                };
                out.extend_from_slice(s.as_bytes());
            }
            It::S(mut s) => {
                if upper {
                    s = s.to_uppercase();
                }
                if chcase {
                    s = if dc == b'p' || dc == b'Z' { s.to_lowercase() } else { s.to_uppercase() };
                }
                if let Some(w) = width {
                    if !nopad && s.len() < w {
                        let p = padc.unwrap_or(b' ') as char;
                        s = std::iter::repeat_n(p, w - s.len()).collect::<String>() + &s;
                    }
                }
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
    out
}

fn sf_str(d: &D, fmt: &str) -> String {
    String::from_utf8_lossy(&strftime(d, fmt.as_bytes())).into_owned()
}
fn iso_time_fmt(n: i64) -> String {
    if n > 0 { format!("T%H:%M:%S.%{n}N%:z") } else { "T%H:%M:%S%:z".to_string() }
}
fn jisx0301_date(d: &D) -> String {
    let (y, _, _) = d.civil();
    let (c, base) = match d.jd {
        j if j < 2405160 => return sf_str(d, "%Y-%m-%d"),
        j if j < 2419614 => ('M', 1867),
        j if j < 2424875 => ('T', 1911),
        j if j < 2447535 => ('S', 1925),
        j if j < 2458605 => ('H', 1988),
        _ => ('R', 2018),
    };
    format!("{}{:02}{}", c, y - base, sf_str(d, ".%m.%d"))
}

// ---------- parsing (ports of date_parse.c / date_strptime.c) ----------

#[derive(Clone, Debug)]
enum F {
    I(i64),
    S(String),
    R(i128, i128),
    B(bool),
}
#[derive(Default, Debug)]
struct Frags(Vec<(&'static str, F)>);
impl Frags {
    fn set(&mut self, k: &'static str, v: F) {
        if let Some(e) = self.0.iter_mut().find(|e| e.0 == k) {
            e.1 = v;
        } else {
            self.0.push((k, v));
        }
    }
    fn seti(&mut self, k: &'static str, v: i64) {
        self.set(k, F::I(v));
    }
    fn get(&self, k: &str) -> Option<&F> {
        self.0.iter().find(|e| e.0 == k).map(|e| &e.1)
    }
    fn has(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    fn i(&self, k: &str) -> Option<i64> {
        match self.get(k)? {
            F::I(i) => Some(*i),
            F::R(n, d) => Some(n.div_euclid(*d) as i64),
            _ => None,
        }
    }
    fn del(&mut self, k: &str) -> Option<F> {
        let i = self.0.iter().position(|e| e.0 == k)?;
        Some(self.0.remove(i).1)
    }
    fn to_hash(&self) -> V {
        let h = hash_new();
        for (k, v) in &self.0 {
            if k.starts_with('_') {
                continue;
            }
            let val = match v {
                F::I(i) => mkfix(*i),
                F::S(s) => str_from(s),
                F::R(n, d) => mk_ratio(*n, *d),
                F::B(b) => bool_v(*b),
            };
            crate::hash::hset(h, mksym(sym(k)), val);
        }
        h
    }
}

static RE_CACHE: crate::mt::L<Option<HashMap<&'static str, &'static Regex>>> = crate::mt::L::new(None);
fn re(p: &'static str) -> &'static Regex {
    let mut g = RE_CACHE.get();
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(r) = m.get(p) {
        return r;
    }
    let r: &'static Regex = Box::leak(Box::new(
        Regex::with_options(p, RegexOptions::REGEX_OPTION_IGNORECASE, Syntax::ruby()).unwrap_or_else(|e| panic!("date regex {p}: {e}")),
    ));
    m.insert(p, r);
    r
}
type Caps = Vec<Option<String>>;
fn match_caps(p: &'static str, s: &str) -> Option<(usize, usize, Caps)> {
    let c = re(p).captures(s)?;
    let (b, e) = c.pos(0)?;
    Some((b, e, (0..c.len()).map(|i| c.at(i).map(|x| x.to_string())).collect()))
}
/// Match, blank out the matched text (CRuby's `subx`) and return the groups.
fn subx(s: &mut String, p: &'static str) -> Option<Caps> {
    let (b, e, c) = match_caps(p, s)?;
    s.replace_range(b..e, " ");
    Some(c)
}

fn str2num(s: &str) -> i64 {
    let t: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '+').collect();
    t.parse::<i64>().unwrap_or_else(|_| if t.starts_with('-') { i64::MIN / 4 } else if t.is_empty() { 0 } else { i64::MAX / 4 })
}
fn sec_fraction(s: &str) -> F {
    let s: String = s.chars().take(18).collect();
    F::R(str2num(&s) as i128, 10i128.pow(s.len() as u32))
}
fn comp_year69(y: i64) -> i64 {
    if y >= 69 { y + 1900 } else { y + 2000 }
}
fn mon_num(s: &str) -> i64 {
    let l = s.to_ascii_lowercase();
    MONTHS.iter().position(|m| l.starts_with(&m[..3].to_ascii_lowercase())).map(|i| i as i64 + 1).unwrap_or(0)
}
fn day_num(s: &str) -> i64 {
    let l = s.to_ascii_lowercase();
    DAYS.iter().position(|m| l.starts_with(&m[..3].to_ascii_lowercase())).map(|i| i as i64).unwrap_or(0)
}

macro_rules! abbr_days {
    () => {
        "sun|mon|tue|wed|thu|fri|sat"
    };
}
macro_rules! abbr_months {
    () => {
        "jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec"
    };
}
macro_rules! full_days {
    () => {
        "sunday|monday|tuesday|wednesday|thursday|friday|saturday"
    };
}
macro_rules! era_bc {
    () => {
        r"(c(?:e|\.e\.)|b(?:ce|\.c\.e\.)|a(?:d|\.d\.)|b(?:c|\.c\.))?"
    };
}
macro_rules! zone_alts {
    () => {
        r"(?:gmt|utc?)?[-+]\d+(?:[,.:]\d+(?::\d+)?)?|(?-i:[[:alpha:].\s]+)(?:standard|daylight)\stime\b|(?-i:[[:alpha:]]+)(?:\sdst)?\b"
    };
}

const PAT_DAY: &str = concat!(r"\b(", abbr_days!(), r")[^-/\d\s]*");
const PAT_TIME: &str = concat!(
    r"((?:\d+\s*:\s*\d+(?:\s*:\s*\d+(?:[,.]\d*)?)?|\d+\s*h(?:\s*\d+m?(?:\s*\d+s?)?)?)(?:\s*[ap](?:m\b|\.m\.))?|\d+\s*[ap](?:m\b|\.m\.))",
    r"(?:\s*(",
    zone_alts!(),
    r"))?"
);
const PAT_TIME2: &str = r"\A(\d+)h?(?:\s*:?\s*(\d+)m?(?:\s*:?\s*(\d+)(?:[,.](\d+))?s?)?)?(?:\s*([ap])(?:m\b|\.m\.))?";
const PAT_EU: &str =
    concat!(r"'?\b(\d+)[^-\d\s]*\s*(", abbr_months!(), r")[^-\d\s']*(?:\s*", era_bc!(), r"\s*('?-?\d+(?:(?:st|nd|rd|th)\b)?))?");
const PAT_US: &str = concat!(r"\b(", abbr_months!(), r")[^-\d\s']*\s*('?\d+)[^-\d\s']*(?:\s*,?\s*", era_bc!(), r"\s*('?-?\d+))?");
const PAT_ISO: &str = r"('?[-+]?\d+)-(\d+)-('?-?\d+)";
const PAT_JIS: &str = r"\b([mtshr])(\d+)\.(\d+)\.(\d+)";
const PAT_VMS11: &str = concat!(r"('?-?\d+)-(", abbr_months!(), r")[^-/.]*-('?-?\d+)");
const PAT_VMS12: &str = concat!(r"\b(", abbr_months!(), r")[^-/.]*-('?-?\d+)(?:-('?-?\d+))?");
const PAT_SLA: &str = r"('?-?\d+)/\s*('?\d+)(?:\D\s*('?-?\d+))?";
const PAT_DOT: &str = r"('?-?\d+)\.\s*('?\d+)\.\s*('?-?\d+)";
const PAT_ISO21: &str = r"\b(\d{2}|\d{4})?-?w(\d{2})(?:-?(\d))?\b";
const PAT_ISO22: &str = r"-w-(\d)\b";
const PAT_ISO23: &str = r"--(\d{2})?-(\d{2})\b";
const PAT_ISO24: &str = r"--(\d{2})(\d{2})?\b";
const PAT_ISO25G: &str = r"[,.](\d{2}|\d{4})-\d{3}\b";
const PAT_ISO25: &str = r"\b(\d{2}|\d{4})-(\d{3})\b";
const PAT_ISO26G: &str = r"\d-\d{3}\b";
const PAT_ISO26: &str = r"\b-(\d{3})\b";
const PAT_YEAR: &str = r"'(\d+)\b";
const PAT_MON: &str = concat!(r"\b(", abbr_months!(), r")\S*");
const PAT_MDAY: &str = r"(\d+)(st|nd|rd|th)\b";
const PAT_DDD: &str = r"([-+]?)(\d{2,14})(?:\s*t?\s*(\d{2,6})?(?:[,.](\d*))?)?(?:\s*(z\b|[-+]\d{1,4}\b|\[[-+]?\d[^\]]*\]))?";
const PAT_BC: &str = r"\b(bc\b|bce\b|b\.c\.|b\.c\.e\.)";
const PAT_FRAG: &str = r"\A\s*(\d{1,2})\s*\z";

fn is_sign(b: u8) -> bool {
    b == b'-' || b == b'+'
}

/// CRuby's s3e: assign three loosely-ordered fields to year/mon/mday.
fn s3e(h: &mut Frags, y: Option<String>, m: Option<String>, d: Option<String>, bc: bool) {
    let (mut y, mut m, mut d) = (y, m, d);
    let mut comp: Option<bool> = None;
    if y.is_some() && m.is_some() && d.is_none() {
        let (oy, om) = (y.take(), m.take());
        y = d.take();
        m = oy;
        d = om;
    }
    if y.is_none() {
        if d.as_ref().is_some_and(|x| x.len() > 2) {
            y = d.take();
        }
        if d.as_ref().is_some_and(|x| x.starts_with('\'')) {
            y = d.take();
        }
    }
    if let Some(ys) = y.clone() {
        let b = ys.as_bytes();
        let mut s = 0;
        while s < b.len() && !is_sign(b[s]) && !b[s].is_ascii_digit() {
            s += 1;
        }
        if s < b.len() {
            let bp = s;
            if is_sign(b[s]) {
                s += 1;
            }
            let l = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
            if s + l < b.len() {
                let nd = ys[bp..].to_string();
                y = d.take();
                d = Some(nd);
            }
        }
    }
    if m.as_ref().is_some_and(|x| x.starts_with('\'') || x.len() > 2) {
        // us -> be
        let (oy, om, od) = (y.take(), m.take(), d.take());
        y = om;
        m = od;
        d = oy;
    }
    if d.as_ref().is_some_and(|x| x.starts_with('\'') || x.len() > 2) {
        std::mem::swap(&mut y, &mut d);
    }
    if let Some(ys) = &y {
        let b = ys.as_bytes();
        let mut s = 0;
        while s < b.len() && !is_sign(b[s]) && !b[s].is_ascii_digit() {
            s += 1;
        }
        if s < b.len() {
            let bp = s;
            if is_sign(b[s]) {
                s += 1;
                comp = Some(false);
            }
            let l = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
            if l > 2 {
                comp = Some(false);
            }
            h.seti("year", str2num(&ys[bp..s + l]));
        }
    }
    if bc {
        h.set("_bc", F::B(true));
    }
    for (v, key) in [(&m, "mon"), (&d, "mday")] {
        if let Some(xs) = v {
            let b = xs.as_bytes();
            let s = b.iter().take_while(|c| !c.is_ascii_digit()).count();
            if s < b.len() {
                let l = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
                h.seti(key, str2num(&xs[s..s + l]));
            }
        }
    }
    if let Some(c) = comp {
        h.set("_comp", F::B(c));
    }
}

fn is_bc(b: &Option<String>) -> bool {
    b.as_ref().is_some_and(|x| x.starts_with('b') || x.starts_with('B'))
}

fn parse_time(s: &mut String, h: &mut Frags) {
    let Some(c) = subx(s, PAT_TIME) else { return };
    if let Some(z) = &c[2] {
        h.set("zone", F::S(z.clone()));
    }
    let Some(s1) = &c[1] else { return };
    let Some((_, _, t)) = match_caps(PAT_TIME2, s1) else { return };
    let mut hour = str2num(t[1].as_deref().unwrap_or("0"));
    if let Some(p) = &t[5] {
        hour %= 12;
        if p.starts_with('p') || p.starts_with('P') {
            hour += 12;
        }
    }
    h.seti("hour", hour);
    if let Some(x) = &t[2] {
        h.seti("min", str2num(x));
    }
    if let Some(x) = &t[3] {
        h.seti("sec", str2num(x));
    }
    if let Some(x) = &t[4] {
        h.set("sec_fraction", sec_fraction(x));
    }
}

fn parse_ddd(s: &mut String, h: &mut Frags) -> bool {
    let Some(c) = subx(s, PAT_DDD) else { return false };
    let sign = c[1].clone().unwrap_or_default();
    let s2 = c[2].clone().unwrap_or_default();
    let (s3, s4, s5) = (&c[3], &c[4], &c[5]);
    let l2 = s2.len();
    let n = |a: usize, l: usize| str2num(&s2[a..a + l]);
    let signed = |a: usize, l: usize| str2num(&format!("{}{}", sign, &s2[a..a + l]));
    let time_only = s3.is_none() && s4.is_some();
    match l2 {
        2 => {
            if time_only {
                h.seti("sec", n(0, 2));
            } else {
                h.seti("mday", n(0, 2));
            }
        }
        4 => {
            if time_only {
                h.seti("sec", n(2, 2));
                h.seti("min", n(0, 2));
            } else {
                h.seti("mon", n(0, 2));
                h.seti("mday", n(2, 2));
            }
        }
        6 => {
            if time_only {
                h.seti("sec", n(4, 2));
                h.seti("min", n(2, 2));
                h.seti("hour", n(0, 2));
            } else {
                h.seti("year", signed(0, 2));
                h.seti("mon", n(2, 2));
                h.seti("mday", n(4, 2));
            }
        }
        8 | 10 | 12 | 14 => {
            if time_only {
                h.seti("sec", n(l2 - 2, 2));
                h.seti("min", n(l2 - 4, 2));
                h.seti("hour", n(l2 - 6, 2));
                h.seti("mday", n(l2 - 8, 2));
                if l2 >= 10 {
                    h.seti("mon", n(l2 - 10, 2));
                }
                if l2 == 12 {
                    h.seti("year", signed(0, 2));
                }
                if l2 == 14 {
                    h.seti("year", signed(0, 4));
                    h.set("_comp", F::B(false));
                }
            } else {
                h.seti("year", signed(0, 4));
                h.seti("mon", n(4, 2));
                h.seti("mday", n(6, 2));
                if l2 >= 10 {
                    h.seti("hour", n(8, 2));
                }
                if l2 >= 12 {
                    h.seti("min", n(10, 2));
                }
                if l2 >= 14 {
                    h.seti("sec", n(12, 2));
                }
                h.set("_comp", F::B(false));
            }
        }
        3 => {
            if time_only {
                h.seti("sec", n(1, 2));
                h.seti("hour", n(0, 1));
            } else {
                h.seti("yday", n(0, 3));
            }
        }
        5 => {
            if time_only {
                h.seti("sec", n(3, 2));
                h.seti("min", n(1, 2));
                h.seti("hour", n(0, 1));
            } else {
                h.seti("year", signed(0, 2));
                h.seti("yday", n(2, 3));
            }
        }
        7 => {
            if time_only {
                h.seti("sec", n(5, 2));
                h.seti("min", n(3, 2));
                h.seti("hour", n(1, 2));
                h.seti("mday", n(0, 1));
            } else {
                h.seti("year", signed(0, 4));
                h.seti("yday", n(4, 3));
            }
        }
        _ => {}
    }
    if let Some(s3) = s3 {
        let l3 = s3.len();
        let n3 = |a: usize| str2num(&s3[a..a + 2]);
        if matches!(l3, 2 | 4 | 6) {
            if s4.is_some() {
                h.seti("sec", n3(l3 - 2));
                if l3 >= 4 {
                    h.seti("min", n3(l3 - 4));
                }
                if l3 >= 6 {
                    h.seti("hour", n3(l3 - 6));
                }
            } else {
                h.seti("hour", n3(0));
                if l3 >= 4 {
                    h.seti("min", n3(2));
                }
                if l3 >= 6 {
                    h.seti("sec", n3(4));
                }
            }
        }
    }
    if let Some(s4) = s4 {
        h.set("sec_fraction", sec_fraction(s4));
    }
    if let Some(s5) = s5 {
        if let Some(inner) = s5.strip_prefix('[') {
            let inner = inner.strip_suffix(']').unwrap_or(inner);
            let (zone, off_src) = match inner.find(':') {
                Some(p) => (inner[p + 1..].to_string(), inner[..p].to_string()),
                None => {
                    let z = inner.to_string();
                    let src = if inner.starts_with(|c: char| c.is_ascii_digit()) { format!("+{z}") } else { z.clone() };
                    (z, src)
                }
            };
            h.set("zone", F::S(zone));
            if let Some(o) = zone_to_diff(&off_src) {
                h.seti("offset", o);
            }
        } else {
            h.set("zone", F::S(s5.clone()));
        }
    }
    true
}

const HAVE_ALPHA: u32 = 1;
const HAVE_DIGIT: u32 = 2;
const HAVE_DASH: u32 = 4;
const HAVE_DOT: u32 = 8;
const HAVE_SLASH: u32 = 16;

fn date_parse(input: &str, comp: bool) -> Frags {
    // replace everything that cannot be part of a date with spaces
    let mut s = String::with_capacity(input.len());
    let mut in_junk = false;
    for ch in input.chars() {
        if ch.is_alphanumeric() || "-+',./:@[]".contains(ch) {
            s.push(ch);
            in_junk = false;
        } else if !in_junk {
            s.push(' ');
            in_junk = true;
        }
    }
    let mut cls = 0;
    for ch in s.chars() {
        cls |= if ch.is_alphabetic() {
            HAVE_ALPHA
        } else if ch.is_ascii_digit() {
            HAVE_DIGIT
        } else if ch == '-' {
            HAVE_DASH
        } else if ch == '.' {
            HAVE_DOT
        } else if ch == '/' {
            HAVE_SLASH
        } else {
            0
        };
    }
    let has = |f: u32| cls & f == f;
    let mut h = Frags::default();
    h.set("_comp", F::B(comp));
    if has(HAVE_ALPHA) {
        if let Some(c) = subx(&mut s, PAT_DAY) {
            h.seti("wday", day_num(c[1].as_deref().unwrap_or("")));
        }
    }
    if has(HAVE_DIGIT) {
        parse_time(&mut s, &mut h);
    }
    #[allow(clippy::never_loop)]
    'ok: loop {
        if has(HAVE_ALPHA | HAVE_DIGIT) {
            if let Some(c) = subx(&mut s, PAT_EU) {
                let mon = mon_num(c[2].as_deref().unwrap_or(""));
                s3e(&mut h, c[4].clone(), Some(mon.to_string()), c[1].clone(), is_bc(&c[3]));
                break 'ok;
            }
            if let Some(c) = subx(&mut s, PAT_US) {
                let mon = mon_num(c[1].as_deref().unwrap_or(""));
                s3e(&mut h, c[4].clone(), Some(mon.to_string()), c[2].clone(), is_bc(&c[3]));
                break 'ok;
            }
        }
        if has(HAVE_DIGIT | HAVE_DASH) {
            if let Some(c) = subx(&mut s, PAT_ISO) {
                s3e(&mut h, c[1].clone(), c[2].clone(), c[3].clone(), false);
                break 'ok;
            }
        }
        if has(HAVE_DIGIT | HAVE_DOT) {
            if let Some(c) = subx(&mut s, PAT_JIS) {
                let era = match c[1].as_deref().unwrap_or("").to_ascii_lowercase().as_str() {
                    "m" => 1867,
                    "t" => 1911,
                    "s" => 1925,
                    "h" => 1988,
                    "r" => 2018,
                    _ => 0,
                };
                h.seti("year", str2num(c[2].as_deref().unwrap_or("0")) + era);
                h.seti("mon", str2num(c[3].as_deref().unwrap_or("0")));
                h.seti("mday", str2num(c[4].as_deref().unwrap_or("0")));
                break 'ok;
            }
        }
        if has(HAVE_ALPHA | HAVE_DIGIT | HAVE_DASH) {
            if let Some(c) = subx(&mut s, PAT_VMS11) {
                let mon = mon_num(c[2].as_deref().unwrap_or(""));
                s3e(&mut h, c[3].clone(), Some(mon.to_string()), c[1].clone(), false);
                break 'ok;
            }
            if let Some(c) = subx(&mut s, PAT_VMS12) {
                let mon = mon_num(c[1].as_deref().unwrap_or(""));
                s3e(&mut h, c[3].clone(), Some(mon.to_string()), c[2].clone(), false);
                break 'ok;
            }
        }
        if has(HAVE_DIGIT | HAVE_SLASH) {
            if let Some(c) = subx(&mut s, PAT_SLA) {
                s3e(&mut h, c[1].clone(), c[2].clone(), c[3].clone(), false);
                break 'ok;
            }
        }
        if has(HAVE_DIGIT | HAVE_DOT) {
            if let Some(c) = subx(&mut s, PAT_DOT) {
                s3e(&mut h, c[1].clone(), c[2].clone(), c[3].clone(), false);
                break 'ok;
            }
        }
        if has(HAVE_DIGIT) {
            if let Some(c) = subx(&mut s, PAT_ISO21) {
                if let Some(y) = &c[1] {
                    h.seti("cwyear", str2num(y));
                }
                h.seti("cweek", str2num(c[2].as_deref().unwrap_or("0")));
                if let Some(d) = &c[3] {
                    h.seti("cwday", str2num(d));
                }
                break 'ok;
            }
            if let Some(c) = subx(&mut s, PAT_ISO22) {
                h.seti("cwday", str2num(c[1].as_deref().unwrap_or("0")));
                break 'ok;
            }
            if let Some(c) = subx(&mut s, PAT_ISO23) {
                if let Some(m) = &c[1] {
                    h.seti("mon", str2num(m));
                }
                h.seti("mday", str2num(c[2].as_deref().unwrap_or("0")));
                break 'ok;
            }
            if let Some(c) = subx(&mut s, PAT_ISO24) {
                h.seti("mon", str2num(c[1].as_deref().unwrap_or("0")));
                if let Some(d) = &c[2] {
                    h.seti("mday", str2num(d));
                }
                break 'ok;
            }
            if match_caps(PAT_ISO25G, &s).is_none() {
                if let Some(c) = subx(&mut s, PAT_ISO25) {
                    h.seti("year", str2num(c[1].as_deref().unwrap_or("0")));
                    h.seti("yday", str2num(c[2].as_deref().unwrap_or("0")));
                    break 'ok;
                }
            }
            if match_caps(PAT_ISO26G, &s).is_none() {
                if let Some(c) = subx(&mut s, PAT_ISO26) {
                    h.seti("yday", str2num(c[1].as_deref().unwrap_or("0")));
                    break 'ok;
                }
            }
            if let Some(c) = subx(&mut s, PAT_YEAR) {
                h.seti("year", str2num(c[1].as_deref().unwrap_or("0")));
                break 'ok;
            }
        }
        if has(HAVE_ALPHA) {
            if let Some(c) = subx(&mut s, PAT_MON) {
                h.seti("mon", mon_num(c[1].as_deref().unwrap_or("")));
                break 'ok;
            }
        }
        if has(HAVE_DIGIT) {
            if let Some(c) = subx(&mut s, PAT_MDAY) {
                h.seti("mday", str2num(c[1].as_deref().unwrap_or("0")));
                break 'ok;
            }
            if parse_ddd(&mut s, &mut h) {
                break 'ok;
            }
        }
        break 'ok;
    }
    if has(HAVE_ALPHA) && subx(&mut s, PAT_BC).is_some() {
        h.set("_bc", F::B(true));
    }
    if has(HAVE_DIGIT) {
        if let Some((_, _, c)) = match_caps(PAT_FRAG, &s) {
            let n = str2num(c[1].as_deref().unwrap_or("0"));
            if h.has("hour") && !h.has("mday") && (1..=31).contains(&n) {
                h.seti("mday", n);
            }
            if h.has("mday") && !h.has("hour") && (0..=24).contains(&n) {
                h.seti("hour", n);
            }
        }
    }
    if matches!(h.del("_bc"), Some(F::B(true))) {
        for k in ["cwyear", "year"] {
            if let Some(y) = h.i(k) {
                h.seti(k, 1 - y);
            }
        }
    }
    if matches!(h.del("_comp"), Some(F::B(true))) {
        for k in ["cwyear", "year"] {
            if let Some(y) = h.i(k) {
                if (0..=99).contains(&y) {
                    h.seti(k, if y >= 69 { y + 1900 } else { y + 2000 });
                }
            }
        }
    }
    if let (Some(F::S(z)), false) = (h.get("zone").cloned(), h.has("offset")) {
        if let Some(o) = zone_to_diff(&z) {
            h.seti("offset", o);
        }
    }
    h
}

const ZONES: &[(&str, f64)] = &[
    ("ut", 0.0), ("gmt", 0.0), ("utc", 0.0), ("z", 0.0), ("wet", 0.0),
    ("est", -5.0), ("edt", -4.0), ("cst", -6.0), ("cdt", -5.0), ("mst", -7.0), ("mdt", -6.0), ("pst", -8.0), ("pdt", -7.0),
    ("a", 1.0), ("b", 2.0), ("c", 3.0), ("d", 4.0), ("e", 5.0), ("f", 6.0), ("g", 7.0), ("h", 8.0), ("i", 9.0),
    ("k", 10.0), ("l", 11.0), ("m", 12.0), ("n", -1.0), ("o", -2.0), ("p", -3.0), ("q", -4.0), ("r", -5.0),
    ("s", -6.0), ("t", -7.0), ("u", -8.0), ("v", -9.0), ("w", -10.0), ("x", -11.0), ("y", -12.0),
    ("akst", -9.0), ("akdt", -8.0), ("hst", -10.0), ("hast", -10.0), ("hadt", -9.0), ("ast", -4.0), ("adt", -3.0),
    ("nst", -3.5), ("ndt", -2.5), ("yst", -9.0), ("ydt", -8.0), ("ahst", -10.0), ("nt", -11.0), ("idlw", -12.0),
    ("at", -2.0), ("brt", -3.0), ("brst", -2.0), ("art", -3.0), ("clt", -4.0), ("clst", -3.0),
    ("west", 1.0), ("bst", 1.0), ("cet", 1.0), ("met", 1.0), ("mewt", 1.0), ("mez", 1.0), ("fwt", 1.0), ("swt", 1.0), ("wat", 1.0),
    ("cest", 2.0), ("mest", 2.0), ("mesz", 2.0), ("eet", 2.0), ("fst", 2.0), ("sst", 2.0), ("sast", 2.0),
    ("eest", 3.0), ("msk", 3.0), ("bt", 3.0), ("eat", 3.0), ("msd", 4.0), ("zp4", 4.0), ("zp5", 5.0), ("ist", 5.5), ("zp6", 6.0),
    ("wast", 7.0), ("wadt", 8.0), ("cct", 8.0), ("sgt", 8.0), ("hkt", 8.0), ("awst", 8.0), ("jst", 9.0), ("kst", 9.0),
    ("acst", 9.5), ("acdt", 10.5), ("aest", 10.0), ("aedt", 11.0), ("east", 10.0), ("eadt", 11.0), ("gst", 10.0),
    ("idle", 12.0), ("nzst", 12.0), ("nzt", 12.0), ("nzdt", 13.0),
];

/// Zone name or numeric offset ("+09:00", "-0500", "EST", "GMT+9", "UTC") to seconds east of UTC.
fn zone_to_diff(zone: &str) -> Option<i64> {
    let mut s = zone.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let mut dst = false;
    for (suf, d) in [(" standard time", false), (" daylight time", true), (" dst", true)] {
        if let Some(x) = s.strip_suffix(suf) {
            s = x.to_string();
            dst = d;
            break;
        }
    }
    if let Some(&(_, off)) = ZONES.iter().find(|(n, _)| *n == s) {
        return Some((off * 3600.0) as i64 + if dst { 3600 } else { 0 });
    }
    let mut t = s.as_str();
    if t.len() > 3 && (t.starts_with("gmt") || t.starts_with("utc")) {
        t = &t[3..];
    }
    let sign = match t.bytes().next() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return None,
    };
    let t = &t[1..];
    let num = |x: &str| -> Option<i64> { if x.is_empty() || !x.bytes().all(|c| c.is_ascii_digit()) { None } else { x.parse().ok() } };
    let secs = if t.contains(':') {
        let p: Vec<&str> = t.split(':').collect();
        let h = num(p[0])?;
        let m = match p.get(1) { Some(x) => num(x)?, None => 0 };
        let s = match p.get(2) { Some(x) => num(x)?, None => 0 };
        h * 3600 + m * 60 + s
    } else if let Some(p) = t.find([',', '.']) {
        let h = num(&t[..p])?;
        let frac = &t[p + 1..];
        let f = if frac.is_empty() { 0 } else { num(&frac[..frac.len().min(9)])? };
        let den = 10i64.pow(frac.len().min(9) as u32);
        h * 3600 + f * 3600 / den
    } else {
        let l = t.len();
        if l == 0 || !t.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let hl = if l <= 2 { l } else { 2 - l % 2 };
        let h = num(&t[..hl])?;
        let m = if l >= hl + 2 { num(&t[hl..hl + 2])? } else { 0 };
        let s = if l >= hl + 4 { num(&t[hl + 2..(hl + 4)])? } else { 0 };
        h * 3600 + m * 60 + s
    };
    Some(sign * secs)
}

fn set_zone(h: &mut Frags, z: &str) {
    h.set("zone", F::S(z.to_string()));
    if let Some(o) = zone_to_diff(z) {
        h.seti("offset", o);
    }
}
fn year_len_comp(s: &str) -> i64 {
    let y = str2num(s);
    if s.len() < 4 { comp_year69(y) } else { y }
}
fn cap<'a>(c: &'a Caps, i: usize) -> Option<&'a str> {
    c.get(i).and_then(|x| x.as_deref())
}

const ISO_EXT_DT: &str = r"\A\s*(?:([-+]?\d{2,}|-)-(\d{2})?(?:-(\d{2}))?|([-+]?\d{2,})?-(\d{3})|(\d{4}|\d{2})?-w(\d{2})-(\d)|-w-(\d))(?:t(\d{2}):(\d{2})(?::(\d{2})(?:[,.](\d+))?)?(z|[-+]\d{2}(?::?\d{2})?)?)?\s*\z";
const ISO_BAS_DT: &str = r"\A\s*(?:([-+]?(?:\d{4}|\d{2})|--)(\d{2}|-)(\d{2})|([-+]?(?:\d{4}|\d{2}))(\d{3})|-(\d{3})|(\d{4}|\d{2})w(\d{2})(\d)|-w(\d{2})(\d)|-w-(\d))(?:t?(\d{2})(\d{2})(?:(\d{2})(?:[,.](\d+))?)?(z|[-+]\d{2}(?:\d{2})?)?)?\s*\z";
const ISO_EXT_T: &str = r"\A\s*(\d{2}):(\d{2})(?::(\d{2})(?:[,.](\d+))?(z|[-+]\d{2}(:?\d{2})?)?)?\s*\z";
const ISO_BAS_T: &str = r"\A\s*(\d{2})(\d{2})(?:(\d{2})(?:[,.](\d+))?(z|[-+]\d{2}(\d{2})?)?)?\s*\z";

fn time_groups(h: &mut Frags, c: &Caps, i: usize) {
    if let Some(x) = cap(c, i) {
        h.seti("hour", str2num(x));
        h.seti("min", str2num(cap(c, i + 1).unwrap_or("0")));
        if let Some(x) = cap(c, i + 2) {
            h.seti("sec", str2num(x));
        }
    }
    if let Some(x) = cap(c, i + 3) {
        h.set("sec_fraction", sec_fraction(x));
    }
    if let Some(x) = cap(c, i + 4) {
        set_zone(h, x);
    }
}

fn date_iso8601(s: &str) -> Frags {
    let mut h = Frags::default();
    if let Some((_, _, c)) = match_caps(ISO_EXT_DT, s) {
        if let Some(y) = cap(&c, 1) {
            if let Some(d) = cap(&c, 3) {
                h.seti("mday", str2num(d));
            }
            if y != "-" {
                h.seti("year", year_len_comp(y));
            }
            match cap(&c, 2) {
                None if y != "-" => return Frags::default(),
                None => {}
                Some(m) => h.seti("mon", str2num(m)),
            }
        } else if let Some(yd) = cap(&c, 5) {
            h.seti("yday", str2num(yd));
            if let Some(y) = cap(&c, 4) {
                h.seti("year", year_len_comp(y));
            }
        } else if let Some(d) = cap(&c, 8) {
            h.seti("cweek", str2num(cap(&c, 7).unwrap_or("0")));
            h.seti("cwday", str2num(d));
            if let Some(y) = cap(&c, 6) {
                h.seti("cwyear", year_len_comp(y));
            }
        } else if let Some(d) = cap(&c, 9) {
            h.seti("cwday", str2num(d));
        }
        time_groups(&mut h, &c, 10);
    } else if let Some((_, _, c)) = match_caps(ISO_BAS_DT, s) {
        if let Some(d) = cap(&c, 3) {
            h.seti("mday", str2num(d));
            let y = cap(&c, 1).unwrap_or("--");
            if y != "--" {
                h.seti("year", year_len_comp(y));
            }
            let m = cap(&c, 2).unwrap_or("-");
            if m.starts_with('-') {
                if y != "--" {
                    return Frags::default();
                }
            } else {
                h.seti("mon", str2num(m));
            }
        } else if let Some(yd) = cap(&c, 5) {
            h.seti("yday", str2num(yd));
            h.seti("year", year_len_comp(cap(&c, 4).unwrap_or("0")));
        } else if let Some(yd) = cap(&c, 6) {
            h.seti("yday", str2num(yd));
        } else if let Some(d) = cap(&c, 9) {
            h.seti("cweek", str2num(cap(&c, 8).unwrap_or("0")));
            h.seti("cwday", str2num(d));
            h.seti("cwyear", year_len_comp(cap(&c, 7).unwrap_or("0")));
        } else if let Some(d) = cap(&c, 11) {
            h.seti("cweek", str2num(cap(&c, 10).unwrap_or("0")));
            h.seti("cwday", str2num(d));
        } else if let Some(d) = cap(&c, 12) {
            h.seti("cwday", str2num(d));
        }
        time_groups(&mut h, &c, 13);
    } else if let Some((_, _, c)) = match_caps(ISO_EXT_T, s).or_else(|| match_caps(ISO_BAS_T, s)) {
        time_groups(&mut h, &c, 1);
    }
    h
}

const RFC3339: &str = r"\A\s*(-?\d{4})-(\d{2})-(\d{2})(?:t|\s)(\d{2}):(\d{2}):(\d{2})(?:[,.](\d+))?(z|[-+]\d{2}:\d{2})\s*\z";
fn date_rfc3339(s: &str) -> Frags {
    let mut h = Frags::default();
    if let Some((_, _, c)) = match_caps(RFC3339, s) {
        for (i, k) in [(1, "year"), (2, "mon"), (3, "mday"), (4, "hour"), (5, "min"), (6, "sec")] {
            h.seti(k, str2num(cap(&c, i).unwrap_or("0")));
        }
        set_zone(&mut h, cap(&c, 8).unwrap_or("z"));
        if let Some(f) = cap(&c, 7) {
            h.set("sec_fraction", sec_fraction(f));
        }
    }
    h
}

const XML_DT: &str = r"\A\s*(-?\d{4,})(?:-(\d{2})(?:-(\d{2}))?)?(?:t(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?)?(z|[-+]\d{2}:\d{2})?\s*\z";
const XML_T: &str = r"\A\s*(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(z|[-+]\d{2}:\d{2})?\s*\z";
const XML_TRUNC: &str = r"\A\s*(?:--(\d{2})(?:-(\d{2}))?|---(\d{2}))(z|[-+]\d{2}:\d{2})?\s*\z";
fn date_xmlschema(s: &str) -> Frags {
    let mut h = Frags::default();
    if let Some((_, _, c)) = match_caps(XML_DT, s) {
        h.seti("year", str2num(cap(&c, 1).unwrap_or("0")));
        for (i, k) in [(2, "mon"), (3, "mday"), (4, "hour"), (5, "min"), (6, "sec")] {
            if let Some(x) = cap(&c, i) {
                h.seti(k, str2num(x));
            }
        }
        if let Some(f) = cap(&c, 7) {
            h.set("sec_fraction", sec_fraction(f));
        }
        if let Some(z) = cap(&c, 8) {
            set_zone(&mut h, z);
        }
    } else if let Some((_, _, c)) = match_caps(XML_T, s) {
        for (i, k) in [(1, "hour"), (2, "min"), (3, "sec")] {
            h.seti(k, str2num(cap(&c, i).unwrap_or("0")));
        }
        if let Some(f) = cap(&c, 4) {
            h.set("sec_fraction", sec_fraction(f));
        }
        if let Some(z) = cap(&c, 5) {
            set_zone(&mut h, z);
        }
    } else if let Some((_, _, c)) = match_caps(XML_TRUNC, s) {
        if let Some(m) = cap(&c, 1) {
            h.seti("mon", str2num(m));
        }
        if let Some(d) = cap(&c, 2).or(cap(&c, 3)) {
            h.seti("mday", str2num(d));
        }
        if let Some(z) = cap(&c, 4) {
            set_zone(&mut h, z);
        }
    }
    h
}

const RFC2822: &str = concat!(
    r"\A\s*(?:(",
    abbr_days!(),
    r")\s*,\s+)?(\d{1,2})\s+(",
    abbr_months!(),
    r")\s+(-?\d{2,})\s+(\d{2}):(\d{2})(?::(\d{2}))?\s*([-+]\d{4}|ut|gmt|e[sd]t|c[sd]t|m[sd]t|p[sd]t|[a-ik-z])\s*\z"
);
fn date_rfc2822(s: &str) -> Frags {
    let mut h = Frags::default();
    if let Some((_, _, c)) = match_caps(RFC2822, s) {
        if let Some(w) = cap(&c, 1) {
            h.seti("wday", day_num(w));
        }
        h.seti("mday", str2num(cap(&c, 2).unwrap_or("0")));
        h.seti("mon", mon_num(cap(&c, 3).unwrap_or("")));
        let ys = cap(&c, 4).unwrap_or("0");
        let mut y = str2num(ys);
        if ys.len() < 4 {
            y += if y >= 50 { 1900 } else { 2000 };
        }
        h.seti("year", y);
        h.seti("hour", str2num(cap(&c, 5).unwrap_or("0")));
        h.seti("min", str2num(cap(&c, 6).unwrap_or("0")));
        if let Some(x) = cap(&c, 7) {
            h.seti("sec", str2num(x));
        }
        set_zone(&mut h, cap(&c, 8).unwrap_or("gmt"));
    }
    h
}

const HTTPDATE1: &str = concat!(
    r"\A\s*(",
    abbr_days!(),
    r")\s*,\s+(\d{2})\s+(",
    abbr_months!(),
    r")\s+(-?\d{4})\s+(\d{2}):(\d{2}):(\d{2})\s+(gmt)\s*\z"
);
const HTTPDATE2: &str = concat!(
    r"\A\s*(",
    full_days!(),
    r")\s*,\s+(\d{2})\s*-\s*(",
    abbr_months!(),
    r")\s*-\s*(\d{2})\s+(\d{2}):(\d{2}):(\d{2})\s+(gmt)\s*\z"
);
const HTTPDATE3: &str =
    concat!(r"\A\s*(", abbr_days!(), r")\s+(", abbr_months!(), r")\s+(\d{1,2})\s+(\d{2}):(\d{2}):(\d{2})\s+(\d{4})\s*\z");
fn date_httpdate(s: &str) -> Frags {
    let mut h = Frags::default();
    let hms = |h: &mut Frags, c: &Caps, i: usize| {
        h.seti("hour", str2num(cap(c, i).unwrap_or("0")));
        h.seti("min", str2num(cap(c, i + 1).unwrap_or("0")));
        h.seti("sec", str2num(cap(c, i + 2).unwrap_or("0")));
    };
    if let Some((_, _, c)) = match_caps(HTTPDATE1, s).or_else(|| match_caps(HTTPDATE2, s)) {
        h.seti("wday", day_num(cap(&c, 1).unwrap_or("")));
        h.seti("mday", str2num(cap(&c, 2).unwrap_or("0")));
        h.seti("mon", mon_num(cap(&c, 3).unwrap_or("")));
        let ys = cap(&c, 4).unwrap_or("0");
        let mut y = str2num(ys);
        if ys.len() < 4 && (0..=99).contains(&y) {
            y += if y >= 50 { 1900 } else { 2000 };
        }
        h.seti("year", y);
        hms(&mut h, &c, 5);
        h.set("zone", F::S(cap(&c, 8).unwrap_or("GMT").to_string()));
        h.seti("offset", 0);
    } else if let Some((_, _, c)) = match_caps(HTTPDATE3, s) {
        h.seti("wday", day_num(cap(&c, 1).unwrap_or("")));
        h.seti("mon", mon_num(cap(&c, 2).unwrap_or("")));
        h.seti("mday", str2num(cap(&c, 3).unwrap_or("0")));
        hms(&mut h, &c, 4);
        h.seti("year", str2num(cap(&c, 7).unwrap_or("0")));
    }
    h
}

const JISX0301: &str = r"\A\s*([mtshr])?(\d{2})\.(\d{2})\.(\d{2})(?:t(?:(\d{2}):(\d{2})(?::(\d{2})(?:[,.](\d*))?)?(z|[-+]\d{2}(?::?\d{2})?)?)?)?\s*\z";
fn date_jisx0301(s: &str) -> Frags {
    let Some((_, _, c)) = match_caps(JISX0301, s) else { return date_iso8601(s) };
    let mut h = Frags::default();
    let era = match cap(&c, 1).unwrap_or("h").to_ascii_lowercase().as_str() {
        "m" => 1867,
        "t" => 1911,
        "s" => 1925,
        "r" => 2018,
        _ => 1988,
    };
    h.seti("year", str2num(cap(&c, 2).unwrap_or("0")) + era);
    h.seti("mon", str2num(cap(&c, 3).unwrap_or("0")));
    h.seti("mday", str2num(cap(&c, 4).unwrap_or("0")));
    time_groups(&mut h, &c, 5);
    h
}

// ---- strptime ----

const STRPTIME_ZONE: &str = concat!(r"\A(", zone_alts!(), r")");

fn num_pattern_p(fmt: &[u8], fi: usize) -> bool {
    match fmt.get(fi) {
        Some(c) if c.is_ascii_digit() => true,
        Some(b'%') => {
            let mut j = fi + 1;
            if matches!(fmt.get(j), Some(b'E') | Some(b'O')) {
                j += 1;
            }
            fmt.get(j).is_some_and(|c| b"CDdeFGgHIjkLlMmNQRrSsTUuVvWwXxYy".contains(c))
        }
        _ => false,
    }
}
fn read_digits(s: &[u8], si: &mut usize, width: usize) -> Option<(i64, usize)> {
    let mut n: i64 = 0;
    let mut l = 0;
    while *si + l < s.len() && l < width && s[*si + l].is_ascii_digit() {
        n = n.saturating_mul(10).saturating_add((s[*si + l] - b'0') as i64);
        l += 1;
    }
    if l == 0 {
        return None;
    }
    *si += l;
    Some((n, l))
}
fn read_sign(s: &[u8], si: &mut usize) -> i64 {
    match s.get(*si) {
        Some(b'-') => {
            *si += 1;
            -1
        }
        Some(b'+') => {
            *si += 1;
            1
        }
        _ => 1,
    }
}
fn match_name(s: &[u8], si: &mut usize, names: &[&str]) -> Option<usize> {
    for (i, n) in names.iter().enumerate() {
        for cand in [*n, &n[..3]] {
            let c = cand.as_bytes();
            if s.len() >= *si + c.len() && s[*si..*si + c.len()].eq_ignore_ascii_case(c) {
                *si += c.len();
                return Some(i);
            }
        }
    }
    None
}

fn strptime_i(s: &[u8], si: &mut usize, fmt: &[u8], h: &mut Frags) -> bool {
    let mut fi = 0;
    while fi < fmt.len() {
        let c = fmt[fi];
        if c.is_ascii_whitespace() {
            while *si < s.len() && s[*si].is_ascii_whitespace() {
                *si += 1;
            }
            fi += 1;
            continue;
        }
        if c != b'%' {
            if s.get(*si) != Some(&c) {
                return false;
            }
            *si += 1;
            fi += 1;
            continue;
        }
        fi += 1;
        if matches!(fmt.get(fi), Some(b'E') | Some(b'O')) {
            fi += 1;
        }
        while fmt.get(fi) == Some(&b':') {
            fi += 1;
        }
        let Some(&d) = fmt.get(fi) else {
            if s.get(*si) != Some(&b'%') {
                return false;
            }
            *si += 1;
            break;
        };
        fi += 1;
        let next_num = num_pattern_p(fmt, fi);
        macro_rules! digits {
            ($w:expr, $lo:expr, $hi:expr) => {{
                match read_digits(s, si, $w) {
                    Some((n, _)) if n >= $lo && n <= $hi => n,
                    _ => return false,
                }
            }};
        }
        macro_rules! recur {
            ($f:expr) => {{
                if !strptime_i(s, si, $f, h) {
                    return false;
                }
            }};
        }
        let skip_space = |si: &mut usize| {
            if s.get(*si) == Some(&b' ') {
                *si += 1;
            }
        };
        match d {
            b'A' | b'a' => match match_name(s, si, &DAYS) {
                Some(i) => h.seti("wday", i as i64),
                None => return false,
            },
            b'B' | b'b' | b'h' => match match_name(s, si, &MONTHS) {
                Some(i) => h.seti("mon", i as i64 + 1),
                None => return false,
            },
            b'C' => {
                let sg = read_sign(s, si);
                let n = digits!(if next_num { 2 } else { usize::MAX }, i64::MIN, i64::MAX);
                h.seti("_cent", sg * n);
            }
            b'c' => recur!(b"%a %b %e %H:%M:%S %Y"),
            b'D' | b'x' => recur!(b"%m/%d/%y"),
            b'd' | b'e' => {
                skip_space(si);
                h.seti("mday", digits!(2, 1, 31));
            }
            b'F' => recur!(b"%Y-%m-%d"),
            b'G' => {
                let sg = read_sign(s, si);
                let n = digits!(if next_num { 4 } else { usize::MAX }, i64::MIN, i64::MAX);
                h.seti("cwyear", sg * n);
            }
            b'g' => {
                let n = digits!(2, 0, 99);
                h.seti("cwyear", n);
                if !h.has("_cent") {
                    h.seti("_cent", if n >= 69 { 19 } else { 20 });
                }
            }
            b'H' | b'k' => {
                skip_space(si);
                h.seti("hour", digits!(2, 0, 24));
            }
            b'I' | b'l' => {
                skip_space(si);
                h.seti("hour", digits!(2, 1, 12));
            }
            b'j' => h.seti("yday", digits!(3, 1, 366)),
            b'L' | b'N' => {
                let sg = read_sign(s, si);
                let w = if next_num { if d == b'L' { 3 } else { 9 } } else { 18 };
                match read_digits(s, si, w) {
                    Some((n, l)) => h.set("sec_fraction", F::R((sg * n) as i128, 10i128.pow(l as u32))),
                    None => return false,
                }
            }
            b'M' => h.seti("min", digits!(2, 0, 59)),
            b'm' => h.seti("mon", digits!(2, 1, 12)),
            b'n' | b't' => recur!(b" "),
            b'P' | b'p' => {
                let hour = match s.get(*si) {
                    Some(b'P') | Some(b'p') => 12,
                    Some(b'A') | Some(b'a') => 0,
                    _ => return false,
                };
                let is_m = |i: usize| matches!(s.get(i), Some(b'M') | Some(b'm'));
                if s.get(*si + 1) == Some(&b'.') {
                    if !is_m(*si + 2) || s.get(*si + 3) != Some(&b'.') {
                        return false;
                    }
                    *si += 4;
                } else {
                    if !is_m(*si + 1) {
                        return false;
                    }
                    *si += 2;
                }
                h.seti("_merid", hour);
            }
            b'Q' => {
                let sg = read_sign(s, si);
                let n = digits!(usize::MAX, i64::MIN, i64::MAX);
                h.set("seconds", F::R((sg * n) as i128, 1000));
            }
            b'R' => recur!(b"%H:%M"),
            b'r' => recur!(b"%I:%M:%S %p"),
            b'S' => h.seti("sec", digits!(2, 0, 60)),
            b's' => {
                let sg = read_sign(s, si);
                let n = digits!(usize::MAX, i64::MIN, i64::MAX);
                h.seti("seconds", sg * n);
            }
            b'T' | b'X' => recur!(b"%H:%M:%S"),
            b'U' => h.seti("wnum0", digits!(2, 0, 53)),
            b'W' => h.seti("wnum1", digits!(2, 0, 53)),
            b'u' => h.seti("cwday", digits!(1, 1, 7)),
            b'V' => h.seti("cweek", digits!(2, 1, 53)),
            b'v' => recur!(b"%e-%b-%Y"),
            b'w' => h.seti("wday", digits!(1, 0, 6)),
            b'Y' => {
                let sg = read_sign(s, si);
                let n = digits!(if next_num { 4 } else { usize::MAX }, i64::MIN, i64::MAX);
                h.seti("year", sg * n);
            }
            b'y' => {
                let n = digits!(2, 0, 99);
                h.seti("year", n);
                if !h.has("_cent") {
                    h.seti("_cent", if n >= 69 { 19 } else { 20 });
                }
            }
            b'Z' | b'z' => {
                let rest = String::from_utf8_lossy(&s[*si..]).into_owned();
                match match_caps(STRPTIME_ZONE, &rest) {
                    Some((_, e, c)) => {
                        let z = c[1].clone().unwrap_or_default();
                        h.set("zone", F::S(z.clone()));
                        if let Some(o) = zone_to_diff(&z) {
                            h.seti("offset", o);
                        }
                        *si += e;
                    }
                    None => return false,
                }
            }
            b'%' => {
                if s.get(*si) != Some(&b'%') {
                    return false;
                }
                *si += 1;
            }
            b'+' => recur!(b"%a %b %e %H:%M:%S %Z %Y"),
            _ => {
                if s.get(*si) != Some(&b'%') || s.get(*si + 1) != Some(&d) {
                    return false;
                }
                *si += 2;
            }
        }
    }
    true
}

fn date_strptime(s: &str, fmt: &str) -> Option<Frags> {
    let mut h = Frags::default();
    let mut si = 0;
    let b = s.as_bytes();
    if !strptime_i(b, &mut si, fmt.as_bytes(), &mut h) {
        return None;
    }
    if si < b.len() {
        h.set("leftover", F::S(String::from_utf8_lossy(&b[si..]).into_owned()));
    }
    if let Some(F::I(cent)) = h.del("_cent") {
        for k in ["cwyear", "year"] {
            if let Some(y) = h.i(k) {
                h.seti(k, y + cent * 100);
            }
        }
    }
    if let Some(F::I(merid)) = h.del("_merid") {
        if let Some(hr) = h.i("hour") {
            h.seti("hour", hr % 12 + merid);
        }
    }
    Some(h)
}

// ---- fragments -> date ----

fn rewrite_frags(h: &mut Frags) {
    let Some(secs) = h.del("seconds") else { return };
    let (mut n, d) = match secs {
        F::I(i) => (i as i128, 1i128),
        F::R(n, d) => (n, d),
        _ => return,
    };
    if let Some(of) = h.i("offset") {
        n += of as i128 * d;
    }
    let day = DAY as i128 * d;
    let days = n.div_euclid(day);
    let fr = n.rem_euclid(day);
    let secs_in = fr / d;
    h.seti("jd", UNIX_EPOCH_JD + days as i64);
    h.seti("hour", (secs_in / 3600) as i64);
    h.seti("min", (secs_in % 3600 / 60) as i64);
    h.seti("sec", (secs_in % 60) as i64);
    h.set("sec_fraction", F::R(fr % d, d));
}

fn complete_frags(h: &mut Frags, datetime: bool) {
    const TAB: &[(&str, &[&str])] = &[
        ("time", &["hour", "min", "sec"]),
        ("", &["jd"]),
        ("ordinal", &["year", "yday", "hour", "min", "sec"]),
        ("civil", &["year", "mon", "mday", "hour", "min", "sec"]),
        ("commercial", &["cwyear", "cweek", "cwday", "hour", "min", "sec"]),
        ("wday", &["wday", "hour", "min", "sec"]),
        ("wnum0", &["year", "wnum0", "wday", "hour", "min", "sec"]),
        ("wnum1", &["year", "wnum1", "wday", "hour", "min", "sec"]),
        ("", &["cwyear", "cweek", "wday", "hour", "min", "sec"]),
        ("", &["year", "wnum0", "cwday", "hour", "min", "sec"]),
        ("", &["year", "wnum1", "cwday", "hour", "min", "sec"]),
    ];
    let mut best = None;
    let mut eno = 0;
    for (i, (_, a)) in TAB.iter().enumerate() {
        let n = a.iter().filter(|k| h.has(k)).count();
        if n > eno {
            eno = n;
            best = Some(i);
        }
    }
    let today = || {
        let jd = today_jd();
        let (y, m, d) = jd_to_civil(jd);
        let (cy, cw, cd) = jd_to_commercial(jd);
        (jd, y, m, d, cy, cw, cd)
    };
    let fill_leading = |h: &mut Frags, a: &[&'static str]| {
        let t = today();
        for &k in a {
            if h.has(k) {
                break;
            }
            let v = match k {
                "year" => t.1,
                "mon" => t.2,
                "mday" => t.3,
                "cwyear" => t.4,
                "cweek" => t.5,
                "cwday" => t.6,
                "yday" => jd_to_ordinal(t.0).1,
                "wday" => wday_of(t.0),
                "wnum0" => jd_to_weeknum(t.0, 0).1,
                "wnum1" => jd_to_weeknum(t.0, 1).1,
                _ => 0,
            };
            h.seti(k, v);
        }
    };
    if let Some(i) = best {
        let (k, a) = TAB[i];
        if !k.is_empty() && a.len() != eno {
            match k {
                "ordinal" => {
                    if !h.has("year") {
                        h.seti("year", today().1);
                    }
                    if !h.has("yday") {
                        h.seti("yday", 1);
                    }
                }
                "civil" => {
                    fill_leading(h, a);
                    if !h.has("mon") {
                        h.seti("mon", 1);
                    }
                    if !h.has("mday") {
                        h.seti("mday", 1);
                    }
                }
                "commercial" => {
                    fill_leading(h, a);
                    if !h.has("cweek") {
                        h.seti("cweek", 1);
                    }
                    if !h.has("cwday") {
                        h.seti("cwday", 1);
                    }
                }
                "wday" => {
                    let jd = today().0;
                    let w = h.i("wday").unwrap_or(0);
                    h.seti("jd", jd - wday_of(jd) + w);
                }
                "wnum0" | "wnum1" => {
                    fill_leading(h, a);
                    if !h.has(k) {
                        h.seti(k, 0);
                    }
                    if !h.has("wday") {
                        h.seti("wday", if k == "wnum0" { 0 } else { 1 });
                    }
                }
                _ => {}
            }
        }
        if k == "time" && datetime && !h.has("jd") {
            h.seti("jd", today().0);
        }
    }
    if !h.has("hour") {
        h.seti("hour", 0);
    }
    if !h.has("min") {
        h.seti("min", 0);
    }
    match h.i("sec") {
        None => h.seti("sec", 0),
        Some(s) if s > 59 => h.seti("sec", 59),
        _ => {}
    }
}

fn valid_date_frags(h: &Frags) -> Option<i64> {
    if let Some(jd) = h.i("jd") {
        return Some(jd);
    }
    if let (Some(yd), Some(y)) = (h.i("yday"), h.i("year")) {
        if let Some(jd) = valid_ordinal(y, yd) {
            return Some(jd);
        }
    }
    if let (Some(d), Some(m), Some(y)) = (h.i("mday"), h.i("mon"), h.i("year")) {
        if let Some(jd) = valid_civil(y, m, d) {
            return Some(jd);
        }
    }
    let wd = h.i("cwday").or_else(|| h.i("wday").map(|w| if w == 0 { 7 } else { w }));
    if let (Some(wd), Some(w), Some(y)) = (wd, h.i("cweek"), h.i("cwyear")) {
        if let Some(jd) = valid_commercial(y, w, wd) {
            return Some(jd);
        }
    }
    let wd = h.i("wday").or_else(|| h.i("cwday").map(|w| if w == 7 { 0 } else { w }));
    if let (Some(wd), Some(w), Some(y)) = (wd, h.i("wnum0"), h.i("year")) {
        if let Some(jd) = valid_weeknum(y, w, wd, 0) {
            return Some(jd);
        }
    }
    let wd = h.i("wday").or_else(|| h.i("cwday")).map(|w| (w - 1).rem_euclid(7));
    if let (Some(wd), Some(w), Some(y)) = (wd, h.i("wnum1"), h.i("year")) {
        if let Some(jd) = valid_weeknum(y, w, wd, 1) {
            return Some(jd);
        }
    }
    None
}

fn frags_to_d(h: Option<Frags>, datetime: bool) -> Result<D, V> {
    let Some(mut h) = h else { return Err(date_err()) };
    if h.0.is_empty() {
        return Err(date_err());
    }
    let jd = if !h.has("jd") && !h.has("yday") && h.has("year") && h.has("mon") && h.has("mday") {
        let jd = valid_civil(h.i("year").unwrap_or(0), h.i("mon").unwrap_or(0), h.i("mday").unwrap_or(0));
        if datetime {
            for k in ["hour", "min"] {
                if !h.has(k) {
                    h.seti(k, 0);
                }
            }
            match h.i("sec") {
                None => h.seti("sec", 0),
                Some(s) if s > 59 => h.seti("sec", 59),
                _ => {}
            }
        }
        jd
    } else {
        rewrite_frags(&mut h);
        complete_frags(&mut h, datetime);
        valid_date_frags(&h)
    };
    let Some(jd) = jd else { return Err(date_err()) };
    if !datetime {
        return Ok(D::date(jd));
    }
    let Some((hh, mm, ss)) = valid_time(h.i("hour").unwrap_or(0), h.i("min").unwrap_or(0), h.i("sec").unwrap_or(0)) else {
        return Err(date_err());
    };
    let (jd, hh) = if hh == 24 { (jd + 1, 0) } else { (jd, hh) };
    let sf = match h.get("sec_fraction") {
        Some(F::R(n, d)) => (n * NS / d) as i64,
        Some(F::I(i)) => *i * NS as i64,
        _ => 0,
    };
    let of = h.i("offset").unwrap_or(0);
    let of = if of.abs() > DAY { 0 } else { of };
    Ok(D { jd, df: hh * 3600 + mm * 60 + ss, sf, of })
}

// ---------- Ruby-facing helpers ----------

fn civil_args(a: &[V], datetime: bool) -> Result<D, V> {
    let y = opt_int(a, 0, -4712, "year")?;
    let m = opt_int(a, 1, 1, "month")?;
    let d = opt_int(a, 2, 1, "day")?;
    let jd = valid_civil(y, m, d).ok_or_else(date_err)?;
    if datetime { time_tail(jd, a.get(3..).unwrap_or(&[])) } else { Ok(D::date(jd)) }
}

/// hour, minute, second (may be fractional) and offset arguments of the DateTime constructors.
fn time_tail(jd: i64, a: &[V]) -> Result<D, V> {
    let h = opt_int(a, 0, 0, "hour")?;
    let mi = opt_int(a, 1, 0, "minute")?;
    let (s, sf) = match a.get(2) {
        Some(&v) => {
            let n = num_of(v).ok_or_else(|| type_error("invalid second (not numeric)"))?;
            let ns = num_times(&n, NS);
            (ns.div_euclid(NS) as i64, ns.rem_euclid(NS) as i64)
        }
        None => (0, 0),
    };
    let of = match a.get(3) {
        Some(&v) => offset_arg(v)?,
        None => 0,
    };
    let (h, mi, s) = valid_time(h, mi, s).ok_or_else(date_err)?;
    let (jd, h) = if h == 24 { (jd + 1, 0) } else { (jd, h) };
    Ok(D { jd, df: h * 3600 + mi * 60 + s, sf, of })
}

fn plus(s: V, x: V) -> V {
    let d = dget(s);
    match num_of(x) {
        Some(n) => same(s, D::from_abs(d.abs() + num_times(&n, DAY_NS), d.of)),
        None => type_error("expected numeric"),
    }
}
fn add_months(s: V, n: i64) -> V {
    let d = dget(s);
    let (y, m, md) = d.civil();
    let t = y * 12 + (m - 1) + n;
    let (y2, m2) = (t.div_euclid(12), t.rem_euclid(12) + 1);
    let mut dd = md;
    let jd = loop {
        if let Some(jd) = valid_civil(y2, m2, dd) {
            break jd;
        }
        dd -= 1;
        if dd < 1 {
            return date_err();
        }
    };
    same(s, D { jd, ..d })
}
fn cmp_d(s: V, o: V) -> Option<std::cmp::Ordering> {
    let d = dget(s);
    if is_date(o) {
        return Some(d.abs().cmp(&dget(o).abs()));
    }
    // numerics compare against the astronomical Julian day number
    let x = d.abs() - DAY_NS / 2;
    match num_of(o)? {
        Num::Int(i) => Some(x.cmp(&(i * DAY_NS))),
        Num::Frac(a, b) => Some((x * b).cmp(&(a * DAY_NS))),
        Num::Flt(f) => (x as f64 / DAY_NS as f64).partial_cmp(&f),
    }
}
fn date_hash(d: &D) -> V {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    d.abs().hash(&mut h);
    mkfix((h.finish() >> 2) as i64)
}
fn step_impl(s: V, limit: V, by: V, blk: V) -> V {
    if !is_date(limit) {
        return arg_error("expected date");
    }
    let Some(n) = num_of(by) else { return type_error("expected numeric") };
    if num_is_zero(&n) {
        return arg_error("step can't be 0");
    }
    let by_ns = num_times(&n, DAY_NS);
    let lim = dget(limit).abs();
    let k = real_class(s);
    let mut cur = dget(s);
    loop {
        let ab = cur.abs();
        if (by_ns > 0 && ab > lim) || (by_ns < 0 && ab < lim) {
            break;
        }
        tri!(call_block(blk, &[dnew(k, cur)]));
        cur = D::from_abs(ab + by_ns, cur.of);
    }
    s
}
fn str_v(s: String) -> V {
    new_str_v(s)
}
fn parse_args(a: &[V], def: &str) -> Result<(String, bool), V> {
    let a = strip_kw(a);
    let s = match a.first() {
        Some(&v) => str_arg(v)?,
        None => def.to_string(),
    };
    Ok((s, a.get(1).is_none_or(|v| truthy(*v))))
}
fn frozen_strs(items: &[Option<&str>]) -> V {
    let v = items
        .iter()
        .map(|x| match x {
            Some(s) => {
                let st = str_from(s);
                hdr(st).flags |= FL_FROZEN;
                st
            }
            None => QNIL,
        })
        .collect();
    let a = ary_new(v);
    hdr(a).flags |= FL_FROZEN;
    a
}
/// Define a method whose body is a native proc, so hash keys (`hash`/`eql?`) see it as user-defined.
fn def_proc(c: *mut Class, name: &str, min: i32, max: i32, f: NativeFn) {
    let p = proc_new_native(Box::new(move |s, a, b| f(s, a, b)), min, max);
    add_method(c, sym(name), Body::Proc(p));
}

/// `Date.parse`-family class method body: string -> fragments -> Date/DateTime.
fn by_frags(klass: V, h: Option<Frags>) -> V {
    let d = ok!(frags_to_d(h, dt_class(klass)));
    dnew(klass as *mut Class, d)
}

pub fn init() {
    let r = rt();
    let dc = new_class("Date", r.c.object);
    set_global_const("Date", dc as V);
    unsafe { (*dc).includes.push(r.c.comparable) };
    let dtc = new_class("DateTime", dc);
    set_global_const("DateTime", dtc as V);
    *DATE.get() = dc;
    *DATETIME.get() = dtc;
    let err = new_class("", r.c.argument_error);
    const_set(dc, sym("Error"), err as V);
    *DATE_ERROR.get() = err;
    crate::compat::provide_feature(&["date_core"], None);

    let mut mn: Vec<Option<&str>> = vec![None];
    mn.extend(MONTHS.iter().map(|m| Some(*m)));
    const_set(dc, sym("MONTHNAMES"), frozen_strs(&mn));
    let mut amn: Vec<Option<&str>> = vec![None];
    amn.extend(MONTHS.iter().map(|m| Some(&m[..3])));
    const_set(dc, sym("ABBR_MONTHNAMES"), frozen_strs(&amn));
    const_set(dc, sym("DAYNAMES"), frozen_strs(&DAYS.iter().map(|d| Some(*d)).collect::<Vec<_>>()));
    const_set(dc, sym("ABBR_DAYNAMES"), frozen_strs(&DAYS.iter().map(|d| Some(&d[..3])).collect::<Vec<_>>()));
    const_set(dc, sym("ITALY"), mkfix(ITALY));
    const_set(dc, sym("ENGLAND"), mkfix(ENGLAND));
    const_set(dc, sym("JULIAN"), float_new(f64::INFINITY));
    const_set(dc, sym("GREGORIAN"), float_new(f64::NEG_INFINITY));

    // ---- constructors ----
    fn civil(s: V, a: &[V], _: V) -> V {
        let d = ok!(civil_args(strip_kw(a), dt_class(s)));
        dnew(s as *mut Class, d)
    }
    defs(dc, "new", 0, 8, civil);
    defs(dc, "civil", 0, 8, civil);
    defs(dc, "jd", 0, 6, |s, a, _| {
        let jd = ok!(opt_int(a, 0, 0, "jd"));
        let d = if dt_class(s) { ok!(time_tail(jd, a.get(1..).unwrap_or(&[]))) } else { D::date(jd) };
        dnew(s as *mut Class, d)
    });
    defs(dc, "ordinal", 0, 7, |s, a, _| {
        let y = ok!(opt_int(a, 0, -4712, "year"));
        let yd = ok!(opt_int(a, 1, 1, "yday"));
        let Some(jd) = valid_ordinal(y, yd) else { return date_err() };
        let d = if dt_class(s) { ok!(time_tail(jd, a.get(2..).unwrap_or(&[]))) } else { D::date(jd) };
        dnew(s as *mut Class, d)
    });
    defs(dc, "commercial", 0, 8, |s, a, _| {
        let y = ok!(opt_int(a, 0, -4712, "cwyear"));
        let w = ok!(opt_int(a, 1, 1, "cweek"));
        let wd = ok!(opt_int(a, 2, 1, "cwday"));
        let Some(jd) = valid_commercial(y, w, wd) else { return date_err() };
        let d = if dt_class(s) { ok!(time_tail(jd, a.get(3..).unwrap_or(&[]))) } else { D::date(jd) };
        dnew(s as *mut Class, d)
    });
    defs(dc, "today", 0, 1, |s, _, _| dnew(s as *mut Class, D::date(today_jd())));
    defs(dtc, "now", 0, 1, |s, _, _| {
        let (secs, ns) = now();
        let of = local_offset(secs);
        dnew(s as *mut Class, D::from_abs((secs + UNIX_EPOCH_JD * DAY) as i128 * NS + ns as i128, of))
    });

    // ---- validity / calendar predicates ----
    fn valid_civil_p(_: V, a: &[V], _: V) -> V {
        match (int_arg(a[0], "year"), int_arg(a[1], "month"), int_arg(a[2], "day")) {
            (Ok(y), Ok(m), Ok(d)) => bool_v(valid_civil(y, m, d).is_some()),
            _ => {
                pst().pending = Throw::None;
                QFALSE
            }
        }
    }
    defs(dc, "valid_date?", 3, 4, valid_civil_p);
    defs(dc, "valid_civil?", 3, 4, valid_civil_p);
    defs(dc, "valid_jd?", 1, 2, |_, a, _| bool_v(num_of(a[0]).is_some()));
    defs(dc, "valid_ordinal?", 2, 3, |_, a, _| match (int_arg(a[0], "year"), int_arg(a[1], "yday")) {
        (Ok(y), Ok(d)) => bool_v(valid_ordinal(y, d).is_some()),
        _ => type_error("expected numeric"),
    });
    defs(dc, "valid_commercial?", 3, 4, |_, a, _| match (int_arg(a[0], "year"), int_arg(a[1], "week"), int_arg(a[2], "day")) {
        (Ok(y), Ok(w), Ok(d)) => bool_v(valid_commercial(y, w, d).is_some()),
        _ => type_error("expected numeric"),
    });
    fn greg_leap_p(_: V, a: &[V], _: V) -> V {
        let y = ok!(int_arg(a[0], "year"));
        bool_v(gregorian_leap(y))
    }
    defs(dc, "leap?", 1, 1, greg_leap_p);
    defs(dc, "gregorian_leap?", 1, 1, greg_leap_p);
    defs(dc, "julian_leap?", 1, 1, |_, a, _| {
        let y = ok!(int_arg(a[0], "year"));
        bool_v(julian_leap(y))
    });

    // ---- parsing ----
    defs(dc, "_parse", 1, 3, |_, a, _| {
        let (s, comp) = ok!(parse_args(a, ""));
        date_parse(&s, comp).to_hash()
    });
    defs(dc, "parse", 0, 4, |s, a, _| {
        let (st, comp) = ok!(parse_args(a, "-4712-01-01"));
        by_frags(s, Some(date_parse(&st, comp)))
    });
    macro_rules! parser {
        ($pub:expr, $priv:expr, $f:expr, $def:expr) => {
            defs(dc, $priv, 1, 2, |_, a, _| {
                let (s, _) = ok!(parse_args(a, ""));
                $f(&s).to_hash()
            });
            defs(dc, $pub, 0, 3, |s, a, _| {
                let (st, _) = ok!(parse_args(a, $def));
                by_frags(s, Some($f(&st)))
            });
        };
    }
    parser!("iso8601", "_iso8601", date_iso8601, "-4712-01-01");
    parser!("rfc3339", "_rfc3339", date_rfc3339, "-4712-01-01T00:00:00+00:00");
    parser!("xmlschema", "_xmlschema", date_xmlschema, "-4712-01-01");
    parser!("rfc2822", "_rfc2822", date_rfc2822, "Mon, 1 Jan -4712 00:00:00 +0000");
    parser!("rfc822", "_rfc822", date_rfc2822, "Mon, 1 Jan -4712 00:00:00 +0000");
    parser!("httpdate", "_httpdate", date_httpdate, "Mon, 01 Jan -4712 00:00:00 GMT");
    parser!("jisx0301", "_jisx0301", date_jisx0301, "-4712-01-01");
    fn strptime_fmt(s: V, a: &[V]) -> Result<(String, String), V> {
        let a = strip_kw(a);
        let dt = dt_class(s);
        let st = match a.first() {
            Some(&v) => str_arg(v)?,
            None => (if dt { "-4712-01-01T00:00:00+00:00" } else { "-4712-01-01" }).to_string(),
        };
        let f = match a.get(1) {
            Some(&v) => str_arg(v)?,
            None => (if dt { "%FT%T%z" } else { "%F" }).to_string(),
        };
        Ok((st, f))
    }
    defs(dc, "_strptime", 1, 2, |s, a, _| {
        let (st, f) = ok!(strptime_fmt(s, a));
        match date_strptime(&st, &f) {
            Some(h) => h.to_hash(),
            None => QNIL,
        }
    });
    defs(dc, "strptime", 0, 3, |s, a, _| {
        let (st, f) = ok!(strptime_fmt(s, a));
        by_frags(s, date_strptime(&st, &f))
    });

    // ---- accessors ----
    defn(dc, "year", 0, 0, |s, _, _| mkfix(dget(s).civil().0));
    defn(dc, "mon", 0, 0, |s, _, _| mkfix(dget(s).civil().1));
    defn(dc, "month", 0, 0, |s, _, _| mkfix(dget(s).civil().1));
    defn(dc, "mday", 0, 0, |s, _, _| mkfix(dget(s).civil().2));
    defn(dc, "day", 0, 0, |s, _, _| mkfix(dget(s).civil().2));
    defn(dc, "wday", 0, 0, |s, _, _| mkfix(wday_of(dget(s).jd)));
    defn(dc, "yday", 0, 0, |s, _, _| mkfix(jd_to_ordinal(dget(s).jd).1));
    defn(dc, "jd", 0, 0, |s, _, _| mkfix(dget(s).jd));
    defn(dc, "mjd", 0, 0, |s, _, _| mkfix(dget(s).jd - MJD_EPOCH));
    defn(dc, "ld", 0, 0, |s, _, _| mkfix(dget(s).jd - 2299160));
    defn(dc, "ajd", 0, 0, |s, _, _| mk_ratio(dget(s).abs() * 2 - DAY_NS, DAY_NS * 2));
    defn(dc, "amjd", 0, 0, |s, _, _| mk_ratio(dget(s).abs() - MJD_EPOCH as i128 * DAY_NS, DAY_NS));
    defn(dc, "day_fraction", 0, 0, |s, _, _| {
        let d = dget(s);
        mk_ratio(d.df as i128 * NS + d.sf as i128, DAY_NS)
    });
    defn(dc, "cwday", 0, 0, |s, _, _| mkfix(jd_to_commercial(dget(s).jd).2));
    defn(dc, "cweek", 0, 0, |s, _, _| mkfix(jd_to_commercial(dget(s).jd).1));
    defn(dc, "cwyear", 0, 0, |s, _, _| mkfix(jd_to_commercial(dget(s).jd).0));
    defn(dc, "start", 0, 0, |_, _, _| float_new(ITALY as f64));
    for n in ["new_start", "italy", "england", "julian", "gregorian"] {
        defn(dc, n, 0, 1, |s, _, _| same(s, dget(s)));
    }
    defn(dc, "julian?", 0, 0, |s, _, _| bool_v(dget(s).jd < ITALY));
    defn(dc, "gregorian?", 0, 0, |s, _, _| bool_v(dget(s).jd >= ITALY));
    defn(dc, "leap?", 0, 0, |s, _, _| {
        let (y, _, _) = dget(s).civil();
        bool_v(jd_to_civil(civil_to_jd(y, 3, 1) - 1).2 == 29)
    });
    for (i, n) in ["sunday?", "monday?", "tuesday?", "wednesday?", "thursday?", "friday?", "saturday?"].iter().enumerate() {
        let f: NativeFn = match i {
            0 => |s, _, _| bool_v(wday_of(dget(s).jd) == 0),
            1 => |s, _, _| bool_v(wday_of(dget(s).jd) == 1),
            2 => |s, _, _| bool_v(wday_of(dget(s).jd) == 2),
            3 => |s, _, _| bool_v(wday_of(dget(s).jd) == 3),
            4 => |s, _, _| bool_v(wday_of(dget(s).jd) == 4),
            5 => |s, _, _| bool_v(wday_of(dget(s).jd) == 5),
            _ => |s, _, _| bool_v(wday_of(dget(s).jd) == 6),
        };
        defn(dc, n, 0, 0, f);
    }

    // ---- arithmetic & comparison ----
    defn(dc, "+", 1, 1, |s, a, _| plus(s, a[0]));
    defn(dc, "-", 1, 1, |s, a, _| {
        if is_date(a[0]) {
            return mk_ratio(dget(s).abs() - dget(a[0]).abs(), DAY_NS);
        }
        let d = dget(s);
        match num_of(a[0]) {
            Some(n) => same(s, D::from_abs(d.abs() - num_times(&n, DAY_NS), d.of)),
            None => type_error("expected numeric"),
        }
    });
    defn(dc, ">>", 1, 1, |s, a, _| {
        let n = ok!(int_arg(a[0], "month"));
        add_months(s, n)
    });
    defn(dc, "<<", 1, 1, |s, a, _| {
        let n = ok!(int_arg(a[0], "month"));
        add_months(s, -n)
    });
    defn(dc, "next_day", 0, 1, |s, a, _| plus(s, a.first().copied().unwrap_or(mkfix(1))));
    defn(dc, "prev_day", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 1, "day"));
        plus(s, mkfix(-n))
    });
    defn(dc, "succ", 0, 0, |s, _, _| plus(s, mkfix(1)));
    defn(dc, "next", 0, 0, |s, _, _| plus(s, mkfix(1)));
    defn(dc, "next_month", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 1, "month"));
        add_months(s, n)
    });
    defn(dc, "prev_month", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 1, "month"));
        add_months(s, -n)
    });
    defn(dc, "next_year", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 1, "year"));
        add_months(s, n * 12)
    });
    defn(dc, "prev_year", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 1, "year"));
        add_months(s, -n * 12)
    });
    defn(dc, "<=>", 1, 1, |s, a, _| match cmp_d(s, a[0]) {
        Some(o) => mkfix(o as i64),
        None => QNIL,
    });
    defn(dc, "==", 1, 1, |s, a, _| bool_v(cmp_d(s, a[0]) == Some(std::cmp::Ordering::Equal)));
    defn(dc, "===", 1, 1, |s, a, _| {
        let jd = dget(s).jd;
        if is_date(a[0]) {
            return bool_v(jd == dget(a[0]).jd);
        }
        match num_of(a[0]) {
            Some(Num::Int(i)) => bool_v(jd as i128 == i),
            Some(Num::Flt(f)) => bool_v(jd as f64 == f),
            _ => QFALSE,
        }
    });
    def_proc(dc, "eql?", 1, 1, |s, a, _| bool_v(is_date(a[0]) && dget(s).abs() == dget(a[0]).abs()));
    def_proc(dc, "hash", 0, 0, |s, _, _| date_hash(&dget(s)));
    defn(dc, "step", 1, 2, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "step", a);
        }
        step_impl(s, a[0], a.get(1).copied().unwrap_or(mkfix(1)), b)
    });
    defn(dc, "upto", 1, 1, |s, a, b| if is_proc(b) { step_impl(s, a[0], mkfix(1), b) } else { crate::misc::enum_for(s, "upto", a) });
    defn(dc, "downto", 1, 1, |s, a, b| if is_proc(b) { step_impl(s, a[0], mkfix(-1), b) } else { crate::misc::enum_for(s, "downto", a) });

    // ---- formatting ----
    defn(dc, "strftime", 0, 1, |s, a, _| {
        let f = match a.first() {
            Some(&v) => ok!(str_arg(v)),
            None => (if is_dt(s) { "%FT%T%:z" } else { "%F" }).to_string(),
        };
        str_new(strftime(&dget(s), f.as_bytes()))
    });
    defn(dc, "to_s", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%Y-%m-%d")));
    defn(dc, "inspect", 0, 0, |s, _, _| {
        let d = dget(s);
        let u = D::from_abs(d.abs(), 0);
        let ts = tri!(send0(s, "to_s"));
        let ts = String::from_utf8_lossy(&crate::fmt::to_s(ts).unwrap_or_default()).into_owned();
        str_v(format!("#<{}: {} (({}j,{}s,{}n),{:+}s,{}j)>", class_name(real_class(s)), ts, u.jd, u.df, u.sf, d.of, ITALY))
    });
    defn(dc, "iso8601", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%Y-%m-%d")));
    defn(dc, "xmlschema", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%Y-%m-%d")));
    defn(dc, "rfc3339", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%Y-%m-%dT%H:%M:%S%:z")));
    defn(dc, "rfc2822", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%a, %-d %b %Y %T %z")));
    defn(dc, "rfc822", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%a, %-d %b %Y %T %z")));
    defn(dc, "httpdate", 0, 0, |s, _, _| {
        let d = dget(s);
        str_v(sf_str(&D::from_abs(d.abs(), 0), "%a, %d %b %Y %T GMT"))
    });
    defn(dc, "jisx0301", 0, 0, |s, _, _| str_v(jisx0301_date(&dget(s))));
    defn(dc, "asctime", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%a %b %e %H:%M:%S %Y")));
    defn(dc, "ctime", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%a %b %e %H:%M:%S %Y")));
    defn(dc, "deconstruct_keys", 1, 1, |s, a, _| {
        let d = dget(s);
        let (y, m, md) = d.civil();
        let mut pairs: Vec<(&str, V)> =
            vec![("year", mkfix(y)), ("month", mkfix(m)), ("day", mkfix(md)), ("yday", mkfix(jd_to_ordinal(d.jd).1)), ("wday", mkfix(wday_of(d.jd)))];
        if is_dt(s) {
            pairs.push(("hour", mkfix(d.df / 3600)));
            pairs.push(("min", mkfix(d.df % 3600 / 60)));
            pairs.push(("sec", mkfix(d.df % 60)));
            pairs.push(("sec_fraction", mk_ratio(d.sf as i128, NS)));
            pairs.push(("zone", str_from(&fmt_offset(d.of, 1))));
        }
        let h = hash_new();
        let want: Option<Vec<V>> = if is_ary(a[0]) { Some(rary(a[0]).v.clone()) } else { None };
        for (k, v) in pairs {
            let key = mksym(sym(k));
            if want.as_ref().is_none_or(|w| w.contains(&key)) {
                crate::hash::hset(h, key, v);
            }
        }
        h
    });

    // ---- conversions ----
    defn(dc, "to_date", 0, 0, |s, _, _| if is_dt(s) { dnew(*DATE.get(), D::date(dget(s).jd)) } else { s });
    defn(dc, "to_datetime", 0, 0, |s, _, _| if is_dt(s) { s } else { dnew(*DATETIME.get(), dget(s)) });
    defn(dc, "to_time", 0, 0, |s, _, _| {
        let d = dget(s);
        if is_dt(s) {
            let ab = d.abs();
            let secs = ab.div_euclid(NS) as i64 - UNIX_EPOCH_JD * DAY;
            let ns = ab.rem_euclid(NS);
            return time_at(if ns == 0 { mkfix(secs) } else { float_new(secs as f64 + ns as f64 / 1e9) });
        }
        let (y, m, md) = d.civil();
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = (y - 1900) as i32;
        tm.tm_mon = (m - 1) as i32;
        tm.tm_mday = md as i32;
        tm.tm_isdst = -1;
        let t = unsafe { libc::mktime(&mut tm) };
        time_at(mkfix(t as i64))
    });
    defn(dc, "new_offset", 0, 1, |s, a, _| {
        let of = match a.first() {
            Some(&v) => ok!(offset_arg(v)),
            None => 0,
        };
        let d = dget(s);
        same(s, D::from_abs(d.abs(), of))
    });

    // Date's own hour/min/sec are private (date_core.c; DateTime's are public)
    defn(dc, "hour", 0, 0, |s, _, _| mkfix(dget(s).df / 3600));
    for name in ["min", "minute"] {
        defn(dc, name, 0, 0, |s, _, _| mkfix(dget(s).df % 3600 / 60));
    }
    for name in ["sec", "second"] {
        defn(dc, name, 0, 0, |s, _, _| mkfix(dget(s).df % 60));
    }
    for name in ["hour", "min", "minute", "sec", "second"] {
        unsafe { (*find_method(dc, sym(name)).cast_mut()).vis = VIS_PRIVATE };
    }
    // ---- DateTime ----
    defn(dtc, "hour", 0, 0, |s, _, _| mkfix(dget(s).df / 3600));
    defn(dtc, "min", 0, 0, |s, _, _| mkfix(dget(s).df % 3600 / 60));
    defn(dtc, "minute", 0, 0, |s, _, _| mkfix(dget(s).df % 3600 / 60));
    defn(dtc, "sec", 0, 0, |s, _, _| mkfix(dget(s).df % 60));
    defn(dtc, "second", 0, 0, |s, _, _| mkfix(dget(s).df % 60));
    defn(dtc, "sec_fraction", 0, 0, |s, _, _| mk_ratio(dget(s).sf as i128, NS));
    defn(dtc, "second_fraction", 0, 0, |s, _, _| mk_ratio(dget(s).sf as i128, NS));
    defn(dtc, "offset", 0, 0, |s, _, _| mk_ratio(dget(s).of as i128, DAY as i128));
    defn(dtc, "zone", 0, 0, |s, _, _| str_v(fmt_offset(dget(s).of, 1)));
    defn(dtc, "to_s", 0, 0, |s, _, _| str_v(sf_str(&dget(s), "%Y-%m-%dT%H:%M:%S%:z")));
    fn dt_iso(s: V, a: &[V], _: V) -> V {
        let n = ok!(opt_int(a, 0, 0, "digits"));
        str_v(sf_str(&dget(s), &format!("%Y-%m-%d{}", iso_time_fmt(n))))
    }
    defn(dtc, "iso8601", 0, 1, dt_iso);
    defn(dtc, "xmlschema", 0, 1, dt_iso);
    defn(dtc, "rfc3339", 0, 1, dt_iso);
    defn(dtc, "jisx0301", 0, 1, |s, a, _| {
        let n = ok!(opt_int(a, 0, 0, "digits"));
        let d = dget(s);
        str_v(format!("{}{}", jisx0301_date(&d), sf_str(&d, &iso_time_fmt(n))))
    });

    // ---- Time interop ----
    fn time_parts(t: V) -> Result<(i64, i64, i64), V> {
        let f = send0(t, "to_f");
        if f == UNDEF {
            return Err(UNDEF);
        }
        let f = crate::fmt::num_to_f(f).unwrap_or(0.0);
        let off = send0(t, "utc_offset");
        if off == UNDEF {
            return Err(UNDEF);
        }
        let secs = f.floor();
        let ns = (((f - secs) * 1e9).round() as i64).min(999_999_999);
        Ok((secs as i64, ns, if is_fix(off) { fix(off) } else { 0 }))
    }
    defn(r.c.time, "to_date", 0, 0, |s, _, _| {
        let (secs, _, off) = ok!(time_parts(s));
        dnew(*DATE.get(), D::date(UNIX_EPOCH_JD + (secs + off).div_euclid(DAY)))
    });
    defn(r.c.time, "to_datetime", 0, 0, |s, _, _| {
        let (secs, ns, off) = ok!(time_parts(s));
        dnew(*DATETIME.get(), D::from_abs((secs + UNIX_EPOCH_JD * DAY) as i128 * NS + ns as i128, off))
    });
    defn(r.c.time, "to_time", 0, 0, |s, _, _| s);
}
