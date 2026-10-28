//! Method and UnboundMethod objects (`obj.method(:x)`, `Klass.instance_method(:x)`).

use crate::core::*;
use crate::kernel::{new_str_v, sym_arg};
use crate::tri;

static METHOD: G<*mut Class> = G::new(std::ptr::null_mut());
static UNBOUND: G<*mut Class> = G::new(std::ptr::null_mut());

/// [file, line] of compiled code starting at `addr`, or nil.
fn code_location(addr: usize) -> V {
    match crate::backtrace::def_location(addr) {
        Some((file, line)) => ary_new(vec![str_from(&file), mkfix(line)]),
        None => QNIL,
    }
}

/// `Proc#source_location`: where its block starts (nil for a native block).
pub fn proc_location(p: V) -> V {
    if !is_proc(p) {
        return QNIL;
    }
    let pr = unsafe { &*(p as *const RProc) };
    if !pr.native.is_null() {
        return QNIL;
    }
    code_location(pr.func as usize)
}

/// `Method#source_location`: where its `def` (or `define_method` block) is; nil for a
/// method built into Kenny.
pub fn method_location(m: *const Method) -> V {
    if m.is_null() {
        return QNIL;
    }
    let m = unsafe { &*m };
    match &m.body {
        Body::Compiled(f) => {
            let r = code_location(*f as usize);
            if r != QNIL || m.fast == 0 {
                return r;
            }
            // (a method with a fast entry: the body is that function; `f` wraps it)
            code_location(m.fast & ((1 << 56) - 1))
        }
        Body::Proc(p) => proc_location(*p),
        _ => QNIL,
    }
}

fn mptr(v: V) -> *const Method {
    fix(ivar_get(v, sym("@__m"))) as usize as *const Method
}
/// `Method#super_method` / `UnboundMethod#super_method`: the method `super` would call from
/// this one (the next definition after its owner in the receiver's ancestors), or nil.
fn super_method(s: V) -> V {
    let m = mptr(s);
    if m.is_null() {
        return QNIL;
    }
    let owner = unsafe { (*m).owner };
    let name = unsafe { (*m).name };
    let bound = real_class(s) == *METHOD.get();
    let recv = ivar_get(s, sym("@__recv"));
    let anc = if bound { ancestors(class_of(recv)) } else { ancestors(owner) };
    let Some(pos) = anc.iter().position(|&k| k == owner) else { return QNIL };
    for &k in &anc[pos + 1..] {
        if let Some(&sm) = unsafe { (*k).methods.get(&name) } {
            if matches!(unsafe { &(*sm).body }, Body::Undefined) {
                return QNIL;
            }
            return wrap(if bound { *METHOD.get() } else { *UNBOUND.get() }, if bound { recv } else { QNIL }, sm, sym_id(ivar_get(s, sym("@__name"))));
        }
    }
    QNIL
}

fn wrap(class: *mut Class, recv: V, m: *const Method, name: u32) -> V {
    let o = object_new(class);
    ivar_set(o, sym("@__m"), mkfix(m as i64));
    ivar_set(o, sym("@__recv"), recv);
    ivar_set(o, sym("@__name"), mksym(name));
    if !m.is_null() {
        // keeps the method's class (and so the method and its code) alive: a singleton class
        // is freed once nothing reaches it
        ivar_set(o, sym("@__owner"), unsafe { (*m).owner } as V);
        crate::gc::note_method_object(m);
    }
    o
}

/// Parameters from a descriptor ("req:a,opt:b,rest:c,..."); `proc_style`: required ones
/// are optional (a proc that isn't a lambda).
pub fn desc_parameters(d: &str, proc_style: bool) -> V {
    let mut out = Vec::new();
    for part in d.split(',').filter(|p| !p.is_empty()) {
        let (kind, name) = part.split_once(':').unwrap_or((part, ""));
        let kind = if proc_style && kind == "req" { "opt" } else { kind };
        let mut e = vec![mksym(sym(kind))];
        if !name.is_empty() && !matches!(name, "*" | "**" | "&") {
            e.push(mksym(sym(name)));
        } else if proc_style && kind == "opt" && !crate::misc::ruby4() {
            e.push(QNIL); // a destructuring or `it` parameter: [:opt, nil] (Ruby 4.0: [:opt])
        }
        out.push(ary_new(e));
    }
    ary_new(out)
}

