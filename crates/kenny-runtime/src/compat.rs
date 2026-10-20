//! Small compatibility shims found while compiling real gems (Rack, stdlib).

use crate::core::*;

pub const FL_BINARY: u32 = 16;

fn enc_obj(name: &str) -> V {
    let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class;
    const_get_under(enc, sym(name)).unwrap_or(QNIL)
}

fn init_encoding() {
    let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class;
    let mk = |n: &str| {
        let o = object_new(enc);
        ivar_set(o, sym("@__name"), str_from(n));
        o
    };
    let ascii = mk("US-ASCII");
    const_set(enc, sym("US_ASCII"), ascii);
    const_set(enc, sym("ASCII"), ascii);
    const_set(enc, sym("UTF8"), enc_obj("UTF_8"));
    // Encodings no Kenny string can carry, defined so that code comparing against them
    // (`file.encoding == Encoding::UTF_16LE`, as the sqlite3 gem does) runs.
    for (c, n) in [("UTF_16LE", "UTF-16LE"), ("UTF_16BE", "UTF-16BE"), ("UTF_16", "UTF-16"), ("UTF_32LE", "UTF-32LE"), ("UTF_32BE", "UTF-32BE"), ("UTF_32", "UTF-32")] {
        if const_get_under(enc, sym(c)).is_none() {
            const_set(enc, sym(c), mk(n));
        }
    }
    for e in ["CompatibilityError", "UndefinedConversionError", "InvalidByteSequenceError", "ConverterNotFoundError"] {
        let k = new_class("", const_lookup(rt().c.object, sym("EncodingError")).unwrap() as *mut Class);
        const_set(enc, sym(e), k as V);
    }
    defs(enc, "default_external", 0, 0, |_, _, _| enc_obj("UTF_8"));
    defs(enc, "default_internal", 0, 0, |_, _, _| QNIL);
    defs(enc, "default_external=", 1, 1, |_, a, _| a[0]);
    defs(enc, "default_internal=", 1, 1, |_, a, _| a[0]);
    defs(enc, "find", 1, 1, |_, a, _| {
        let n = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).to_ascii_uppercase();
        match n.as_str() {
            "UTF-8" | "UTF8" | "LOCALE" | "EXTERNAL" | "FILESYSTEM" => enc_obj("UTF_8"),
            "ASCII-8BIT" | "BINARY" => enc_obj("BINARY"),
            "US-ASCII" | "ASCII" => enc_obj("US_ASCII"),
            _ => arg_error(&format!("unknown encoding name - {n}")),
        }
    });
    defs(enc, "list", 0, 0, |_, _, _| ary_new(vec![enc_obj("UTF_8"), enc_obj("BINARY"), enc_obj("US_ASCII")]));
    defn(enc, "ascii_compatible?", 0, 0, |_, _, _| QTRUE);
    defn(enc, "dummy?", 0, 0, |_, _, _| QFALSE);
    defn(enc, "names", 0, 0, |s, _, _| ary_new(vec![ivar_get(s, sym("@__name"))]));
    defn(enc, "==", 1, 1, |s, a, _| bool_v(s == a[0]));
    let st = rt().c.string;
    defn(st, "encoding", 0, 0, |s, _, _| {
        let f = hdr(s).flags;
        if f & FL_BINARY != 0 {
            enc_obj("BINARY")
        } else if f & (FL_BINARY << 1) != 0 {
            enc_obj("US_ASCII")
        } else {
            enc_obj("UTF_8")
        }
    });
    fn set_enc(s: V, e: V) {
        let name = if is_str(e) {
            String::from_utf8_lossy(&rstr(e).b).to_ascii_uppercase()
        } else {
            let n = ivar_get(e, sym("@__name"));
            if is_str(n) { String::from_utf8_lossy(&rstr(n).b).into_owned() } else { "UTF-8".into() }
        };
        let h = hdr(s);
        h.flags &= !(FL_BINARY | (FL_BINARY << 1));
        match name.as_str() {
            "ASCII-8BIT" | "BINARY" => h.flags |= FL_BINARY,
            "US-ASCII" | "ASCII" => h.flags |= FL_BINARY << 1,
            _ => {}
        }
    }
    defn(st, "force_encoding", 1, 1, |s, a, _| {
        set_enc(s, a[0]);
        s
    });
    defn(st, "b", 0, 0, |s, _, _| {
        let n = str_new(rstr(s).b.clone());
        hdr(n).flags |= FL_BINARY;
        n
    });
    defn(st, "encode", 0, -1, |s, a, _| {
        let n = str_new(rstr(s).b.clone());
        if let Some(&e) = a.first().filter(|x| !is_hash(**x)) {
            set_enc(n, e);
        }
        n
    });
    defn(st, "encode!", 0, -1, |s, a, _| {
        if let Some(&e) = a.first().filter(|x| !is_hash(**x)) {
            set_enc(s, e);
        }
        s
    });
}

