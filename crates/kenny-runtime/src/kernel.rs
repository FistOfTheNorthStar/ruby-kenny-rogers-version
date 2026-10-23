//! Kernel, BasicObject/Object, Module/Class, nil/true/false, Comparable, Exception.

use crate::core::*;
use crate::gc::RVec;
use crate::fmt::*;
use crate::tri;
use std::io::Write;

// ---------- output ----------

pub fn out(b: &[u8]) {
    let mut o = rt().out.get();
    o.extend_from_slice(b);
    if o.len() > (1 << 16) {
        drop(o);
        flush_out();
    }
}
pub fn flush_out() {
    let mut o = rt().out.get();
    if !o.is_empty() {
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(&o);
        let _ = so.flush();
        o.clear();
    }
}
pub fn err_out(b: &[u8]) {
    flush_out();
    let _ = std::io::stderr().write_all(b);
}

pub fn puts_to(args: &[V], w: &mut dyn FnMut(&[u8])) -> V {
    if args.is_empty() {
        w(b"\n");
        return QNIL;
    }
    for &a in args {
        if is_ary(a) {
            let items = rary(a).v.clone();
            if items.is_empty() {
                w(b"\n");
            }
            tri!(puts_to(&items, w));
            continue;
        }
        let s = match to_s(a) {
            Ok(s) => s,
            Err(()) => return UNDEF,
        };
        w(&s);
        if !s.ends_with(b"\n") {
            w(b"\n");
        }
    }
    QNIL
}

fn str_arg(v: V) -> Option<Vec<u8>> {
    if is_str(v) {
        Some(rstr(v).b.clone())
    } else if is_sym(v) {
        Some(sym_name(sym_id(v)).to_vec())
    } else {
        None
    }
}
pub fn sym_arg(v: V) -> Result<u32, V> {
    match str_arg(v) {
        Some(b) => Ok(intern(&b)),
        None => Err(type_error(&format!("{} is not a symbol nor a string", inspect_lossy(v)))),
    }
}
pub fn new_str_v(s: String) -> V {
    str_new(s.into_bytes())
}

fn is_user_method(v: V, name: &str) -> bool {
    let m = find_method(class_of(v), sym(name));
    !m.is_null() && !matches!(unsafe { &(*m).body }, Body::Native(..))
}

pub fn dup(v: V, keep_frozen: bool) -> V {
    let r = match kind(v) {
        None => return v,
        Some(Kind::String) => {
            let n = str_new(rstr(v).b.clone());
            hdr(n).flags |= hdr(v).flags & FL_BINARY;
            n
        }
        Some(Kind::Array) => ary_new(rary(v).v.clone()),
        Some(Kind::Hash) => {
            let h = hash_new();
            let (src, dst) = (rhash(v), rhash(h));
            for (k, x) in src.m.iter() {
                dst.m.insert(HKey(k.0), *x);
            }
            dst.default = src.default; // (h is new: young, no barrier)
            dst.default_proc = src.default_proc;
            h
        }
        Some(Kind::Object) => {
            let o = obj(v);
            let n = object_new(o.shape);
            obj(n).ivars.set(o.ivars.clone());
            n
        }
        Some(Kind::Class) => return dup_module(v),
        Some(Kind::Float) | Some(Kind::Bignum) | Some(Kind::Range) | Some(Kind::Proc) | Some(Kind::Data) | Some(Kind::Env) => return v,
    };
    hdr(r).klass = real_class(v);
    gen_ivar_copy(v, r);
    // dup calls initialize_dup, clone initialize_clone; both default to initialize_copy
    let hook = if keep_frozen { "initialize_clone" } else { "initialize_dup" };
    if is_user_method(r, hook) {
        tri!(send(r, sym(hook), &[v], QNIL));
    } else if is_user_method(r, "initialize_copy") {
        tri!(send(r, sym("initialize_copy"), &[v], QNIL));
    }
    if keep_frozen && is_frozen(v) {
        hdr(r).flags |= FL_FROZEN;
    }
    r
}

// ---------- random ----------

pub fn rand_u64() -> u64 {
    let r = rt();
    let mut x = r.rng;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    r.rng = x;
    x.wrapping_mul(0x2545F4914F6CDD1D)
}
pub fn rand_f() -> f64 {
    (rand_u64() >> 11) as f64 / (1u64 << 53) as f64
}

// ---------- class creation helpers ----------

pub fn alloc_instance(c: *mut Class) -> V {
    if let Some(f) = crate::capi::alloc_func(c) {
        return crate::capi::call_cfunc(f as *const std::ffi::c_void, 0, c as V, &[]);
    }
    let v = match unsafe { (*c).alloc } {
        Kind::String => str_new(Vec::new()),
        Kind::Array => ary_new(Vec::new()),
        Kind::Hash => hash_new(),
        Kind::Data => crate::thread::mutex_alloc(c),
        _ => object_new(c),
    };
    hdr(v).klass = c;
    v
}

/// `object_id`: Fixnums 2n+1, flonums their bits without the constant tag (all of the
/// rest matters, including the sign bit), other objects their address / 8.
fn object_id_of(s: V) -> i64 {
    if is_heap(s) {
        // the id is the address: the object must never move
        hdr(s).flags |= crate::gc::FL_ID;
    }
    if is_fix(s) {
        fix(s).wrapping_mul(2).wrapping_add(1)
    } else if is_flonum(s) {
        (s >> 2) as i64
    } else {
        (s >> 3) as i64
    }
}

pub fn class_new_instance(c: V, args: &[V], blk: V) -> V {
    let k = c as *mut Class;
    unsafe {
        if (*k).is_module {
            return raise(rt().c.no_method_error, &format!("undefined method 'new' for module {}", (*k).name));
        }
    }
    let o = tri!(alloc_instance(k));
    static INITIALIZE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let m = find_method(k, *INITIALIZE.get_or_init(|| sym("initialize")));
    if !m.is_null() {
        tri!(invoke(m, o, args, blk));
    } else if !args.is_empty() {
        return arity_error(args.len() as i32, 0, 0);
    }
    o
}

pub fn do_include(target: *mut Class, m: V) -> V {
    if !is_class(m) || !rclass(m).is_module {
        return type_error(&format!("wrong argument type {} (expected Module)", class_name(real_class(m))));
    }
    if !ancestors(target).contains(&(m as *mut Class)) {
        unsafe { (*target).includes.push(m as *mut Class) };
        crate::core::touch_mixin(target, m as *mut Class);
        note_include(target, m as *mut Class);
        bump_serial();
    }
    QNIL
}


/// Runtime compiler for `eval`-family calls (installed by the JIT-enabled runtime).
/// Arguments: source, mode (0 = eval/instance_eval, 1 = class_eval), self.
pub static EVAL_HOOK: G<Option<fn(&[u8], u8, V) -> V>> = G::new(None);

pub fn eval_string(src: V, mode: u8, self_: V) -> V {
    if !is_str(src) {
        return type_error(&format!("no implicit conversion of {} into String", class_name(real_class(src))));
    }
    match *EVAL_HOOK.get() {
        Some(f) => f(&rstr(src).b.clone(), mode, self_),
        None => raise(
            rt().c.not_implemented_error,
            "eval of strings needs the runtime compiler; this binary was built without it (kenny links it automatically when it sees eval/class_eval/instance_eval with a string)",
        ),
    }
}

