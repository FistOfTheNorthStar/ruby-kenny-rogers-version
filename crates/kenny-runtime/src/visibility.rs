//! Method visibility: public / private / protected, and the reflection that depends on it.
//! Initialised last so it supersedes the visibility-blind versions elsewhere.

use crate::core::*;
use crate::kernel::sym_arg;
use crate::tri;

pub fn private_call_error(recv: V, mid: u32, args: &[V], blk: V) -> V {
    vis_call_error(recv, mid, args, blk, "private")
}
pub fn protected_call_error(recv: V, mid: u32, args: &[V], blk: V) -> V {
    vis_call_error(recv, mid, args, blk, "protected")
}
thread_local! {
    /// Why method_missing was called ("private"/"protected" for a visibility violation), for
    /// the default method_missing's message (CRuby's method_missing_reason).
    pub static MM_REASON: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}
fn vis_call_error(recv: V, mid: u32, args: &[V], blk: V, what: &'static str) -> V {
    // like CRuby, method_missing gets a chance first
    let mm = find_method(class_of(recv), sym("method_missing"));
    if !mm.is_null() && !matches!(unsafe { &(*mm).body }, Body::Native(..)) {
        let mut a = vec![mksym(mid)];
        a.extend_from_slice(args);
        MM_REASON.with(|r| r.set(what));
        let r = invoke(mm, recv, &a, blk);
        MM_REASON.with(|r| r.set(""));
        return r;
    }
    vis_error(recv, mid, what)
}
pub fn vis_error(recv: V, mid: u32, what: &str) -> V {
    let e = exc_new(rt().c.no_method_error, &format!("{what} method '{}' called for {}", sym_str(mid), describe(recv)));
    ivar_set(e, sym("@name"), mksym(mid));
    ivar_set(e, sym("@receiver"), recv);
    raise_exc(e)
}

/// Whether the running code may call protected method `m` on a receiver: CRuby's rule is
/// `self.kind_of?(m's owner)`. Compiled call sites don't pass their self, so the caller is
/// told by its frame's label ("A#foo", "block in A#foo", "<class:A>", "<main>"): the call is
/// refused only when that caller's self can't be a kind of the owner.
pub fn protected_ok(m: *const Method) -> bool {
    let owner = unsafe { (*m).owner };
    let Some(label) = crate::backtrace::caller_label() else { return true };
    let mut l = label.as_str();
    if let Some(i) = l.find(" in ") {
        if l.starts_with("block") || l.starts_with("rescue") || l.starts_with("ensure") {
            l = &l[i + 4..];
        }
    }
    let lookup = |name: &str| -> Option<*mut Class> {
        let mut cur = rt().c.object;
        for part in name.split("::") {
            let v = const_get_under(cur, sym(part))?;
            if !is_class(v) {
                return None;
            }
            cur = v as *mut Class;
        }
        Some(cur)
    };
    let kind_of = |obj: V| is_a(obj, owner);
    if l == "<main>" || l == "<top (required)>" {
        return kind_of(rt().main_obj);
    }
    let obj_name = l.strip_prefix("<class:").or_else(|| l.strip_prefix("<module:")).and_then(|x| x.strip_suffix('>'));
    let (cls, singleton) = if let Some(n) = obj_name {
        (n, true)
    } else if let Some(i) = l.rfind('#') {
        (&l[..i], false)
    } else if let Some(i) = l.rfind('.') {
        (&l[..i], true)
    } else {
        return true;
    };
    let Some(c) = lookup(cls) else { return true };
    if singleton {
        return kind_of(c as V);
    }
    // an instance of c (or of a subclass of it): refused when c and the owner are unrelated
    // classes (a module's method can run on anything that includes it)
    if unsafe { (*c).is_module } {
        return true;
    }
    let anc = ancestors_rc(c);
    anc.contains(&owner) || ancestors_rc(owner).contains(&c)
}
fn describe(v: V) -> String {
    if v == QNIL {
        "nil".into()
    } else if is_class(v) {
        format!("class {}", class_name(v as *mut Class))
    } else {
        format!("an instance of {}", class_name(real_class(v)))
    }
}

fn names_of(a: &[V]) -> Result<Vec<u32>, V> {
    let mut out = Vec::new();
    for &x in a {
        if is_ary(x) {
            out.extend(names_of(&rary(x).v.clone())?);
        } else {
            out.push(sym_arg(x)?);
        }
    }
    Ok(out)
}