fn urandom(n: usize) -> Vec<u8> {
    crate::openssl::urandom(n)
}
fn rng_next(s: V) -> u64 {
    let st = ivar_get(s, sym("@__state"));
    let mut x = if is_fix(st) { fix(st) as u64 } else { crate::kernel::rand_u64() } | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    ivar_set(s, sym("@__state"), mkfix((x >> 2) as i64));
    x.wrapping_mul(0x2545F4914F6CDD1D)
}
fn init_random() {
    let r = new_class("Random", rt().c.object);
    set_global_const("Random", r as V);
    defn(r, "initialize", 0, 1, |s, a, _| {
        let seed = a.first().copied().filter(|x| is_fix(*x)).map(fix).unwrap_or_else(|| crate::kernel::rand_u64() as i64 >> 2);
        ivar_set(s, sym("@__seed"), mkfix(seed));
        ivar_set(s, sym("@__state"), mkfix(seed ^ 0x5DEECE66D));
        QNIL
    });
    defn(r, "seed", 0, 0, |s, _, _| ivar_get(s, sym("@__seed")));
    fn rand_with(a: &[V], next: &mut dyn FnMut() -> u64) -> V {
        let f = || 0.0f64;
        let _ = f;
        let unit = |x: u64| (x >> 11) as f64 / (1u64 << 53) as f64;
        match a.first().copied() {
            None | Some(QNIL) => float_new(unit(next())),
            Some(n) if is_fix(n) && fix(n) > 0 => mkfix((next() % fix(n) as u64) as i64),
            Some(n) if is_float(n) => float_new(unit(next()) * float_val(n)),
            Some(rg) if is_range(rg) => {
                let rr = rrange(rg);
                if is_fix(rr.lo) && is_fix(rr.hi) {
                    let hi = fix(rr.hi) - if rr.excl { 1 } else { 0 };
                    let span = (hi - fix(rr.lo) + 1).max(1) as u64;
                    mkfix(fix(rr.lo) + (next() % span) as i64)
                } else {
                    let lo = crate::fmt::num_to_f(rr.lo).unwrap_or(0.0);
                    let hi = crate::fmt::num_to_f(rr.hi).unwrap_or(1.0);
                    float_new(lo + unit(next()) * (hi - lo))
                }
            }
            Some(x) => arg_error(&format!("invalid argument - {}", crate::fmt::inspect_lossy(x))),
        }
    }
    defn(r, "rand", 0, 1, |s, a, _| rand_with(a, &mut || rng_next(s)));
    defn(r, "bytes", 1, 1, |s, a, _| {
        let n = fix(a[0]).max(0) as usize;
        let v: Vec<u8> = (0..n).map(|_| (rng_next(s) >> 32) as u8).collect();
        let v = str_new(v);
        hdr(v).flags |= FL_BINARY;
        v
    });
    defs(r, "rand", 0, 1, |_, a, _| rand_with(a, &mut crate::kernel::rand_u64));
    // random bytes are binary strings (SecureRandom checks `.length` against the request)
    fn urandom_v(_: V, a: &[V], _: V) -> V {
        let s = str_new(urandom(if is_fix(a[0]) { fix(a[0]).max(0) as usize } else { 0 }));
        hdr(s).flags |= FL_BINARY;
        s
    }
    defs(r, "bytes", 1, 1, urandom_v);
    defs(r, "urandom", 1, 1, urandom_v);
    defs(r, "raw_seed", 1, 1, urandom_v);
    defs(r, "new_seed", 0, 0, |_, _, _| {
        let b = urandom(8);
        mkfix((u64::from_le_bytes(b.try_into().unwrap()) >> 2) as i64)
    });
    defs(r, "srand", 0, 1, |_, a, _| send(rt().main_obj, sym("srand"), a, QNIL));
    let d = object_new(r);
    ivar_set(d, sym("@__seed"), mkfix(0));
    ivar_set(d, sym("@__state"), mkfix(0x1234567));
    const_set(r, sym("DEFAULT"), d);
}