/// The class `def` currently defines into. An entry may stand for "the singleton class of
/// this object" (low bit set; see `with_singleton_definee`), created only when needed.
pub fn definee_top() -> Option<*mut Class> {
    let &c = pst().definee.last()?;
    if c as usize & 1 == 0 {
        return Some(c);
    }
    let sc = singleton_class((c as usize & !1) as V);
    // (re-read: making the class may run Ruby code, and the thread may have moved P)
    *pst().definee.last_mut().unwrap() = sc;
    Some(sc)
}

/// instance_eval / instance_exec: `def` inside defines singleton methods of `obj`, but most
/// blocks never do, so its singleton class is made only on demand (Rails runs proc
/// callbacks through instance_exec on every request).
pub fn with_singleton_definee(obj: V, f: impl FnOnce() -> V) -> V {
    let entry = if is_heap(obj) { (obj as usize | 1) as *mut Class } else { class_of(obj) };
    pst().definee.push(entry);
    let r = f();
    pst().definee.pop();
    r
}

pub fn with_definee(c: *mut Class, f: impl FnOnce() -> V) -> V {
    pst().definee.push(c);
    let r = f();
    pst().definee.pop();
    r
}

pub fn class_exec(c: V, args: &[V], blk: V) -> V {
    if !is_proc(blk) {
        return arg_error("no block given");
    }
    let (saved, mf) = (rclass(c).default_vis, rclass(c).module_function);
    rclass(c).default_vis = VIS_PUBLIC;
    rclass(c).module_function = false;
    let r = with_definee(c as *mut Class, || call_proc_with_self(blk, c, args, QNIL, false));
    rclass(c).default_vis = saved;
    rclass(c).module_function = mf;
    r
}

fn cmp_int(a: V, b: V) -> Result<Option<i64>, ()> {
    let r = send(a, sym("<=>"), &[b], QNIL);
    if r == UNDEF {
        return Err(());
    }
    if r == QNIL { Ok(None) } else if is_fix(r) { Ok(Some(fix(r))) } else { Ok(None) }
}
fn cmp_fail(a: V, b: V) -> V {
    arg_error(&format!("comparison of {} with {} failed", class_name(real_class(a)), inspect_lossy(b)))
}

pub fn parse_int(b: &[u8], strict: bool) -> Option<V> {
    // the common case: an optional sign and up to 18 decimal digits, nothing else
    let (neg, ds) = match b.first() {
        Some(b'-') => (true, &b[1..]),
        Some(b'+') => (false, &b[1..]),
        _ => (false, b),
    };
    if !ds.is_empty() && ds.len() <= 18 && ds.iter().all(|c| c.is_ascii_digit()) {
        let n = ds.iter().fold(0i64, |n, c| n * 10 + (c - b'0') as i64);
        return Some(mkfix(if neg { -n } else { n }));
    }
    let s = String::from_utf8_lossy(b);
    let t = s.trim();
    let t: String = t.chars().filter(|c| *c != '_').collect();
    let (neg, body) = match t.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, t.strip_prefix('+').unwrap_or(&t).to_string()),
    };
    // (radix prefixes for Integer() only: "0x1A".to_i is 0)
    let (radix, digits) = if !strict {
        (10, body.clone())
    } else if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        (16, r.to_string())
    } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        (2, r.to_string())
    } else if let Some(r) = body.strip_prefix("0o") {
        (8, r.to_string())
    } else {
        (10, body.clone())
    };
    let digits: String = if strict {
        digits
    } else {
        digits.chars().take_while(|c| c.is_digit(radix)).collect()
    };
    if digits.is_empty() {
        return if strict { None } else { Some(mkfix(0)) };
    }
    let n = num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(int_from_big(if neg { -n } else { n }))
}