/// Set the visibility of `names` in class `c` (copying inherited methods into `c`, like CRuby).
fn set_vis(c: *mut Class, names: &[u32], vis: u8) -> V {
    for &id in names {
        let own = unsafe { (*c).methods.get(&id).copied() };
        match own {
            Some(m) => unsafe {
                crate::core::touch_method_name(c, id);
                (*(m as *mut Method)).vis = vis
            },
            None => {
                let m = find_method(c, id);
                if m.is_null() {
                    return raise(
                        rt().c.name_error,
                        &format!("undefined method '{}' for {} '{}'", sym_str(id), if unsafe { (*c).is_module } { "module" } else { "class" }, class_name(c)),
                    );
                }
                let body = crate::kernel::clone_body(unsafe { &(*m).body });
                let nm = add_method(c, id, body);
                unsafe {
                    (*(nm as *mut Method)).vis = vis;
                    (*(nm as *mut Method)).params = (*m).params;
                }
                // (CRuby's ZSUPER entry, not a method of its own: method coverage skips it)
                crate::coverage::vis_copy(nm);
            }
        }
    }
    bump_serial();
    QNIL
}

fn vis_call(s: V, a: &[V], vis: u8) -> V {
    if a.is_empty() {
        rclass(s).default_vis = vis;
        // (ends a `module_function` section too)
        rclass(s).module_function = false;
        return QNIL;
    }
    let names = match names_of(a) {
        Ok(n) => n,
        Err(e) => return e,
    };
    tri!(set_vis(s as *mut Class, &names, vis));
    if a.len() == 1 { a[0] } else { ary_new(a.to_vec()) }
}

fn method_vis(c: *mut Class, id: u32) -> Option<u8> {
    let m = find_method(c, id);
    if m.is_null() { None } else { Some(unsafe { (*m).vis }) }
}

/// Method names of `c` (and ancestors if `inherit`) whose visibility passes `want`.
fn list(c: *mut Class, inherit: bool, want: &dyn Fn(u8) -> bool) -> V {
    let classes = if inherit { ancestors(c) } else { vec![c] };
    list_in(&classes, want)
}

fn list_in(classes: &[*mut Class], want: &dyn Fn(u8) -> bool) -> V {
    let classes = classes.iter().copied();
    let mut seen: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    for k in classes {
        for (&id, &m) in unsafe { (*k).methods.iter() } {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let mm = unsafe { &*m };
            if matches!(mm.body, Body::Undefined) {
                continue;
            }
            if want(mm.vis) {
                out.push(mksym(id));
            }
        }
    }
    ary_new(out)
}

/// After every native class is set up: what CRuby keeps private on Object, Kernel and
/// BasicObject but the runtime defines elsewhere (or later than `init`), so that
/// `obj.methods` / `Foo.instance_methods` list what CRuby lists.
pub fn finish_boot() {
    let c = rt().c;
    let names: &[(*mut Class, &[&str])] = &[
        (
            c.kernel,
            &[
                "Rational", "Complex", "gem_original_require", "test", "ruby2_keywords", "using", "syscall", "trace_var",
                "untrace_var", "autoload?", "__callee__", "__kenny_openssl_init",
                // config.ru's Rack::Builder DSL (`run App` at the top level)
                "run", "use",
            ],
        ),
        (c.object, &["ruby2_keywords", "using"]),
        (c.basic_object, &["method_missing", "singleton_method_added", "singleton_method_removed", "singleton_method_undefined"]),
    ];
    for &(k, list) in names {
        for name in list {
            if let Some(&m) = unsafe { (*k).methods.get(&sym(name)) } {
                unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
            }
        }
    }
}