fn init_locations() {
    let th = const_lookup(rt().c.object, sym("Thread")).unwrap() as *mut Class;
    let bt = new_module("Thread::Backtrace");
    const_set(th, sym("Backtrace"), bt as V);
    let loc = new_class("Thread::Backtrace::Location", rt().c.object);
    const_set(bt, sym("Location"), loc as V);
    fn parts(s: V) -> (String, i64) {
        let t = String::from_utf8_lossy(&rstr(ivar_get(s, sym("@__loc"))).b).into_owned();
        match t.rsplit_once(':') {
            Some((p, l)) => (p.to_string(), l.parse().unwrap_or(0)),
            None => (t, 0),
        }
    }
    fn label(s: V) -> V {
        let l = ivar_get(s, sym("@__label"));
        if l == QNIL { str_from("<main>") } else { l }
    }
    fn to_s(s: V) -> V {
        let l = label(s);
        let mut b = rstr(ivar_get(s, sym("@__loc"))).b.clone();
        b.extend_from_slice(b":in '");
        b.extend_from_slice(&rstr(l).b);
        b.push(b'\'');
        str_new(b)
    }
    defn(loc, "path", 0, 0, |s, _, _| str_from(&parts(s).0));
    defn(loc, "absolute_path", 0, 0, |s, _, _| str_from(&parts(s).0));
    defn(loc, "lineno", 0, 0, |s, _, _| mkfix(parts(s).1));
    defn(loc, "label", 0, 0, |s, _, _| label(s));
    defn(loc, "base_label", 0, 0, |s, _, _| {
        let l = String::from_utf8_lossy(&rstr(label(s)).b).into_owned();
        str_from(l.rsplit("block in ").next().unwrap_or(&l))
    });
    defn(loc, "to_s", 0, 0, |s, _, _| to_s(s));
    defn(loc, "inspect", 0, 0, |s, _, _| crate::kernel::new_str_v(crate::fmt::str_inspect(&rstr(to_s(s)).b)));
    // caller(start = 1, length = nil) / caller(range): frames above the method calling it
    fn frames(a: &[V]) -> Result<Vec<usize>, V> {
        let mut addrs = crate::backtrace::capture();
        let (start, len) = match a.first() {
            None => (1, None),
            Some(&r) if is_range(r) => {
                let (lo, hi, excl) = (rrange(r).lo, rrange(r).hi, rrange(r).excl);
                let lo = if lo == QNIL { 0 } else { fix(lo) };
                let len = if hi == QNIL { None } else { Some((fix(hi) - lo + if excl { 0 } else { 1 }).max(0)) };
                (lo, len)
            }
            Some(&st) => {
                if !is_fix(st) {
                    return Err(type_error("no implicit conversion into Integer"));
                }
                (fix(st), a.get(1).filter(|v| **v != QNIL).map(|v| fix(*v)))
            }
        };
        if start < 0 {
            return Err(arg_error(&format!("negative level ({start})")));
        }
        let start = (start as usize).min(addrs.len());
        addrs.drain(..start);
        if let Some(n) = len {
            addrs.truncate(n.max(0) as usize);
        }
        Ok(addrs)
    }
    defn(rt().c.kernel, "caller_locations", 0, 2, |_, a, _| match frames(a) {
        Ok(f) => crate::backtrace::locations(&f),
        Err(e) => e,
    });
    defn(rt().c.kernel, "caller", 0, 2, |_, a, _| match frames(a) {
        Ok(f) => ary_new(crate::backtrace::lines(&f).iter().map(|l| str_from(l)).collect()),
        Err(e) => e,
    });
}

