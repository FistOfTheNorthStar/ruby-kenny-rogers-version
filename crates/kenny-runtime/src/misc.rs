//! Range, Proc, Enumerable, Enumerator, Struct, Time, Math, Process (clock), ENV, Mutex, Thread.current.

use crate::core::*;
use crate::gc::RVec;
use crate::fmt::*;
use crate::kernel::{new_str_v, sym_arg};
use crate::numeric::{arith, num_cmp, Op};
use crate::tri;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The Ruby a program was built against: its standard library and gems are compiled in, and it
/// reports this version (RUBY_VERSION ...). Kenny implements Ruby 3.4 and 4.0; a program built
/// against 4.0 or later gets 4.0's behavior where the two differ (see `ruby4`).
pub struct RubyRelease {
    pub version: String,
    pub major: u32,
    pub minor: u32,
    pub teeny: u32,
    pub patchlevel: i64,
    pub release_date: String,
}

static RELEASE: std::sync::OnceLock<RubyRelease> = std::sync::OnceLock::new();
static RUBY4: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by the program's `main` before the runtime starts: "VERSION PATCHLEVEL RELEASE_DATE"
/// of the Ruby `kenny build` used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_ruby(p: *const u8, len: u64) {
    let line = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, len as usize) }).into_owned();
    let mut w = line.split_whitespace();
    let (Some(version), patchlevel, date) = (w.next(), w.next(), w.next()) else { return };
    let r = release_of(version, patchlevel.and_then(|p| p.parse().ok()).unwrap_or(0), date.unwrap_or(""));
    RUBY4.store(r.major >= 4, std::sync::atomic::Ordering::Relaxed);
    let _ = RELEASE.set(r);
}

fn release_of(version: &str, patchlevel: i64, date: &str) -> RubyRelease {
    let n: Vec<u32> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    RubyRelease {
        version: version.to_string(),
        major: n.first().copied().unwrap_or(3),
        minor: n.get(1).copied().unwrap_or(4),
        teeny: n.get(2).copied().unwrap_or(0),
        patchlevel,
        release_date: date.to_string(),
    }
}

/// The Ruby the program was built against (Ruby 3.4.12 when the build named none).
pub fn ruby_release() -> &'static RubyRelease {
    RELEASE.get_or_init(|| release_of("3.4.12", 148, "2026-10-23"))
}

/// Whether the program has Ruby 4.0's behavior (built against Ruby 4.0 or later).
#[inline]
pub fn ruby4() -> bool {
    RUBY4.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------- iteration helpers ----------

fn yielded(args: &[V]) -> V {
    if args.len() == 1 { args[0] } else { ary_new(args.to_vec()) }
}

/// Collect everything `each` yields.
pub fn each_to_vec(v: V) -> Result<Vec<V>, V> {
    if is_ary(v) {
        return Ok(rary(v).v.clone());
    }
    if is_hash(v) {
        return Ok(crate::hash::pairs(v).into_iter().map(|(k, x)| ary_new(vec![k, x])).collect());
    }
    if is_range(v) {
        if let Some(items) = range_items(v) {
            return Ok(items);
        }
    }
    let acc = Rc::new(RefCell::new(RVec::new()));
    let a2 = acc.clone();
    let nb = proc_new_native(
        Box::new(move |_, args, _| {
            a2.borrow_mut().push(yielded(args));
            QNIL
        }),
        0,
        -1,
    );
    rproc(nb).autosplat = false; // a yielded [x] must stay an array
    let r = send(v, sym("each"), &[], nb);
    if r == UNDEF {
        return Err(UNDEF);
    }
    let out = acc.borrow_mut().take();
    Ok(out)
}

static STOP_TAG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 40);

/// Iterate `each`, stopping early when `f` returns Ok(true).
pub fn each_until(v: V, mut f: impl FnMut(V) -> Result<bool, V> + 'static) -> Result<(), V> {
    each_args_until(v, move |a| f(yielded(a)))
}

/// Iterate `each` with the values of each yield as given (several for `yield a, b`),
/// stopping early when `f` returns Ok(true).
pub fn each_args_until(v: V, f: impl FnMut(&[V]) -> Result<bool, V> + 'static) -> Result<(), V> {
    let tag = STOP_TAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let f = Rc::new(RefCell::new(f));
    let nb = proc_new_native(
        Box::new(move |_, args, _| match (f.borrow_mut())(args) {
            Ok(false) => QNIL,
            Ok(true) => {
                pst().pending = Throw::Break { tag, val: QNIL };
                UNDEF
            }
            Err(e) => e,
        }),
        0,
        -1,
    );
    rproc(nb).autosplat = false; // a yielded [x] must stay an array
    let r = send(v, sym("each"), &[], nb);
    if r == UNDEF {
        if let Throw::Break { tag: t, .. } = pst().pending {
            if t == tag {
                pst().pending = Throw::None;
                return Ok(());
            }
        }
        return Err(UNDEF);
    }
    Ok(())
}

/// What Enumerable#find returns when nothing matched: `ifnone.call`, or nil.
pub fn find_ifnone(a: &[V]) -> V {
    match a.first() {
        Some(&f) if f != QNIL => send(f, sym("call"), &[], QNIL),
        _ => QNIL,
    }
}

pub fn enum_for(recv: V, meth: &str, args: &[V]) -> V {
    let e = object_new(rt().c.enumerator);
    ivar_set(e, sym("@__recv"), recv);
    ivar_set(e, sym("@__meth"), mksym(sym(meth)));
    ivar_set(e, sym("@__args"), ary_new(args.to_vec()));
    e
}
/// Enumerator#size of `recv.m(*args)` without a given size.
fn enum_size(recv: V, m: &str, args: &[V]) -> V {
    let inf = || float_new(f64::INFINITY);
    // the receiver's size, nil when it has none (or it raises)
    let size = || -> V {
        if !respond_to(recv, sym("size")) {
            return QNIL;
        }
        if is_range(recv) && !is_fix(rrange(recv).lo) {
            return QNIL;
        }
        let r = send(recv, sym("size"), &[], QNIL);
        if r == UNDEF {
            pst().pending = Throw::None;
            return QNIL;
        }
        r
    };
    let n_arg = |i: usize| args.get(i).filter(|x| is_fix(**x)).map(|x| fix(*x));
    match m {
        "loop" => inf(),
        "each" | "map" | "collect" | "flat_map" | "collect_concat" | "select" | "filter" | "filter_map" | "reject"
        | "each_with_index" | "each_entry" | "sort_by" | "min_by" | "max_by" | "minmax_by" | "group_by" | "partition"
        | "each_with_object" | "with_index" | "with_object" | "each_index" | "reverse_each" | "each_key" | "each_value"
        | "each_pair" | "delete_if" | "keep_if" | "select!" | "filter!" | "reject!" | "map!" | "collect!" | "each_char"
        | "chars" | "each_codepoint" | "codepoints" | "each_grapheme_cluster" | "transform_values" | "transform_keys"
        | "transform_values!" | "transform_keys!" | "filter_map!" | "sum" | "uniq" | "to_h" | "each_child" => {
            size()
        }
        "each_byte" | "bytes" => send(recv, sym("bytesize"), &[], QNIL),
        "times" if is_fix(recv) => mkfix(fix(recv).max(0)),
        "upto" if is_fix(recv) => n_arg(0).map(|t| mkfix((t - fix(recv) + 1).max(0))).unwrap_or(QNIL),
        "downto" if is_fix(recv) => n_arg(0).map(|t| mkfix((fix(recv) - t + 1).max(0))).unwrap_or(QNIL),
        "each_slice" | "each_cons" => {
            let (Some(k), sz) = (n_arg(0), size()) else { return QNIL };
            if !is_fix(sz) || k <= 0 {
                return QNIL;
            }
            let n = fix(sz);
            mkfix(if m == "each_slice" { (n + k - 1) / k } else { (n - k + 1).max(0) })
        }
        "cycle" => {
            let sz = size();
            if !is_fix(sz) {
                return QNIL;
            }
            match args.first() {
                None => if fix(sz) == 0 { mkfix(0) } else { inf() },
                Some(&x) if x == QNIL => if fix(sz) == 0 { mkfix(0) } else { inf() },
                Some(&x) if is_fix(x) => mkfix(fix(sz) * fix(x).max(0)),
                _ => QNIL,
            }
        }
        _ => QNIL,
    }
}
fn enum_parts(e: V) -> (V, u32, Vec<V>) {
    let args = ivar_get(e, sym("@__args"));
    (
        ivar_get(e, sym("@__recv")),
        sym_id(ivar_get(e, sym("@__meth"))),
        if is_ary(args) { rary(args).v.clone() } else { vec![] },
    )
}

// ---------- Range ----------

fn range_items(r: V) -> Option<Vec<V>> {
    let rg = rrange(r);
    if is_fix(rg.lo) && is_fix(rg.hi) {
        let hi = fix(rg.hi) - if rg.excl { 1 } else { 0 };
        return Some((fix(rg.lo)..=hi).map(mkfix).collect());
    }
    if is_str(rg.lo) && is_str(rg.hi) {
        // (String#upto's elements: CRuby's String ranges iterate with it)
        let en = send(rg.lo, sym("enum_for"), &[mksym(sym("upto")), rg.hi, bool_v(rg.excl)], QNIL);
        let a = if en == UNDEF { UNDEF } else { send0(en, "to_a") };
        if a == UNDEF || !is_ary(a) {
            return Some(Vec::new());
        }
        return Some(rary(a).v.to_vec());
    }
    None
}

fn range_each(s: V, b: V) -> V {
    let rg = rrange(s);
    if is_fix(rg.lo) && (is_fix(rg.hi) || rg.hi == QNIL) {
        let mut i = fix(rg.lo);
        let hi = if rg.hi == QNIL { i64::MAX } else { fix(rg.hi) - if rg.excl { 1 } else { 0 } };
        while i <= hi {
            tri!(call_block(b, &[mkfix(i)]));
            i += 1;
        }
        return s;
    }
    if is_float(rg.lo) {
        return type_error("can't iterate from Float");
    }
    if is_int(rg.lo) {
        let mut i = rg.lo;
        loop {
            if rg.hi != QNIL {
                let c = num_cmp(i, rg.hi).unwrap();
                if c.is_gt() || (rg.excl && c.is_eq()) {
                    break;
                }
            }
            tri!(call_block(b, &[i]));
            i = tri!(arith(Op::Add, i, mkfix(1)));
        }
        return s;
    }
    match range_items(s) {
        Some(items) => {
            let items = RVec::from(items);
            for i in 0..items.len() {
                tri!(call_block(b, &[items[i]]));
            }
            s
        }
        None => {
            // generic: succ until > hi
            let mut cur = rg.lo;
            loop {
                let c = tri!(send1(cur, "<=>", rg.hi));
                if !is_fix(c) || fix(c) > 0 || (rg.excl && fix(c) == 0) {
                    break;
                }
                tri!(call_block(b, &[cur]));
                cur = tri!(send0(cur, "succ"));
            }
            s
        }
    }
}

/// a <=> b as -1/0/1, or None when they don't compare (CRuby's r_less: INT_MAX)
fn r_less(a: V, b: V) -> Result<Option<i64>, V> {
    let c = send1(a, "<=>", b);
    if c == UNDEF {
        return Err(c);
    }
    if c == QNIL {
        return Ok(None);
    }
    if is_fix(c) {
        return Ok(Some(fix(c).signum()));
    }
    let gt = send1(c, ">", mkfix(0));
    if gt == UNDEF {
        return Err(gt);
    }
    if truthy(gt) {
        return Ok(Some(1));
    }
    let lt = send1(c, "<", mkfix(0));
    if lt == UNDEF {
        return Err(lt);
    }
    Ok(Some(if truthy(lt) { -1 } else { 0 }))
}