/// Block functions' parameter descriptors (compiled code registers them at startup).
static BLOCK_PARAMS: std::sync::RwLock<Option<std::collections::HashMap<usize, usize>>> = std::sync::RwLock::new(None);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_block_params(table: *const u64, n: u64, strs: *const u8) {
    let mut t = BLOCK_PARAMS.write().unwrap();
    let m = t.get_or_insert_with(std::collections::HashMap::new);
    for i in 0..n as usize {
        let (f, off) = unsafe { (*table.add(2 * i), *table.add(2 * i + 1)) };
        m.insert(f as usize, strs as usize + off as usize);
    }
}

/// A compiled block's descriptor.
pub fn block_desc(p: V) -> Option<String> {
    let pr = rproc(p);
    if !pr.native.is_null() {
        return None;
    }
    let t = BLOCK_PARAMS.read().unwrap();
    let a = *t.as_ref()?.get(&(pr.func as usize))?;
    Some(unsafe { std::ffi::CStr::from_ptr(a as *const std::ffi::c_char) }.to_string_lossy().into_owned())
}

/// Proc#parameters (`lambda`: as for a lambda, or as for a proc; else as the proc is).
pub fn proc_parameters(p: V, lambda: Option<bool>) -> V {
    let pr = rproc(p);
    let as_lambda = lambda.unwrap_or(pr.lambda);
    if let Some(d) = block_desc(p) {
        return desc_parameters(&d, !as_lambda);
    }
    if pr.native.is_null() && pr.min == 0 && pr.max == 0 {
        return ary_new(vec![]);
    }
    // a native proc (a Method's or Symbol's to_proc, say): from the arity
    let mut out = Vec::new();
    for _ in 0..pr.min.max(0) {
        out.push(ary_new(vec![mksym(sym(if as_lambda { "req" } else { "opt" }))]));
    }
    if pr.max < 0 {
        out.push(ary_new(vec![mksym(sym("rest"))]));
    }
    ary_new(out)
}

/// [[:req, :a], [:opt, :b], ...] from a compiled method's descriptor, or from arity.
pub fn parameters(m: *const Method) -> V {
    if m.is_null() {
        return ary_new(vec![ary_new(vec![mksym(sym("rest"))])]);
    }
    let mm = unsafe { &*m };
    if !mm.params.is_null() {
        let d = unsafe { std::ffi::CStr::from_ptr(mm.params as *const std::ffi::c_char) }.to_string_lossy().into_owned();
        return desc_parameters(&d, false);
    }
    if let Body::Proc(p) = &mm.body {
        // define_method: the block's parameters, as for a lambda
        return proc_parameters(*p, Some(true));
    }
    let a = arity(m);
    let mut out = Vec::new();
    let req = if a < 0 { -a - 1 } else { a };
    for _ in 0..req {
        out.push(ary_new(vec![mksym(sym("req"))]));
    }
    if a < 0 {
        out.push(ary_new(vec![mksym(sym("rest"))]));
    }
    ary_new(out)
}

/// Arity from a parameter descriptor, as CRuby counts it: at least the required positional
/// parameters (and one more for required keywords), at most all positional ones (and one for
/// any keywords), unlimited with a rest parameter. `lambda`: method or lambda rules (fixed
/// only when min == max); a proc's is fixed unless it takes a rest parameter.
pub fn desc_arity(d: &str, lambda: bool) -> i64 {
    let (mut lead, mut opt, mut rest, mut kw, mut keyreq) = (0i64, 0i64, false, false, false);
    for part in d.split(',').filter(|p| !p.is_empty()) {
        match part.split(':').next().unwrap_or("") {
            "req" => lead += 1,
            "opt" => opt += 1,
            "rest" => rest = true,
            "keyreq" => {
                kw = true;
                keyreq = true;
            }
            "key" | "keyrest" => kw = true,
            _ => {}
        }
    }
    let min = lead + keyreq as i64;
    let max = if rest { None } else { Some(lead + opt + kw as i64) };
    let fixed = if lambda { max == Some(min) } else { max.is_some() };
    if fixed { min } else { -min - 1 }
}