fn init_monitor() {
    let mon = const_lookup(rt().c.object, sym("Monitor")).unwrap() as *mut Class;
    fn me() -> V {
        send0(const_lookup(rt().c.object, sym("Thread")).unwrap(), "current")
    }
    fn try_enter(s: V) -> V {
        if ivar_get(s, sym("@__owner")) == me() {
            crate::tri!(send0(s, "enter"));
            return QTRUE;
        }
        let m = ivar_get(s, sym("@__mutex"));
        if m != QNIL && truthy(send0(m, "locked?")) {
            return QFALSE;
        }
        crate::tri!(send0(s, "enter"));
        QTRUE
    }
    defn(mon, "try_enter", 0, 0, |s, _, _| try_enter(s));
    defn(mon, "try_mon_enter", 0, 0, |s, _, _| try_enter(s));
    defn(mon, "mon_try_enter", 0, 0, |s, _, _| try_enter(s));
    defn(mon, "mon_locked?", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__owner")))));
    defn(mon, "mon_check_owner", 0, 0, |s, _, _| {
        if ivar_get(s, sym("@__owner")) != me() {
            let te = const_lookup(rt().c.object, sym("ThreadError")).unwrap() as *mut Class;
            return raise(te, "current thread not owner");
        }
        QNIL
    });
    // used by MonitorMixin::ConditionVariable#wait
    defn(mon, "wait_for_cond", 2, 2, |s, a, _| {
        let count = ivar_get(s, sym("@__count"));
        let m = ivar_get(s, sym("@__mutex"));
        ivar_set(s, sym("@__owner"), QNIL);
        let r = send(a[0], sym("wait"), &[m, a[1]], QNIL);
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        ivar_set(s, sym("@__owner"), me());
        ivar_set(s, sym("@__count"), count);
        pst().pending = saved;
        if r == UNDEF { UNDEF } else { QTRUE }
    });
    defn(mon, "new_cond", 0, 0, |s, _, _| {
        let mm = const_lookup(rt().c.object, sym("MonitorMixin"));
        match mm.and_then(|m| const_get_under(m as *mut Class, sym("ConditionVariable"))) {
            Some(cv) => send1(cv, "new", s),
            None => send0(const_lookup(rt().c.object, sym("ConditionVariable")).unwrap(), "new"),
        }
    });
}

static BINDING: G<*mut Class> = G::new(std::ptr::null_mut());

/// A Binding in Kenny captures `self` only (locals live in registers; see docs/LANGUAGE.md).
#[unsafe(no_mangle)]
pub extern "C" fn kn_binding_new(self_: V) -> V {
    let b = object_new(*BINDING.get());
    ivar_set(b, sym("@__self"), self_);
    b
}
pub fn binding_receiver(b: V) -> Option<V> {
    if is_kind(b, Kind::Object) && real_class(b) == *BINDING.get() { Some(ivar_get(b, sym("@__self"))) } else { None }
}

fn init_binding() {
    let b = new_class("Binding", rt().c.object);
    set_global_const("Binding", b as V);
    *BINDING.get() = b;
    set_global_const("TOPLEVEL_BINDING", kn_binding_new(rt().main_obj));
    defn(b, "receiver", 0, 0, |s, _, _| ivar_get(s, sym("@__self")));
    defn(b, "eval", 1, 3, |s, a, _| crate::kernel::eval_string(a[0], 0, ivar_get(s, sym("@__self"))));
    defn(b, "local_variables", 0, 0, |_, _, _| ary_new(vec![]));
    defn(b, "local_variable_defined?", 1, 1, |_, _, _| QFALSE);
    for name in ["local_variable_get", "local_variable_set"] {
        defn(b, name, 1, 2, |_, _, _| {
            raise(rt().c.not_implemented_error, "Kenny bindings capture self only, not local variables (see docs/LANGUAGE.md)")
        });
    }
    defn(b, "irb", 0, 0, |_, _, _| raise(rt().c.not_implemented_error, "binding.irb is not available in compiled programs"));
    defn(rt().c.proc_, "binding", 0, 0, |s, _, _| kn_binding_new(rproc(s).self_));
    defn(rt().c.kernel, "binding", 0, 0, |s, _, _| kn_binding_new(s));
}

fn init_objectspace() {
    // Only the cheap parts: Kenny tracks classes anyway, and WeakMap is a plain map here.
    let os = new_module("ObjectSpace");
    set_global_const("ObjectSpace", os as V);
    defs(os, "each_object", 0, 1, |_, a, b| {
        let want = a.first().copied();
        let ok = want.is_some_and(|w| w == rt().c.class as V || w == rt().c.module as V)
            || want.is_some_and(|w| is_class(w) && rclass(w).is_singleton);
        if !ok {
            return raise(rt().c.not_implemented_error, "ObjectSpace.each_object only supports Class/Module (see docs/LANGUAGE.md)");
        }
        let modules_too = want == Some(rt().c.module as V);
        let classes: Vec<V> = crate::gc::all_classes()
            .iter()
            .filter(|c| unsafe { !(***c).is_singleton && (modules_too || !(***c).is_module) && !(***c).name.is_empty() })
            .map(|c| *c as V)
            .collect();
        let classes = crate::gc::RVec::from(classes);
        if !is_proc(b) {
            return crate::misc::enum_for(ary_new(classes.to_vec()), "each", &[]);
        }
        for i in 0..classes.len() {
            crate::tri!(call_block(b, &[classes[i]]));
        }
        mkfix(classes.len() as i64)
    });
    defs(os, "garbage_collect", 0, 1, |_, _, _| {
        crate::gc::collect_now();
        QNIL
    });
    defs(os, "count_objects", 0, 1, |_, _, _| {
        let h = hash_new();
        crate::hash::hset(h, mksym(sym("TOTAL")), mkfix(crate::gc::stats().1 as i64));
        h
    });
    defs(os, "define_finalizer", 1, 2, |_, a, _| ary_new(vec![mkfix(0), a.get(1).copied().unwrap_or(QNIL)]));
    defs(os, "undefine_finalizer", 1, 1, |_, a, _| a[0]);
    for name in ["WeakMap", "WeakKeyMap"] {
        let wm = new_class("", rt().c.object);
        const_set(os, sym(name), wm as V);
        defn(wm, "initialize", 0, 0, |s, _, _| {
            ivar_set(s, sym("@__h"), hash_new());
            QNIL
        });
        defn(wm, "[]", 1, 1, |s, a, _| crate::hash::hget(ivar_get(s, sym("@__h")), a[0]));
        defn(wm, "[]=", 2, 2, |s, a, _| crate::hash::hset(ivar_get(s, sym("@__h")), a[0], a[1]));
        defn(wm, "key?", 1, 1, |s, a, _| bool_v(rhash(ivar_get(s, sym("@__h"))).m.contains_key(&HKey(a[0]))));
        defn(wm, "include?", 1, 1, |s, a, _| bool_v(rhash(ivar_get(s, sym("@__h"))).m.contains_key(&HKey(a[0]))));
        defn(wm, "delete", 1, 1, |s, a, _| rhash(ivar_get(s, sym("@__h"))).m.shift_remove(&HKey(a[0])).unwrap_or(QNIL));
        defn(wm, "getkey", 1, 1, |s, a, _| rhash(ivar_get(s, sym("@__h"))).m.get_key_value(&HKey(a[0])).map(|(k, _)| k.0).unwrap_or(QNIL));
        defn(wm, "size", 0, 0, |s, _, _| mkfix(rhash(ivar_get(s, sym("@__h"))).m.len() as i64));
        defn(wm, "keys", 0, 0, |s, _, _| send0(ivar_get(s, sym("@__h")), "keys"));
        defn(wm, "values", 0, 0, |s, _, _| send0(ivar_get(s, sym("@__h")), "values"));
        defn(wm, "each", 0, 0, |s, _, b| send(ivar_get(s, sym("@__h")), sym("each"), &[], b));
    }
    // Class#subclasses (Ruby 3.1) / #attached_object (3.2)
    defn(rt().c.class, "subclasses", 0, 0, |s, _, _| {
        let v: Vec<V> = crate::gc::all_classes()
            .iter()
            .filter(|c| unsafe {
                let mut sup = (***c).superclass;
                while !sup.is_null() && ((*sup).is_singleton || (*sup).is_module) {
                    sup = (*sup).superclass;
                }
                !(***c).is_singleton && sup == s as *mut Class
            })
            .map(|c| *c as V)
            .collect();
        ary_new(v)
    });
    defn(rt().c.class, "attached_object", 0, 0, |s, _, _| {
        let k = rclass(s);
        if k.is_singleton { k.attached } else { type_error(&format!("'{}' is not a singleton class", class_name(s as *mut Class))) }
    });
}

pub fn init() {
    defn(rt().c.kernel, "load", 1, 2, |_, a, _| kernel_load(a));
    if let Some(le) = const_lookup(rt().c.object, sym("LoadError")) {
        defn(le as *mut Class, "path", 0, 0, |s, _, _| ivar_get(s, sym("@path")));
    }
    // $LOAD_PATH / $: and $LOADED_FEATURES / $" (files are resolved at build time; these
    // exist so code that inspects or extends them keeps working)
    let lp = ary_new(vec![]);
    let lf = ary_new(vec![]);
    for (n, v) in [("$LOAD_PATH", lp), ("$:", lp), ("$-I", lp), ("$LOADED_FEATURES", lf), ("$\"", lf)] {
        rt().gvars.insert(sym(n), v);
    }
    let c = rt().c;
    init_objectspace();
    init_binding();
    init_monitor();
    init_encoding();
    init_random();
    init_locations();
    defn(c.module, "ruby2_keywords", 0, -1, |_, _, _| QNIL);
    defn(c.module, "deprecate_constant", 0, -1, |s, _, _| s);
    defn(c.module, "protected_method_defined?", 1, 2, |_, _, _| QFALSE);
    defn(c.module, "public_instance_method", 1, 1, |s, a, _| send(s, sym("instance_method"), a, QNIL));
    defn(c.module, "singleton_class?", 0, 0, |s, _, _| bool_v(rclass(s).is_singleton));
    defn(c.module, "remove_const", 1, 1, |s, a, _| match crate::kernel::sym_arg(a[0]) {
        Ok(id) => {
            bump_const_serial();
            rclass(s).consts.remove(&id).unwrap_or(QNIL)
        }
        Err(e) => e,
    });
    defn(c.module, "class_variables", 0, 1, |s, _, _| ary_new(rclass(s).cvars.keys().map(|k| mksym(*k)).collect()));
    // most gems branch on RUBY_ENGINE == "ruby"; Kenny is detectable via KENNY_VERSION
    set_global_const("RUBY_ENGINE", str_from("ruby"));
    set_global_const("RUBY_ENGINE_VERSION", str_from("3.4.0"));
    set_global_const("KENNY_VERSION", str_from(env!("CARGO_PKG_VERSION")));
    defn(c.kernel, "ruby2_keywords", 0, -1, |_, _, _| QNIL);
    defn(c.proc_, "ruby2_keywords", 0, 0, |s, _, _| s);
    // bang versions built on the non-bang ones (nil when unchanged, like Ruby)
    for (bang, plain) in [("delete!", "delete"), ("tr!", "tr"), ("tr_s!", "tr"), ("delete_prefix!", "delete_prefix"), ("delete_suffix!", "delete_suffix")] {
        let plain_id = sym(plain);
        let p = proc_new_native(
            Box::new(move |s, a, b| {
                if is_frozen(s) {
                    return check_frozen(s);
                }
                let r = crate::tri!(send(s, plain_id, a, b));
                if is_str(r) && rstr(r).b != rstr(s).b {
                    rstr(s).b.set(rstr(r).b.clone());
                    s
                } else {
                    QNIL
                }
            }),
            0,
            -1,
        );
        add_method(c.string, sym(bang), Body::Proc(p));
    }
}

/// Drop a pending exception (used by `defined?(expr)` which never raises).
#[unsafe(no_mangle)]
pub extern "C" fn kn_clear_pending() -> V {
    pst().pending = Throw::None;
    QNIL
}

/// `class << obj` opened inside a module body: constants resolve through that module.
#[unsafe(no_mangle)]
pub extern "C" fn kn_set_lexical_parent(c: V, cref: V) -> V {
    if is_class(c) && is_class(cref) && c != cref {
        let k = rclass(c);
        if k.lexical_parent.is_null() {
            k.lexical_parent = cref as *mut Class;
        }
    }
    c
}

// ---------- runtime-provided features (native replacements for C extensions) ----------

struct Feature {
    names: &'static [&'static str],
    init: Option<fn()>,
    loaded: bool,
}
static FEATURES: crate::mt::L<Vec<Feature>> = crate::mt::L::new(Vec::new());

