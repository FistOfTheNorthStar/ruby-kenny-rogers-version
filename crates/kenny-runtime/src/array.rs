//! Array.

use crate::core::*;
use crate::gc::RVec;
use crate::fmt::*;
use crate::kernel::new_str_v;
use crate::numeric::{arith, num_cmp, Op};
use crate::tri;
use std::cmp::Ordering;

pub fn av(v: V) -> &'static mut SVec<V> {
    &mut rary(v).v
}
/// The elements, rooted while iterated (see `gc::Rooted`).
fn snapshot(v: V) -> crate::gc::Rooted<V> {
    crate::gc::Rooted::of(rary(v).v.clone())
}
/// An Array argument: an Array, or an object converted with `to_ary` (implicit conversion).
/// (Rooted: callers compare the items with `eql?`/`==`, which may run Ruby code.)
pub fn ary_arg(v: V) -> Result<crate::gc::Rooted<V>, V> {
    if is_ary(v) {
        return Ok(snapshot(v));
    }
    if respond_to(v, sym("to_ary")) {
        let r = send0(v, "to_ary");
        if r == UNDEF {
            return Err(UNDEF);
        }
        if is_ary(r) {
            return Ok(snapshot(r));
        }
    }
    Err(type_error(&format!("no implicit conversion of {} into Array", class_name(real_class(v)))))
}
fn mutate(s: V) -> Result<&'static mut SVec<V>, V> {
    if is_frozen(s) {
        return Err(check_frozen(s));
    }
    Ok(av(s))
}
macro_rules! mutv {
    ($s:expr) => {
        match mutate($s) {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

/// Ruby `<=>` returning an Ordering, raising ArgumentError if incomparable.
pub fn cmp(a: V, b: V) -> Result<Ordering, V> {
    if is_fix(a) && is_fix(b) {
        return Ok(fix(a).cmp(&fix(b)));
    }
    if let Some(o) = num_cmp(a, b) {
        return Ok(o);
    }
    if is_str(a) && is_str(b) {
        return Ok(rstr(a).b.cmp(&rstr(b).b));
    }
    let r = send(a, sym("<=>"), &[b], QNIL);
    if r == UNDEF {
        return Err(UNDEF);
    }
    if is_fix(r) {
        return Ok(fix(r).cmp(&0));
    }
    Err(arg_error(&format!("comparison of {} with {} failed", class_name(real_class(a)), inspect_lossy(b))))
}
fn cmp_blk(blk: V, a: V, b: V) -> Result<Ordering, V> {
    let r = call_block(blk, &[a, b]);
    if r == UNDEF {
        return Err(UNDEF);
    }
    if is_fix(r) {
        Ok(fix(r).cmp(&0))
    } else {
        Err(arg_error(&format!("comparison of {} with {} failed", class_name(real_class(a)), inspect_lossy(b))))
    }
}

/// Stable merge sort with a fallible comparator.
pub fn sort_by_fn(v: &mut Vec<V>, f: &mut dyn FnMut(V, V) -> Result<Ordering, V>) -> Result<(), V> {
    let n = v.len();
    if n < 2 {
        return Ok(());
    }
    let mut buf = v.clone();
    let mut width = 1;
    // insertion sort small runs
    let run = 16;
    for start in (0..n).step_by(run) {
        let end = (start + run).min(n);
        for i in start + 1..end {
            let mut j = i;
            while j > start && f(v[j - 1], v[j])? == Ordering::Greater {
                v.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    width = width.max(run);
    while width < n {
        let mut i = 0;
        while i < n {
            let mid = (i + width).min(n);
            let hi = (i + 2 * width).min(n);
            let (mut l, mut r, mut k) = (i, mid, i);
            while l < mid && r < hi {
                if f(v[l], v[r])? != Ordering::Greater {
                    buf[k] = v[l];
                    l += 1;
                } else {
                    buf[k] = v[r];
                    r += 1;
                }
                k += 1;
            }
            while l < mid {
                buf[k] = v[l];
                l += 1;
                k += 1;
            }
            while r < hi {
                buf[k] = v[r];
                r += 1;
                k += 1;
            }
            i += 2 * width;
        }
        std::mem::swap(v, &mut buf);
        width *= 2;
    }
    Ok(())
}

fn sorted(s: V, blk: V) -> Result<Vec<V>, V> {
    let mut v = snapshot(s);
    if is_proc(blk) {
        sort_by_fn(&mut v, &mut |a, b| cmp_blk(blk, a, b))?;
    } else if v.iter().all(|x| is_fix(*x)) {
        // all Fixnums: a total order, so the standard library sort is safe (and fast)
        v.sort_unstable_by_key(|x| fix(*x));
    } else {
        sort_by_fn(&mut v, &mut cmp)?;
    }
    Ok(v.into_vec())
}
fn sort_by_keys(s: V, blk: V) -> Result<Vec<V>, V> {
    // keys and items stay rooted while sorting; the sort permutes indices
    let items = RVec::from(snapshot(s).into_vec());
    let mut keys = RVec::with_capacity(items.len());
    for &x in items.iter() {
        let k = call_block(blk, &[x]);
        if k == UNDEF {
            return Err(UNDEF);
        }
        keys.push(k);
    }
    let mut idx: Vec<V> = (0..items.len() as u64).collect();
    if keys.iter().all(|k| is_fix(*k)) {
        idx.sort_by_key(|i| fix(keys[*i as usize]));
    } else {
        sort_by_fn(&mut idx, &mut |a, b| cmp(keys[a as usize], keys[b as usize]))?;
    }
    Ok(idx.into_iter().map(|i| items[i as usize]).collect())
}

fn norm_index(i: i64, len: usize) -> Option<usize> {
    let i = if i < 0 { i + len as i64 } else { i };
    if i < 0 { None } else { Some(i as usize) }
}

pub fn aref(s: V, a: &[V]) -> V {
    let v = av(s);
    let len = v.len() as i64;
    if a.len() == 2 {
        let (st, n) = (fix(a[0]), fix(a[1]));
        let st = if st < 0 { st + len } else { st };
        if st < 0 || st > len || n < 0 {
            return QNIL;
        }
        let end = (st + n).min(len);
        return ary_new(v[st as usize..end as usize].to_vec());
    }
    let i = a[0];
    if is_fix(i) {
        return norm_index(fix(i), v.len()).and_then(|i| v.get(i).copied()).unwrap_or(QNIL);
    }
    if is_range(i) {
        return match crate::string::range_bounds(i, len) {
            Some((st, n)) => ary_new(v[st as usize..(st + n).min(len) as usize].to_vec()),
            None => QNIL,
        };
    }
    if is_float(i) {
        return aref(s, &[mkfix(float_val(i) as i64)]);
    }
    type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(i))))
}

fn aset(s: V, a: &[V]) -> V {
    let v = mutv!(s);
    let len = v.len() as i64;
    let val = *a.last().unwrap();
    let (st, n) = if a.len() == 3 {
        (fix(a[0]), fix(a[1]))
    } else if is_range(a[0]) {
        match crate::string::range_bounds(a[0], len) {
            Some(x) => x,
            None => return raise(rt().c.range_error, "range out of range"),
        }
    } else if is_fix(a[0]) {
        let i = fix(a[0]);
        let i = if i < 0 { i + len } else { i };
        if i < 0 {
            return raise(rt().c.index_error, &format!("index {} too small for array; minimum: -{}", fix(a[0]), len));
        }
        let i = i as usize;
        if i >= v.len() {
            v.resize(i + 1, QNIL);
        }
        v[i] = val;
        return val;
    } else {
        return type_error("no implicit conversion into Integer");
    };
    let st = if st < 0 { st + len } else { st };
    if st < 0 {
        return raise(rt().c.index_error, "index out of range");
    }
    let st = st as usize;
    if st > v.len() {
        v.resize(st, QNIL);
    }
    let end = (st + n.max(0) as usize).min(v.len());
    let rep = if is_ary(val) { snapshot(val).into_vec() } else { vec![val] };
    v.splice(st..end, rep);
    val
}

pub fn flatten_into(out: &mut Vec<V>, v: &[V], depth: i64, seen: &mut Vec<V>) -> Result<bool, V> {
    let mut changed = false;
    for &x in v {
        if is_ary(x) && depth != 0 {
            if seen.contains(&x) {
                return Err(arg_error("tried to flatten recursive array"));
            }
            seen.push(x);
            flatten_into(out, &snapshot(x), depth - 1, seen)?;
            seen.pop();
            changed = true;
        } else {
            out.push(x);
        }
    }
    Ok(changed)
}

pub fn join(s: V, sep: &[u8], seen: &mut Vec<V>) -> Result<Vec<u8>, V> {
    let mut out = Vec::new();
    for (i, x) in snapshot(s).into_iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        if is_ary(x) {
            if seen.contains(&x) {
                return Err(arg_error("recursive array join"));
            }
            seen.push(x);
            out.extend(join(x, sep, seen)?);
            seen.pop();
        } else {
            out.extend(to_s(x).map_err(|_| UNDEF)?);
        }
    }
    Ok(out)
}

/// `sum` of the block values an inlined `ary.sum { ... }` collected (as Array#sum does after
/// mapping with the block).
#[unsafe(no_mangle)]
pub extern "C" fn kn_ary_sum_vals(vals: V) -> V {
    kahan_sum(mkfix(0), &snapshot(vals))
}
fn kahan_sum(init: V, vals: &[V]) -> V {
    let mut acc = init;
    let mut i = 0;
    while i < vals.len() {
        let x = vals[i];
        if is_float(acc) || (is_float(x) && num_cmp(acc, mkfix(0)).is_some()) {
            // Kahan-Babuska from here on while values stay numeric
            let mut sum = crate::fmt::num_to_f(acc).unwrap();
            let mut c = 0.0;
            while i < vals.len() {
                // (Rationals join the Float sum, as in CRuby; anything else is added with `+`)
                let f = match crate::fmt::num_to_f(vals[i]) {
                    Some(f) => f,
                    None if crate::rational::is_rational(vals[i]) => match crate::fmt::num_to_f(tri!(send0(vals[i], "to_f"))) {
                        Some(f) => f,
                        None => break,
                    },
                    None => break,
                };
                i += 1;
                // (as CRuby: NaN and infinities skip the compensation, which they'd turn into
                // NaN: `[1, Float::INFINITY].sum` is Infinity)
                if sum.is_nan() {
                    continue;
                }
                if f.is_nan() {
                    sum = f;
                    continue;
                }
                if f.is_infinite() {
                    sum = if sum.is_infinite() && f.is_sign_negative() != sum.is_sign_negative() { f64::NAN } else { f };
                    continue;
                }
                if sum.is_infinite() {
                    continue;
                }
                let t = sum + f;
                if sum.abs() >= f.abs() {
                    c += (sum - t) + f;
                } else {
                    c += (f - t) + sum;
                }
                sum = t;
            }
            acc = float_new(if sum.is_finite() { sum + c } else { sum });
            if i == vals.len() {
                break;
            }
            // a non-Float-able value (a Complex, ...): `+` on it, then on
            acc = tri!(send1(acc, "+", vals[i]));
            i += 1;
            continue;
        }
        acc = tri!(if is_fix(acc) || is_big(acc) { arith(Op::Add, acc, x) } else { send1(acc, "+", x) });
        i += 1;
    }
    acc
}

pub fn enum_or<F: FnOnce() -> V>(s: V, name: &str, a: &[V], blk: V, f: F) -> V {
    if !is_proc(blk) {
        return crate::misc::enum_for(s, name, a);
    }
    f()
}

fn pattern_match(pat: V, x: V) -> V {
    send1(pat, "===", x)
}

fn combinations(v: &[V], k: usize) -> Vec<Vec<V>> {
    let n = v.len();
    if k > n {
        return vec![];
    }
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        out.push(idx.iter().map(|&i| v[i]).collect());
        let mut i = k;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - k {
                break;
            }
            if i == 0 && idx[0] == n - k {
                return out;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}
fn permutations(v: &[V], k: usize) -> Vec<Vec<V>> {
    if k == 0 {
        return vec![vec![]];
    }
    let mut out = Vec::new();
    for i in 0..v.len() {
        let mut rest = v.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest, k - 1) {
            p.insert(0, x);
            out.push(p);
        }
    }
    out
}

pub fn init() {
    let c = rt().c.array;
    fn fill_init(target: V, a: &[V], b: V) -> V {
        let mut v = RVec::new();
        if let Some(&n) = a.first() {
            if is_ary(n) {
                v.extend(snapshot(n));
            } else {
                if !is_fix(n) || fix(n) < 0 {
                    return arg_error("negative array size");
                }
                for i in 0..fix(n) {
                    let x = if is_proc(b) { tri!(call_block(b, &[mkfix(i)])) } else { a.get(1).copied().unwrap_or(QNIL) };
                    v.push(x);
                }
            }
        }
        av(target).set(v.take());
        target
    }
    defs(c, "new", 0, 2, |s, a, b| {
        if s as *mut Class != rt().c.array {
            return crate::kernel::class_new_instance(s, a, b);
        }
        let r = ary_new(vec![]);
        tri!(fill_init(r, a, b));
        r
    });
    defs(c, "[]", 0, -1, |s, a, _| {
        let r = ary_new(a.to_vec());
        // `Sub[...]` builds an instance of the receiver (an Array subclass)
        if is_class(s) && s != rt().c.array as V {
            hdr(r).klass = s as *mut Class;
        }
        r
    });
    defn(c, "initialize", 0, 2, |s, a, b| {
        tri!(fill_init(s, a, b));
        QNIL
    });
    defn(c, "[]", 1, 2, |s, a, _| aref(s, a));
    defn(c, "slice", 1, 2, |s, a, _| aref(s, a));
    defn(c, "[]=", 2, 3, |s, a, _| aset(s, a));
    defn(c, "at", 1, 1, |s, a, _| aref(s, a));
    defn(c, "dig", 1, -1, |s, a, _| {
        let x = tri!(aref(s, &a[..1]));
        if a.len() == 1 || x == QNIL { x } else { send(x, sym("dig"), &a[1..], QNIL) }
    });
    defn(c, "fetch", 1, 2, |s, a, b| {
        let v = av(s);
        match norm_index(fix(a[0]), v.len()).and_then(|i| v.get(i).copied()) {
            Some(x) => x,
            None if is_proc(b) => call_block(b, &[a[0]]),
            None if a.len() == 2 => a[1],
            None => raise(rt().c.index_error, &format!("index {} outside of array bounds: {}...{}", fix(a[0]), -(v.len() as i64), v.len())),
        }
    });
    defn(c, "values_at", 0, -1, |s, a, _| {
        let v = av(s);
        ary_new(a.iter().map(|i| norm_index(fix(*i), v.len()).and_then(|i| v.get(i).copied()).unwrap_or(QNIL)).collect())
    });
    defn(c, "first", 0, 1, |s, a, _| {
        let v = av(s);
        match a.first() {
            None => v.first().copied().unwrap_or(QNIL),
            Some(n) => ary_new(v[..(fix(*n).max(0) as usize).min(v.len())].to_vec()),
        }
    });
    defn(c, "last", 0, 1, |s, a, _| {
        let v = av(s);
        match a.first() {
            None => v.last().copied().unwrap_or(QNIL),
            Some(n) => {
                let k = (fix(*n).max(0) as usize).min(v.len());
                ary_new(v[v.len() - k..].to_vec())
            }
        }
    });
    fn push(s: V, a: &[V]) -> V {
        mutv!(s).extend_from_slice(a);
        s
    }
    defn(c, "push", 0, -1, |s, a, _| push(s, a));
    defn(c, "append", 0, -1, |s, a, _| push(s, a));
    defn(c, "<<", 1, 1, |s, a, _| push(s, a));
    defn(c, "pop", 0, 1, |s, a, _| {
        let v = mutv!(s);
        match a.first() {
            None => v.pop().unwrap_or(QNIL),
            Some(n) => {
                let k = (fix(*n).max(0) as usize).min(v.len());
                ary_new(v.split_off(v.len() - k))
            }
        }
    });
    defn(c, "shift", 0, 1, |s, a, _| {
        let v = mutv!(s);
        match a.first() {
            None => {
                v.shift().unwrap_or(QNIL)
            }
            Some(n) => {
                let k = (fix(*n).max(0) as usize).min(v.len());
                ary_new(v.drain(..k).collect())
            }
        }
    });
    fn unshift(s: V, a: &[V]) -> V {
        let v = mutv!(s);
        v.splice(0..0, a.iter().copied());
        s
    }
    defn(c, "unshift", 0, -1, |s, a, _| unshift(s, a));
    defn(c, "prepend", 0, -1, |s, a, _| unshift(s, a));
    defn(c, "insert", 1, -1, |s, a, _| {
        let v = mutv!(s);
        let i = fix(a[0]);
        let i = if i < 0 { i + v.len() as i64 + 1 } else { i };
        if i < 0 {
            return raise(rt().c.index_error, "index out of array");
        }
        let i = i as usize;
        if i > v.len() {
            v.resize(i, QNIL);
        }
        v.splice(i..i, a[1..].iter().copied());
        s
    });
    defn(c, "concat", 0, -1, |s, a, _| {
        let mut add = Vec::new();
        for &x in a {
            match ary_arg(x) {
                Ok(v) => add.extend(v),
                Err(e) => return e,
            }
        }
        mutv!(s).extend(add);
        s
    });
    defn(c, "length", 0, 0, |s, _, _| mkfix(av(s).len() as i64));
    defn(c, "size", 0, 0, |s, _, _| mkfix(av(s).len() as i64));
    defn(c, "empty?", 0, 0, |s, _, _| bool_v(av(s).is_empty()));
    defn(c, "include?", 1, 1, |s, a, _| {
        for x in snapshot(s) {
            if truthy(tri!(equal(x, a[0]))) {
                return QTRUE;
            }
        }
        QFALSE
    });
    defn(c, "member?", 1, 1, |s, a, _| send(s, sym("include?"), a, QNIL));
    fn index(s: V, a: &[V], b: V, rev: bool) -> V {
        let items = snapshot(s);
        let order: Vec<usize> = if rev { (0..items.len()).rev().collect() } else { (0..items.len()).collect() };
        for i in order {
            let hit = if let Some(&t) = a.first() { tri!(equal(items[i], t)) } else { tri!(call_block(b, &[items[i]])) };
            if truthy(hit) {
                return mkfix(i as i64);
            }
        }
        QNIL
    }
    defn(c, "index", 0, 1, |s, a, b| index(s, a, b, false));
    defn(c, "find_index", 0, 1, |s, a, b| index(s, a, b, false));
    defn(c, "rindex", 0, 1, |s, a, b| index(s, a, b, true));
    defn(c, "each", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each", &[]);
        }
        let mut i = 0;
        while i < rary(s).v.len() {
            let x = rary(s).v[i];
            tri!(call_block(b, &[x]));
            i += 1;
        }
        s
    });
    defn(c, "each_index", 0, 0, |s, _, b| {
        let mut i = 0;
        while i < rary(s).v.len() {
            tri!(call_block(b, &[mkfix(i as i64)]));
            i += 1;
        }
        s
    });
    defn(c, "each_with_index", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each_with_index", &[]);
        }
        // (Enumerable's in Ruby: through a redefined `each`)
        if unsafe { find_method(class_of(s), sym("each")).as_ref() }.is_none_or(|m| m.opt != crate::opt::OPT_ARY_EACH) {
            return crate::misc::each_with_index_via_each(s, b);
        }
        let mut i = 0;
        while i < rary(s).v.len() {
            let x = rary(s).v[i];
            tri!(call_block(b, &[x, mkfix(i as i64)]));
            i += 1;
        }
        s
    });
    defn(c, "reverse_each", 0, 0, |s, a, b| {
        enum_or(s, "reverse_each", a, b, || {
            for x in snapshot(s).into_iter().rev() {
                tri!(call_block(b, &[x]));
            }
            s
        })
    });
    defn(c, "each_with_object", 1, 1, |s, a, b| {
        for x in snapshot(s) {
            tri!(call_block(b, &[x, a[0]]));
        }
        a[0]
    });
    fn map(s: V, b: V) -> V {
        // like CRuby, re-reads the length each time (the block may change the array)
        let mut out = RVec::with_capacity(rary(s).v.len());
        let mut i = 0;
        while i < rary(s).v.len() {
            let x = rary(s).v[i];
            out.push(tri!(call_block(b, &[x])));
            i += 1;
        }
        ary_new(out.take())
    }
    defn(c, "map", 0, 0, |s, a, b| enum_or(s, "map", a, b, || map(s, b)));
    defn(c, "collect", 0, 0, |s, a, b| enum_or(s, "collect", a, b, || map(s, b)));
    defn(c, "flat_map", 0, 0, |s, a, b| {
        enum_or(s, "flat_map", a, b, || {
            let mut out = RVec::new();
            for x in snapshot(s) {
                let mut r = tri!(call_block(b, &[x]));
                // like CRuby (rb_check_array_type): objects with to_ary are flattened too
                // (ActiveRecord's preloader flat_maps Relations)
                if !is_ary(r) && is_heap(r) && respond_to(r, sym("to_ary")) {
                    let t = tri!(send(r, sym("to_ary"), &[], QNIL));
                    if is_ary(t) {
                        r = t;
                    }
                }
                if is_ary(r) {
                    out.extend(snapshot(r));
                } else {
                    out.push(r);
                }
            }
            ary_new(out.take())
        })
    });
    defn(c, "collect_concat", 0, 0, |s, _, b| send(s, sym("flat_map"), &[], b));
    fn map_bang(s: V, b: V) -> V {
        let r = tri!(map(s, b));
        mutv!(s).set(snapshot(r).into_vec());
        s
    }
    defn(c, "map!", 0, 0, |s, _, b| map_bang(s, b));
    defn(c, "collect!", 0, 0, |s, _, b| map_bang(s, b));
    fn filter(s: V, b: V, keep: bool) -> V {
        // like CRuby, re-reads the length each time (the block may change the array)
        let mut out = RVec::new();
        let mut i = 0;
        while i < rary(s).v.len() {
            let x = rary(s).v[i];
            if truthy(tri!(call_block(b, &[x]))) == keep {
                out.push(x);
            }
            i += 1;
        }
        ary_new(out.take())
    }
    defn(c, "select", 0, 0, |s, a, b| enum_or(s, "select", a, b, || filter(s, b, true)));
    defn(c, "filter", 0, 0, |s, a, b| enum_or(s, "filter", a, b, || filter(s, b, true)));
    defn(c, "find_all", 0, 0, |s, a, b| enum_or(s, "find_all", a, b, || filter(s, b, true)));
    defn(c, "reject", 0, 0, |s, a, b| enum_or(s, "reject", a, b, || filter(s, b, false)));
    fn filter_bang(s: V, b: V, keep: bool) -> (V, bool) {
        let before = av(s).len();
        let r = filter(s, b, keep);
        if r == UNDEF {
            return (UNDEF, false);
        }
        match mutate(s) {
            Ok(v) => v.set(snapshot(r).into_vec()),
            Err(e) => return (e, false),
        }
        (s, av(s).len() != before)
    }
    defn(c, "select!", 0, 0, |s, _, b| match filter_bang(s, b, true) {
        (UNDEF, _) => UNDEF,
        (_, true) => s,
        _ => QNIL,
    });
    defn(c, "filter!", 0, 0, |s, _, b| match filter_bang(s, b, true) {
        (UNDEF, _) => UNDEF,
        (_, true) => s,
        _ => QNIL,
    });
    defn(c, "keep_if", 0, 0, |s, _, b| filter_bang(s, b, true).0);
    defn(c, "reject!", 0, 0, |s, _, b| match filter_bang(s, b, false) {
        (UNDEF, _) => UNDEF,
        (_, true) => s,
        _ => QNIL,
    });
    defn(c, "delete_if", 0, 0, |s, _, b| filter_bang(s, b, false).0);
    defn(c, "filter_map", 0, 0, |s, a, b| {
        enum_or(s, "filter_map", a, b, || {
            let mut out = RVec::new();
            for x in snapshot(s) {
                let r = tri!(call_block(b, &[x]));
                if truthy(r) {
                    out.push(r);
                }
            }
            ary_new(out.take())
        })
    });
    // find(ifnone = nil) { |x| }: ifnone.call when nothing is found
    fn find(s: V, a: &[V], b: V, name: &str) -> V {
        if b == QNIL {
            return crate::misc::enum_for(s, name, a);
        }
        for x in snapshot(s) {
            if truthy(tri!(call_block(b, &[x]))) {
                return x;
            }
        }
        crate::misc::find_ifnone(a)
    }
    defn(c, "find", 0, 1, |s, a, b| find(s, a, b, "find"));
    defn(c, "detect", 0, 1, |s, a, b| find(s, a, b, "detect"));
    // Ruby 4.0: the last element the block is true for
    if crate::misc::ruby4() {
        defn(c, "rfind", 0, 1, |s, a, b| {
            if b == QNIL {
                return crate::misc::enum_for(s, "rfind", a);
            }
            let v = snapshot(s);
            for &x in v.iter().rev() {
                if truthy(tri!(call_block(b, &[x]))) {
                    return x;
                }
            }
            crate::misc::find_ifnone(a)
        });
    }
    fn inject(s: V, a: &[V], b: V) -> V {
        let mut items = snapshot(s).into_iter();
        let (mut acc, op) = match (a.len(), is_proc(b)) {
            (0, _) => (items.next().unwrap_or(QNIL), None),
            (1, true) => (a[0], None),
            (1, false) => (items.next().unwrap_or(QNIL), Some(a[0])),
            _ => (a[0], Some(a[1])),
        };
        for x in items {
            acc = match op {
                Some(o) => {
                    let id = match crate::kernel::sym_arg(o) {
                        Ok(i) => i,
                        Err(e) => return e,
                    };
                    if is_fix(acc) && is_fix(x) && sym_name(id) == b"+" {
                        arith(Op::Add, acc, x)
                    } else {
                        tri!(send(acc, id, &[x], QNIL))
                    }
                }
                None => tri!(call_block(b, &[acc, x])),
            };
        }
        acc
    }
    defn(c, "inject", 0, 2, inject);
    defn(c, "reduce", 0, 2, inject);
    defn(c, "sum", 0, 1, |s, a, b| {
        let init = a.first().copied().unwrap_or(mkfix(0));
        let vals = if is_proc(b) { tri!(map(s, b)) } else { s };
        kahan_sum(init, &snapshot(vals))
    });
    fn minmax_impl(s: V, a: &[V], b: V, want: Ordering) -> V {
        let mut v = match sorted(s, b) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if want == Ordering::Greater {
            v.reverse();
        }
        match a.first() {
            Some(n) => ary_new(v.into_iter().take(fix(*n).max(0) as usize).collect()),
            None => v.first().copied().unwrap_or(QNIL),
        }
    }
    defn(c, "min", 0, 1, |s, a, b| {
        if a.is_empty() && !is_proc(b) {
            let mut best = QNIL;
            for x in snapshot(s) {
                if best == QNIL {
                    best = x;
                    continue;
                }
                match cmp(x, best) {
                    Ok(Ordering::Less) => best = x,
                    Ok(_) => {}
                    Err(e) => return e,
                }
            }
            return best;
        }
        minmax_impl(s, a, b, Ordering::Less)
    });
    defn(c, "max", 0, 1, |s, a, b| {
        if a.is_empty() && !is_proc(b) {
            let mut best = QNIL;
            for x in snapshot(s) {
                if best == QNIL {
                    best = x;
                    continue;
                }
                match cmp(x, best) {
                    Ok(Ordering::Greater) => best = x,
                    Ok(_) => {}
                    Err(e) => return e,
                }
            }
            return best;
        }
        minmax_impl(s, a, b, Ordering::Greater)
    });
    defn(c, "minmax", 0, 0, |s, _, b| match sorted(s, b) {
        Ok(v) => ary_new(vec![v.first().copied().unwrap_or(QNIL), v.last().copied().unwrap_or(QNIL)]),
        Err(e) => e,
    });
    fn by(s: V, a: &[V], b: V, rev: bool) -> V {
        let mut v = match sort_by_keys(s, b) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if rev {
            v.reverse();
        }
        match a.first() {
            Some(n) => ary_new(v.into_iter().take(fix(*n).max(0) as usize).collect()),
            None => v.first().copied().unwrap_or(QNIL),
        }
    }
    defn(c, "min_by", 0, 1, |s, a, b| enum_or(s, "min_by", a, b, || by(s, a, b, false)));
    defn(c, "max_by", 0, 1, |s, a, b| {
        enum_or(s, "max_by", a, b, || {
            if a.is_empty() {
                // first maximal element, like CRuby
                let mut best = QNIL;
                let mut bestk = QNIL;
                for x in snapshot(s) {
                    let k = tri!(call_block(b, &[x]));
                    if best == QNIL && bestk == QNIL {
                        best = x;
                        bestk = k;
                        continue;
                    }
                    match cmp(k, bestk) {
                        Ok(Ordering::Greater) => {
                            best = x;
                            bestk = k
                        }
                        Ok(_) => {}
                        Err(e) => return e,
                    }
                }
                return best;
            }
            by(s, a, b, true)
        })
    });
    defn(c, "minmax_by", 0, 0, |s, _, b| match sort_by_keys(s, b) {
        Ok(v) => ary_new(vec![v.first().copied().unwrap_or(QNIL), v.last().copied().unwrap_or(QNIL)]),
        Err(e) => e,
    });
    defn(c, "sort", 0, 0, |s, _, b| match sorted(s, b) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(c, "sort!", 0, 0, |s, _, b| match sorted(s, b) {
        Ok(v) => {
            mutv!(s).set(v);
            s
        }
        Err(e) => e,
    });
    defn(c, "sort_by", 0, 0, |s, a, b| {
        enum_or(s, "sort_by", a, b, || match sort_by_keys(s, b) {
            Ok(v) => ary_new(v),
            Err(e) => e,
        })
    });
    defn(c, "sort_by!", 0, 0, |s, _, b| match sort_by_keys(s, b) {
        Ok(v) => {
            mutv!(s).set(v);
            s
        }
        Err(e) => e,
    });
    defn(c, "reverse", 0, 0, |s, _, _| ary_new(snapshot(s).into_iter().rev().collect()));
    defn(c, "reverse!", 0, 0, |s, _, _| {
        mutv!(s).reverse();
        s
    });
    defn(c, "rotate", 0, 1, |s, a, _| {
        let mut v = snapshot(s);
        if !v.is_empty() {
            let n = a.first().map(|x| fix(*x)).unwrap_or(1).rem_euclid(v.len() as i64) as usize;
            v.rotate_left(n);
        }
        ary_new(v.into_vec())
    });
    defn(c, "join", 0, 1, |s, a, _| {
        let sep = match a.first() {
            Some(x) if is_str(*x) => rstr(*x).b.clone(),
            _ => Vec::new(),
        };
        match join(s, &sep, &mut vec![s]) {
            Ok(b) => {
                // joining binary (ASCII-8BIT) strings yields a binary string
                let bin = !b.is_ascii() && rary(s).v.iter().any(|x| is_str(*x) && hdr(*x).flags & crate::compat::FL_BINARY != 0);
                let r = str_new(b);
                if bin {
                    hdr(r).flags |= crate::compat::FL_BINARY;
                }
                r
            }
            Err(e) => e,
        }
    });
    defn(c, "inspect", 0, 0, |s, _, _| match ary_inspect(s) {
        Ok(x) => new_str_v(x),
        Err(()) => UNDEF,
    });
    defn(c, "to_s", 0, 0, |s, _, _| match ary_inspect(s) {
        Ok(x) => new_str_v(x),
        Err(()) => UNDEF,
    });
    // on an Array subclass, to_a returns a plain Array
    defn(c, "to_a", 0, 0, |s, _, _| if real_class(s) == rt().c.array { s } else { ary_new(snapshot(s).into_vec()) });
    defn(c, "entries", 0, 0, |s, _, _| ary_new(snapshot(s).into_vec()));
    defn(c, "to_ary", 0, 0, |s, _, _| s);
    defn(c, "deconstruct", 0, 0, |s, _, _| s);
    defn(c, "to_h", 0, 0, |s, _, b| {
        let h = hash_new();
        for x in snapshot(s) {
            let pair = if is_proc(b) { tri!(call_block(b, &[x])) } else { x };
            if !is_ary(pair) || rary(pair).v.len() != 2 {
                return type_error(&format!("wrong element type {} (expected array)", class_name(real_class(pair))));
            }
            rhash(h).m.insert(HKey(crate::hash::key_dup(rary(pair).v[0])), rary(pair).v[1]);
        }
        h
    });
    defn(c, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(c, "eql?", 1, 1, |s, a, _| bool_v(eql(s, a[0])));
    defn(c, "==", 1, 1, |s, a, _| {
        if !is_ary(a[0]) {
            // like CRuby: an object that converts to an Array decides equality itself
            if respond_to(a[0], sym("to_ary")) {
                return bool_v(truthy(tri!(send(a[0], sym("=="), &[s], QNIL))));
            }
            return QFALSE;
        }
        let (x, y) = (snapshot(s), snapshot(a[0]));
        if x.len() != y.len() {
            return QFALSE;
        }
        for (p, q) in x.into_iter().zip(y) {
            if p != q && !truthy(tri!(equal(p, q))) {
                return QFALSE;
            }
        }
        QTRUE
    });
    defn(c, "<=>", 1, 1, |s, a, _| {
        if !is_ary(a[0]) {
            return QNIL;
        }
        let (x, y) = (snapshot(s), snapshot(a[0]));
        for (p, q) in x.iter().zip(&y) {
            match cmp(*p, *q) {
                Ok(Ordering::Equal) => {}
                Ok(o) => return mkfix(o as i64),
                Err(e) => return e,
            }
        }
        mkfix(x.len().cmp(&y.len()) as i64)
    });
    defn(c, "+", 1, 1, |s, a, _| {
        let o = match ary_arg(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let mut v = snapshot(s);
        v.extend(o);
        ary_new(v.into_vec())
    });
    fn contains(v: &[V], x: V) -> bool {
        v.iter().any(|y| eql(*y, x))
    }
    defn(c, "-", 1, 1, |s, a, _| {
        let o = match ary_arg(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        ary_new(snapshot(s).into_iter().filter(|x| !contains(&o, *x)).collect())
    });
    defn(c, "difference", 0, -1, |s, a, _| {
        let mut v = snapshot(s);
        for &o in a {
            let o = snapshot(o);
            v.retain(|x| !contains(&o, *x));
        }
        ary_new(v.into_vec())
    });
    defn(c, "*", 1, 1, |s, a, _| {
        if is_str(a[0]) {
            return match join(s, &rstr(a[0]).b.clone(), &mut vec![s]) {
                Ok(b) => str_new(b),
                Err(e) => e,
            };
        }
        let n = fix(a[0]);
        if n < 0 {
            return arg_error("negative argument");
        }
        ary_new(snapshot(s).repeat(n as usize))
    });
    fn uniq(v: Vec<V>) -> Vec<V> {
        // rooted: `hash`/`eql?` may run Ruby code (and collect) while `v` holds the items
        let v = crate::gc::Rooted::of(v);
        let mut seen = indexmap::IndexSet::new();
        let mut out = RVec::new();
        for x in v {
            if seen.insert(HKey(x)) {
                out.push(x);
            }
        }
        out.take()
    }
    defn(c, "&", 1, 1, |s, a, _| {
        let o = match ary_arg(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        ary_new(uniq(snapshot(s).into_iter().filter(|x| contains(&o, *x)).collect()))
    });
    defn(c, "intersection", 0, -1, |s, a, _| {
        let mut v = uniq(snapshot(s).into_vec());
        for &o in a {
            let o = snapshot(o);
            v.retain(|x| contains(&o, *x));
        }
        ary_new(v)
    });
    defn(c, "intersect?", 1, 1, |s, a, _| {
        let o = match ary_arg(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        bool_v(snapshot(s).into_iter().any(|x| contains(&o, x)))
    });
    defn(c, "|", 1, 1, |s, a, _| {
        let o = match ary_arg(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let mut v = snapshot(s);
        v.extend(o);
        ary_new(uniq(v.into_vec()))
    });
    defn(c, "union", 0, -1, |s, a, _| {
        let mut v = snapshot(s);
        for &o in a {
            v.extend(snapshot(o));
        }
        ary_new(uniq(v.into_vec()))
    });
    defn(c, "uniq", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return ary_new(uniq(snapshot(s).into_vec()));
        }
        let mut seen = indexmap::IndexSet::new();
        let mut keys = RVec::new();
        let mut out = RVec::new();
        for x in snapshot(s) {
            let k = tri!(call_block(b, &[x]));
            keys.push(k);
            if seen.insert(HKey(k)) {
                out.push(x);
            }
        }
        ary_new(out.take())
    });
    defn(c, "uniq!", 0, 0, |s, _, _| {
        let before = av(s).len();
        let u = uniq(snapshot(s).into_vec());
        let changed = u.len() != before;
        mutv!(s).set(u);
        if changed { s } else { QNIL }
    });
    defn(c, "flatten", 0, 1, |s, a, _| {
        let depth = a.first().map(|x| fix(*x)).unwrap_or(-1);
        let mut out = RVec::new();
        match flatten_into(&mut out, &snapshot(s), depth, &mut vec![s]) {
            Ok(_) => ary_new(out.take()),
            Err(e) => e,
        }
    });
    defn(c, "flatten!", 0, 1, |s, a, _| {
        let depth = a.first().map(|x| fix(*x)).unwrap_or(-1);
        let mut out = RVec::new();
        match flatten_into(&mut out, &snapshot(s), depth, &mut vec![s]) {
            Ok(changed) => {
                mutv!(s).set(out.take());
                if changed { s } else { QNIL }
            }
            Err(e) => e,
        }
    });
    defn(c, "compact", 0, 0, |s, _, _| ary_new(snapshot(s).into_iter().filter(|x| *x != QNIL).collect()));
    defn(c, "compact!", 0, 0, |s, _, _| {
        let v = mutv!(s);
        let before = v.len();
        v.retain(|x| *x != QNIL);
        if v.len() != before { s } else { QNIL }
    });
    defn(c, "count", 0, 1, |s, a, b| {
        let mut n = 0;
        for x in snapshot(s) {
            let hit = if let Some(&t) = a.first() {
                truthy(tri!(equal(x, t)))
            } else if is_proc(b) {
                truthy(tri!(call_block(b, &[x])))
            } else {
                true
            };
            if hit {
                n += 1;
            }
        }
        mkfix(n)
    });
    fn quant(s: V, a: &[V], b: V, mode: u8) -> V {
        let mut hits = 0;
        let mut i = 0;
        while i < rary(s).v.len() {
            let x = rary(s).v[i];
            i += 1;
            let t = if let Some(&p) = a.first() {
                truthy(tri!(pattern_match(p, x)))
            } else if is_proc(b) {
                truthy(tri!(call_block(b, &[x])))
            } else {
                truthy(x)
            };
            match (mode, t) {
                (b'a', true) => return QTRUE,
                (b'l', false) => return QFALSE,
                (b'n', true) => return QFALSE,
                (b'o', true) => {
                    hits += 1;
                    if hits > 1 {
                        return QFALSE;
                    }
                }
                _ => {}
            }
        }
        match mode {
            b'a' => QFALSE,
            b'o' => bool_v(hits == 1),
            _ => QTRUE,
        }
    }
    defn(c, "any?", 0, 1, |s, a, b| quant(s, a, b, b'a'));
    defn(c, "all?", 0, 1, |s, a, b| quant(s, a, b, b'l'));
    defn(c, "none?", 0, 1, |s, a, b| quant(s, a, b, b'n'));
    defn(c, "one?", 0, 1, |s, a, b| quant(s, a, b, b'o'));
    defn(c, "take", 1, 1, |s, a, _| {
        if fix(a[0]) < 0 {
            return arg_error("attempt to take negative size");
        }
        ary_new(snapshot(s).into_iter().take(fix(a[0]) as usize).collect())
    });
    defn(c, "drop", 1, 1, |s, a, _| {
        if fix(a[0]) < 0 {
            return arg_error("attempt to drop negative size");
        }
        ary_new(snapshot(s).into_iter().skip(fix(a[0]) as usize).collect())
    });
    defn(c, "take_while", 0, 0, |s, _, b| {
        let mut out = RVec::new();
        for x in snapshot(s) {
            if !truthy(tri!(call_block(b, &[x]))) {
                break;
            }
            out.push(x);
        }
        ary_new(out.take())
    });
    defn(c, "drop_while", 0, 0, |s, _, b| {
        let items = snapshot(s);
        let mut i = 0;
        while i < items.len() && truthy(tri!(call_block(b, &[items[i]]))) {
            i += 1;
        }
        ary_new(items[i..].to_vec())
    });
    defn(c, "zip", 0, -1, |s, a, b| {
        // other arguments: Arrays, `to_ary`, else their first elements by `each` (`take`,
        // which stops early: an endless Enumerator is fine)
        let n = av(s).len();
        let mut others: Vec<crate::gc::Rooted<V>> = Vec::with_capacity(a.len());
        for &o in a {
            if is_ary(o) {
                others.push(snapshot(o));
            } else if respond_to(o, sym("to_ary")) {
                let r = tri!(send0(o, "to_ary"));
                if !is_ary(r) {
                    return type_error(&format!("can't convert {} to Array", class_name(real_class(o))));
                }
                others.push(snapshot(r));
            } else if respond_to(o, sym("each")) {
                let r = tri!(send1(o, "take", mkfix(n as i64)));
                others.push(if is_ary(r) { snapshot(r) } else { crate::gc::Rooted::of(vec![]) });
            } else {
                return type_error(&format!("wrong argument type {} (must respond to :each)", class_name(real_class(o))));
            }
        }
        let mut out = RVec::new();
        for (i, x) in snapshot(s).into_iter().enumerate() {
            let mut row = vec![x];
            for o in &others {
                row.push(o.get(i).copied().unwrap_or(QNIL));
            }
            out.push(ary_new(row));
        }
        if is_proc(b) {
            // (the rows stay rooted in `out` while the block runs)
            for i in 0..out.len() {
                let r = out[i];
                tri!(call_block(b, &[r]));
            }
            return QNIL;
        }
        ary_new(out.take())
    });
    defn(c, "product", 0, -1, |s, a, _| {
        let mut acc: Vec<Vec<V>> = snapshot(s).into_iter().map(|x| vec![x]).collect();
        for &o in a {
            let o = snapshot(o);
            let mut next = Vec::new();
            for row in &acc {
                for &y in &o {
                    let mut r = row.clone();
                    r.push(y);
                    next.push(r);
                }
            }
            acc = next;
        }
        ary_new(acc.into_iter().map(ary_new).collect())
    });
    defn(c, "transpose", 0, 0, |s, _, _| {
        let rows: Vec<Vec<V>> = snapshot(s).into_vec().into_iter().map(|r| snapshot(r).into_vec()).collect();
        if rows.is_empty() {
            return ary_new(vec![]);
        }
        let n = rows[0].len();
        if rows.iter().any(|r| r.len() != n) {
            return raise(rt().c.index_error, "element size differs");
        }
        ary_new((0..n).map(|j| ary_new(rows.iter().map(|r| r[j]).collect())).collect())
    });
    defn(c, "combination", 1, 1, |s, a, b| {
        let combos = combinations(&snapshot(s), fix(a[0]).max(0) as usize);
        if !is_proc(b) {
            return crate::misc::enum_for(ary_new(combos.into_iter().map(ary_new).collect()), "each", &[]);
        }
        for cmb in combos {
            tri!(call_block(b, &[ary_new(cmb)]));
        }
        s
    });
    defn(c, "permutation", 0, 1, |s, a, b| {
        let v = snapshot(s);
        let k = a.first().map(|x| fix(*x) as usize).unwrap_or(v.len());
        let perms = permutations(&v, k);
        if !is_proc(b) {
            return crate::misc::enum_for(ary_new(perms.into_iter().map(ary_new).collect()), "each", &[]);
        }
        for p in perms {
            tri!(call_block(b, &[ary_new(p)]));
        }
        s
    });
    defn(c, "each_slice", 1, 1, |s, a, b| {
        let n = fix(a[0]);
        if n <= 0 {
            return arg_error("invalid slice size");
        }
        let chunks = RVec::from(snapshot(s).chunks(n as usize).map(|c| ary_new(c.to_vec())).collect());
        if !is_proc(b) {
            return crate::misc::enum_for(ary_new(chunks.to_vec()), "each", &[]);
        }
        for i in 0..chunks.len() {
            tri!(call_block(b, &[chunks[i]]));
        }
        s
    });
    defn(c, "each_cons", 1, 1, |s, a, b| {
        let n = fix(a[0]);
        if n <= 0 {
            return arg_error("invalid size");
        }
        let wins = RVec::from(snapshot(s).windows(n as usize).map(|c| ary_new(c.to_vec())).collect());
        if !is_proc(b) {
            return crate::misc::enum_for(ary_new(wins.to_vec()), "each", &[]);
        }
        for i in 0..wins.len() {
            tri!(call_block(b, &[wins[i]]));
        }
        s
    });
    defn(c, "group_by", 0, 0, |s, a, b| {
        enum_or(s, "group_by", a, b, || {
            let h = hash_new();
            for x in snapshot(s) {
                let k = tri!(call_block(b, &[x]));
                let m = &mut rhash(h).m;
                let e = m.entry(HKey(k)).or_insert_with(|| ary_new(vec![]));
                av(*e).push(x);
            }
            h
        })
    });
    defn(c, "partition", 0, 0, |s, _, b| {
        let (mut y, mut n) = (RVec::new(), RVec::new());
        for x in snapshot(s) {
            if truthy(tri!(call_block(b, &[x]))) {
                y.push(x)
            } else {
                n.push(x)
            }
        }
        ary_new(vec![ary_new(y.take()), ary_new(n.take())])
    });
    defn(c, "tally", 0, 0, |s, _, _| {
        let h = hash_new();
        for x in snapshot(s) {
            let m = &mut rhash(h).m;
            let e = m.entry(HKey(x)).or_insert(mkfix(0));
            *e = mkfix(fix(*e) + 1);
        }
        h
    });
    fn chunk_impl(s: V, b: V, split_when_true: bool) -> V {
        let items = snapshot(s);
        let mut out = RVec::new();
        let mut cur = RVec::new();
        for (i, &x) in items.iter().enumerate() {
            if i > 0 {
                let r = truthy(tri!(call_block(b, &[items[i - 1], x])));
                if r == split_when_true {
                    out.push(ary_new(std::mem::take(&mut cur)));
                }
            }
            cur.push(x);
        }
        if !cur.is_empty() {
            out.push(ary_new(cur.take()));
        }
        ary_new(out.take())
    }
    defn(c, "chunk_while", 0, 0, |s, _, b| chunk_impl(s, b, false));
    defn(c, "slice_when", 0, 0, |s, _, b| chunk_impl(s, b, true));
    defn(c, "each_entry", 0, 0, |s, _, b| send(s, sym("each"), &[], b));
    defn(c, "cycle", 0, 1, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "cycle", a);
        }
        let n = a.first().filter(|x| **x != QNIL).map(|x| fix(*x));
        let mut k = 0;
        loop {
            if n.is_some_and(|n| k >= n) || av(s).is_empty() {
                return QNIL;
            }
            for x in snapshot(s) {
                tri!(call_block(b, &[x]));
            }
            k += 1;
        }
    });
    defn(c, "clear", 0, 0, |s, _, _| {
        mutv!(s).clear();
        s
    });
    defn(c, "replace", 1, 1, |s, a, _| {
        let n = match ary_arg(a[0]) {
            Ok(n) => n,
            Err(e) => return e,
        };
        mutv!(s).set(n.into_vec());
        s
    });
    defn(c, "delete", 1, 1, |s, a, b| {
        let items = snapshot(s);
        let mut found = QNIL;
        let mut keep = Vec::new();
        for x in items {
            if truthy(tri!(equal(x, a[0]))) {
                found = x;
            } else {
                keep.push(x);
            }
        }
        mutv!(s).set(keep);
        if found == QNIL && is_proc(b) { call_block(b, &[a[0]]) } else { found }
    });
    defn(c, "delete_at", 1, 1, |s, a, _| {
        let v = mutv!(s);
        match norm_index(fix(a[0]), v.len()) {
            Some(i) if i < v.len() => v.remove(i),
            _ => QNIL,
        }
    });
    defn(c, "slice!", 1, 2, |s, a, _| {
        let r = tri!(aref(s, a));
        if r == QNIL {
            return QNIL;
        }
        let v = mutv!(s);
        let len = v.len() as i64;
        if a.len() == 2 || is_range(a[0]) {
            let (st, n) = if a.len() == 2 {
                let st = fix(a[0]);
                (if st < 0 { st + len } else { st }, fix(a[1]))
            } else {
                crate::string::range_bounds(a[0], len).unwrap()
            };
            let end = (st + n).min(len);
            v.drain(st as usize..end as usize);
        } else if let Some(i) = norm_index(fix(a[0]), v.len()) {
            v.remove(i);
        }
        r
    });
    defn(c, "fill", 0, 3, |s, a, b| {
        let v = mutv!(s);
        if is_proc(b) {
            for i in 0..v.len() {
                v[i] = tri!(call_block(b, &[mkfix(i as i64)]));
            }
            return s;
        }
        // fill(x), fill(x, start [, length]), fill(x, range); start may be negative or nil
        let Some(&x) = a.first() else { return arg_error("wrong number of arguments (given 0, expected 1..3)") };
        let len = v.len() as i64;
        let (start, end) = match a.get(1).copied() {
            Some(r) if is_range(r) => {
                if a.len() > 2 {
                    return arg_error("wrong number of arguments (given 3, expected 1..2)");
                }
                let rr = unsafe { &*(r as *const RRange) };
                let b = if rr.lo == QNIL { 0 } else if is_fix(rr.lo) { fix(rr.lo) } else { return type_error("no implicit conversion into Integer") };
                let b = if b < 0 { b + len } else { b };
                if b < 0 {
                    return raise(rt().c.range_error, &format!("{} out of range", crate::fmt::inspect_lossy(r)));
                }
                let e = if rr.hi == QNIL { len } else if is_fix(rr.hi) { fix(rr.hi) } else { return type_error("no implicit conversion into Integer") };
                let e = if e < 0 { e + len } else { e };
                let e = if rr.hi != QNIL && !rr.excl { e + 1 } else { e };
                (b, e.max(b))
            }
            st => {
                let b = match st {
                    None => 0,
                    Some(q) if q == QNIL => 0,
                    Some(q) if is_fix(q) => fix(q),
                    Some(_) => return type_error("no implicit conversion into Integer"),
                };
                let b = if b < 0 { (b + len).max(0) } else { b };
                let e = match a.get(2).copied() {
                    None => len.max(b),
                    Some(q) if q == QNIL => len.max(b),
                    Some(q) if is_fix(q) => b + fix(q).max(0),
                    Some(_) => return type_error("no implicit conversion into Integer"),
                };
                (b, e)
            }
        };
        if end <= start {
            return s;
        }
        let v = mutv!(s);
        if end as usize > v.len() {
            v.resize(end as usize, QNIL);
        }
        for y in &mut v[start as usize..end as usize] {
            *y = x;
        }
        s
    });
    defn(c, "assoc", 1, 1, |s, a, _| {
        for x in snapshot(s) {
            if is_ary(x) && !av(x).is_empty() && truthy(tri!(equal(av(x)[0], a[0]))) {
                return x;
            }
        }
        QNIL
    });
    defn(c, "sample", 0, 1, |s, _, _| {
        let v = av(s);
        if v.is_empty() {
            return QNIL;
        }
        v[(crate::kernel::rand_u64() % v.len() as u64) as usize]
    });
    fn shuffle(v: &mut [V]) {
        for i in (1..v.len()).rev() {
            let j = (crate::kernel::rand_u64() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
    defn(c, "shuffle", 0, 1, |s, _, _| {
        let mut v = snapshot(s);
        shuffle(&mut v);
        ary_new(v.into_vec())
    });
    defn(c, "shuffle!", 0, 1, |s, _, _| {
        shuffle(mutv!(s));
        s
    });
    defn(c, "bsearch", 0, 0, |s, _, b| {
        let v = snapshot(s);
        let (mut lo, mut hi) = (0, v.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = tri!(call_block(b, &[v[mid]]));
            let go_left = if r == QTRUE {
                true
            } else if r == QFALSE || r == QNIL {
                false
            } else if is_fix(r) {
                if fix(r) == 0 {
                    return v[mid];
                }
                fix(r) < 0
            } else {
                false
            };
            if go_left { hi = mid } else { lo = mid + 1 }
        }
        if lo < v.len() && truthy(tri!(call_block(b, &[v[lo]]))) { v[lo] } else { QNIL }
    });
    defn(c, "__kn_case_any", 1, 1, |s, a, _| {
        for x in snapshot(s) {
            if truthy(tri!(send1(x, "===", a[0]))) {
                return QTRUE;
            }
        }
        QFALSE
    });
    defn(c, "pack", 1, 1, |_, _, _| raise(rt().c.not_implemented_error, "Array#pack is not supported yet"));
    defn(c, "freeze", 0, 0, |s, _, _| {
        hdr(s).flags |= FL_FROZEN;
        s
    });
}