/// Range#cover?(range): CRuby's r_cover_range_p
fn range_cover_range(s: V, v: V) -> V {
    macro_rules! tri_r {
        ($e:expr) => {
            match $e {
                Ok(x) => x,
                Err(e) => return e,
            }
        };
    }
    let (r, o) = (rrange(s), rrange(v));
    if (r.hi != QNIL && o.hi == QNIL) || (r.lo != QNIL && o.lo == QNIL) {
        return QFALSE;
    }
    // an empty (or backwards) range is covered by none
    if o.lo != QNIL && o.hi != QNIL {
        match tri_r!(r_less(o.lo, o.hi)) {
            None => return QFALSE,
            Some(c) if c > if o.excl { -1 } else { 0 } => return QFALSE,
            _ => {}
        }
    }
    if o.lo != QNIL && !truthy(tri!(range_cover(s, o.lo))) {
        return QFALSE;
    }
    let cmp_end = match tri_r!(r_less(r.hi, o.hi)) {
        Some(c) => c,
        None => return QFALSE,
    };
    if r.excl == o.excl {
        return bool_v(cmp_end >= 0);
    }
    if r.excl {
        return bool_v(cmp_end > 0);
    }
    if cmp_end >= 0 {
        return QTRUE;
    }
    // (1..5).cover?(1...6): the other's last element
    let max = send0(v, "max");
    if max == UNDEF {
        if let Throw::Raise(e) = pst().pending {
            if is_a(e, rt().c.type_error) {
                pst().pending = Throw::None;
                return QFALSE;
            }
        }
        return UNDEF;
    }
    if max == QNIL {
        return QFALSE;
    }
    match tri_r!(r_less(r.hi, max)) {
        Some(c) => bool_v(c >= 0),
        None => QFALSE,
    }
}

fn range_cover(s: V, x: V) -> V {
    let rg = rrange(s);
    if rg.lo != QNIL {
        let c = tri!(send1(rg.lo, "<=>", x));
        if !is_fix(c) || fix(c) > 0 {
            return QFALSE;
        }
    }
    if rg.hi != QNIL {
        let c = tri!(send1(x, "<=>", rg.hi));
        if !is_fix(c) || fix(c) > 0 || (rg.excl && fix(c) == 0) {
            return QFALSE;
        }
    }
    QTRUE
}

fn range_to_s(s: V, insp: bool) -> V {
    let rg = rrange(s);
    let f = |v: V| -> Result<String, ()> {
        if v == QNIL && insp {
            return Ok(String::new());
        }
        Ok(String::from_utf8_lossy(&if insp { inspect(v)? } else { to_s(v)? }).into_owned())
    };
    match (f(rg.lo), f(rg.hi)) {
        (Ok(a), Ok(b)) => new_str_v(format!("{}{}{}", a, if rg.excl { "..." } else { ".." }, b)),
        _ => UNDEF,
    }
}

// ---------- Struct ----------

fn struct_members(s: V) -> Vec<u32> {
    unsafe { (*real_class(s)).struct_members.clone() }
}
fn ivname(m: u32) -> u32 {
    sym(&format!("@{}", sym_str(m)))
}
fn struct_values(s: V) -> Vec<V> {
    struct_members(s).into_iter().map(|m| ivar_get(s, ivname(m))).collect()
}
fn struct_index(s: V, k: V) -> Result<u32, V> {
    let ms = struct_members(s);
    if is_fix(k) {
        let i = fix(k);
        let i = if i < 0 { i + ms.len() as i64 } else { i };
        return ms.get(i as usize).copied().ok_or_else(|| raise(rt().c.index_error, &format!("offset {} too large for struct(size:{})", fix(k), ms.len())));
    }
    let id = sym_arg(k)?;
    if ms.contains(&id) { Ok(id) } else { Err(raise(rt().c.name_error, &format!("no member '{}' in struct", sym_str(id)))) }
}

// ---------- Time ----------

#[repr(C)]
pub struct Tm {
    pub tm_sec: i32,
    pub tm_min: i32,
    pub tm_hour: i32,
    pub tm_mday: i32,
    pub tm_mon: i32,
    pub tm_year: i32,
    pub tm_wday: i32,
    pub tm_yday: i32,
    pub tm_isdst: i32,
    pub tm_gmtoff: i64,
    pub tm_zone: *const i8,
}
unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}
pub fn time_secs(t: V) -> f64 {
    num_to_f(ivar_get(t, sym("@__t"))).unwrap_or(0.0)
}
/// Exact time in nanoseconds since the epoch (`@__ns`, set by extra.rs), else from the float.
pub fn time_ns(t: V) -> i128 {
    let ns = ivar_get(t, sym("@__ns"));
    if is_fix(ns) {
        return fix(ns) as i128;
    }
    if is_big(ns) {
        use num_traits::ToPrimitive;
        return rbig(ns).n.to_i128().unwrap_or(0);
    }
    (time_secs(t) * 1e9).round() as i128
}
pub fn time_new(secs: f64, utc: bool) -> V {
    let t = object_new(rt().c.time);
    ivar_set(t, sym("@__t"), float_new(secs));
    ivar_set(t, sym("@__utc"), bool_v(utc));
    t
}
/// Broken-down time: UTC, a fixed offset (`@__off`), or the local zone.
pub fn tm_of(t: V) -> Tm {
    let secs = time_ns(t).div_euclid(1_000_000_000) as i64;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    let off = ivar_get(t, sym("@__off"));
    unsafe {
        if truthy(ivar_get(t, sym("@__utc"))) {
            gmtime_r(&secs, &mut tm);
        } else if is_fix(off) {
            let shifted = secs + fix(off);
            gmtime_r(&shifted, &mut tm);
            tm.tm_gmtoff = fix(off);
            tm.tm_zone = std::ptr::null();
        } else {
            localtime_r(&secs, &mut tm);
        }
    }
    tm
}
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
pub fn strftime(t: V, f: &str) -> String {
    let tm = tm_of(t);
    let ns = time_ns(t);
    let subns = ns.rem_euclid(1_000_000_000) as i64;
    let utc = truthy(ivar_get(t, sym("@__utc")));
    let off = tm.tm_gmtoff;
    let zone = if utc {
        "UTC".to_string()
    } else if tm.tm_zone.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(tm.tm_zone).to_string_lossy().into_owned() }
    };
    let mut out = String::new();
    let mut it = f.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut flag = None;
        if matches!(it.peek(), Some('-') | Some('0') | Some('^')) {
            flag = it.next();
        }
        let mut colons = 0;
        while it.peek() == Some(&':') {
            it.next();
            colons += 1;
        }
        let mut width: Option<usize> = None;
        while let Some(dg) = it.peek().and_then(|c| c.to_digit(10)) {
            it.next();
            width = Some(width.unwrap_or(0) * 10 + dg as usize);
        }
        let Some(d) = it.next() else { break };
        let pad2 = |n: i32| if flag == Some('-') { n.to_string() } else { format!("{:02}", n) };
        let s = match d {
            'Y' => (tm.tm_year + 1900).to_string(),
            'C' => ((tm.tm_year + 1900) / 100).to_string(),
            'y' => format!("{:02}", (tm.tm_year + 1900) % 100),
            'm' => pad2(tm.tm_mon + 1),
            'd' => pad2(tm.tm_mday),
            'e' => format!("{:>2}", tm.tm_mday),
            'j' => format!("{:03}", tm.tm_yday + 1),
            'H' => pad2(tm.tm_hour),
            'k' => format!("{:>2}", tm.tm_hour),
            'I' => pad2(if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 }),
            'l' => format!("{:>2}", if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 }),
            'M' => pad2(tm.tm_min),
            'S' => pad2(tm.tm_sec),
            'L' | 'N' => {
                let digits = format!("{:09}", subns);
                let w = width.unwrap_or(if d == 'L' { 3 } else { 9 });
                if w <= 9 { digits[..w].to_string() } else { format!("{digits}{}", "0".repeat(w - 9)) }
            }
            'p' => (if tm.tm_hour < 12 { "AM" } else { "PM" }).to_string(),
            'P' => (if tm.tm_hour < 12 { "am" } else { "pm" }).to_string(),
            'B' => MONTHS[tm.tm_mon as usize].to_string(),
            'b' | 'h' => MONTHS[tm.tm_mon as usize][..3].to_string(),
            'A' => DAYS[tm.tm_wday as usize].to_string(),
            'a' => DAYS[tm.tm_wday as usize][..3].to_string(),
            'u' => (if tm.tm_wday == 0 { 7 } else { tm.tm_wday }).to_string(),
            'w' => tm.tm_wday.to_string(),
            'z' => {
                let (sg, h, m, sec) = (if off < 0 { '-' } else { '+' }, off.abs() / 3600, (off.abs() % 3600) / 60, off.abs() % 60);
                match colons {
                    0 => format!("{sg}{h:02}{m:02}"),
                    1 => format!("{sg}{h:02}:{m:02}"),
                    _ => format!("{sg}{h:02}:{m:02}:{sec:02}"),
                }
            }
            'Z' => zone.clone(),
            's' => ns.div_euclid(1_000_000_000).to_string(),
            'F' => format!("{}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday),
            'T' | 'X' => format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec),
            'D' | 'x' => format!("{:02}/{:02}/{:02}", tm.tm_mon + 1, tm.tm_mday, (tm.tm_year + 1900) % 100),
            'R' => format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
            '%' => "%".into(),
            other => format!("%{other}"),
        };
        out.push_str(&if flag == Some('^') { s.to_uppercase() } else { s });
    }
    out
}

// ---------- helpers ----------

fn path_arg(v: V) -> Result<String, V> {
    if is_str(v) { Ok(String::from_utf8_lossy(&rstr(v).b).into_owned()) } else { Err(type_error("no implicit conversion into String")) }
}
fn has_kw(a: &[V], key: &str) -> bool {
    a.iter().any(|h| is_hash(*h) && rhash(*h).m.get(&HKey(mksym(sym(key)))).is_some_and(|v| truthy(*v)))
}

/// Float value of a Math function argument (Integer, Float or BigDecimal).
fn math_f(v: V) -> Option<f64> {
    num_to_f(v).or_else(|| crate::bigdecimal::to_f(v))
}