/// Register a feature implemented natively by the runtime: `require "name"` succeeds and
/// runs `init` once (so e.g. `JSON` only exists after `require "json"`, like CRuby).
pub fn provide_feature(names: &'static [&'static str], init: Option<fn()>) {
    FEATURES.get().push(Feature { names, init, loaded: false });
}

/// Kernel#require hook: Some(true/false) if `base` names a runtime-provided feature.
pub fn require_feature(base: &str) -> Option<V> {
    // the lock is held while `init` runs, so a second thread requiring it waits
    let mut fs = FEATURES.get();
    let i = fs.iter().position(|f| f.names.contains(&base))?;
    if fs[i].loaded {
        return Some(QFALSE);
    }
    fs[i].loaded = true;
    if let Some(init) = fs[i].init {
        init();
    }
    Some(QTRUE)
}

// ---------- precompiled files reachable by runtime `require` / autoload ----------

static FILE_TABLE: G<Option<std::collections::HashMap<String, (CompiledFn, *mut V)>>> = G::new(None);

/// Called by the program's `main` before the runtime starts (no Ruby objects involved).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_file(key: *const u8, len: u64, f: CompiledFn, flag: *mut V) {
    let k = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(key, len as usize) }).into_owned();
    FILE_TABLE.get().get_or_insert_with(std::collections::HashMap::new).entry(k).or_insert((f, flag));
}