pub fn arity(m: *const Method) -> i64 {
    if m.is_null() {
        return -1;
    }
    let mm = unsafe { &*m };
    match &mm.body {
        Body::Native(_, min, max) => {
            if min == max { *min as i64 } else { -(*min as i64) - 1 }
        }
        Body::AttrReader(_) => 0,
        Body::AttrWriter(_) => 1,
        Body::CFunc(_, a) => *a as i64,
        Body::Proc(p) => {
            // (define_method: the block's parameters, with lambda rules)
            if let Some(d) = block_desc(*p) {
                return desc_arity(&d, true);
            }
            let pr = rproc(*p);
            if pr.max < 0 || pr.min != pr.max { -(pr.min as i64) - 1 } else { pr.min as i64 }
        }
        Body::Compiled(_) => {
            if mm.params.is_null() {
                return -1;
            }
            let d = unsafe { std::ffi::CStr::from_ptr(mm.params as *const std::ffi::c_char) }.to_string_lossy().into_owned();
            desc_arity(&d, true)
        }
        Body::Undefined => 0,
    }
}

fn call(s: V, a: &[V], b: V) -> V {
    let m = mptr(s);
    let recv = ivar_get(s, sym("@__recv"));
    if m.is_null() {
        return send(recv, sym_id(ivar_get(s, sym("@__name"))), a, b);
    }
    invoke(m, recv, a, b)
}

pub fn method_object(recv: V, name: u32) -> V {
    let m = find_method(class_of(recv), name);
    if m.is_null() {
        let rm = send(recv, sym("respond_to_missing?"), &[mksym(name), QTRUE], QNIL);
        if !truthy(rm) {
            return raise(
                rt().c.name_error,
                &format!("undefined method '{}' for {}", sym_str(name), crate::fmt::inspect_lossy(recv)),
            );
        }
    }
    wrap(*METHOD.get(), recv, m, name)
}

fn describe(s: V, bound: bool) -> String {
    let m = mptr(s);
    let owner = if m.is_null() { "?".into() } else { class_name(unsafe { (*m).owner }) };
    let name = sym_str(sym_id(ivar_get(s, sym("@__name"))));
    let kind = if bound { "Method" } else { "UnboundMethod" };
    format!("#<{kind}: {owner}#{name}>")
}

/// A define_method body from a Method (its to_proc) or an UnboundMethod (called on the new
/// method's receiver); other values are returned unchanged.
pub fn method_body(v: V) -> V {
    if !is_kind(v, Kind::Object) {
        return v;
    }
    let k = real_class(v);
    if k == *METHOD.get() {
        return send0(v, "to_proc");
    }
    if k == *UNBOUND.get() {
        let um = v;
        let m = mptr(v);
        let name = sym_id(ivar_get(v, sym("@__name")));
        let ar = arity(m);
        let p = proc_new_native_rooted(
            Box::new(move |slf, a, b| {
                let _ = um;
                if m.is_null() { send(slf, name, a, b) } else { invoke(m, slf, a, b) }
            }),
            0,
            -1,
            vec![v],
        );
        let pr = rproc(p);
        pr.lambda = true;
        pr.min = if ar < 0 { (-ar - 1) as i32 } else { ar as i32 };
        pr.max = if ar < 0 { -1 } else { ar as i32 };
        return p;
    }
    v
}

