//! Hash (insertion-ordered).

use crate::core::*;
use crate::gc::RVec;
use crate::fmt::*;
use crate::kernel::new_str_v;
use crate::tri;

/// String keys are copied and frozen on insert, like CRuby.
pub fn key_dup(k: V) -> V {
    if is_str(k) && !is_frozen(k) {
        let n = str_new(rstr(k).b.clone());
        hdr(n).flags |= FL_FROZEN;
        n
    } else {
        k
    }
}
/// The pairs, rooted while iterated (see `gc::Rooted`).
pub fn pairs(h: V) -> crate::gc::Rooted<(V, V)> {
    let items: Vec<(V, V)> = rhash(h).m.iter().map(|(k, v)| (k.0, *v)).collect();
    let flat: Vec<V> = items.iter().flat_map(|(k, v)| [*k, *v]).collect();
    crate::gc::Rooted::new(items, flat)
}
fn pair_ary(h: V) -> Vec<V> {
    pairs(h).into_iter().map(|(k, v)| ary_new(vec![k, v])).collect()
}
pub fn hget(h: V, k: V) -> V {
    let hh = rhash(h);
    if let Some(v) = hh.m.get(&HKey(k)) {
        return *v;
    }
    if hh.default_proc != QNIL {
        return call_block(hh.default_proc, &[h, k]);
    }
    hh.default
}
pub fn hset(h: V, k: V, v: V) -> V {
    if is_frozen(h) {
        return check_frozen(h);
    }
    rhash(h).m.insert(HKey(key_dup(k)), v);
    v
}
fn mutate(h: V) -> Result<(), V> {
    if is_frozen(h) { Err(check_frozen(h)) } else { Ok(()) }
}
macro_rules! mutchk {
    ($h:expr) => {
        if let Err(e) = mutate($h) {
            return e;
        }
    };
}
fn new_like(_h: V) -> V {
    hash_new()
}
/// A Hash argument, converted with `to_hash` like CRuby's implicit conversion.
fn want_hash(v: V) -> Result<V, V> {
    if is_hash(v) {
        return Ok(v);
    }
    if is_heap(v) && respond_to(v, sym("to_hash")) {
        let h = send0(v, "to_hash");
        if h == UNDEF {
            return Err(UNDEF);
        }
        if is_hash(h) {
            return Ok(h);
        }
    }
    Err(type_error(&format!("no implicit conversion of {} into Hash", class_name(real_class(v)))))
}

fn merge_into(dst: V, src: V, b: V) -> V {
    for (k, v) in pairs(src) {
        let nv = if is_proc(b) {
            match rhash(dst).m.get(&HKey(k)) {
                Some(&old) => tri!(call_block(b, &[k, old, v])),
                None => v,
            }
        } else {
            v
        };
        rhash(dst).m.insert(HKey(key_dup(k)), nv);
    }
    dst
}

fn filter(h: V, b: V, keep: bool) -> V {
    let out = new_like(h);
    for (k, v) in pairs(h) {
        if truthy(tri!(call_block(b, &[k, v]))) == keep {
            rhash(out).m.insert(HKey(k), v);
        }
    }
    out
}
fn filter_bang(h: V, b: V, keep: bool) -> (V, bool) {
    if let Err(e) = mutate(h) {
        return (e, false);
    }
    let before = rhash(h).m.len();
    let r = filter(h, b, keep);
    if r == UNDEF {
        return (UNDEF, false);
    }
    rhash(h).m.assign(std::mem::take(&mut rhash(r).m));
    (h, rhash(h).m.len() != before)
}