pub fn init() {
    let r = rt();
    let c = r.c;

    // ---- Range ----
    let rg = c.range;
    defs(rg, "new", 2, 3, |_, a, _| range_new(a[0], a[1], a.get(2).is_some_and(|x| truthy(*x))));
    defn(rg, "each", 0, 0, |s, _, b| if is_proc(b) { range_each(s, b) } else { enum_for(s, "each", &[]) });
    defn(rg, "reverse_each", 0, 0, |s, _, b| match range_items(s) {
        Some(items) => {
            let items = RVec::from(items);
            for i in (0..items.len()).rev() {
                tri!(call_block(b, &[items[i]]));
            }
            s
        }
        None => type_error("can't iterate"),
    });
    defn(rg, "step", 1, 1, |s, a, b| {
        let rgv = rrange(s);
        if !is_proc(b) {
            return enum_for(s, "step", a);
        }
        // a Float step makes every value a Float, as in CRuby
        let mut i = if is_float(a[0]) && (is_fix(rgv.lo) || is_big(rgv.lo)) { tri!(send0(rgv.lo, "to_f")) } else { rgv.lo };
        loop {
            if rgv.hi != QNIL {
                let c = match num_cmp(i, rgv.hi) {
                    Some(c) => c,
                    // Rational, BigDecimal, ...: their own <=>
                    None => {
                        let c = tri!(send1(i, "<=>", rgv.hi));
                        if !is_fix(c) {
                            return type_error("step on non-numeric range");
                        }
                        fix(c).cmp(&0)
                    }
                };
                if c.is_gt() || (rgv.excl && c.is_eq()) {
                    break;
                }
            }
            tri!(call_block(b, &[i]));
            let builtin = |v: V| is_fix(v) || is_float(v) || is_big(v);
            i = if builtin(i) && builtin(a[0]) { tri!(arith(Op::Add, i, a[0])) } else { tri!(send1(i, "+", a[0])) };
        }
        s
    });
    defn(rg, "%", 1, 1, |s, a, _| enum_for(s, "step", a));
    defn(rg, "to_a", 0, 0, |s, _, _| match each_to_vec(s) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(rg, "to_ary", 0, 0, |s, _, _| send0(s, "to_a"));
    defn(rg, "entries", 0, 0, |s, _, _| send0(s, "to_a"));
    defn(rg, "size", 0, 0, |s, _, _| {
        let r = rrange(s);
        if is_fix(r.lo) && is_fix(r.hi) {
            mkfix((fix(r.hi) - fix(r.lo) + if r.excl { 0 } else { 1 }).max(0))
        } else if r.hi == QNIL {
            float_new(f64::INFINITY)
        } else {
            QNIL
        }
    });
    defn(rg, "count", 0, 1, |s, a, b| {
        let r = rrange(s);
        if a.is_empty() && !is_proc(b) && is_fix(r.lo) && is_fix(r.hi) {
            return mkfix((fix(r.hi) - fix(r.lo) + if r.excl { 0 } else { 1 }).max(0));
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("count"), a, b),
            Err(e) => e,
        }
    });
    defn(rg, "sum", 0, 1, |s, a, b| {
        let r = rrange(s);
        if a.is_empty() && !is_proc(b) && is_fix(r.lo) && is_fix(r.hi) {
            let (lo, hi) = (fix(r.lo) as i128, fix(r.hi) as i128 - if r.excl { 1 } else { 0 });
            if hi < lo {
                return mkfix(0);
            }
            return int_from_i128((lo + hi) * (hi - lo + 1) / 2);
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("sum"), a, b),
            Err(e) => e,
        }
    });
    defn(rg, "first", 0, 1, |s, a, _| {
        let r = rrange(s);
        if a.is_empty() {
            return r.lo;
        }
        let n = fix(a[0]).max(0) as usize;
        let acc = Rc::new(RefCell::new(RVec::new()));
        let a2 = acc.clone();
        if n == 0 {
            return ary_new(vec![]);
        }
        if let Err(e) = each_until(s, move |x| {
            a2.borrow_mut().push(x);
            Ok(a2.borrow().len() >= n)
        }) {
            return e;
        }
        ary_new(acc.borrow_mut().take())
    });
    defn(rg, "begin", 0, 0, |s, _, _| rrange(s).lo);
    defn(rg, "end", 0, 0, |s, _, _| rrange(s).hi);
    defn(rg, "last", 0, 1, |s, a, _| {
        if a.is_empty() {
            return rrange(s).hi;
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("last"), a, QNIL),
            Err(e) => e,
        }
    });
    // (as CRuby's range_min / range_max)
    fn numeric_end(v: V) -> bool {
        is_fix(v) || is_a(v, rt().c.numeric)
    }
    defn(rg, "min", 0, 1, |s, a, b| {
        let r = rrange(s);
        if r.lo == QNIL {
            return raise(rt().c.range_error, "cannot get the minimum of beginless range");
        }
        if is_proc(b) || (r.excl && !numeric_end(r.hi)) || !a.is_empty() {
            if r.hi == QNIL {
                return raise(rt().c.range_error, "cannot get the minimum of endless range with custom comparison method");
            }
        } else if r.hi == QNIL {
            return r.lo;
        } else {
            let c = tri!(send1(r.lo, "<=>", r.hi));
            return if is_fix(c) && (fix(c) < 0 || (fix(c) == 0 && !r.excl)) { r.lo } else { QNIL };
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("min"), a, b),
            Err(e) => e,
        }
    });
    defn(rg, "max", 0, 1, |s, a, b| {
        let r = rrange(s);
        if r.hi == QNIL {
            return raise(rt().c.range_error, "cannot get the maximum of endless range");
        }
        if is_proc(b) || (r.excl && !numeric_end(r.hi)) || !a.is_empty() {
            if r.lo == QNIL {
                return raise(rt().c.range_error, "cannot get the maximum of beginless range with custom comparison method");
            }
        } else {
            let c = if r.lo == QNIL { mkfix(-1) } else { tri!(send1(r.lo, "<=>", r.hi)) };
            if !is_fix(c) || fix(c) > 0 {
                return QNIL;
            }
            if r.excl {
                if !is_int(r.hi) {
                    return type_error("cannot exclude non Integer end value");
                }
                if fix(c) == 0 {
                    return QNIL;
                }
                // (Ruby 4.0: a beginless range's too)
                if !is_int(r.lo) && !(r.lo == QNIL && ruby4()) {
                    return type_error("cannot exclude end value with non Integer begin value");
                }
                return send(r.hi, sym("-"), &[mkfix(1)], QNIL);
            }
            return r.hi;
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("max"), a, b),
            Err(e) => e,
        }
    });
    // (as CRuby's range_minmax: min and max without iterating, so `(0..Float::INFINITY).minmax`
    // is [0, Infinity]; with a block, Enumerable's)
    defn(rg, "minmax", 0, 0, |s, _, b| {
        if is_proc(b) {
            return match each_to_vec(s) {
                Ok(v) => send(ary_new(v), sym("minmax"), &[], b),
                Err(e) => e,
            };
        }
        let lo = tri!(send(s, sym("min"), &[], QNIL));
        let hi = tri!(send(s, sym("max"), &[], QNIL));
        ary_new(vec![lo, hi])
    });
    if ruby4() {
        defn(rg, "to_set", 0, -1, |s, a, b| {
            if rrange(s).hi == QNIL {
                return raise(rt().c.range_error, "cannot convert endless range to a set");
            }
            let m = find_method(rt().c.enumerable, sym("to_set"));
            invoke(m, s, a, b)
        });
    }
    // overlap?(range): whether some value is in both (as CRuby's range_overlap)
    defn(rg, "overlap?", 1, 1, |s, a, _| {
        if !is_range(a[0]) {
            return type_error(&format!("wrong argument type {} (expected Range)", class_name(real_class(a[0]))));
        }
        let (x, y) = (rrange(s), rrange(a[0]));
        // <=> of a and b (i32::MAX: not comparable)
        fn less(a: V, b: V) -> Result<i64, V> {
            let c = send1(a, "<=>", b);
            if c == UNDEF {
                return Err(c);
            }
            if c == QNIL {
                return Ok(i32::MAX as i64);
            }
            let n = send1(c, "<=>", mkfix(0));
            if n == UNDEF {
                return Err(n);
            }
            Ok(if is_fix(n) { fix(n) } else { 0 })
        }
        fn empty(lo: V, hi: V, excl: bool) -> Result<bool, V> {
            if lo == QNIL || hi == QNIL {
                return Ok(false);
            }
            let l = less(lo, hi)?;
            Ok(l > 0 || (excl && l == 0))
        }
        let r = (|| -> Result<bool, V> {
            if empty(x.lo, y.hi, y.excl)? || empty(y.lo, x.hi, x.excl)? {
                return Ok(false);
            }
            if x.lo != QNIL && y.lo != QNIL {
                let c = send1(x.lo, "<=>", y.lo);
                if c == UNDEF {
                    return Err(c);
                }
                if c == QNIL {
                    return Ok(false);
                }
                if less(x.lo, y.lo)? == 0 {
                    return Ok(true);
                }
            }
            Ok(!empty(x.lo, x.hi, x.excl)? && !empty(y.lo, y.hi, y.excl)?)
        })();
        match r {
            Ok(b) => bool_v(b),
            Err(e) => e,
        }
    });
    // include?/member? of a String range: whether succ from the beginning reaches the value
    // (CRuby's rb_str_include_range_p), not cover?
    for name in ["include?", "member?"] {
        defn(rg, name, 1, 1, |s, a, _| {
            let r = rrange(s);
            if is_str(r.lo) && is_str(r.hi) {
                let v = if is_str(a[0]) {
                    a[0]
                } else if is_heap(a[0]) && respond_to(a[0], sym("to_str")) {
                    tri!(send0(a[0], "to_str"))
                } else {
                    return QFALSE;
                };
                let en = tri!(send(r.lo, sym("enum_for"), &[mksym(sym("upto")), r.hi, bool_v(r.excl)], QNIL));
                return send(en, sym("include?"), &[v], QNIL);
            }
            if (is_str(r.lo) && r.hi == QNIL) || (r.lo == QNIL && (is_str(r.hi) || r.hi == QNIL)) {
                return type_error("cannot determine inclusion in beginless/endless ranges");
            }
            range_cover(s, a[0])
        });
    }
    // (=== with a Range compares it as one value, as CRuby's does)
    defn(rg, "===", 1, 1, |s, a, _| range_cover(s, a[0]));
    defn(rg, "cover?", 1, 1, |s, a, _| {
        if is_range(a[0]) {
            return range_cover_range(s, a[0]);
        }
        range_cover(s, a[0])
    });
    defn(rg, "exclude_end?", 0, 0, |s, _, _| bool_v(rrange(s).excl));
    defn(rg, "to_s", 0, 0, |s, _, _| range_to_s(s, false));
    defn(rg, "inspect", 0, 0, |s, _, _| range_to_s(s, true));
    defn(rg, "==", 1, 1, |s, a, _| bool_v(is_range(a[0]) && eql(s, a[0])));
    defn(rg, "eql?", 1, 1, |s, a, _| bool_v(is_range(a[0]) && eql(s, a[0])));
    defn(rg, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(rg, "dup", 0, 0, |s, _, _| s);

    // ---- Proc ----
    let pc = c.proc_;
    defs(pc, "new", 0, 0, |_, _, b| {
        if !is_proc(b) {
            return arg_error("tried to create Proc object without a block");
        }
        b
    });
    for name in ["call", "()", "yield", "[]", "==="] {
        defn(pc, name, 0, -1, |s, a, b| call_block_with_blk(s, a, b));
    }
    defn(pc, "to_proc", 0, 0, |s, _, _| s);
    defn(pc, "lambda?", 0, 0, |s, _, _| bool_v(rproc(s).lambda));
    defn(pc, "arity", 0, 0, |s, _, _| {
        if let Some(d) = crate::methodobj::block_desc(s) {
            return mkfix(crate::methodobj::desc_arity(&d, rproc(s).lambda));
        }
        let p = rproc(s);
        if p.max < 0 || (p.min != p.max && p.lambda) { mkfix(-(p.min as i64) - 1) } else { mkfix(p.min as i64) }
    });
    defn(pc, "parameters", 0, 1, |s, a, _| {
        // parameters(lambda: true/false)
        let lambda = a.first().filter(|h| is_hash(**h)).map(|h| crate::hash::hget(*h, mksym(sym("lambda")))).filter(|v| *v != QNIL).map(truthy);
        crate::methodobj::proc_parameters(s, lambda)
    });
    defn(pc, "curry", 0, 1, |s, a, _| {
        let p = rproc(s);
        let n = a.first().map(|x| fix(*x) as usize).unwrap_or(p.min.max(0) as usize);
        fn curried(f: V, n: usize, got: Vec<V>) -> V {
            let mut roots = got.clone();
            roots.push(f);
            let pr = proc_new_native_rooted(
                Box::new(move |_, args, _| {
                    let mut g = RVec::from(got.clone());
                    g.extend_from_slice(args);
                    if g.len() >= n { call_block(f, &g) } else { curried(f, n, g.take()) }
                }),
                0,
                -1,
                roots,
            );
            rproc(pr).lambda = true;
            pr
        }
        curried(s, n, vec![])
    });
    defn(pc, ">>", 1, 1, |s, a, _| {
        let g = a[0];
        proc_new_native_rooted(Box::new(move |_, args, _| {
            let x = tri!(call_block(s, args));
            send(g, sym("call"), &[x], QNIL)
        }), 0, -1, vec![s, g])
    });
    defn(pc, "<<", 1, 1, |s, a, _| {
        let g = a[0];
        proc_new_native_rooted(Box::new(move |_, args, _| {
            let x = tri!(send(g, sym("call"), args, QNIL));
            call_block(s, &[x])
        }), 0, -1, vec![s, g])
    });
    defn(pc, "inspect", 0, 0, |s, _, _| new_str_v(format!("#<Proc:0x{:016x}{}>", s, if rproc(s).lambda { " (lambda)" } else { "" })));

    // ---- Enumerable (generic: collect via each, then use Array's implementation) ----
    let en = c.enumerable;
    for name in [
        "map", "collect", "flat_map", "collect_concat", "select", "filter", "filter_map", "reject", "find_all", "inject",
        "reduce", "sum", "min", "max", "minmax", "min_by", "max_by", "minmax_by", "sort", "sort_by", "group_by",
        "partition", "tally", "zip", "uniq", "to_h", "each_slice", "each_cons", "each_with_object", "count",
        "drop", "drop_while", "chunk_while", "slice_when", "reverse_each", "none?", "one?", "all?", "cycle",
        "entries", "join",
    ] {
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
    init_enum_streaming(en);
    // as in CRuby's prelude: set.rb leaves this definition alone (`unless method_defined?`)
    defn(en, "to_set", 0, -1, |s, a, b| {
        let klass = match a.first() {
            Some(k) => *k,
            None => tri!(send(rt().c.object as V, sym("const_get"), &[mksym(sym("Set"))], QNIL)),
        };
        let mut args = vec![s];
        args.extend_from_slice(a.get(1..).unwrap_or(&[]));
        send(klass, sym("new"), &args, b)
    });
    defn(en, "to_a", 0, -1, |s, _, _| match each_to_vec(s) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(en, "each_entry", 0, 0, |s, _, b| send(s, sym("each"), &[], b));
    defn(en, "each_with_index", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "each_with_index", &[]);
        }
        each_with_index_via_each(s, b)
    });
    fn find_impl(s: V, a: &[V], b: V, name: &str) -> V {
        if b == QNIL {
            return enum_for(s, name, a);
        }
        let found = Rc::new(RefCell::new(RVec::new()));
        let f2 = found.clone();
        match each_until(s, move |x| {
            let r = call_block(b, &[x]);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if truthy(r) {
                f2.borrow_mut().push(x);
                return Ok(true);
            }
            Ok(false)
        }) {
            Ok(()) => match found.borrow().first().copied() {
                Some(x) => x,
                None => find_ifnone(a),
            },
            Err(e) => e,
        }
    }
    defn(en, "find", 0, 1, |s, a, b| find_impl(s, a, b, "find"));
    defn(en, "detect", 0, 1, |s, a, b| find_impl(s, a, b, "detect"));
    defn(en, "find_index", 0, 1, |s, a, b| {
        let idx = Rc::new(Cell::new(-1i64));
        let i2 = idx.clone();
        let target = a.first().copied();
        let n = Rc::new(Cell::new(0i64));
        match each_until(s, move |x| {
            let hit = match target {
                Some(t) => {
                    let r = equal(x, t);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    truthy(r)
                }
                None => {
                    let r = call_block(b, &[x]);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    truthy(r)
                }
            };
            if hit {
                i2.set(n.get());
                return Ok(true);
            }
            n.set(n.get() + 1);
            Ok(false)
        }) {
            Ok(()) => {
                if idx.get() < 0 {
                    QNIL
                } else {
                    mkfix(idx.get())
                }
            }
            Err(e) => e,
        }
    });
    defn(en, "first", 0, 1, |s, a, _| {
        let want = a.first().map(|x| fix(*x).max(0) as usize);
        if want == Some(0) {
            return ary_new(vec![]);
        }
        let acc = Rc::new(RefCell::new(RVec::new()));
        let a2 = acc.clone();
        let n = want.unwrap_or(1);
        if let Err(e) = each_until(s, move |x| {
            a2.borrow_mut().push(x);
            Ok(a2.borrow().len() >= n)
        }) {
            return e;
        }
        let v = acc.borrow_mut().take();
        match want {
            Some(_) => ary_new(v),
            None => v.first().copied().unwrap_or(QNIL),
        }
    });
    defn(en, "take", 1, 1, |s, a, _| send(s, sym("first"), a, QNIL));
    defn(en, "take_while", 0, 0, |s, _, b| {
        let acc = Rc::new(RefCell::new(RVec::new()));
        let a2 = acc.clone();
        if let Err(e) = each_until(s, move |x| {
            let r = call_block(b, &[x]);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if !truthy(r) {
                return Ok(true);
            }
            a2.borrow_mut().push(x);
            Ok(false)
        }) {
            return e;
        }
        ary_new(acc.borrow_mut().take())
    });
    fn include_impl(s: V, t: V) -> V {
        let found = Rc::new(Cell::new(false));
        let f2 = found.clone();
        match each_until(s, move |x| {
            let r = equal(x, t);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if truthy(r) {
                f2.set(true);
            }
            Ok(truthy(r))
        }) {
            Ok(()) => bool_v(found.get()),
            Err(e) => e,
        }
    }
    defn(en, "include?", 1, 1, |s, a, _| include_impl(s, a[0]));
    defn(en, "member?", 1, 1, |s, a, _| include_impl(s, a[0]));
    defn(en, "any?", 0, 1, |s, a, b| {
        let found = Rc::new(Cell::new(false));
        let f2 = found.clone();
        let pat = a.first().copied();
        match each_until(s, move |x| {
            let r = match pat {
                Some(p) => send1(p, "===", x),
                None if is_proc(b) => call_block(b, &[x]),
                None => x,
            };
            if r == UNDEF {
                return Err(UNDEF);
            }
            if truthy(r) {
                f2.set(true);
            }
            Ok(truthy(r))
        }) {
            Ok(()) => bool_v(found.get()),
            Err(e) => e,
        }
    });

    // ---- Enumerator ----
    let er = c.enumerator;
    defn(er, "each", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return s;
        }
        let generator = ivar_get(s, sym("@__gen"));
        if generator != QNIL {
            let yc = const_get_under(rt().c.enumerator, sym("Yielder")).unwrap();
            let y = object_new(yc as *mut Class);
            ivar_set(y, sym("@__blk"), b);
            return call_block(generator, &[y]);
        }
        let (recv, m, args) = enum_parts(s);
        send(recv, m, &args, b)
    });
    fn with_index(s: V, off: i64, b: V) -> V {
        let (recv, m, args) = enum_parts(s);
        if !is_proc(b) {
            return enum_for(s, "with_index", &[mkfix(off)]);
        }
        let i = Rc::new(Cell::new(off));
        let nb = proc_new_native(
            Box::new(move |_, a, _| {
                let r = call_block(b, &[yielded(a), mkfix(i.get())]);
                i.set(i.get() + 1);
                r
            }),
            0,
            -1,
        );
        rproc(nb).autosplat = false;
        send(recv, m, &args, nb)
    }
    defn(er, "with_index", 0, 1, |s, a, b| with_index(s, a.first().filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0), b));
    defn(er, "each_with_index", 0, 0, |s, _, b| with_index(s, 0, b));
    defn(er, "with_object", 1, 1, |s, a, b| {
        let (recv, m, args) = enum_parts(s);
        let memo = a[0];
        let nb = proc_new_native_rooted(Box::new(move |_, x, _| call_block(b, &[yielded(x), memo])), 0, -1, vec![b, memo]);
        rproc(nb).autosplat = false;
        tri!(send(recv, m, &args, nb));
        memo
    });
    // the size without enumerating (CRuby's size functions): given (Enumerator.new(n), a
    // to_enum block), from the receiver for the methods it determines, else nil
    defn(er, "size", 0, 0, |s, _, _| {
        let given = ivar_get(s, sym("@__size"));
        let (recv, m, args) = enum_parts(s);
        if given != QNIL {
            if is_proc(given) {
                let args = crate::gc::Rooted::of(args);
                return call_block(given, &args);
            }
            if respond_to(given, sym("call")) {
                return send(given, sym("call"), &[], QNIL);
            }
            return given;
        }
        if ivar_get(s, sym("@__gen")) != QNIL {
            return QNIL;
        }
        enum_size(recv, &sym_str(m), &args)
    });
    // Enumerator.new { |yielder| ... }
    let yc = new_class("Enumerator::Yielder", rt().c.object);
    const_set(er, sym("Yielder"), yc as V);
    for name in ["<<", "yield", "call"] {
        defn(yc, name, 0, -1, |s, a, _| {
            let b = ivar_get(s, sym("@__blk"));
            let r = tri!(call_block(b, a));
            let _ = r;
            s
        });
    }
    defs(er, "new", 0, 1, |s, a, b| {
        if !is_proc(b) {
            return arg_error("no block given");
        }
        let e = object_new(s as *mut Class);
        ivar_set(e, sym("@__gen"), b);
        if let Some(&n) = a.first() {
            ivar_set(e, sym("@__size"), n);
        }
        e
    });
    defn(er, "next", 0, 0, |s, _, _| crate::thread::enum_next(s, false));
    defn(er, "peek", 0, 0, |s, _, _| crate::thread::enum_next(s, true));
    defn(er, "rewind", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__fiber"), QNIL);
        ivar_set(s, sym("@__done"), QFALSE);
        ivar_set(s, sym("@__peeked"), QNIL);
        s
    });
    defn(er, "inspect", 0, 0, |s, _, _| {
        let (recv, m, _) = enum_parts(s);
        new_str_v(format!("#<Enumerator: {}:{}>", inspect_lossy(recv), sym_str(m)))
    });

    // ---- Struct ----
    let st = c.struct_;
    defs(st, "new", 0, -1, |s, a, b| {
        // a struct class makes an instance; Struct and a subclass of it with no members yet
        // (`class Options < Struct; end; Options.new(:a, :b)`, Faraday's) make a struct class
        if ancestors(s as *mut Class).iter().any(|c| unsafe { (**c).civars.contains_key(&sym("@__struct")) }) {
            return crate::kernel::class_new_instance(s, a, b);
        }
        let kw_init = has_kw(a, "keyword_init");
        // Struct.new("Name", ...) also names the class Struct::Name
        let (cname, a) = match a.first() {
            Some(&f) if is_str(f) => (Some(String::from_utf8_lossy(&rstr(f).b).into_owned()), &a[1..]),
            // Struct.new(nil, ...): no class name (websocket-driver's `Struct.new(nil)`)
            Some(&f) if f == QNIL => (None, &a[1..]),
            _ => (None, a),
        };
        let mut members = Vec::new();
        for &m in a {
            if is_hash(m) {
                continue;
            }
            match sym_arg(m) {
                Ok(id) => members.push(id),
                Err(e) => return e,
            }
        }
        let k = new_class("", s as *mut Class);
        unsafe {
            (*k).struct_members.set(members.clone());
        }
        // (a class Struct.new made: its `new` makes instances)
        rclass(k as V).civars.insert(sym("@__struct"), QTRUE);
        if let Some(n) = cname {
            const_set(rt().c.struct_, sym(&n), k as V);
        }
        if kw_init {
            rclass(k as V).civars.insert(sym("@__kw"), QTRUE);
        }
        for &m in &members {
            add_method(k, m, Body::AttrReader(ivname(m)));
            add_method(k, sym(&format!("{}=", sym_str(m))), Body::AttrWriter(ivname(m)));
        }
        if is_proc(b) {
            tri!(crate::kernel::class_exec(k as V, &[k as V], b));
        }
        k as V
    });
    defs(st, "members", 0, 0, |s, _, _| ary_new(unsafe { (*(s as *mut Class)).struct_members.iter().map(|m| mksym(*m)).collect() }));
    defn(st, "initialize", 0, -1, |s, a, _| {
        let ms = struct_members(s);
        let kw = a.len() == 1 && is_hash(a[0]) && (hdr(a[0]).flags & FL_KWHASH != 0 || ancestors(real_class(s)).iter().any(|c| unsafe { (**c).civars.contains_key(&sym("@__kw")) }));
        for &m in &ms {
            ivar_set(s, ivname(m), QNIL);
        }
        if kw {
            for (k, v) in crate::hash::pairs(a[0]) {
                let id = match sym_arg(k) {
                    Ok(i) => i,
                    Err(e) => return e,
                };
                if !ms.contains(&id) {
                    return arg_error(&format!("unknown keywords: {}", sym_str(id)));
                }
                ivar_set(s, ivname(id), v);
            }
            return QNIL;
        }
        if a.len() > ms.len() {
            return arg_error("struct size differs");
        }
        for (i, &v) in a.iter().enumerate() {
            ivar_set(s, ivname(ms[i]), v);
        }
        QNIL
    });
    defn(st, "members", 0, 0, |s, _, _| ary_new(struct_members(s).into_iter().map(mksym).collect()));
    defn(st, "to_a", 0, 0, |s, _, _| ary_new(struct_values(s)));
    defn(st, "deconstruct", 0, 0, |s, _, _| ary_new(struct_values(s)));
    defn(st, "values", 0, 0, |s, _, _| ary_new(struct_values(s)));
    defn(st, "to_h", 0, 0, |s, _, _| {
        let h = hash_new();
        for m in struct_members(s) {
            rhash(h).m.insert(HKey(mksym(m)), ivar_get(s, ivname(m)));
        }
        h
    });
    defn(st, "deconstruct_keys", 1, 1, |s, _, _| send0(s, "to_h"));
    defn(st, "each", 0, 0, |s, _, b| {
        for v in struct_values(s) {
            tri!(call_block(b, &[v]));
        }
        s
    });
    defn(st, "[]", 1, 1, |s, a, _| match struct_index(s, a[0]) {
        Ok(m) => ivar_get(s, ivname(m)),
        Err(e) => e,
    });
    defn(st, "[]=", 2, 2, |s, a, _| match struct_index(s, a[0]) {
        Ok(m) => ivar_set(s, ivname(m), a[1]),
        Err(e) => e,
    });
    defn(st, "==", 1, 1, |s, a, _| {
        if real_class(s) != real_class(a[0]) {
            return QFALSE;
        }
        for (x, y) in struct_values(s).into_iter().zip(struct_values(a[0])) {
            if !truthy(tri!(equal(x, y))) {
                return QFALSE;
            }
        }
        QTRUE
    });
    defn(st, "eql?", 1, 1, |s, a, _| bool_v(real_class(s) == real_class(a[0]) && struct_values(s).into_iter().zip(struct_values(a[0])).all(|(x, y)| eql(x, y))));
    defn(st, "hash", 0, 0, |s, _, _| mkfix((hash_of(ary_new(struct_values(s))) >> 2) as i64));
    fn struct_inspect(s: V) -> V {
        let name = class_name(real_class(s));
        let mut out = if name.starts_with("#<") { "#<struct ".to_string() } else { format!("#<struct {name} ") };
        for (i, m) in struct_members(s).into_iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&sym_str(m));
            out.push('=');
            match inspect(ivar_get(s, ivname(m))) {
                Ok(b) => out.push_str(&String::from_utf8_lossy(&b)),
                Err(()) => return UNDEF,
            }
        }
        if out.ends_with(' ') {
            out.pop(); // no members: `#<struct E>`
        }
        out.push('>');
        new_str_v(out)
    }
    defn(st, "inspect", 0, 0, |s, _, _| struct_inspect(s));
    defn(st, "to_s", 0, 0, |s, _, _| struct_inspect(s));

    // ---- Time ----
    let tm = c.time;
    defs(tm, "now", 0, 1, |_, _, _| {
        let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        time_new(d.as_secs_f64(), false)
    });
    defs(tm, "at", 1, 2, |_, a, _| time_new(num_to_f(a[0]).unwrap_or(0.0), false));
    defn(tm, "to_f", 0, 0, |s, _, _| float_new(time_secs(s)));
    defn(tm, "to_i", 0, 0, |s, _, _| mkfix(time_secs(s).floor() as i64));
    defn(tm, "-", 1, 1, |s, a, _| {
        if is_heap(a[0]) && real_class(a[0]) == rt().c.time {
            float_new(time_secs(s) - time_secs(a[0]))
        } else {
            time_new(time_secs(s) - num_to_f(a[0]).unwrap_or(0.0), truthy(ivar_get(s, sym("@__utc"))))
        }
    });
    defn(tm, "+", 1, 1, |s, a, _| time_new(time_secs(s) + num_to_f(a[0]).unwrap_or(0.0), truthy(ivar_get(s, sym("@__utc")))));
    defn(tm, "<=>", 1, 1, |s, a, _| {
        if !(is_heap(a[0]) && real_class(a[0]) == rt().c.time) {
            return QNIL;
        }
        mkfix(time_secs(s).partial_cmp(&time_secs(a[0])).map(|o| o as i64).unwrap_or(0))
    });
    defn(tm, "utc", 0, 0, |s, _, _| time_new(time_secs(s), true));
    defn(tm, "getutc", 0, 0, |s, _, _| time_new(time_secs(s), true));
    defn(tm, "localtime", 0, 1, |s, _, _| time_new(time_secs(s), false));
    defn(tm, "utc?", 0, 0, |s, _, _| ivar_get(s, sym("@__utc")));
    defn(tm, "year", 0, 0, |s, _, _| mkfix(tm_of(s).tm_year as i64 + 1900));
    defn(tm, "month", 0, 0, |s, _, _| mkfix(tm_of(s).tm_mon as i64 + 1));
    defn(tm, "mon", 0, 0, |s, _, _| mkfix(tm_of(s).tm_mon as i64 + 1));
    defn(tm, "day", 0, 0, |s, _, _| mkfix(tm_of(s).tm_mday as i64));
    defn(tm, "mday", 0, 0, |s, _, _| mkfix(tm_of(s).tm_mday as i64));
    defn(tm, "hour", 0, 0, |s, _, _| mkfix(tm_of(s).tm_hour as i64));
    defn(tm, "min", 0, 0, |s, _, _| mkfix(tm_of(s).tm_min as i64));
    defn(tm, "sec", 0, 0, |s, _, _| mkfix(tm_of(s).tm_sec as i64));
    defn(tm, "usec", 0, 0, |s, _, _| mkfix((time_secs(s).fract() * 1e6) as i64));
    defn(tm, "nsec", 0, 0, |s, _, _| mkfix((time_secs(s).fract() * 1e9) as i64));
    defn(tm, "wday", 0, 0, |s, _, _| mkfix(tm_of(s).tm_wday as i64));
    defn(tm, "yday", 0, 0, |s, _, _| mkfix(tm_of(s).tm_yday as i64 + 1));
    defn(tm, "utc_offset", 0, 0, |s, _, _| mkfix(tm_of(s).tm_gmtoff));
    defn(tm, "zone", 0, 0, |s, _, _| new_str_v(strftime(s, "%Z")));
    defn(tm, "strftime", 1, 1, |s, a, _| match path_arg(a[0]) {
        Ok(f) => new_str_v(strftime(s, &f)),
        Err(e) => e,
    });
    defn(tm, "iso8601", 0, 1, |s, _, _| {
        let z = strftime(s, "%z");
        let z = if truthy(ivar_get(s, sym("@__utc"))) { "Z".to_string() } else { format!("{}:{}", &z[..3], &z[3..]) };
        new_str_v(format!("{}{}", strftime(s, "%Y-%m-%dT%H:%M:%S"), z))
    });
    defn(tm, "to_s", 0, 0, |s, _, _| {
        if truthy(ivar_get(s, sym("@__utc"))) { new_str_v(strftime(s, "%Y-%m-%d %H:%M:%S UTC")) } else { new_str_v(strftime(s, "%Y-%m-%d %H:%M:%S %z")) }
    });
    defn(tm, "inspect", 0, 0, |s, _, _| send0(s, "to_s"));

    // ---- Process ----
    let pr = new_module("Process");
    set_global_const("Process", pr as V);
    // the platform's clock ids (as CRuby defines them), read with clock_gettime(2)
    let clocks: &[(&str, libc::clockid_t)] = &[
        ("CLOCK_REALTIME", libc::CLOCK_REALTIME),
        ("CLOCK_MONOTONIC", libc::CLOCK_MONOTONIC),
        ("CLOCK_PROCESS_CPUTIME_ID", libc::CLOCK_PROCESS_CPUTIME_ID),
        ("CLOCK_THREAD_CPUTIME_ID", libc::CLOCK_THREAD_CPUTIME_ID),
        #[cfg(target_os = "macos")]
        ("CLOCK_MONOTONIC_RAW", libc::CLOCK_MONOTONIC_RAW),
        #[cfg(target_os = "macos")]
        ("CLOCK_MONOTONIC_RAW_APPROX", libc::CLOCK_MONOTONIC_RAW_APPROX),
        #[cfg(target_os = "macos")]
        ("CLOCK_UPTIME_RAW", libc::CLOCK_UPTIME_RAW),
        #[cfg(target_os = "macos")]
        ("CLOCK_UPTIME_RAW_APPROX", libc::CLOCK_UPTIME_RAW_APPROX),
        #[cfg(target_os = "linux")]
        ("CLOCK_MONOTONIC_RAW", libc::CLOCK_MONOTONIC_RAW),
        #[cfg(target_os = "linux")]
        ("CLOCK_BOOTTIME", libc::CLOCK_BOOTTIME),
        #[cfg(target_os = "linux")]
        ("CLOCK_REALTIME_COARSE", libc::CLOCK_REALTIME_COARSE),
        #[cfg(target_os = "linux")]
        ("CLOCK_MONOTONIC_COARSE", libc::CLOCK_MONOTONIC_COARSE),
    ];
    for &(n, id) in clocks {
        const_set(pr, sym(n), mkfix(id as i64));
    }
    defs(pr, "clock_gettime", 1, 2, |_, a, _| {
        if !is_fix(a[0]) {
            return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(a[0]))));
        }
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        if unsafe { libc::clock_gettime(fix(a[0]) as libc::clockid_t, &mut ts) } != 0 {
            return raise(crate::io::errno_class(libc::EINVAL), "Invalid argument - clock_gettime");
        }
        let ns = ts.tv_sec as i128 * 1_000_000_000 + ts.tv_nsec as i128;
        let secs = ns as f64 / 1e9;
        match a.get(1).filter(|u| is_sym(**u)).map(|u| sym_str(sym_id(*u))) {
            Some(u) if u == "millisecond" => mkfix((ns / 1_000_000) as i64),
            Some(u) if u == "microsecond" => mkfix((ns / 1_000) as i64),
            Some(u) if u == "nanosecond" => mkfix(ns as i64),
            Some(u) if u == "second" => mkfix((ns / 1_000_000_000) as i64),
            Some(u) if u == "float_millisecond" => float_new(ns as f64 / 1e6),
            Some(u) if u == "float_microsecond" => float_new(ns as f64 / 1e3),
            Some(u) if u == "float_second" => float_new(secs),
            Some(u) => arg_error(&format!("unexpected unit: {u}")),
            None => float_new(secs),
        }
    });
    defs(pr, "pid", 0, 0, |_, _, _| mkfix(std::process::id() as i64));

    // ---- Math ----
    let m = new_module("Math");
    set_global_const("Math", m as V);
    const_set(m, sym("PI"), float_new(std::f64::consts::PI));
    const_set(m, sym("E"), float_new(std::f64::consts::E));
    let dom = new_class("Math::DomainError", c.argument_error);
    const_set(m, sym("DomainError"), dom as V);
    use crate::extra::{float_arg, math_domain};
    macro_rules! math1 {
        ($name:expr, $f:expr) => {
            defs(m, $name, 1, 1, |_, a, _| match float_arg(a[0]) {
                Ok(x) => float_new($f(x)),
                Err(e) => e,
            });
        };
    }
    /// one argument, which must not be below zero
    macro_rules! math1_pos {
        ($name:expr, $f:expr) => {
            defs(m, $name, 1, 1, |_, a, _| match float_arg(a[0]) {
                Ok(x) if x < 0.0 => math_domain($name),
                Ok(x) => float_new($f(x)),
                Err(e) => e,
            });
        };
    }
    // (+0.0 for -0.0, as CRuby)
    math1_pos!("sqrt", |x: f64| x.sqrt() + 0.0);
    math1!("cbrt", f64::cbrt);
    math1!("sin", f64::sin);
    math1!("cos", f64::cos);
    math1!("tan", f64::tan);
    /// one argument within [-1, 1]
    macro_rules! math1_unit {
        ($name:expr, $f:expr) => {
            defs(m, $name, 1, 1, |_, a, _| match float_arg(a[0]) {
                Ok(x) if !(-1.0..=1.0).contains(&x) && !x.is_nan() => math_domain($name),
                Ok(x) => float_new($f(x)),
                Err(e) => e,
            });
        };
    }
    math1_unit!("asin", f64::asin);
    math1_unit!("acos", f64::acos);
    math1!("atan", f64::atan);
    math1!("sinh", f64::sinh);
    math1!("cosh", f64::cosh);
    math1!("tanh", f64::tanh);
    math1!("exp", f64::exp);
    math1_pos!("log2", f64::log2);
    if ruby4() {
        defs(m, "log1p", 1, 1, |_, a, _| match float_arg(a[0]) {
            Ok(x) if x < -1.0 => math_domain("log1p"),
            Ok(x) => float_new(x.ln_1p()),
            Err(e) => e,
        });
        math1!("expm1", f64::exp_m1);
    }
    math1_pos!("log10", f64::log10);
    defs(m, "log", 1, 2, |_, a, _| {
        let x = match float_arg(a[0]) {
            Ok(x) if x < 0.0 => return math_domain("log"),
            Ok(x) => x,
            Err(e) => return e,
        };
        match a.get(1).map(|b| float_arg(*b)) {
            Some(Ok(b)) if b < 0.0 => math_domain("log"),
            Some(Ok(b)) => float_new(x.ln() / b.ln()),
            Some(Err(e)) => e,
            None => float_new(x.ln()),
        }
    });
    macro_rules! math2 {
        ($name:expr, $f:expr) => {
            defs(m, $name, 2, 2, |_, a, _| match (float_arg(a[0]), float_arg(a[1])) {
                (Ok(x), Ok(y)) => float_new($f(x, y)),
                (Err(e), _) | (_, Err(e)) => e,
            });
        };
    }
    math2!("atan2", f64::atan2);
    math2!("hypot", f64::hypot);
    defs(m, "pow", 2, 2, |_, a, _| float_new(math_f(a[0]).unwrap_or(0.0).powf(math_f(a[1]).unwrap_or(0.0))));

    // ---- IO, File, Dir: see io.rs ----

    // ---- ENV ----
    let env = object_new(c.object);
    set_global_const("ENV", env);
    let envc = singleton_class(env);
    defn(envc, "[]", 1, 1, |_, a, _| match path_arg(a[0]).ok().and_then(|k| std::env::var(k).ok()) {
        Some(v) => new_str_v(v),
        None => QNIL,
    });
    defn(envc, "fetch", 1, 2, |_, a, b| {
        let k = match path_arg(a[0]) {
            Ok(k) => k,
            Err(e) => return e,
        };
        match std::env::var(&k) {
            Ok(v) => new_str_v(v),
            Err(_) if is_proc(b) => call_block(b, &[a[0]]),
            Err(_) if a.len() == 2 => a[1],
            Err(_) => raise(rt().c.key_error, &format!("key not found: \"{k}\"")),
        }
    });
    defn(envc, "key?", 1, 1, |_, a, _| bool_v(path_arg(a[0]).ok().is_some_and(|k| std::env::var(k).is_ok())));
    fn env_set(k: V, v: V) -> V {
        let k = match path_arg(k) {
            Ok(k) => k,
            Err(e) => return e,
        };
        if v == QNIL {
            unsafe { std::env::remove_var(k) };
            return QNIL;
        }
        match path_arg(v) {
            Ok(v) => unsafe { std::env::set_var(k, v) },
            Err(e) => return e,
        }
        v
    }
    defn(envc, "[]=", 2, 2, |_, a, _| env_set(a[0], a[1]));
    defn(envc, "store", 2, 2, |_, a, _| env_set(a[0], a[1]));
    fn env_hash() -> V {
        let h = hash_new();
        for (k, v) in std::env::vars_os() {
            crate::hash::hset(h, new_str_v(k.to_string_lossy().into_owned()), new_str_v(v.to_string_lossy().into_owned()));
        }
        h
    }
    defn(envc, "to_hash", 0, 0, |_, _, _| env_hash());
    defn(envc, "to_h", 0, 0, |_, _, b| if is_proc(b) { send(env_hash(), sym("to_h"), &[], b) } else { env_hash() });
    defn(envc, "inspect", 0, 0, |_, _, _| send(env_hash(), sym("inspect"), &[], QNIL));
    defn(envc, "to_s", 0, 0, |_, _, _| str_from("ENV"));
    defn(envc, "include?", 1, 1, |_, a, _| bool_v(path_arg(a[0]).ok().is_some_and(|k| std::env::var_os(k).is_some())));
    defn(envc, "has_key?", 1, 1, |_, a, _| bool_v(path_arg(a[0]).ok().is_some_and(|k| std::env::var_os(k).is_some())));
    defn(envc, "member?", 1, 1, |_, a, _| bool_v(path_arg(a[0]).ok().is_some_and(|k| std::env::var_os(k).is_some())));
    fn env_each(s: V, b: V, name: &str) -> V {
        if !is_proc(b) {
            return enum_for(s, name, &[]);
        }
        let h = env_hash();
        for (k, v) in crate::hash::pairs(h).iter().copied() {
            tri!(call_block(b, &[ary_new(vec![k, v])]));
        }
        s
    }
    defn(envc, "each", 0, 0, |s, _, b| env_each(s, b, "each"));
    defn(envc, "each_pair", 0, 0, |s, _, b| env_each(s, b, "each_pair"));
    fn env_delete(k: V) -> Option<V> {
        let k = path_arg(k).ok()?;
        let old = std::env::var_os(&k)?;
        unsafe { std::env::remove_var(&k) };
        Some(new_str_v(old.to_string_lossy().into_owned()))
    }
    defn(envc, "delete", 1, 1, |_, a, b| match env_delete(a[0]) {
        Some(v) => v,
        None if is_proc(b) => call_block(b, &[a[0]]),
        None => QNIL,
    });
    fn env_update(s: V, a: &[V], b: V) -> V {
        for &h in a {
            let h = tri!(send(h, sym("to_hash"), &[], QNIL));
            for (k, v) in crate::hash::pairs(h).iter().copied() {
                let mut v = v;
                if is_proc(b) {
                    let old = send(s, sym("[]"), &[k], QNIL);
                    if old != QNIL {
                        v = tri!(call_block(b, &[k, old, v]));
                    }
                }
                tri!(env_set(k, v));
            }
        }
        s
    }
    defn(envc, "update", 0, -1, env_update);
    defn(envc, "merge!", 0, -1, env_update);
    defn(envc, "replace", 1, 1, |s, a, _| {
        let h = tri!(send(a[0], sym("to_hash"), &[], QNIL));
        let mut keep = Vec::new();
        for (k, _) in crate::hash::pairs(h).iter().copied() {
            match path_arg(k) {
                Ok(k) => keep.push(k),
                Err(e) => return e,
            }
        }
        for (k, _) in std::env::vars_os() {
            if !keep.iter().any(|x| std::ffi::OsStr::new(x) == k) {
                unsafe { std::env::remove_var(&k) };
            }
        }
        env_update(s, &[h], QNIL)
    });
    defn(envc, "clear", 0, 0, |s, _, _| {
        for (k, _) in std::env::vars_os() {
            unsafe { std::env::remove_var(k) };
        }
        s
    });
    // delete_if/reject! (drop when the block is true), keep_if/select!/filter! (drop when false)
    fn env_filter(s: V, b: V, name: &str, drop_when: bool) -> V {
        if !is_proc(b) {
            return enum_for(s, name, &[]);
        }
        let h = env_hash();
        let mut changed = false;
        for (k, v) in crate::hash::pairs(h).iter().copied() {
            if truthy(tri!(call_block(b, &[k, v]))) == drop_when {
                env_delete(k);
                changed = true;
            }
        }
        if !changed && (name == "reject!" || name == "select!" || name == "filter!") { QNIL } else { s }
    }
    defn(envc, "delete_if", 0, 0, |s, _, b| env_filter(s, b, "delete_if", true));
    defn(envc, "reject!", 0, 0, |s, _, b| env_filter(s, b, "reject!", true));
    defn(envc, "keep_if", 0, 0, |s, _, b| env_filter(s, b, "keep_if", false));
    defn(envc, "select!", 0, 0, |s, _, b| env_filter(s, b, "select!", false));
    defn(envc, "filter!", 0, 0, |s, _, b| env_filter(s, b, "filter!", false));
    defn(envc, "rehash", 0, 0, |_, _, _| QNIL);
    defn(envc, "freeze", 0, 0, |_, _, _| raise(rt().c.type_error, "cannot freeze ENV"));
    defn(envc, "dup", 0, 0, |_, _, _| raise(rt().c.type_error, "Cannot dup ENV, use ENV.to_h to get a copy of ENV as a hash"));
    // the read-only rest acts on a Hash snapshot, as CRuby's ENV methods return Hashes and Arrays
    macro_rules! env_via_hash {
        ($($name:literal),*) => {
            $(defn(envc, $name, 0, -1, |s, a, b| {
                let r = send(env_hash(), sym($name), a, b);
                if !is_proc(b) && is_a(r, rt().c.enumerator) { enum_for(s, $name, a) } else { r }
            });)*
        };
    }
    env_via_hash!(
        "keys", "values", "to_a", "size", "length", "empty?", "value?", "has_value?", "key", "values_at",
        "fetch_values", "slice", "except", "assoc", "rassoc", "invert", "select", "filter", "reject",
        "each_key", "each_value", "filter_map", "any?", "find", "detect", "map", "count", "sort_by", "min_by",
        "max_by", "sum", "each_with_object", "partition", "group_by", "to_set"
    );
    crate::kernel::do_include(envc, rt().c.enumerable as V);

    // ---- GC ----
    let gc = new_module("GC");
    set_global_const("GC", gc as V);
    defs(gc, "start", 0, 1, |_, _, _| {
        crate::gc::collect_now();
        QNIL
    });
    defs(gc, "enable", 0, 0, |_, _, _| {
        let h = crate::gc::heap();
        let was = h.disabled > 0;
        h.disabled = 0;
        bool_v(was)
    });
    defs(gc, "disable", 0, 0, |_, _, _| {
        let h = crate::gc::heap();
        let was = h.disabled > 0;
        h.disabled = 1;
        bool_v(was)
    });
    defs(gc, "stat", 0, 1, |_, a, _| {
        let (count, live) = crate::gc::stats();
        let h = hash_new();
        rhash(h).m.insert(HKey(mksym(sym("count"))), mkfix(count as i64));
        rhash(h).m.insert(HKey(mksym(sym("heap_live_slots"))), mkfix(live as i64));
        match a.first() {
            Some(&k) => rhash(h).m.get(&HKey(k)).copied().unwrap_or(QNIL),
            None => h,
        }
    });
    defs(gc, "count", 0, 0, |_, _, _| mkfix(crate::gc::stats().0 as i64));

    let sig = new_module("Signal");
    set_global_const("Signal", sig as V);
    defs(sig, "trap", 1, 2, |_, _, _| QNIL);

    let rbc = new_module("RbConfig");
    set_global_const("RbConfig", rbc as V);
    let cfg = hash_new();
    // "darwin24" like CRuby (with the running kernel's major version): gems match on it, e.g.
    // listen only uses FSEvents for /darwin(1|2)\d+/
    let os = if cfg!(target_os = "macos") { format!("darwin{}", kernel_major().unwrap_or(24)) } else { "linux".into() };
    for k in ["host_os", "target_os"] {
        crate::hash::hset(cfg, str_from(k), str_from(&os));
    }
    // "arm64-darwin27" / "x86_64-linux", as CRuby names its platform
    let cpu = if cfg!(target_arch = "aarch64") { if cfg!(target_os = "macos") { "arm64" } else { "aarch64" } } else { "x86_64" };
    let arch = format!("{cpu}-{os}");
    crate::hash::hset(cfg, str_from("arch"), str_from(&arch));
    let rel = ruby_release();
    let (abi, major, minor, teeny, patchlevel) =
        (format!("{}.{}.0", rel.major, rel.minor), rel.major.to_string(), rel.minor.to_string(), rel.teeny.to_string(), rel.patchlevel.to_string());
    for (k, v) in [
        ("ruby_version", abi.as_str()),
        ("MAJOR", major.as_str()),
        ("MINOR", minor.as_str()),
        ("TEENY", teeny.as_str()),
        ("PATCHLEVEL", patchlevel.as_str()),
        ("host_cpu", if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" }),
        ("EXEEXT", ""),
        ("DLEXT", "bundle"),
        ("ruby_install_name", "ruby"),
        ("RUBY_INSTALL_NAME", "ruby"),
        ("bindir", "/usr/local/bin"),
        ("engine", "kenny"),
    ] {
        crate::hash::hset(cfg, str_from(k), str_from(v));
    }
    // the install layout CRuby's RbConfig describes (RubyGems builds its default paths from
    // it); a Kenny binary has no Ruby install, so these are CRuby's defaults under /usr/local
    let lib = "/usr/local/lib/ruby".to_string();
    for (k, v) in [
        ("prefix", "/usr/local".to_string()),
        ("exec_prefix", "/usr/local".to_string()),
        ("libdir", "/usr/local/lib".to_string()),
        ("datadir", "/usr/local/share".to_string()),
        ("sysconfdir", "/usr/local/etc".to_string()),
        ("rubylibprefix", lib.clone()),
        ("rubylibdir", format!("{lib}/{abi}")),
        ("rubyarchdir", format!("{lib}/{abi}/{arch}")),
        ("archdir", format!("{lib}/{abi}/{arch}")),
        ("sitedir", format!("{lib}/site_ruby")),
        ("sitelibdir", format!("{lib}/site_ruby/{abi}")),
        ("sitearchdir", format!("{lib}/site_ruby/{abi}/{arch}")),
        ("vendordir", format!("{lib}/vendor_ruby")),
        ("vendorlibdir", format!("{lib}/vendor_ruby/{abi}")),
        ("vendorarchdir", format!("{lib}/vendor_ruby/{abi}/{arch}")),
    ] {
        crate::hash::hset(cfg, str_from(k), str_from(&v));
    }
    const_set(rbc, sym("CONFIG"), cfg);
    // CRuby's has the unexpanded Makefile values; Kenny's values have nothing to expand
    const_set(rbc, sym("MAKEFILE_CONFIG"), cfg);
    defs(rbc, "ruby", 0, 0, |_, _, _| str_from("ruby"));
    set_global_const("RUBY_VERSION", str_from(&rel.version));
    set_global_const("RUBY_PLATFORM", str_from(&arch));
    set_global_const("RUBY_ENGINE", str_from("kenny"));
    set_global_const("RUBY_PATCHLEVEL", mkfix(rel.patchlevel));
    set_global_const("RUBY_RELEASE_DATE", str_from(&rel.release_date));
    set_global_const("RUBY_COPYRIGHT", str_from("Kenny Ruby"));
    let platform = arch.as_str();
    let description = format!("kenny (ruby {} compatible) [{platform}]", rel.version);
    set_global_const("RUBY_DESCRIPTION", str_from(&description));
    // Ruby 4.0's `Ruby` module: the same facts as the RUBY_* constants
    if ruby4() {
        let m = new_module("Ruby");
        set_global_const("Ruby", m as V);
        for (k, v) in [
            ("VERSION", str_from(&rel.version)),
            ("PATCHLEVEL", mkfix(rel.patchlevel)),
            ("RELEASE_DATE", str_from(&rel.release_date)),
            ("PLATFORM", str_from(platform)),
            ("DESCRIPTION", str_from(&description)),
            ("COPYRIGHT", str_from("Kenny Ruby")),
            ("ENGINE", str_from("ruby")),
            ("ENGINE_VERSION", str_from(&rel.version)),
        ] {
            const_set(m, sym(k), v);
        }
        // Pathname is a core class in 4.0, loaded on first use: of the constant (abi.rs's
        // implicit_const) or of Kernel#Pathname, which kenny/pathname4 then redefines
        defn(rt().c.kernel, "Pathname", 1, 1, |s, a, b| {
            if crate::compat::require_compiled("kenny/pathname4").is_none() {
                return raise(rt().c.name_error, "uninitialized constant Pathname");
            }
            send(s, sym("Pathname"), a, b)
        });
    }
}