pub fn init() {
    let c = rt().c;
    let md = c.module;
    defn(md, "private", 0, -1, |s, a, _| vis_call(s, a, VIS_PRIVATE));
    defn(md, "public", 0, -1, |s, a, _| vis_call(s, a, VIS_PUBLIC));
    defn(md, "protected", 0, -1, |s, a, _| vis_call(s, a, VIS_PROTECTED));
    defn(md, "private_class_method", 0, -1, |s, a, _| match names_of(a) {
        Ok(n) => {
            tri!(set_vis(singleton_class(s), &n, VIS_PRIVATE));
            QNIL
        }
        Err(e) => e,
    });
    defn(md, "public_class_method", 0, -1, |s, a, _| match names_of(a) {
        Ok(n) => {
            tri!(set_vis(singleton_class(s), &n, VIS_PUBLIC));
            QNIL
        }
        Err(e) => e,
    });
    defn(md, "method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(method_vis(s as *mut Class, id).is_some_and(|v| v != VIS_PRIVATE)),
        Err(e) => e,
    });
    defn(md, "public_method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(method_vis(s as *mut Class, id) == Some(VIS_PUBLIC)),
        Err(e) => e,
    });
    defn(md, "private_method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(method_vis(s as *mut Class, id) == Some(VIS_PRIVATE)),
        Err(e) => e,
    });
    defn(md, "protected_method_defined?", 1, 2, |s, a, _| match sym_arg(a[0]) {
        Ok(id) => bool_v(method_vis(s as *mut Class, id) == Some(VIS_PROTECTED)),
        Err(e) => e,
    });
    fn inherit(a: &[V]) -> bool {
        a.first().is_none_or(|x| truthy(*x))
    }
    defn(md, "instance_methods", 0, 1, |s, a, _| list(s as *mut Class, inherit(a), &|v| v != VIS_PRIVATE));
    defn(md, "public_instance_methods", 0, 1, |s, a, _| list(s as *mut Class, inherit(a), &|v| v == VIS_PUBLIC));
    defn(md, "private_instance_methods", 0, 1, |s, a, _| list(s as *mut Class, inherit(a), &|v| v == VIS_PRIVATE));
    defn(md, "protected_instance_methods", 0, 1, |s, a, _| list(s as *mut Class, inherit(a), &|v| v == VIS_PROTECTED));

    let k = c.kernel;
    defn(k, "respond_to?", 1, 2, |s, a, _| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let include_all = a.get(1).is_some_and(|x| truthy(*x));
        match method_vis(class_of(s), id) {
            Some(VIS_PRIVATE) | Some(VIS_PROTECTED) if !include_all => QFALSE,
            Some(_) => QTRUE,
            None => {
                let m = find_method(class_of(s), sym("respond_to_missing?"));
                if !m.is_null() && !matches!(unsafe { &(*m).body }, Body::Native(..)) {
                    return send(s, sym("respond_to_missing?"), &[mksym(id), bool_v(include_all)], QNIL);
                }
                QFALSE
            }
        }
    });
    defn(k, "public_send", 1, -1, |s, a, b| {
        let id = match sym_arg(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        if method_vis(class_of(s), id).is_some_and(|v| v != VIS_PUBLIC) {
            return private_call_error(s, id, &a[1..], b);
        }
        send(s, id, &a[1..], b)
    });
    // (all = false: the methods of the object's singleton class and its class, not of the
    // ancestors above)
    fn obj_list(s: V, a: &[V], want: &dyn Fn(u8) -> bool) -> V {
        if a.first().is_none_or(|x| truthy(*x)) {
            return list(class_of(s), true, want);
        }
        let real = real_class(s);
        let mut own = Vec::new();
        for k in ancestors(class_of(s)) {
            own.push(k);
            if k == real {
                break;
            }
        }
        list_in(&own, want)
    }
    defn(k, "private_methods", 0, 1, |s, a, _| obj_list(s, a, &|v| v == VIS_PRIVATE));
    defn(k, "protected_methods", 0, 1, |s, a, _| obj_list(s, a, &|v| v == VIS_PROTECTED));
    defn(k, "public_methods", 0, 1, |s, a, _| obj_list(s, a, &|v| v == VIS_PUBLIC));
    // public and protected, as in CRuby (`methods(false)`: the singleton methods)
    defn(k, "methods", 0, 1, |s, a, _| {
        if a.first().is_some_and(|x| !truthy(*x)) {
            return send0(s, "singleton_methods");
        }
        list(class_of(s), true, &|v| v != VIS_PRIVATE)
    });

    // Kernel's "global functions" are private in Ruby (`obj.puts` is a NoMethodError).
    for name in [
        "puts", "print", "p", "pp", "printf", "putc", "gets", "require", "require_relative", "load", "raise", "fail",
        "loop", "lambda", "proc", "format", "sprintf", "sleep", "rand", "srand", "exit", "exit!", "abort", "at_exit",
        "catch", "throw", "binding", "block_given?", "caller", "caller_locations", "Integer", "Float", "String",
        "Array", "Hash", "Rational", "Complex", "system", "`", "exec", "fork", "spawn", "open", "eval", "warn",
        "trap", "autoload", "readline", "readlines", "select", "local_variables", "global_variables", "gem",
        "set_trace_func", "iterator?", "__method__", "__dir__", "BigDecimal", "__kenny_require_relative",
    ] {
        global_function(name);
    }
    bump_serial();
}

/// Kernel#`name` made a private "global function", callable as `Kernel.name` too (CRuby's
/// rb_define_global_function). For the ones a library defines when it loads (BigDecimal).
pub fn global_function(name: &str) {
    let k = rt().c.kernel;
    let id = sym(name);
    if let Some(&m) = unsafe { (*k).methods.get(&id) } {
        unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
        // `Kernel.format(...)`, `Kernel.raise` etc. stay callable (module_function style)
        let ks = singleton_class(k as V);
        if unsafe { !(*ks).methods.contains_key(&id) } {
            let nm = add_method(ks, id, crate::kernel::clone_body(unsafe { &(*m).body }));
            unsafe { (*(nm as *mut Method)).vis = VIS_PUBLIC };
        }
        bump_serial();
    }
}