/// Run a precompiled file for `require name`: Some(true) loaded now, Some(false) already loaded,
/// Some(UNDEF) raised, None if it isn't in the binary.
/// `load(path)`: run a compiled-in file again (unlike require, every time).
pub fn load_compiled(name: &str) -> Option<V> {
    let key = name.trim_end_matches(".rb");
    let (f, flag) = *FILE_TABLE.get().as_ref()?.get(key)?;
    unsafe {
        (*(flag as *const std::sync::atomic::AtomicU64)).store(1, std::sync::atomic::Ordering::Release);
        let r = f(rt().main_obj, 0, std::ptr::null(), QNIL);
        if r == UNDEF {
            return Some(UNDEF);
        }
    }
    Some(QTRUE)
}

fn kernel_load(a: &[V]) -> V {
    let path = if is_str(a[0]) { a[0] } else { crate::tri!(send0(a[0], "to_path")) };
    if !is_str(path) {
        return type_error("no implicit conversion into String");
    }
    let name = String::from_utf8_lossy(&rstr(path).b).into_owned();
    match load_compiled(&name) {
        Some(r) => r,
        None => {
            let le = const_lookup(rt().c.object, sym("LoadError")).unwrap();
            let e = exc_new(le as *mut Class, &format!("cannot load such file -- {name}"));
            ivar_set(e, sym("@path"), str_from(&name));
            raise_exc(e)
        }
    }
}

