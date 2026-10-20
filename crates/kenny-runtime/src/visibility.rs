//! Method visibility: public / private / protected, and the reflection that depends on it.
//! Initialised last so it supersedes the visibility-blind versions elsewhere.

use crate::core::*;
use crate::kernel::sym_arg;
use crate::tri;

pub fn private_call_error(recv: V, mid: u32, args: &[V], blk: V) -> V {
    // like CRuby, method_missing gets a chance first
    let mm = find_method(class_of(recv), sym("method_missing"));
    if !mm.is_null() && !matches!(unsafe { &(*mm).body }, Body::Native(..)) {
        let mut a = vec![mksym(mid)];
        a.extend_from_slice(args);
        return invoke(mm, recv, &a, blk);
    }
    let e = exc_new(
        rt().c.no_method_error,
        &format!("private method '{}' called for {}", sym_str(mid), describe(recv)),
    );
    ivar_set(e, sym("@name"), mksym(mid));
    raise_exc(e)
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
            Some(m) => unsafe { (*(m as *mut Method)).vis = vis },
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
            }
        }
    }
    bump_serial();
    QNIL
}

fn vis_call(s: V, a: &[V], vis: u8) -> V {
    if a.is_empty() {
        rclass(s).default_vis = vis;
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
    let mut seen: Vec<u32> = Vec::new();
    let mut out = Vec::new();
    for k in classes {
        if inherit && k != c && (k == rt().c.object || k == rt().c.kernel || k == rt().c.basic_object) {
            continue;
        }
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
    defn(k, "private_methods", 0, 1, |s, _, _| list(class_of(s), true, &|v| v == VIS_PRIVATE));
    defn(k, "protected_methods", 0, 1, |s, _, _| list(class_of(s), true, &|v| v == VIS_PROTECTED));
    defn(k, "public_methods", 0, 1, |s, _, _| list(class_of(s), true, &|v| v == VIS_PUBLIC));

    // Kernel's "global functions" are private in Ruby (`obj.puts` is a NoMethodError).
    for name in [
        "puts", "print", "p", "pp", "printf", "putc", "gets", "require", "require_relative", "load", "raise", "fail",
        "loop", "lambda", "proc", "format", "sprintf", "sleep", "rand", "srand", "exit", "exit!", "abort", "at_exit",
        "catch", "throw", "binding", "block_given?", "caller", "caller_locations", "Integer", "Float", "String",
        "Array", "Hash", "Rational", "Complex", "system", "`", "exec", "fork", "spawn", "open", "eval", "warn",
        "trap", "autoload", "readline", "readlines", "select", "local_variables", "global_variables", "gem",
        "set_trace_func", "iterator?", "__method__", "__dir__",
    ] {
        let id = sym(name);
        if let Some(&m) = unsafe { (*k).methods.get(&id) } {
            unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
            // `Kernel.format(...)`, `Kernel.raise` etc. stay callable (module_function style)
            let ks = singleton_class(k as V);
            if unsafe { !(*ks).methods.contains_key(&id) } {
                let nm = add_method(ks, id, crate::kernel::clone_body(unsafe { &(*m).body }));
                unsafe { (*(nm as *mut Method)).vis = VIS_PUBLIC };
            }
        }
    }
    bump_serial();
}