pub fn init() {
    let c = rt().c;
    let k = c.kernel;

    defn(k, "puts", 0, -1, |_, a, _| puts_to(a, &mut |b| out(b)));
    defn(k, "print", 0, -1, |_, a, _| {
        for &x in a {
            match to_s(x) {
                Ok(s) => out(&s),
                Err(()) => return UNDEF,
            }
        }
        QNIL
    });
    fn p_impl(a: &[V]) -> V {
        for &x in a {
            match inspect(x) {
                Ok(mut s) => {
                    s.push(b'\n');
                    out(&s)
                }
                Err(()) => return UNDEF,
            }
        }
        match a.len() {
            0 => QNIL,
            1 => a[0],
            _ => ary_new(a.to_vec()),
        }
    }
    defn(k, "p", 0, -1, |_, a, _| p_impl(a));
    defn(k, "pp", 0, -1, |_, a, _| p_impl(a));
    defn(k, "warn", 0, -1, |_, a, _| {
        let a: Vec<V> = a.iter().copied().filter(|x| !is_hash(*x)).collect();
        puts_to(&a, &mut |b| err_out(b))
    });
    defn(k, "require", 1, 1, |_, a, _| {
        let name = match crate::fmt::to_s(a[0]) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(()) => return UNDEF,
        };
        if let Some(r) = crate::compat::require_compiled(&name) {
            return r;
        }
        if let Some(r) = crate::compat::require_from_disk(&name) {
            return r;
        }
        if let Some(r) = crate::capi::require_ext(&name) {
            return r;
        }
        // features built into the Kenny runtime
        let base = name.trim_end_matches(".rb").trim_end_matches(".so").trim_end_matches(".bundle");
        const BUILTIN: &[&str] = &[
            "stringio", "thread", "enumerator", "fiber", "monitor", "rbconfig", "rational", "complex", "etc",
            "io/console", "io/wait", "io/nonblock", "continuation", "rubygems", "bundler/setup", "bundler",
        ];
        if let Some(r) = crate::compat::require_feature(base) {
            return r;
        }
        if BUILTIN.contains(&base) {
            return QFALSE;
        }
        let le = const_lookup(rt().c.object, sym("LoadError")).unwrap() as *mut Class;
        let e = exc_new(le, &format!("cannot load such file -- {name}"));
        ivar_set(e, sym("@path"), str_from(&name));
        raise_exc(e)
    });
    defn(k, "require_relative", 1, 1, |_, _, _| QFALSE);
    fn raise_impl(a: &[V]) -> V {
        let r = rt();
        let e = if a.is_empty() {
            if pst().errinfo != QNIL { pst().errinfo } else { exc_new(r.c.runtime_error, "unhandled exception") }
        } else if is_str(a[0]) {
            exc_new(r.c.runtime_error, &String::from_utf8_lossy(&rstr(a[0]).b))
        } else if is_class(a[0]) {
            let args: Vec<V> = a[1..].iter().copied().filter(|x| !is_hash(*x) || a.len() < 3).collect();
            tri!(send(a[0], sym("new"), &args[..args.len().min(1)], QNIL))
        } else if is_a(a[0], r.c.exception) {
            if a.len() > 1 {
                ivar_set(a[0], sym("@__message"), a[1]);
            }
            a[0]
        } else if respond_to(a[0], sym("exception")) {
            // any object with #exception, e.g. a rejected Concurrent::Promise
            tri!(send(a[0], sym("exception"), &a[1..a.len().min(2)], QNIL))
        } else {
            return type_error("exception class/object expected");
        };
        if !is_a(e, r.c.exception) {
            return type_error("exception object expected");
        }
        raise_exc(e)
    }
    defn(k, "raise", 0, 3, |_, a, _| raise_impl(a));
    defn(k, "fail", 0, 3, |_, a, _| raise_impl(a));
    defn(k, "loop", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "loop", &[]);
        }
        loop {
            let r = call_block(b, &[]);
            if r == UNDEF {
                if let Throw::Raise(e) = pst().pending {
                    if is_a(e, rt().c.stop_iteration) {
                        pst().pending = Throw::None;
                        return ivar_get(e, sym("@__result"));
                    }
                }
                return UNDEF;
            }
        }
    });
    defn(k, "catch", 0, 1, |_, a, b| {
        let tag = if a.is_empty() { object_new(rt().c.object) } else { a[0] };
        let r = call_block(b, &[tag]);
        if r == UNDEF {
            if let Throw::Throw { tag: t, val } = pst().pending {
                if eql(t, tag) {
                    pst().pending = Throw::None;
                    return val;
                }
            }
        }
        r
    });
    defn(k, "throw", 1, 2, |_, a, _| {
        pst().pending = Throw::Throw { tag: a[0], val: a.get(1).copied().unwrap_or(QNIL) };
        UNDEF
    });
    defn(k, "rand", 0, 1, |_, a, _| {
        match a.first().copied() {
            None | Some(QNIL) => float_new(rand_f()),
            Some(n) if is_fix(n) && fix(n) > 0 => mkfix((rand_u64() % fix(n) as u64) as i64),
            Some(n) if is_float(n) => float_new(rand_f() * float_val(n)),
            Some(rg) if is_range(rg) => {
                let r = rrange(rg);
                if is_fix(r.lo) && is_fix(r.hi) {
                    let hi = fix(r.hi) - if r.excl { 1 } else { 0 };
                    let span = (hi - fix(r.lo) + 1).max(1) as u64;
                    mkfix(fix(r.lo) + (rand_u64() % span) as i64)
                } else {
                    let (lo, hi) = (num_to_f(r.lo).unwrap_or(0.0), num_to_f(r.hi).unwrap_or(1.0));
                    float_new(lo + rand_f() * (hi - lo))
                }
            }
            _ => float_new(rand_f()),
        }
    });
    defn(k, "srand", 0, 1, |_, a, _| {
        let old = rt().rng;
        rt().rng = a.first().filter(|x| is_fix(**x)).map(|x| (fix(*x) as u64) | 1).unwrap_or(0x9E3779B97F4A7C15);
        int_from_i128(old as i128)
    });
    defn(k, "sleep", 0, 1, |_, a, _| {
        flush_out();
        let secs = a.first().and_then(|x| num_to_f(*x)).unwrap_or(0.0);
        std::thread::sleep(std::time::Duration::from_secs_f64(secs.max(0.0)));
        mkfix(secs.round() as i64)
    });
    defn(k, "gets", 0, 1, |_, _, _| {
        flush_out();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => QNIL,
            Ok(_) => new_str_v(line),
        }
    });
    fn format_impl(a: &[V]) -> V {
        let Some(f) = a.first().filter(|x| is_str(**x)) else { return type_error("format string expected") };
        match sprintf(&rstr(*f).b.clone(), &a[1..]) {
            Ok(b) => str_new(b),
            Err(e) => e,
        }
    }
    defn(k, "format", 1, -1, |_, a, _| format_impl(a));
    defn(k, "sprintf", 1, -1, |_, a, _| format_impl(a));
    defn(k, "printf", 1, -1, |_, a, _| {
        let s = tri!(format_impl(a));
        out(&rstr(s).b.clone());
        QNIL
    });
    defn(k, "Integer", 1, 2, |_, a, _| {
        let v = a[0];
        if is_int(v) {
            v
        } else if is_float(v) {
            let f = float_val(v);
            if !f.is_finite() {
                return raise(rt().c.float_domain_error, &float_to_s(f));
            }
            int_from_big(num_bigint::BigInt::from(f.trunc() as i128))
        } else if is_str(v) {
            match parse_int(&rstr(v).b.clone(), true) {
                Some(i) => i,
                None => arg_error(&format!("invalid value for Integer(): {}", inspect_lossy(v))),
            }
        } else if v == QNIL {
            type_error("can't convert nil into Integer")
        } else {
            send0(v, "to_i")
        }
    });
    defn(k, "Float", 1, 1, |_, a, _| {
        let v = a[0];
        if let Some(f) = num_to_f(v) {
            return float_new(f);
        }
        if is_str(v) {
            let s = String::from_utf8_lossy(&rstr(v).b).trim().replace('_', "");
            return match s.parse::<f64>() {
                Ok(f) => float_new(f),
                Err(_) => arg_error(&format!("invalid value for Float(): {}", inspect_lossy(v))),
            };
        }
        if v == QNIL {
            return type_error("can't convert nil into Float");
        }
        send0(v, "to_f")
    });
    defn(k, "String", 1, 1, |_, a, _| match to_s(a[0]) {
        Ok(b) => str_new(b),
        Err(()) => UNDEF,
    });
    defn(k, "Array", 1, 1, |_, a, _| {
        let v = a[0];
        if v == QNIL {
            ary_new(vec![])
        } else if is_ary(v) {
            v
        } else if is_hash(v) || is_range(v) {
            send0(v, "to_a")
        } else {
            // Array(obj): to_ary, then to_a, else [obj]
            for conv in ["to_ary", "to_a"] {
                if respond_to(v, sym(conv)) {
                    let r = tri!(send0(v, conv));
                    if is_ary(r) {
                        return r;
                    }
                    if r != QNIL {
                        return type_error(&format!("can't convert {} to Array ({}#{} gives {})", class_name(real_class(v)), class_name(real_class(v)), conv, class_name(real_class(r))));
                    }
                }
            }
            ary_new(vec![v])
        }
    });
    defn(k, "lambda", 0, 0, |_, _, b| {
        if is_proc(b) {
            rproc(b).lambda = true;
        }
        b
    });
    defn(k, "proc", 0, 0, |_, _, b| b);
    defn(k, "block_given?", 0, 0, |_, _, _| QFALSE);
    defn(k, "eval", 1, 4, |s, a, _| {
        let recv = match a.get(1).copied().filter(|b| *b != QNIL) {
            Some(b) => match crate::compat::binding_receiver(b) {
                Some(r) => r,
                None => return type_error("wrong argument type (expected binding)"),
            },
            None => s,
        };
        eval_string(a[0], 0, recv)
    });
    defn(k, "binding", 0, 0, |_, _, _| raise(rt().c.not_implemented_error, "binding is not supported by Kenny"));
    defn(k, "caller", 0, 2, |_, _, _| ary_new(vec![]));
    defn(k, "caller_locations", 0, 2, |_, _, _| ary_new(vec![]));
    defn(k, "exit", 0, 1, |_, a, _| {
        let code = match a.first() {
            Some(&QTRUE) | None => 0,
            Some(&QFALSE) => 1,
            Some(&x) if is_fix(x) => fix(x) as i32,
            _ => 0,
        };
        crate::abi::run_at_exit();
        flush_out();
        std::process::exit(code)
    });
    defn(k, "exit!", 0, 1, |_, a, _| {
        flush_out();
        std::process::exit(a.first().filter(|x| is_fix(**x)).map(|x| fix(*x) as i32).unwrap_or(0))
    });
    defn(k, "abort", 0, 1, |_, a, _| {
        if let Some(&m) = a.first() {
            if let Ok(mut s) = to_s(m) {
                s.push(b'\n');
                err_out(&s);
            }
        }
        crate::abi::run_at_exit();
        flush_out();
        std::process::exit(1)
    });
    defn(k, "at_exit", 0, 0, |_, _, b| {
        crate::abi::AT_EXIT.get().push(b);
        b
    });

    // --- Object protocol (on Kernel so every object gets it) ---
    defn(k, "class", 0, 0, |s, _, _| real_class(s) as V);
    defn(k, "singleton_class", 0, 0, |s, _, _| {
        tri!(check_singleton_ok(s));
        singleton_class(s) as V
    });
    defn(k, "frozen?", 0, 0, |s, _, _| bool_v(is_frozen(s)));
    defn(k, "freeze", 0, 0, |s, _, _| {
        if is_heap(s) {
            hdr(s).flags |= FL_FROZEN;
        }
        s
    });
    defn(k, "nil?", 0, 0, |s, _, _| bool_v(s == QNIL));
    defn(k, "===", 1, 1, |s, a, _| equal(s, a[0]));
    defn(k, "=~", 1, 1, |_, _, _| QNIL);
    // Object#eql? is identity (plus value equality of the core value types); unlike the
    // hash-key `eql`, it must not dispatch to a user eql? (`super` from Mime::Type#eql?)
    defn(k, "eql?", 1, 1, |s, a, _| {
        let o = a[0];
        bool_v(match (kind(s), kind(o)) {
            (Some(Kind::Object), _) | (_, Some(Kind::Object)) | (Some(Kind::Data), _) | (Some(Kind::Class), _) | (Some(Kind::Proc), _) => s == o,
            _ => eql(s, o),
        })
    });
    defn(k, "hash", 0, 0, |s, _, _| mkfix((hash_of(s) >> 2) as i64));
    defn(k, "object_id", 0, 0, |s, _, _| mkfix(object_id_of(s)));
    defn(k, "to_s", 0, 0, |s, _, _| {
        if s == rt().main_obj {
            return str_from("main");
        }
        new_str_v(crate::fmt::any_to_s(s))
    });
    defn(k, "inspect", 0, 0, |s, _, _| {
        if s == rt().main_obj {
            return str_from("main");
        }
        if is_user_method(s, "to_s") && ivar_list(s).is_empty() {
            return send0(s, "to_s");
        }
        match obj_inspect(s) {
            Ok(x) => new_str_v(x),
            Err(()) => UNDEF,
        }
    });
    defn(k, "is_a?", 1, 1, |s, a, _| is_a_v(s, a[0]));
    defn(k, "kind_of?", 1, 1, |s, a, _| is_a_v(s, a[0]));
    defn(k, "instance_of?", 1, 1, |s, a, _| bool_v(real_class(s) as V == a[0]));
    defn(k, "respond_to?", 1, 2, |s, a, _| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        if respond_to(s, id) {
            return QTRUE;
        }
        if is_user_method(s, "respond_to_missing?") {
            return send(s, sym("respond_to_missing?"), &[mksym(id), QFALSE], QNIL);
        }
        QFALSE
    });
    fn send_impl(s: V, a: &[V], b: V) -> V {
        if a.is_empty() {
            return arg_error("no method name given");
        }
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        send(s, id, &a[1..], b)
    }
    defn(k, "send", 1, -1, send_impl);
    defn(k, "__send__", 1, -1, send_impl);
    defn(k, "public_send", 1, -1, send_impl);
    defn(k, "instance_variable_get", 1, 1, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => ivar_get(s, id),
        Err(e) => e,
    });
    defn(k, "instance_variable_set", 2, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => ivar_set(s, id, a[1]),
        Err(e) => e,
    });
    defn(k, "instance_variable_defined?", 1, 1, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(ivar_defined(s, id)),
        Err(e) => e,
    });
    defn(k, "instance_variables", 0, 0, |s, _, _| {
        ary_new(ivar_list(s).into_iter().map(|(n, _)| mksym(n)).collect())
    });
    defn(k, "dup", 0, 0, |s, _, _| dup(s, false));
    defn(k, "clone", 0, 1, |s, a, _| {
        let keep = !a.first().is_some_and(|h| is_hash(*h));
        dup(s, keep)
    });
    defn(k, "itself", 0, 0, |s, _, _| s);
    defn(k, "tap", 0, 0, |s, _, b| {
        tri!(call_block(b, &[s]));
        s
    });
    defn(k, "then", 0, 0, |s, _, b| call_block(b, &[s]));
    defn(k, "yield_self", 0, 0, |s, _, b| call_block(b, &[s]));
    defn(k, "extend", 1, -1, |s, a, _| {
        tri!(check_singleton_ok(s));
        let sc = singleton_class(s);
        for &m in a.iter().rev() {
            tri!(do_include(sc, m));
            if is_user_method(m, "extended") {
                tri!(send(m, sym("extended"), &[s], QNIL));
            }
        }
        s
    });
    defn(k, "define_singleton_method", 1, 2, |s, a, b| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let body = tri!(crate::methodobj::method_body(a.get(1).copied().unwrap_or(b)));
        if !is_proc(body) {
            return arg_error("tried to create Proc object without a block");
        }
        tri!(check_singleton_ok(s));
        add_method(singleton_class(s), id, Body::Proc(body));
        mksym(id)
    });
    defn(k, "singleton_methods", 0, 1, |s, _, _| {
        let sc = class_of(s);
        if unsafe { !(*sc).is_singleton } {
            return ary_new(vec![]);
        }
        ary_new(unsafe { (*sc).methods.keys().map(|&id| mksym(id)).collect() })
    });
    defn(k, "instance_eval", 0, 3, |s, a, b| {
        if !is_proc(b) {
            let Some(&src) = a.first() else { return arg_error("wrong number of arguments (given 0, expected 1..3)") };
            let sc = if is_heap(s) { singleton_class(s) } else { class_of(s) };
            return with_definee(sc, || eval_string(src, 0, s));
        }
        with_singleton_definee(s, || call_proc_with_self(b, s, &[s], QNIL, false))
    });
    defn(k, "instance_exec", 0, -1, |s, a, b| with_singleton_definee(s, || call_proc_with_self(b, s, a, QNIL, false)));
    defn(k, "method", 1, 1, |s, a, _| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        if !respond_to(s, id) {
            return raise(rt().c.name_error, &format!("undefined method '{}' for {}", sym_str(id), inspect_lossy(s)));
        }
        let p = proc_new_native_rooted(Box::new(move |_, args, blk| send(s, id, args, blk)), 0, -1, vec![s]);
        rproc(p).lambda = true;
        p
    });
    defn(k, "methods", 0, 1, |s, _, _| {
        let mut out = RVec::new();
        for c in ancestors(class_of(s)) {
            for &id in unsafe { (*c).methods.keys() } {
                if !out.contains(&mksym(id)) {
                    out.push(mksym(id));
                }
            }
        }
        ary_new(out.take())
    });
    defn(k, "enum_for", 0, -1, |s, a, _| {
        let m = a.first().copied().unwrap_or(mksym(sym("each")));
        crate::misc::enum_for(s, &sym_str(sym_id(m)), a.get(1..).unwrap_or(&[]))
    });
    defn(k, "to_enum", 0, -1, |s, a, _| {
        let m = a.first().copied().unwrap_or(mksym(sym("each")));
        crate::misc::enum_for(s, &sym_str(sym_id(m)), a.get(1..).unwrap_or(&[]))
    });
    defn(k, "display", 0, 0, |s, _, _| match to_s(s) {
        Ok(b) => {
            out(&b);
            QNIL
        }
        Err(()) => UNDEF,
    });

    let bo = c.basic_object;
    defn(bo, "initialize", 0, -1, |_, _, _| QNIL);
    defn(bo, "==", 1, 1, |s, a, _| bool_v(s == a[0]));
    defn(bo, "equal?", 1, 1, |s, a, _| bool_v(s == a[0]));
    defn(bo, "!", 0, 0, |s, _, _| bool_v(!truthy(s)));
    defn(bo, "!=", 1, 1, |s, a, _| {
        // always dispatches to `==` (no identity shortcut: NaN != NaN)
        let r = tri!(send(s, sym("=="), &[a[0]], QNIL));
        bool_v(!truthy(r))
    });
    defn(bo, "__id__", 0, 0, |s, _, _| mkfix(object_id_of(s)));
    defn(bo, "method_missing", 1, -1, |s, a, _| no_method_error(s, sym_id(a[0]), false));
    defn(bo, "respond_to_missing?", 2, 2, |_, _, _| QFALSE);
    defn(bo, "singleton_method_added", 1, 1, |_, _, _| QNIL);

    // --- Module / Class ---
    let md = c.module;
    defn(md, "name", 0, 0, |s, _, _| {
        let c = rclass(s);
        if c.name.is_empty() || c.is_singleton { QNIL } else { str_from(&c.name.clone()) }
    });
    defn(md, "to_s", 0, 0, |s, _, _| new_str_v(class_name(s as *mut Class)));
    defn(md, "inspect", 0, 0, |s, _, _| new_str_v(class_name(s as *mut Class)));
    defn(md, "hash", 0, 0, |s, _, _| mkfix((s >> 3) as i64));
    defn(md, "===", 1, 1, |s, a, _| bool_v(is_a(a[0], s as *mut Class)));
    defn(md, "==", 1, 1, |s, a, _| bool_v(s == a[0]));
    defn(md, "<", 1, 1, |s, a, _| {
        if s == a[0] {
            return QFALSE;
        }
        if ancestors(s as *mut Class).contains(&(a[0] as *mut Class)) {
            QTRUE
        } else if ancestors(a[0] as *mut Class).contains(&(s as *mut Class)) {
            QFALSE
        } else {
            QNIL
        }
    });
    defn(md, "<=", 1, 1, |s, a, _| {
        if s == a[0] || ancestors(s as *mut Class).contains(&(a[0] as *mut Class)) {
            QTRUE
        } else if ancestors(a[0] as *mut Class).contains(&(s as *mut Class)) {
            QFALSE
        } else {
            QNIL
        }
    });
    defn(md, ">", 1, 1, |s, a, _| bool_v(s != a[0] && ancestors(a[0] as *mut Class).contains(&(s as *mut Class))));
    defn(md, ">=", 1, 1, |s, a, _| bool_v(ancestors(a[0] as *mut Class).contains(&(s as *mut Class))));
    defn(md, "ancestors", 0, 0, |s, _, _| ary_new(ancestors(s as *mut Class).into_iter().map(|c| c as V).collect()));
    defn(md, "included_modules", 0, 0, |s, _, _| {
        ary_new(ancestors(s as *mut Class).into_iter().filter(|c| unsafe { (**c).is_module }).map(|c| c as V).collect())
    });
    defn(md, "include?", 1, 1, |s, a, _| bool_v(s != a[0] && ancestors(s as *mut Class).contains(&(a[0] as *mut Class))));
    defn(md, "append_features", 1, 1, |s, a, _| do_include(a[0] as *mut Class, s));
    defn(md, "extend_object", 1, 1, |s, a, _| do_include(singleton_class(a[0]), s));
    defn(md, "included", 1, 1, |_, _, _| QNIL);
    defn(md, "extended", 1, 1, |_, _, _| QNIL);
    defn(md, "prepended", 1, 1, |_, _, _| QNIL);
    defn(md, "inherited", 1, 1, |_, _, _| QNIL);
    defn(md, "method_added", 1, 1, |_, _, _| QNIL);
    defn(md, "const_added", 1, 1, |_, _, _| QNIL);
    fn include_impl(s: V, a: &[V]) -> V {
        if !is_class(s) {
            return type_error("include on non-module");
        }
        for &m in a.iter().rev() {
            if !is_class(m) || !rclass(m).is_module {
                return type_error(&format!("wrong argument type {} (expected Module)", class_name(real_class(m))));
            }
            tri!(send(m, sym("append_features"), &[s], QNIL));
            tri!(send(m, sym("included"), &[s], QNIL));
        }
        s
    }
    defn(md, "include", 1, -1, |s, a, _| include_impl(s, a));
    defn(md, "prepend_features", 1, 1, |s, a, _| {
        let target = a[0] as *mut Class;
        if !is_class(a[0]) {
            return type_error("prepend target must be a module");
        }
        let m = s as *mut Class;
        unsafe {
            if !(*target).prepends.contains(&m) {
                (*target).prepends.push(m);
                crate::core::touch_mixin(target, m);
            }
        }
        bump_serial();
        s
    });
    defn(md, "prepend", 1, -1, |s, a, _| {
        for &m in a.iter().rev() {
            if !is_class(m) || !rclass(m).is_module {
                return type_error(&format!("wrong argument type {} (expected Module)", class_name(real_class(m))));
            }
            tri!(send(m, sym("prepend_features"), &[s], QNIL));
            tri!(send(m, sym("prepended"), &[s], QNIL));
        }
        s
    });
    defn(md, "extend", 1, -1, |s, a, _| {
        for &m in a.iter().rev() {
            tri!(send(m, sym("extend_object"), &[s], QNIL));
            tri!(send(m, sym("extended"), &[s], QNIL));
        }
        s
    });
    fn attr(s: V, a: &[V], r: bool, w: bool) -> V {
        let mut out = RVec::new();
        for &n in a {
            let id = match sym_arg(n) {
                Ok(i) => i,
                Err(e) => return e,
            };
            let name = sym_str(id);
            let iv = sym(&format!("@{name}"));
            if r {
                add_method(s as *mut Class, id, Body::AttrReader(iv));
                out.push(mksym(id));
            }
            if w {
                let wid = sym(&format!("{name}="));
                add_method(s as *mut Class, wid, Body::AttrWriter(iv));
                out.push(mksym(wid));
            }
        }
        ary_new(out.take())
    }
    defn(md, "attr_reader", 0, -1, |s, a, _| attr(s, a, true, false));
    defn(md, "attr", 0, -1, |s, a, _| attr(s, a, true, false));
    defn(md, "attr_writer", 0, -1, |s, a, _| attr(s, a, false, true));
    defn(md, "attr_accessor", 0, -1, |s, a, _| attr(s, a, true, true));
    defn(md, "define_method", 1, 2, |s, a, b| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let body = tri!(crate::methodobj::method_body(a.get(1).copied().unwrap_or(b)));
        if !is_proc(body) {
            return arg_error("tried to create Proc object without a block");
        }
        add_method(s as *mut Class, id, Body::Proc(body));
        mksym(id)
    });
    defn(md, "alias_method", 2, 2, |s, a, _| {
        let (new, old) = match (sym_arg(a[0]), sym_arg(a[1])) {
            (Ok(n), Ok(o)) => (n, o),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        crate::abi::alias(s as *mut Class, new, old)
    });
    defn(md, "remove_method", 0, -1, |s, a, _| {
        for &n in a {
            if let Ok(id) = sym_arg(n) {
                if rclass(s).methods.remove(&id).is_some() {
                    crate::core::method_name_count(id, false);
                    crate::core::touch_method_name(s as *mut Class, id);
                }
            }
        }
        bump_serial();
        s
    });
    defn(md, "undef_method", 0, -1, |s, a, _| {
        for &n in a {
            if let Ok(id) = sym_arg(n) {
                add_method(s as *mut Class, id, Body::Undefined);
            }
        }
        s
    });
    fn visibility(_s: V, a: &[V]) -> V {
        match a.len() {
            0 => QNIL,
            1 => a[0],
            _ => ary_new(a.to_vec()),
        }
    }
    defn(md, "private", 0, -1, |s, a, _| visibility(s, a));
    defn(md, "public", 0, -1, |s, a, _| visibility(s, a));
    defn(md, "protected", 0, -1, |s, a, _| visibility(s, a));
    defn(md, "private_constant", 0, -1, |_, _, _| QNIL);
    defn(md, "public_constant", 0, -1, |_, _, _| QNIL);
    defn(md, "private_class_method", 0, -1, |s, a, _| visibility(s, a));
    defn(md, "public_class_method", 0, -1, |s, a, _| visibility(s, a));
    defn(md, "module_function", 0, -1, |s, a, _| {
        if a.is_empty() {
            rclass(s).module_function = true;
            return QNIL;
        }
        let meta = singleton_class(s);
        for &n in a {
            if let Ok(id) = sym_arg(n) {
                let m = find_method(s as *mut Class, id);
                if !m.is_null() {
                    add_method(meta, id, clone_body(unsafe { &(*m).body }));
                }
            }
        }
        visibility(s, a)
    });
    defn(md, "method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(!find_method(s as *mut Class, id).is_null()),
        Err(e) => e,
    });
    defn(md, "public_method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(!find_method(s as *mut Class, id).is_null()),
        Err(e) => e,
    });
    defn(md, "private_method_defined?", 1, 2, |_, _, _| QFALSE);
    defn(md, "instance_methods", 0, 1, |s, a, _| {
        let inherit = a.first().is_none_or(|x| truthy(*x));
        let classes = if inherit { ancestors(s as *mut Class) } else { vec![s as *mut Class] };
        let mut out = RVec::new();
        let mut hidden: Vec<u32> = Vec::new();
        for c in classes {
            if inherit && c != s as *mut Class && (c == rt().c.object || c == rt().c.kernel || c == rt().c.basic_object) {
                continue;
            }
            for (&id, &m) in unsafe { (*c).methods.iter() } {
                if matches!(unsafe { &(*m).body }, Body::Undefined) {
                    hidden.push(id);
                    continue;
                }
                if !hidden.contains(&id) && !out.contains(&mksym(id)) {
                    out.push(mksym(id));
                }
            }
        }
        ary_new(out.take())
    });
    defn(md, "public_instance_methods", 0, 1, |s, a, _| send(s, sym("instance_methods"), a, QNIL));
    defn(md, "private_instance_methods", 0, 1, |_, _, _| ary_new(vec![]));
    defn(md, "protected_instance_methods", 0, 1, |_, _, _| ary_new(vec![]));
    defn(md, "private_methods", 0, 1, |_, _, _| ary_new(vec![]));
    defn(md, "instance_method", 1, 1, |s, a, _| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        if find_method(s as *mut Class, id).is_null() {
            return raise(rt().c.name_error, &format!("undefined method '{}' for class '{}'", sym_str(id), class_name(s as *mut Class)));
        }
        mksym(id)
    });
    fn const_get_path(s: V, name: &[u8]) -> V {
        let mut cur = s as *mut Class;
        let path = String::from_utf8_lossy(name).into_owned();
        let mut val = QNIL;
        for (i, part) in path.split("::").enumerate() {
            if part.is_empty() {
                cur = rt().c.object;
                continue;
            }
            let id = sym(part);
            let found = if i == 0 { const_lookup(cur, id) } else { const_get_under(cur, id) };
            match found {
                Some(v) => {
                    val = v;
                    if is_class(v) {
                        cur = v as *mut Class;
                    }
                }
                None => return crate::abi::const_missing(cur, id),
            }
        }
        val
    }
    defn(md, "const_get", 1, 2, |s, a, _| match str_arg(a[0]) {
        Some(b) => const_get_path(s, &b),
        None => type_error("constant name expected"),
    });
    defn(md, "const_set", 2, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => {
            const_set(s as *mut Class, id, a[1]);
            if !crate::abi::const_added(s as *mut Class, id) {
                return UNDEF;
            }
            a[1]
        }
        Err(e) => e,
    });
    // Ruby's semantics: never calls const_missing (ActiveSupport::Dependencies relies on that
    // when it decides whether to search outer modules); a pending autoload counts as defined.
    fn const_defined_in(c: *mut Class, id: u32, inherit: bool) -> bool {
        let here = |k: *mut Class| unsafe { (*k).consts.contains_key(&id) } || crate::abi::has_autoload(k, id);
        if here(c) {
            return true;
        }
        if !inherit {
            return false;
        }
        if ancestors_rc(c).iter().any(|&a| here(a)) {
            return true;
        }
        // modules also see Object's constants
        (unsafe { (*c).is_module }) && ancestors_rc(rt().c.object).iter().any(|&a| here(a))
    }
    defn(md, "const_defined?", 1, 2, |s, a, _| match str_arg(a[0]) {
        Some(b) => {
            let inherit = a.len() < 2 || truthy(a[1]);
            let path = String::from_utf8_lossy(&b).into_owned();
            let parts: Vec<&str> = path.split("::").collect();
            let mut cur = s as *mut Class;
            for (i, part) in parts.iter().enumerate() {
                if part.is_empty() && i == 0 && parts.len() > 1 {
                    cur = rt().c.object;
                    continue;
                }
                if !part.chars().next().is_some_and(|c| c.is_uppercase()) || part.contains(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    return raise(rt().c.name_error, &format!("wrong constant name {path}"));
                }
                let id = sym(part);
                if !const_defined_in(cur, id, inherit) {
                    return QFALSE;
                }
                if i + 1 < parts.len() {
                    let saved = pst().pending;
                    let v = const_get_path(cur as V, part.as_bytes());
                    pst().pending = saved;
                    if v == UNDEF || !is_class(v) {
                        return QFALSE;
                    }
                    cur = v as *mut Class;
                }
            }
            QTRUE
        }
        None => QFALSE,
    });
    defn(md, "refine", 1, 1, |s, a, b| {
        if !is_class(a[0]) {
            return type_error("wrong argument type (expected Class or Module)");
        }
        let mut h = ivar_get(s, sym("@__refinements"));
        if !is_hash(h) {
            h = hash_new();
            ivar_set(s, sym("@__refinements"), h);
        }
        let r = match rhash(h).m.get(&HKey(a[0])) {
            Some(&r) => r,
            None => {
                let r = new_module(&format!("#<refinement:{}@{}>", class_name(a[0] as *mut Class), class_name(s as *mut Class)));
                unsafe { (*r).civars.insert(sym("@__refined"), a[0]) };
                crate::hash::hset(h, a[0], r as V);
                r as V
            }
        };
        if is_proc(b) {
            tri!(class_exec(r, &[r], b));
        }
        r
    });
    defn(md, "using", 1, 1, |s, _, _| s);
    defn(k, "using", 1, 1, |s, _, _| s);
    defn(md, "autoload", 2, 2, |s, a, _| match (sym_arg(a[0]), crate::fmt::to_s(a[1])) {
        (Ok(id), Ok(p)) => {
            if unsafe { (*(s as *mut Class)).consts.contains_key(&id) } {
                return QNIL;
            }
            crate::abi::register_autoload(s as *mut Class, id, String::from_utf8_lossy(&p).into_owned(), None, std::ptr::null_mut());
            if !crate::abi::const_added(s as *mut Class, id) {
                return UNDEF;
            }
            QNIL
        }
        (Err(e), _) => e,
        _ => UNDEF,
    });
    defn(md, "autoload?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => crate::abi::autoload_path(s as *mut Class, id).map(|p| str_from(&p)).unwrap_or(QNIL),
        Err(e) => e,
    });
    defn(k, "autoload", 2, 2, |_, a, _| send(rt().c.object as V, sym("autoload"), a, QNIL));
    defn(md, "constants", 0, 1, |s, _, _| ary_new(rclass(s).consts.keys().map(|&k| mksym(k)).collect()));
    defn(md, "const_missing", 1, 1, |s, a, _| {
        let prefix = if s as *mut Class == rt().c.object { String::new() } else { format!("{}::", class_name(s as *mut Class)) };
        raise(rt().c.name_error, &format!("uninitialized constant {}{}", prefix, sym_str(sym_id(a[0]))))
    });
    fn class_eval(s: V, a: &[V], b: V) -> V {
        if !is_proc(b) {
            let Some(&src) = a.first() else { return arg_error("wrong number of arguments (given 0, expected 1..3)") };
            // a new scope: `def` in the string is public even after a bare `private` in the
            // class body that called class_eval (as in CRuby, where visibility is per scope)
            let k = rclass(s);
            let (vis, mf) = (k.default_vis, k.module_function);
            k.default_vis = VIS_PUBLIC;
            k.module_function = false;
            let r = with_definee(s as *mut Class, || eval_string(src, 1, s));
            let k = rclass(s);
            k.default_vis = vis;
            k.module_function = mf;
            return r;
        }
        class_exec(s, &[s], b)
    }
    defn(md, "class_eval", 0, 3, class_eval);
    defn(md, "module_eval", 0, 3, class_eval);
    defn(md, "class_exec", 0, -1, class_exec);
    defn(md, "module_exec", 0, -1, class_exec);
    defn(md, "class_variable_get", 1, 1, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => crate::abi::cvar_get(s as *mut Class, id),
        Err(e) => e,
    });
    defn(md, "class_variable_set", 2, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => crate::abi::cvar_set(s as *mut Class, id, a[1]),
        Err(e) => e,
    });
    defn(md, "class_variable_defined?", 1, 1, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(ancestors(s as *mut Class).iter().any(|c| unsafe { (**c).cvars.contains_key(&id) })),
        Err(e) => e,
    });
    defn(md, "freeze", 0, 0, |s, _, _| s);
    // `Module.new { }`, and `Sub.new(args)` for subclasses of Module (ActiveModel's
    // `AcceptsMultiparameterTime < Module` defines `initialize(defaults: {})`).
    defs(md, "new", 0, -1, |s, a, b| {
        let m = new_module("");
        if is_class(s) && s != rt().c.module as V {
            hdr(m as V).klass = s as *mut Class;
        }
        let init = find_method(class_of(m as V), sym("initialize"));
        if !init.is_null() {
            tri!(invoke(init, m as V, a, b));
        }
        m as V
    });
    defn(md, "initialize", 0, 0, |s, _, b| {
        if is_proc(b) {
            tri!(class_exec(s, &[s], b));
        }
        QNIL
    });

    let cl = c.class;
    defn(cl, "new", 0, -1, class_new_instance);
    defn(cl, "allocate", 0, 0, |s, _, _| alloc_instance(s as *mut Class));
    defn(cl, "superclass", 0, 0, |s, _, _| {
        let mut k = rclass(s).superclass;
        unsafe {
            while !k.is_null() && ((*k).is_singleton || (*k).is_module) {
                k = (*k).superclass;
            }
        }
        if k.is_null() { QNIL } else { k as V }
    });
    defn(cl, "class", 0, 0, |_, _, _| rt().c.class as V);
    defs(cl, "new", 0, 1, |_, a, b| {
        let sup = a.first().copied().unwrap_or(rt().c.object as V);
        if !is_class(sup) {
            return type_error("superclass must be a Class");
        }
        let c = new_class("", sup as *mut Class);
        tri!(send(sup, sym("inherited"), &[c as V], QNIL));
        if is_proc(b) {
            tri!(class_exec(c as V, &[c as V], b));
        }
        c as V
    });

    // --- nil / true / false ---
    let n = c.nil;
    defn(n, "to_s", 0, 0, |_, _, _| str_new(vec![]));
    defn(n, "to_a", 0, 0, |_, _, _| ary_new(vec![]));
    defn(n, "to_h", 0, 0, |_, _, _| hash_new());
    defn(n, "to_i", 0, 0, |_, _, _| mkfix(0));
    defn(n, "to_f", 0, 0, |_, _, _| float_new(0.0));
    defn(n, "inspect", 0, 0, |_, _, _| str_from("nil"));
    defn(n, "&", 1, 1, |_, _, _| QFALSE);
    defn(n, "|", 1, 1, |_, a, _| bool_v(truthy(a[0])));
    for (cls, name) in [(c.true_, "true"), (c.false_, "false")] {
        defn(cls, "to_s", 0, 0, if name == "true" { |_, _, _| str_from("true") } else { |_, _, _| str_from("false") });
        defn(cls, "inspect", 0, 0, if name == "true" { |_, _, _| str_from("true") } else { |_, _, _| str_from("false") });
        defn(cls, "&", 1, 1, |s, a, _| bool_v(truthy(s) && truthy(a[0])));
        defn(cls, "|", 1, 1, |s, a, _| bool_v(truthy(s) || truthy(a[0])));
        defn(cls, "^", 1, 1, |s, a, _| bool_v(truthy(s) != truthy(a[0])));
    }

    // --- Comparable ---
    let cm = c.comparable;
    macro_rules! cmpop {
        ($name:expr, $f:expr) => {
            defn(cm, $name, 1, 1, |s, a, _| match cmp_int(s, a[0]) {
                Ok(Some(o)) => bool_v($f(o)),
                Ok(None) => cmp_fail(s, a[0]),
                Err(()) => UNDEF,
            });
        };
    }
    cmpop!("<", |o: i64| o < 0);
    cmpop!("<=", |o: i64| o <= 0);
    cmpop!(">", |o: i64| o > 0);
    cmpop!(">=", |o: i64| o >= 0);
    defn(cm, "==", 1, 1, |s, a, _| {
        if s == a[0] {
            return QTRUE;
        }
        match cmp_int(s, a[0]) {
            Ok(Some(o)) => bool_v(o == 0),
            Ok(None) => QFALSE,
            Err(()) => UNDEF,
        }
    });
    defn(cm, "between?", 2, 2, |s, a, _| {
        let lo = match cmp_int(s, a[0]) {
            Ok(Some(o)) => o,
            Ok(None) => return cmp_fail(s, a[0]),
            Err(()) => return UNDEF,
        };
        let hi = match cmp_int(s, a[1]) {
            Ok(Some(o)) => o,
            Ok(None) => return cmp_fail(s, a[1]),
            Err(()) => return UNDEF,
        };
        bool_v(lo >= 0 && hi <= 0)
    });
    defn(cm, "clamp", 1, 2, |s, a, _| {
        let (lo, hi) = if a.len() == 2 { (a[0], a[1]) } else if is_range(a[0]) { (rrange(a[0]).lo, rrange(a[0]).hi) } else { return type_error("wrong argument type") };
        if lo != QNIL {
            if let Ok(Some(o)) = cmp_int(s, lo) {
                if o < 0 {
                    return lo;
                }
            }
        }
        if hi != QNIL {
            if let Ok(Some(o)) = cmp_int(s, hi) {
                if o > 0 {
                    return hi;
                }
            }
        }
        s
    });

    // --- Exception ---
    let ex = c.exception;
    defn(ex, "initialize", 0, 1, |s, a, _| {
        ivar_set(s, sym("@__message"), a.first().copied().unwrap_or(QNIL));
        QNIL
    });
    fn exc_to_s(s: V) -> V {
        let m = ivar_get(s, sym("@__message"));
        if m == QNIL {
            return str_from(&class_name(real_class(s)));
        }
        match to_s(m) {
            Ok(b) => str_new(b),
            Err(()) => UNDEF,
        }
    }
    defn(ex, "to_s", 0, 0, |s, _, _| exc_to_s(s));
    defn(ex, "message", 0, 0, |s, _, _| send0(s, "to_s"));
    defn(ex, "detailed_message", 0, 1, |s, _, _| send0(s, "message"));
    defn(ex, "full_message", 0, 1, |s, _, _| {
        let m = tri!(send0(s, "message"));
        let m = String::from_utf8_lossy(&rstr(m).b).into_owned();
        new_str_v(format!("{} ({})", m, class_name(real_class(s))))
    });
    defn(ex, "inspect", 0, 0, |s, _, _| {
        let m = tri!(send0(s, "to_s"));
        let m = String::from_utf8_lossy(&rstr(m).b).into_owned();
        let cn = class_name(real_class(s));
        if m.is_empty() || m == cn { new_str_v(cn) } else { new_str_v(format!("#<{cn}: {m}>")) }
    });
    defn(ex, "backtrace", 0, 0, |s, _, _| crate::backtrace::exc_backtrace(s));
    defn(ex, "backtrace_locations", 0, 0, |s, _, _| {
        let raw = ivar_get(s, sym("@__bt_raw"));
        if raw == QNIL {
            return QNIL;
        }
        crate::backtrace::locations(&crate::backtrace::unpack(raw))
    });
    defn(ex, "set_backtrace", 1, 1, |s, a, _| ivar_set(s, sym("@__backtrace"), a[0]));
    defn(ex, "cause", 0, 0, |s, _, _| ivar_get(s, sym("@__cause")));
    defn(ex, "==", 1, 1, |s, a, _| bool_v(s == a[0] || (real_class(s) == real_class(a[0]) && is_heap(a[0]) && eql(ivar_get(s, sym("@__message")), ivar_get(a[0], sym("@__message"))))));
    defn(ex, "exception", 0, 1, |s, a, _| {
        if a.is_empty() {
            return s;
        }
        let e = dup(s, false);
        ivar_set(e, sym("@__message"), a[0]);
        e
    });
    defs(ex, "exception", 0, -1, |s, a, b| class_new_instance(s, a, b));
    defn(c.stop_iteration, "result", 0, 0, |s, _, _| ivar_get(s, sym("@__result")));
    defn(c.key_error, "key", 0, 0, |s, _, _| ivar_get(s, sym("@key")));
    defn(c.key_error, "receiver", 0, 0, |s, _, _| ivar_get(s, sym("@receiver")));
    let le = const_lookup(rt().c.object, sym("LoadError")).unwrap() as *mut Class;
    defn(le, "path", 0, 0, |s, _, _| ivar_get(s, sym("@path")));
    defn(c.name_error, "name", 0, 0, |s, _, _| ivar_get(s, sym("@name")));
    // NameError.new(msg = nil, name = nil, receiver: nil); NoMethodError adds args
    fn name_error_init(s: V, a: &[V]) -> V {
        let mut a = a.to_vec();
        if a.last().is_some_and(|x| is_hash(*x)) {
            let h = a.pop().unwrap();
            let r = crate::hash::hget(h, mksym(sym("receiver")));
            if r != UNDEF && r != QNIL {
                ivar_set(s, sym("@receiver"), r);
            }
        }
        ivar_set(s, sym("@__message"), a.first().copied().unwrap_or(QNIL));
        ivar_set(s, sym("@name"), a.get(1).copied().unwrap_or(QNIL));
        if a.len() > 2 {
            ivar_set(s, sym("@args"), a[2]);
        }
        QNIL
    }
    defn(c.name_error, "initialize", 0, -1, |s, a, _| name_error_init(s, a));
    defn(c.name_error, "receiver", 0, 0, |s, _, _| ivar_get(s, sym("@receiver")));
    defn(c.no_method_error, "args", 0, 0, |s, _, _| {
        let v = ivar_get(s, sym("@args"));
        if v == QNIL { ary_new(vec![]) } else { v }
    });
}

