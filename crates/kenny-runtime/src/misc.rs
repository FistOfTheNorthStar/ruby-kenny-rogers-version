//! Range, Proc, Enumerable, Enumerator, Struct, Time, Math, Process (clock), ENV, Mutex, Thread.current.

use crate::core::*;
use crate::gc::RVec;
use crate::fmt::*;
use crate::kernel::{new_str_v, sym_arg};
use crate::numeric::{arith, num_cmp, Op};
use crate::tri;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
pub fn each_until(v: V, f: impl FnMut(V) -> Result<bool, V> + 'static) -> Result<(), V> {
    let tag = STOP_TAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let f = Rc::new(RefCell::new(f));
    let nb = proc_new_native(
        Box::new(move |_, args, _| match (f.borrow_mut())(yielded(args)) {
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

pub fn enum_for(recv: V, meth: &str, args: &[V]) -> V {
    let e = object_new(rt().c.enumerator);
    ivar_set(e, sym("@__recv"), recv);
    ivar_set(e, sym("@__meth"), mksym(sym(meth)));
    ivar_set(e, sym("@__args"), ary_new(args.to_vec()));
    e
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
        let mut out = RVec::new();
        let end = rstr(rg.hi).b.clone();
        let mut cur = rg.lo;
        loop {
            let cb = rstr(cur).b.clone();
            if cb.len() > end.len() {
                break;
            }
            if cb == end {
                if !rg.excl {
                    out.push(cur);
                }
                break;
            }
            out.push(cur);
            cur = send0(cur, "succ");
        }
        return Some(out.take());
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
        let mut i = rgv.lo;
        loop {
            if rgv.hi != QNIL {
                let Some(c) = num_cmp(i, rgv.hi) else { return type_error("step on non-numeric range") };
                if c.is_gt() || (rgv.excl && c.is_eq()) {
                    break;
                }
            }
            tri!(call_block(b, &[i]));
            i = tri!(arith(Op::Add, i, a[0]));
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
    defn(rg, "min", 0, 1, |s, a, b| {
        let r = rrange(s);
        if a.is_empty() && !is_proc(b) {
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
        if a.is_empty() && !is_proc(b) && !r.excl {
            let c = tri!(send1(r.lo, "<=>", r.hi));
            return if is_fix(c) && fix(c) <= 0 { r.hi } else { QNIL };
        }
        match each_to_vec(s) {
            Ok(v) => send(ary_new(v), sym("max"), a, b),
            Err(e) => e,
        }
    });
    for name in ["include?", "member?", "===", "cover?"] {
        defn(rg, name, 1, 1, |s, a, _| {
            if is_range(a[0]) {
                let o = rrange(a[0]);
                let lo = tri!(range_cover(s, o.lo));
                let hi = if o.excl { tri!(range_cover(s, o.hi)) } else { tri!(range_cover(s, o.hi)) };
                return bool_v(truthy(lo) && truthy(hi));
            }
            range_cover(s, a[0])
        });
    }
    defn(rg, "exclude_end?", 0, 0, |s, _, _| bool_v(rrange(s).excl));
    defn(rg, "to_s", 0, 0, |s, _, _| range_to_s(s, false));
    defn(rg, "inspect", 0, 0, |s, _, _| range_to_s(s, true));
    defn(rg, "==", 1, 1, |s, a, _| bool_v(is_range(a[0]) && eql(s, a[0])));
    defn(rg, "eql?", 1, 1, |s, a, _| bool_v(is_range(a[0]) && eql(s, a[0])));
    defn(rg, "hash", 0, 0, |s, _, _| mkfix((hash_of(s) >> 2) as i64));
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
        let p = rproc(s);
        if p.max < 0 || (p.min != p.max && p.lambda) { mkfix(-(p.min as i64) - 1) } else { mkfix(p.min as i64) }
    });
    defn(pc, "parameters", 0, 0, |_, _, _| ary_new(vec![]));
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
        "entries", "to_set", "join",
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
    defn(en, "to_a", 0, -1, |s, _, _| match each_to_vec(s) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(en, "each_entry", 0, 0, |s, _, b| send(s, sym("each"), &[], b));
    defn(en, "each_with_index", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return enum_for(s, "each_with_index", &[]);
        }
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
    });
    fn find_impl(s: V, b: V) -> V {
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
            Ok(()) => found.borrow().first().copied().unwrap_or(QNIL),
            Err(e) => e,
        }
    }
    defn(en, "find", 0, 1, |s, _, b| find_impl(s, b));
    defn(en, "detect", 0, 1, |s, _, b| find_impl(s, b));
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
    defn(er, "size", 0, 0, |s, _, _| match each_to_vec(s) {
        Ok(v) => mkfix(v.len() as i64),
        Err(e) => e,
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
    defs(er, "new", 0, 1, |s, _, b| {
        if !is_proc(b) {
            return arg_error("no block given");
        }
        let e = object_new(s as *mut Class);
        ivar_set(e, sym("@__gen"), b);
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
        if s as *mut Class != rt().c.struct_ {
            return crate::kernel::class_new_instance(s, a, b);
        }
        let kw_init = has_kw(a, "keyword_init");
        // Struct.new("Name", ...) also names the class Struct::Name
        let (cname, a) = match a.first() {
            Some(&f) if is_str(f) => (Some(String::from_utf8_lossy(&rstr(f).b).into_owned()), &a[1..]),
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
        let k = new_class("", rt().c.struct_);
        unsafe {
            (*k).struct_members.set(members.clone());
        }
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
    const_set(pr, sym("CLOCK_REALTIME"), mkfix(0));
    const_set(pr, sym("CLOCK_MONOTONIC"), mkfix(1));
    const_set(pr, sym("CLOCK_PROCESS_CPUTIME_ID"), mkfix(2));
    static ORIGIN: G<Option<std::time::Instant>> = G::new(None);
    *ORIGIN.get() = Some(std::time::Instant::now());
    defs(pr, "clock_gettime", 1, 2, |_, a, _| {
        let secs = if is_fix(a[0]) && fix(a[0]) == 0 {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
        } else {
            ORIGIN.get().unwrap().elapsed().as_secs_f64() + 1000.0
        };
        match a.get(1).filter(|u| is_sym(**u)).map(|u| sym_str(sym_id(*u))) {
            Some(u) if u == "millisecond" => mkfix((secs * 1e3) as i64),
            Some(u) if u == "microsecond" => mkfix((secs * 1e6) as i64),
            Some(u) if u == "nanosecond" => mkfix((secs * 1e9) as i64),
            Some(u) if u == "float_millisecond" => float_new(secs * 1e3),
            Some(u) if u == "second" => mkfix(secs as i64),
            _ => float_new(secs),
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
    macro_rules! math1 {
        ($name:expr, $f:expr) => {
            defs(m, $name, 1, 1, |_, a, _| match math_f(a[0]) {
                Some(x) => float_new($f(x)),
                None => type_error("can't convert into Float"),
            });
        };
    }
    defs(m, "sqrt", 1, 1, |_, a, _| match math_f(a[0]) {
        Some(x) if x < 0.0 => {
            let d = const_get_under(const_lookup(rt().c.object, sym("Math")).unwrap() as *mut Class, sym("DomainError")).unwrap();
            raise(d as *mut Class, "Numerical argument is out of domain - \"sqrt\"")
        }
        Some(x) => float_new(x.sqrt()),
        None => type_error("can't convert into Float"),
    });
    math1!("cbrt", f64::cbrt);
    math1!("sin", f64::sin);
    math1!("cos", f64::cos);
    math1!("tan", f64::tan);
    math1!("asin", f64::asin);
    math1!("acos", f64::acos);
    math1!("atan", f64::atan);
    math1!("sinh", f64::sinh);
    math1!("cosh", f64::cosh);
    math1!("tanh", f64::tanh);
    math1!("exp", f64::exp);
    math1!("log2", f64::log2);
    math1!("log10", f64::log10);
    defs(m, "log", 1, 2, |_, a, _| {
        let x = math_f(a[0]).unwrap_or(f64::NAN);
        match a.get(1).and_then(|b| math_f(*b)) {
            Some(b) => float_new(x.ln() / b.ln()),
            None => float_new(x.ln()),
        }
    });
    defs(m, "atan2", 2, 2, |_, a, _| float_new(math_f(a[0]).unwrap_or(0.0).atan2(math_f(a[1]).unwrap_or(0.0))));
    defs(m, "hypot", 2, 2, |_, a, _| float_new(math_f(a[0]).unwrap_or(0.0).hypot(math_f(a[1]).unwrap_or(0.0))));
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
    defn(envc, "[]=", 2, 2, |_, a, _| {
        if let (Ok(k), Ok(v)) = (path_arg(a[0]), path_arg(a[1])) {
            unsafe { std::env::set_var(k, v) };
        }
        a[1]
    });
    defn(envc, "to_h", 0, 0, |_, _, _| {
        let h = hash_new();
        for (k, v) in std::env::vars() {
            crate::hash::hset(h, new_str_v(k), new_str_v(v));
        }
        h
    });
    defn(envc, "inspect", 0, 0, |_, _, _| str_from("ENV"));

    // ---- GC ----
    let gc = new_module("GC");
    set_global_const("GC", gc as V);
    defs(gc, "start", 0, 1, |_, _, _| {
        // (this frame holds no object references: it doesn't stop the GC from moving objects)
        pst().native_depth -= 1;
        crate::gc::collect_now();
        pst().native_depth += 1;
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
    for (k, v) in [
        ("ruby_version", "3.4.0"),
        ("MAJOR", "3"),
        ("MINOR", "4"),
        ("TEENY", "0"),
        ("host_os", if cfg!(target_os = "macos") { "darwin" } else { "linux" }),
        ("target_os", if cfg!(target_os = "macos") { "darwin" } else { "linux" }),
        ("host_cpu", if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" }),
        ("arch", if cfg!(target_os = "macos") { "arm64-darwin" } else { "x86_64-linux" }),
        ("EXEEXT", ""),
        ("DLEXT", "bundle"),
        ("ruby_install_name", "ruby"),
        ("RUBY_INSTALL_NAME", "ruby"),
        ("bindir", "/usr/local/bin"),
        ("rubylibdir", ""),
        ("engine", "kenny"),
    ] {
        crate::hash::hset(cfg, str_from(k), str_from(v));
    }
    const_set(rbc, sym("CONFIG"), cfg);
    // CRuby's has the unexpanded Makefile values; Kenny's values have nothing to expand
    const_set(rbc, sym("MAKEFILE_CONFIG"), cfg);
    defs(rbc, "ruby", 0, 0, |_, _, _| str_from("ruby"));
    set_global_const("RUBY_VERSION", str_from("3.4.0"));
    set_global_const("RUBY_PLATFORM", str_from(if cfg!(target_os = "macos") { "arm64-darwin" } else { "x86_64-linux" }));
    set_global_const("RUBY_ENGINE", str_from("kenny"));
    set_global_const("RUBY_PATCHLEVEL", mkfix(0));
    set_global_const("RUBY_RELEASE_DATE", str_from("2026-10-19"));
    set_global_const("RUBY_COPYRIGHT", str_from("Kenny Ruby"));
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