/// Major version of the running kernel (`uname -r`: "24.6.0" -> 24).
fn kernel_major() -> Option<u32> {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } != 0 {
        return None;
    }
    let rel = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().into_owned();
    rel.split('.').next()?.parse().ok()
}



/// CRuby's RETURN_ENUMERATOR for native methods that otherwise yield to a missing block:
/// called without a block (and with exactly `nargs` arguments) they return
/// `enum_for(name, *args)` (ActiveModel's `attributes.each_key.select { ... }` needs it).
/// Runs after every core module has defined its methods; only wraps methods defined natively
/// directly on the listed class.
pub fn blockless_enumerators() {
    let c = rt().c;
    let common: &[(&'static str, usize)] = &[
        ("each_with_object", 1),
        ("partition", 0),
        ("find_index", 0),
        ("find", 0),
        ("detect", 0),
        ("each_entry", 0),
        ("take_while", 0),
        ("drop_while", 0),
    ];
    let hash: &[(&'static str, usize)] = &[
        ("each_key", 0),
        ("each_value", 0),
        ("select", 0),
        ("filter", 0),
        ("reject", 0),
        ("filter_map", 0),
        ("delete_if", 0),
        ("keep_if", 0),
        ("select!", 0),
        ("reject!", 0),
        ("filter!", 0),
        ("transform_values", 0),
        ("transform_keys", 0),
        ("transform_values!", 0),
        ("transform_keys!", 0),
    ];
    let array: &[(&'static str, usize)] = &[
        ("delete_if", 0),
        ("keep_if", 0),
        ("select!", 0),
        ("reject!", 0),
        ("filter!", 0),
        ("map!", 0),
        ("collect!", 0),
        ("index", 0),
        ("rindex", 0),
        ("sort_by!", 0),
        ("each_index", 0),
    ];
    let string: &[(&'static str, usize)] = &[("gsub", 1)];
    let table: [(*mut Class, &[(&'static str, usize)]); 8] = [
        (c.hash, common),
        (c.hash, hash),
        (c.array, common),
        (c.array, array),
        (c.enumerable, common),
        (c.range, common),
        (c.string, string),
        (c.struct_, common),
    ];
    for (cls, ms) in table {
        if cls.is_null() {
            continue;
        }
        for &(name, nargs) in ms {
            let id = sym(name);
            let Some(&orig) = (unsafe { (*cls).methods.get(&id) }) else { continue };
            if !matches!(unsafe { &(*orig).body }, Body::Native(..)) {
                continue;
            }
            let vis = unsafe { (*orig).vis };
            let p = proc_new_native(
                Box::new(move |s, a, b| {
                    if !is_proc(b) && a.len() == nargs {
                        return enum_for(s, name, a);
                    }
                    invoke(orig, s, a, b)
                }),
                0,
                -1,
            );
            let m = add_method(cls, id, Body::Proc(p));
            unsafe { (*(m as *mut Method)).vis = vis };
        }
    }
}


/// Enumerable#`id` through Array's implementation: collect what `each` yields first.
fn enum_via_array(s: V, id: u32, args: &[V], blk: V) -> V {
    match each_to_vec(s) {
        Ok(v) => {
            // keep the temporary array (and the values only it references) alive
            let tmp = RVec::from(vec![ary_new(v)]);
            send(tmp[0], id, args, blk)
        }
        Err(e) => e,
    }
}

/// Enumerable methods that take a block run it while `each` yields, as in CRuby: code such as
/// Jbuilder's collection rendering (`iter.each_with_info.map { ... }`, where each_with_info
/// wraps every yield in a scope) depends on that, and so do infinite enumerators. Without a
/// block (or with arguments that make the block irrelevant) they use Array's implementation.
/// Enumerable methods that return an Enumerator when called without a block.
const ENUM_WITHOUT_BLOCK: &[&str] = &[
    "map", "collect", "flat_map", "collect_concat", "select", "filter", "find_all", "reject", "filter_map",
    "each_with_object", "group_by", "partition", "sort_by", "min_by", "max_by", "minmax_by", "each_slice", "each_cons",
];

fn init_enum_streaming(en: *mut Class) {
    macro_rules! call {
        ($b:expr, $args:expr) => {{
            let r = call_block($b, $args);
            if r == UNDEF {
                return Err(UNDEF);
            }
            r
        }};
    }
    /// Run `f` over every yield, collecting the values it pushes; the array of them.
    fn collect(s: V, mut f: impl FnMut(&[V], &mut RVec) -> Result<(), V> + 'static) -> V {
        let acc = Rc::new(RefCell::new(RVec::new()));
        let a2 = acc.clone();
        match each_args_until(s, move |a| f(a, &mut a2.borrow_mut()).map(|_| false)) {
            Ok(()) => {
                let v = acc.borrow_mut().take();
                ary_new(v)
            }
            Err(e) => e,
        }
    }
    fn streamed(en: *mut Class, names: &[&str], min: i32, max: i32, f: fn(V, &[V], V) -> V) {
        for name in names {
            let id = sym(name);
            let p = proc_new_native(
                Box::new(move |s, a, b| {
                    // without a block, an Enumerator over the receiver (so `each_slice(2).first(2)`
                    // works on an infinite enumerator, and its inspect names the receiver)
                    if !is_proc(b) && ENUM_WITHOUT_BLOCK.contains(&sym_str(id).as_str()) && (a.len() as i32) >= min {
                        return enum_for(s, &sym_str(id), a);
                    }
                    if !is_proc(b) || (a.len() as i32) < min || (max >= 0 && a.len() as i32 > max) {
                        enum_via_array(s, id, a, b)
                    } else {
                        f(s, a, b)
                    }
                }),
                0,
                -1,
            );
            add_method(en, id, Body::Proc(p));
        }
    }
    streamed(en, &["map", "collect"], 0, 0, |s, _, b| collect(s, move |a, acc| Ok(acc.push(call!(b, a)))));
    streamed(en, &["flat_map", "collect_concat"], 0, 0, |s, _, b| {
        collect(s, move |a, acc| {
            let r = call!(b, a);
            if is_ary(r) {
                let items = rary(r).v.to_vec();
                for x in items {
                    acc.push(x);
                }
            } else {
                acc.push(r);
            }
            Ok(())
        })
    });
    streamed(en, &["select", "filter", "find_all"], 0, 0, |s, _, b| {
        collect(s, move |a, acc| {
            if truthy(call!(b, a)) {
                acc.push(yielded(a));
            }
            Ok(())
        })
    });
    streamed(en, &["reject"], 0, 0, |s, _, b| {
        collect(s, move |a, acc| {
            if !truthy(call!(b, a)) {
                acc.push(yielded(a));
            }
            Ok(())
        })
    });
    streamed(en, &["filter_map"], 0, 0, |s, _, b| {
        collect(s, move |a, acc| {
            let r = call!(b, a);
            if truthy(r) {
                acc.push(r);
            }
            Ok(())
        })
    });
    streamed(en, &["each_with_object"], 1, 1, |s, a, b| {
        let memo = RVec::from(vec![a[0]]);
        let m = memo[0];
        match each_args_until(s, move |x| {
            call!(b, &[yielded(x), m]);
            Ok(false)
        }) {
            Ok(()) => memo[0],
            Err(e) => e,
        }
    });
    // inject { |acc, x| } and inject(init) { |acc, x| }; inject(:sym) goes through Array
    streamed(en, &["inject", "reduce"], 0, 1, |s, a, b| {
        let acc = Rc::new(RefCell::new(RVec::from(a.to_vec())));
        let a2 = acc.clone();
        let r = each_args_until(s, move |x| {
            let v = yielded(x);
            let cur = a2.borrow().first().copied();
            let next = match cur {
                None => v,
                Some(c) => call!(b, &[c, v]),
            };
            let mut st = a2.borrow_mut();
            if st.is_empty() { st.push(next) } else { st[0] = next }
            Ok(false)
        });
        match r {
            Ok(()) => acc.borrow().first().copied().unwrap_or(QNIL),
            Err(e) => e,
        }
    });
    streamed(en, &["count"], 0, 0, |s, _, b| {
        let n = Rc::new(Cell::new(0i64));
        let n2 = n.clone();
        match each_args_until(s, move |a| {
            if truthy(call!(b, a)) {
                n2.set(n2.get() + 1);
            }
            Ok(false)
        }) {
            Ok(()) => mkfix(n.get()),
            Err(e) => e,
        }
    });
    // the block's values, then Array#sum / #to_h on them (Kahan summation, pair checks)
    streamed(en, &["sum"], 0, 1, |s, a, b| {
        let vals = RVec::from(vec![collect(s, move |x, acc| Ok(acc.push(call!(b, x))))]);
        if vals[0] == UNDEF {
            return UNDEF;
        }
        send(vals[0], sym("sum"), a, QNIL)
    });
    streamed(en, &["to_h"], 0, 0, |s, _, b| {
        let vals = RVec::from(vec![collect(s, move |x, acc| Ok(acc.push(call!(b, x))))]);
        if vals[0] == UNDEF {
            return UNDEF;
        }
        send(vals[0], sym("to_h"), &[], QNIL)
    });
    streamed(en, &["group_by"], 0, 0, |s, _, b| {
        let h = RVec::from(vec![hash_new()]);
        let hv = h[0];
        match each_args_until(s, move |a| {
            let k = call!(b, a);
            let cur = rhash(hv).m.get(&HKey(k)).copied();
            match cur {
                Some(l) => rary(l).v.push(yielded(a)),
                None => {
                    crate::hash::hset(hv, k, ary_new(vec![yielded(a)]));
                }
            }
            Ok(false)
        }) {
            Ok(()) => h[0],
            Err(e) => e,
        }
    });
    streamed(en, &["partition"], 0, 0, |s, _, b| {
        let both = RVec::from(vec![ary_new(vec![]), ary_new(vec![])]);
        let (yes, no) = (both[0], both[1]);
        match each_args_until(s, move |a| {
            let r = call!(b, a);
            rary(if truthy(r) { yes } else { no }).v.push(yielded(a));
            Ok(false)
        }) {
            Ok(()) => ary_new(vec![both[0], both[1]]),
            Err(e) => e,
        }
    });
    // keyed by the block: [element, key] pairs while iterating, then Array's method on the
    // pairs by key, mapped back to the elements
    fn by_key(s: V, id: u32, a: &[V], b: V, many: bool) -> V {
        let pairs = collect(s, move |x, acc| {
            let k = call!(b, x);
            acc.push(ary_new(vec![yielded(x), k]));
            Ok(())
        });
        if pairs == UNDEF {
            return UNDEF;
        }
        let keep = RVec::from(vec![pairs]);
        let key = proc_new_native(Box::new(|_, a, _| if is_ary(a[0]) { rary(a[0]).v[1] } else { QNIL }), 1, 1);
        let r = tri!(send(keep[0], id, a, key));
        let elem = |p: V| if is_ary(p) { rary(p).v[0] } else { p };
        if many && is_ary(r) {
            let items: Vec<V> = rary(r).v.iter().map(|p| elem(*p)).collect();
            ary_new(items)
        } else {
            elem(r)
        }
    }
    streamed(en, &["sort_by"], 0, 0, |s, a, b| by_key(s, sym("sort_by"), a, b, true));
    streamed(en, &["uniq"], 0, 0, |s, a, b| by_key(s, sym("uniq"), a, b, true));
    streamed(en, &["min_by"], 0, 1, |s, a, b| by_key(s, sym("min_by"), a, b, !a.is_empty()));
    streamed(en, &["max_by"], 0, 1, |s, a, b| by_key(s, sym("max_by"), a, b, !a.is_empty()));
    streamed(en, &["minmax_by"], 0, 0, |s, a, b| by_key(s, sym("minmax_by"), a, b, true));
    // all? / none? stop at the first element that decides them (a pattern argument uses ===)
    fn pred(s: V, a: &[V], b: V, stop_on: bool) -> V {
        let pat = a.first().copied();
        let stopped = Rc::new(Cell::new(false));
        let st = stopped.clone();
        let r = each_args_until(s, move |x| {
            let t = match pat {
                Some(p) => send(p, sym("==="), &[yielded(x)], QNIL),
                None if is_proc(b) => call_block(b, x),
                None => yielded(x),
            };
            if t == UNDEF {
                return Err(UNDEF);
            }
            if truthy(t) == stop_on {
                st.set(true);
                return Ok(true);
            }
            Ok(false)
        });
        match r {
            Ok(()) => bool_v(!stopped.get()),
            Err(e) => e,
        }
    }
    defn(en, "all?", 0, 1, |s, a, b| pred(s, a, b, false));
    defn(en, "none?", 0, 1, |s, a, b| pred(s, a, b, true));
    streamed(en, &["each_slice"], 1, 1, |s, a, b| {
        let n = if is_fix(a[0]) { fix(a[0]) } else { 0 };
        if n <= 0 {
            return arg_error("invalid slice size");
        }
        let buf = Rc::new(RefCell::new(RVec::new()));
        let b2 = buf.clone();
        let r = each_args_until(s, move |x| {
            let full = {
                let mut bb = b2.borrow_mut();
                bb.push(yielded(x));
                (bb.len() as i64 >= n).then(|| bb.take())
            };
            if let Some(slice) = full {
                call!(b, &[ary_new(slice)]);
            }
            Ok(false)
        });
        if let Err(e) = r {
            return e;
        }
        let rest = buf.borrow_mut().take();
        if !rest.is_empty() {
            tri!(call_block(b, &[ary_new(rest)]));
        }
        s
    });
    streamed(en, &["each_cons"], 1, 1, |s, a, b| {
        let n = if is_fix(a[0]) { fix(a[0]) } else { 0 };
        if n <= 0 {
            return arg_error("invalid size");
        }
        let buf = Rc::new(RefCell::new(RVec::new()));
        let r = each_args_until(s, move |x| {
            let win = {
                let mut bb = buf.borrow_mut();
                bb.push(yielded(x));
                if bb.len() as i64 > n {
                    bb.remove(0);
                }
                (bb.len() as i64 == n).then(|| bb.to_vec())
            };
            if let Some(w) = win {
                call!(b, &[ary_new(w)]);
            }
            Ok(false)
        });
        match r {
            Ok(()) => s,
            Err(e) => e,
        }
    });
}

/// Enumerable#each_with_index: `each`, with the index (also Array's when its `each` is
/// not the builtin one).
pub(crate) fn each_with_index_via_each(s: V, b: V) -> V {
    let i = Rc::new(Cell::new(0i64));
    let nb = proc_new_native(
        Box::new(move |_, args, _| {
            let r = call_block(b, &[yielded(args), mkfix(i.get())]);
            i.set(i.get() + 1);
            r
        }),
        0,
        -1,
    );
    rproc(nb).autosplat = false;
    tri!(send(s, sym("each"), &[], nb));
    s
}