/// `Module#dup` / `Class#dup`: an anonymous copy with its own method table.
fn dup_module(v: V) -> V {
    let src = rclass(v);
    let c = if src.is_module { new_module("") } else { new_class("", src.superclass) };
    let k = unsafe { &mut *c };
    for (&id, &m) in src.methods.iter() {
        let mm = unsafe { &*m };
        if let Body::Compiled(f) = mm.body {
            crate::abi::share_owner_of(f);
        }
        let nm = Box::into_raw(Box::new(Method { fast: mm.fast, body: clone_body(&mm.body), owner: c, name: mm.name, params: mm.params, vis: mm.vis, opt: 0, gc_transparent: mm.gc_transparent, attr_slot: Default::default() }));
        if k.methods.insert(id, nm).is_none() {
            crate::core::method_name_count(id, true);
        }
        crate::core::touch_method_name(c, id);
    }
    k.includes.set(src.includes.clone());
    k.prepends.set(src.prepends.clone());
    k.consts = src.consts.clone();
    k.cvars = src.cvars.clone();
    k.ivar_index = src.ivar_index.clone();
    k.ivar_names.set(src.ivar_names.clone());
    k.alloc = src.alloc;
    k.lexical_parent = src.lexical_parent;
    k.struct_members.set(src.struct_members.clone());
    // like CRuby: the copy has the original's instance variables and singleton methods
    // (`def self.included`, `extend`ed modules)
    k.civars = src.civars.clone();
    let meta = src.h.klass;
    if !meta.is_null() && unsafe { (*meta).is_singleton && (*meta).attached == v } {
        let dm = singleton_class(c as V);
        let (sm, dk) = unsafe { (&*meta, &mut *dm) };
        for (&id, &m) in sm.methods.iter() {
            let mm = unsafe { &*m };
            if let Body::Compiled(f) = mm.body {
                crate::abi::share_owner_of(f);
            }
            let nm = Box::into_raw(Box::new(Method { fast: mm.fast, body: clone_body(&mm.body), owner: dm, name: mm.name, params: mm.params, vis: mm.vis, opt: 0, gc_transparent: mm.gc_transparent, attr_slot: Default::default() }));
            if dk.methods.insert(id, nm).is_none() {
                crate::core::method_name_count(id, true);
            }
            crate::core::touch_method_name(dm, id);
        }
        dk.includes.set(sm.includes.clone());
        dk.prepends.set(sm.prepends.clone());
    }
    bump_serial();
    c as V
}

pub fn clone_body(b: &Body) -> Body {
    match b {
        Body::Compiled(f) => Body::Compiled(*f),
        Body::Native(f, a, z) => Body::Native(*f, *a, *z),
        Body::AttrReader(i) => Body::AttrReader(*i),
        Body::AttrWriter(i) => Body::AttrWriter(*i),
        Body::Proc(p) => Body::Proc(*p),
        Body::CFunc(f, a) => Body::CFunc(*f, *a),
        Body::Undefined => Body::Undefined,
    }
}