pub fn init() {
    let o = rt().c.object;
    let mc = new_class("Method", o);
    set_global_const("Method", mc as V);
    *METHOD.get() = mc;
    let uc = new_class("UnboundMethod", o);
    set_global_const("UnboundMethod", uc as V);
    *UNBOUND.get() = uc;

    let k = rt().c.kernel;
    for name in ["method", "public_method"] {
        defn(k, name, 1, 1, |s, a, _| match sym_arg(a[0]) {
            Ok(id) => method_object(s, id),
            Err(e) => e,
        });
    }
    defn(k, "singleton_method", 1, 1, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => {
            let sc = class_of(s);
            match unsafe { (*sc).methods.get(&id) } {
                Some(&m) if unsafe { (*sc).is_singleton } => wrap(*METHOD.get(), s, m, id),
                _ => raise(rt().c.name_error, &format!("undefined singleton method '{}' for {}", sym_str(id), crate::fmt::inspect_lossy(s))),
            }
        }
        Err(e) => e,
    });
    let md = rt().c.module;
    for name in ["instance_method", "public_instance_method"] {
        defn(md, name, 1, 1, |s, a, _| match sym_arg(a[0]) {
            Ok(id) => {
                let m = find_method(s as *mut Class, id);
                if m.is_null() {
                    return raise(rt().c.name_error, &format!("undefined method '{}' for class '{}'", sym_str(id), class_name(s as *mut Class)));
                }
                wrap(*UNBOUND.get(), QNIL, m, id)
            }
            Err(e) => e,
        });
    }

    for name in ["call", "()", "[]", "==="] {
        defn(mc, name, 0, -1, |s, a, b| call(s, a, b));
    }
    defn(mc, "to_proc", 0, 0, |s, _, _| {
        let me = s;
        let p = proc_new_native_rooted(Box::new(move |_, a, b| call(me, a, b)), 0, -1, vec![s]);
        let pr = rproc(p);
        pr.lambda = true;
        let ar = arity(mptr(s));
        pr.min = if ar < 0 { (-ar - 1) as i32 } else { ar as i32 };
        pr.max = if ar < 0 { -1 } else { ar as i32 };
        p
    });
    defn(mc, "receiver", 0, 0, |s, _, _| ivar_get(s, sym("@__recv")));
    defn(mc, "unbind", 0, 0, |s, _, _| wrap(*UNBOUND.get(), QNIL, mptr(s), sym_id(ivar_get(s, sym("@__name")))));
    defn(mc, "curry", 0, 1, |s, a, _| {
        let p = tri!(send0(s, "to_proc"));
        send(p, sym("curry"), a, QNIL)
    });
    defn(mc, "inspect", 0, 0, |s, _, _| new_str_v(describe(s, true)));
    defn(mc, "to_s", 0, 0, |s, _, _| new_str_v(describe(s, true)));
    defn(mc, "==", 1, 1, |s, a, _| {
        bool_v(is_kind(a[0], Kind::Object) && real_class(a[0]) == real_class(s) && mptr(s) == mptr(a[0]) && ivar_get(s, sym("@__recv")) == ivar_get(a[0], sym("@__recv")))
    });
    defn(uc, "bind", 1, 1, |s, a, _| wrap(*METHOD.get(), a[0], mptr(s), sym_id(ivar_get(s, sym("@__name")))));
    defn(uc, "bind_call", 1, -1, |s, a, b| {
        let m = mptr(s);
        if m.is_null() {
            return send(a[0], sym_id(ivar_get(s, sym("@__name"))), &a[1..], b);
        }
        invoke(m, a[0], &a[1..], b)
    });
    defn(uc, "inspect", 0, 0, |s, _, _| new_str_v(describe(s, false)));
    defn(uc, "to_s", 0, 0, |s, _, _| new_str_v(describe(s, false)));
    defn(uc, "==", 1, 1, |s, a, _| bool_v(is_kind(a[0], Kind::Object) && real_class(a[0]) == real_class(s) && mptr(s) == mptr(a[0])));
    for c in [mc, uc] {
        defn(c, "name", 0, 0, |s, _, _| ivar_get(s, sym("@__name")));
        defn(c, "owner", 0, 0, |s, _, _| {
            let m = mptr(s);
            if m.is_null() { real_class(ivar_get(s, sym("@__recv"))) as V } else { unsafe { (*m).owner as V } }
        });
        defn(c, "arity", 0, 0, |s, _, _| mkfix(arity(mptr(s))));
        defn(c, "parameters", 0, 0, |s, _, _| parameters(mptr(s)));
        defn(c, "source_location", 0, 0, |s, _, _| method_location(mptr(s)));
        defn(c, "super_method", 0, 0, |s, _, _| super_method(s));
        defn(c, "original_name", 0, 0, |s, _, _| {
            let m = mptr(s);
            if m.is_null() { ivar_get(s, sym("@__name")) } else { mksym(unsafe { (*m).name }) }
        });
        defn(c, "hash", 0, 0, |s, _, _| mkfix(mptr(s) as i64 >> 3));
    }
}