pub fn require_compiled(name: &str) -> Option<V> {
    let key = name.trim_end_matches(".rb");
    let (f, flag) = *FILE_TABLE.get().as_ref()?.get(key)?;
    let st = unsafe { &*(flag as *const std::sync::atomic::AtomicU64) }.load(std::sync::atomic::Ordering::Acquire);
    if st == 0 {
        if let Some(&lf) = rt().gvars.get(&sym("$LOADED_FEATURES")) {
            if is_ary(lf) {
                rary(lf).v.push(str_from(&format!("{key}.rb")));
            }
        }
    }
    Some(run_file_once(f, flag))
}

/// Run a compiled file unless it already ran: QTRUE if it ran now, QFALSE if it had (or is
/// being loaded by this same thread: a circular require), UNDEF if it raised. If another
/// thread is loading it right now, wait for that to finish first (CRuby's require lock).
/// `flag`: 0 = not loaded, 1 = loaded, otherwise the token of the loading thread.
pub fn run_file_once(f: CompiledFn, flag: *mut V) -> V {
    use std::sync::atomic::{AtomicU64, Ordering};
    let fl = unsafe { &*(flag as *const AtomicU64) };
    let me = crate::thread::current_token();
    loop {
        match fl.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                let r = unsafe { f(rt().main_obj, 0, std::ptr::null(), QNIL) };
                fl.store(1, Ordering::Release);
                return if r == UNDEF { UNDEF } else { QTRUE };
            }
            Err(1) => return QFALSE,
            Err(t) if t == me => return QFALSE,
            Err(_) => {
                if crate::thread::wait_a_moment() {
                    return UNDEF;
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_require_file(flag: *mut V, f: CompiledFn) -> V {
    run_file_once(f, flag)
}