pub fn init() {
    let c = rt().c.hash;
    defn(c, "initialize", 0, 2, |s, a, b| {
        let h = rhash(s);
        crate::gc::wb_obj(s);
        h.default = a.first().copied().filter(|x| !is_hash(*x) || a.len() == 1).unwrap_or(QNIL);
        if is_proc(b) {
            h.default_proc = b;
        }
        QNIL
    });
    defs(c, "[]", 0, -1, |s, a, _| {
        let h = hash_new();
        // `Sub[...]` builds an instance of the receiver (a Hash subclass)
        if is_class(s) && s != rt().c.hash as V {
            hdr(h).klass = s as *mut Class;
        }
        if a.len() == 1 && is_hash(a[0]) {
            return merge_into(h, a[0], QNIL);
        }
        if a.len() == 1 && is_ary(a[0]) {
            for p in crate::gc::Rooted::of(rary(a[0]).v.clone()) {
                if is_ary(p) && rary(p).v.len() == 2 {
                    hset(h, rary(p).v[0], rary(p).v[1]);
                }
            }
            return h;
        }
        if a.len() % 2 != 0 {
            return arg_error("odd number of arguments for Hash");
        }
        for ch in a.chunks(2) {
            hset(h, ch[0], ch[1]);
        }
        h
    });
    defn(c, "[]", 1, 1, |s, a, _| hget(s, a[0]));
    defn(c, "[]=", 2, 2, |s, a, _| hset(s, a[0], a[1]));
    defn(c, "store", 2, 2, |s, a, _| hset(s, a[0], a[1]));
    defn(c, "fetch", 1, 2, |s, a, b| match rhash(s).m.get(&HKey(a[0])) {
        Some(v) => *v,
        None if is_proc(b) => call_block(b, &[a[0]]),
        None if a.len() == 2 => a[1],
        None => {
            let e = exc_new(rt().c.key_error, &format!("key not found: {}", inspect_lossy(a[0])));
            ivar_set(e, sym("@key"), a[0]);
            ivar_set(e, sym("@receiver"), s);
            raise_exc(e)
        }
    });
    defn(c, "dig", 1, -1, |s, a, _| {
        let x = tri!(hget(s, a[0]));
        if a.len() == 1 || x == QNIL { x } else { send(x, sym("dig"), &a[1..], QNIL) }
    });
    for name in ["key?", "has_key?", "include?", "member?"] {
        defn(c, name, 1, 1, |s, a, _| bool_v(rhash(s).m.contains_key(&HKey(a[0]))));
    }
    defn(c, "value?", 1, 1, |s, a, _| {
        for (_, v) in pairs(s) {
            if truthy(tri!(equal(v, a[0]))) {
                return QTRUE;
            }
        }
        QFALSE
    });
    defn(c, "has_value?", 1, 1, |s, a, _| send(s, sym("value?"), a, QNIL));
    defn(c, "key", 1, 1, |s, a, _| {
        for (k, v) in pairs(s) {
            if truthy(tri!(equal(v, a[0]))) {
                return k;
            }
        }
        QNIL
    });
    defn(c, "delete", 1, 1, |s, a, b| {
        mutchk!(s);
        match rhash(s).m.shift_remove(&HKey(a[0])) {
            Some(v) => v,
            None if is_proc(b) => call_block(b, &[a[0]]),
            None => QNIL,
        }
    });
    defn(c, "keys", 0, 0, |s, _, _| ary_new(rhash(s).m.keys().map(|k| k.0).collect()));
    defn(c, "values", 0, 0, |s, _, _| ary_new(rhash(s).m.values().copied().collect()));
    defn(c, "values_at", 0, -1, |s, a, _| {
        let mut out = RVec::new();
        for &k in a {
            out.push(tri!(hget(s, k)));
        }
        ary_new(out.take())
    });
    defn(c, "fetch_values", 0, -1, |s, a, b| {
        let mut out = RVec::new();
        for &k in a {
            out.push(tri!(send(s, sym("fetch"), &[k], b)));
        }
        ary_new(out.take())
    });
    defn(c, "size", 0, 0, |s, _, _| mkfix(rhash(s).m.len() as i64));
    defn(c, "length", 0, 0, |s, _, _| mkfix(rhash(s).m.len() as i64));
    defn(c, "empty?", 0, 0, |s, _, _| bool_v(rhash(s).m.is_empty()));
    fn each(s: V, b: V) -> V {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each", &[]);
        }
        for (k, v) in pairs(s) {
            tri!(call_block(b, &[ary_new(vec![k, v])]));
        }
        s
    }
    defn(c, "each", 0, 0, |s, _, b| each(s, b));
    defn(c, "each_pair", 0, 0, |s, _, b| each(s, b));
    defn(c, "each_key", 0, 0, |s, _, b| {
        for (k, _) in pairs(s) {
            tri!(call_block(b, &[k]));
        }
        s
    });
    defn(c, "each_value", 0, 0, |s, _, b| {
        for (_, v) in pairs(s) {
            tri!(call_block(b, &[v]));
        }
        s
    });
    defn(c, "select", 0, 0, |s, _, b| filter(s, b, true));
    defn(c, "filter", 0, 0, |s, _, b| filter(s, b, true));
    defn(c, "reject", 0, 0, |s, _, b| filter(s, b, false));
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
    defn(c, "to_a", 0, 0, |s, _, _| ary_new(pair_ary(s)));
    defn(c, "entries", 0, 0, |s, _, _| ary_new(pair_ary(s)));
    defn(c, "to_h", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return s;
        }
        let out = hash_new();
        for (k, v) in pairs(s) {
            let p = tri!(call_block(b, &[k, v]));
            if !is_ary(p) || rary(p).v.len() != 2 {
                return type_error("wrong element type (expected array)");
            }
            hset(out, rary(p).v[0], rary(p).v[1]);
        }
        out
    });
    defn(c, "to_hash", 0, 0, |s, _, _| s);
    defn(c, "deconstruct_keys", 1, 1, |s, _, _| s);
    defn(c, "inspect", 0, 0, |s, _, _| match hash_inspect(s) {
        Ok(x) => new_str_v(x),
        Err(()) => UNDEF,
    });
    defn(c, "to_s", 0, 0, |s, _, _| match hash_inspect(s) {
        Ok(x) => new_str_v(x),
        Err(()) => UNDEF,
    });
    defn(c, "merge", 0, -1, |s, a, b| {
        let out = hash_new();
        merge_into(out, s, QNIL);
        crate::gc::wb_obj(out);
        rhash(out).default = rhash(s).default;
        rhash(out).default_proc = rhash(s).default_proc;
        for &o in a {
            let o = match want_hash(o) {
                Ok(o) => o,
                Err(e) => return e,
            };
            tri!(merge_into(out, o, b));
        }
        out
    });
    fn update(s: V, a: &[V], b: V) -> V {
        mutchk!(s);
        for &o in a {
            let o = match want_hash(o) {
                Ok(o) => o,
                Err(e) => return e,
            };
            tri!(merge_into(s, o, b));
        }
        s
    }
    defn(c, "merge!", 0, -1, update);
    defn(c, "update", 0, -1, update);
    defn(c, "transform_values", 0, 0, |s, _, b| {
        let out = hash_new();
        for (k, v) in pairs(s) {
            rhash(out).m.insert(HKey(k), tri!(call_block(b, &[v])));
        }
        out
    });
    defn(c, "transform_values!", 0, 0, |s, _, b| {
        mutchk!(s);
        for (k, v) in pairs(s) {
            let nv = tri!(call_block(b, &[v]));
            rhash(s).m.insert(HKey(k), nv);
        }
        s
    });
    fn transform_keys(s: V, a: &[V], b: V) -> V {
        let out = hash_new();
        for (k, v) in pairs(s) {
            let nk = if let Some(&m) = a.first().filter(|m| is_hash(**m)) {
                match rhash(m).m.get(&HKey(k)) {
                    Some(x) => *x,
                    None if is_proc(b) => tri!(call_block(b, &[k])),
                    None => k,
                }
            } else {
                tri!(call_block(b, &[k]))
            };
            hset(out, nk, v);
        }
        out
    }
    defn(c, "transform_keys", 0, 1, transform_keys);
    defn(c, "transform_keys!", 0, 1, |s, a, b| {
        mutchk!(s);
        let r = tri!(transform_keys(s, a, b));
        rhash(s).m.assign(std::mem::take(&mut rhash(r).m));
        s
    });
    defn(c, "invert", 0, 0, |s, _, _| {
        let out = hash_new();
        for (k, v) in pairs(s) {
            hset(out, v, k);
        }
        out
    });
    defn(c, "slice", 0, -1, |s, a, _| {
        let out = hash_new();
        for &k in a {
            if let Some(v) = rhash(s).m.get(&HKey(k)) {
                rhash(out).m.insert(HKey(k), *v);
            }
        }
        out
    });
    defn(c, "except", 0, -1, |s, a, _| {
        let out = hash_new();
        for (k, v) in pairs(s) {
            if !a.iter().any(|x| eql(*x, k)) {
                rhash(out).m.insert(HKey(k), v);
            }
        }
        out
    });
    defn(c, "filter_map", 0, 0, |s, _, b| {
        let mut out = RVec::new();
        for (k, v) in pairs(s) {
            let r = tri!(call_block(b, &[k, v]));
            if truthy(r) {
                out.push(r);
            }
        }
        ary_new(out.take())
    });
    defn(c, "compact", 0, 0, |s, _, _| {
        let out = hash_new();
        for (k, v) in pairs(s) {
            if v != QNIL {
                rhash(out).m.insert(HKey(k), v);
            }
        }
        out
    });
    defn(c, "any?", 0, 1, |s, a, b| {
        if !is_proc(b) && a.is_empty() {
            return bool_v(!rhash(s).m.is_empty());
        }
        for (k, v) in pairs(s) {
            let p = ary_new(vec![k, v]);
            let t = if let Some(&pat) = a.first() { tri!(send1(pat, "===", p)) } else { tri!(call_block(b, &[p])) };
            if truthy(t) {
                return QTRUE;
            }
        }
        QFALSE
    });
    defn(c, "count", 0, 1, |s, a, b| {
        if !is_proc(b) && a.is_empty() {
            return mkfix(rhash(s).m.len() as i64);
        }
        let arr = ary_new(pair_ary(s));
        send(arr, sym("count"), a, b)
    });
    defn(c, "find", 0, 1, |s, _, b| {
        for (k, v) in pairs(s) {
            let p = ary_new(vec![k, v]);
            if truthy(tri!(call_block(b, &[p]))) {
                return p;
            }
        }
        QNIL
    });
    defn(c, "detect", 0, 1, |s, a, b| send(s, sym("find"), a, b));
    defn(c, "sum", 0, 1, |s, a, b| send(ary_new(pair_ary(s)), sym("sum"), a, b));
    defn(c, "default", 0, 1, |s, _, _| rhash(s).default);
    defn(c, "default=", 1, 1, |s, a, _| {
        crate::gc::wb_obj(s);
        rhash(s).default = a[0];
        a[0]
    });
    defn(c, "default_proc", 0, 0, |s, _, _| rhash(s).default_proc);
    defn(c, "clear", 0, 0, |s, _, _| {
        mutchk!(s);
        rhash(s).m.clear();
        s
    });
    defn(c, "replace", 1, 1, |s, a, _| {
        mutchk!(s);
        rhash(s).m.clear();
        merge_into(s, a[0], QNIL)
    });
    defn(c, "shift", 0, 0, |s, _, _| {
        mutchk!(s);
        match rhash(s).m.shift_remove_index(0) {
            Some((k, v)) => ary_new(vec![k.0, v]),
            None => QNIL,
        }
    });
    defn(c, "first", 0, 1, |s, a, _| {
        let ps = pair_ary(s);
        match a.first() {
            Some(n) => ary_new(ps.into_iter().take(fix(*n).max(0) as usize).collect()),
            None => ps.first().copied().unwrap_or(QNIL),
        }
    });
    defn(c, "assoc", 1, 1, |s, a, _| match rhash(s).m.get_key_value(&HKey(a[0])) {
        Some((k, v)) => ary_new(vec![k.0, *v]),
        None => QNIL,
    });
    defn(c, "==", 1, 1, |s, a, _| {
        if !is_hash(a[0]) || rhash(s).m.len() != rhash(a[0]).m.len() {
            return QFALSE;
        }
        for (k, v) in pairs(s) {
            match rhash(a[0]).m.get(&HKey(k)) {
                Some(&o) => {
                    if !truthy(tri!(equal(v, o))) {
                        return QFALSE;
                    }
                }
                None => return QFALSE,
            }
        }
        QTRUE
    });
    defn(c, "eql?", 1, 1, |s, a, _| send(s, sym("=="), a, QNIL));
    defn(c, "hash", 0, 0, |s, _, _| {
        let mut h: u64 = rhash(s).m.len() as u64;
        for (k, v) in pairs(s) {
            h = h.wrapping_add(hash_of(k) ^ hash_of(v).rotate_left(7));
        }
        mkfix((h >> 2) as i64)
    });
    defn(c, "compare_by_identity", 0, 0, |s, _, _| s);
    defn(c, "freeze", 0, 0, |s, _, _| {
        hdr(s).flags |= FL_FROZEN;
        s
    });
}
