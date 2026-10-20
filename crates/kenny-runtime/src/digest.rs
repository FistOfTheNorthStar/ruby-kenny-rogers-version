//! Digest (the `digest` C extension): Digest::Instance / Digest::Class / Digest::Base and
//! MD5, SHA1, SHA256, SHA384, SHA512 backed by the RustCrypto hashes.
//!
//! Digest::Instance is generic like CRuby's: it only relies on `update`, `finish`, `reset`
//! and `clone`, so Ruby subclasses of Digest::Class (e.g. stdlib Digest::SHA2) work.
//! Digest::Base instances buffer their input in `@__buf` and hash it on `finish`.

use crate::compat::FL_BINARY;
use crate::core::*;
use crate::tri;
use sha2::Digest as _;

#[derive(Clone, Copy, PartialEq)]
enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}
const ALGOS: [(&str, Algo); 5] =
    [("MD5", Algo::Md5), ("SHA1", Algo::Sha1), ("SHA256", Algo::Sha256), ("SHA384", Algo::Sha384), ("SHA512", Algo::Sha512)];

static CLASSES: G<Vec<(*mut Class, Algo)>> = G::new(Vec::new());
static INSTANCE: G<*mut Class> = G::new(std::ptr::null_mut());

impl Algo {
    fn hash(self, b: &[u8]) -> Vec<u8> {
        match self {
            Algo::Md5 => md5::Md5::digest(b).to_vec(),
            Algo::Sha1 => sha1::Sha1::digest(b).to_vec(),
            Algo::Sha256 => sha2::Sha256::digest(b).to_vec(),
            Algo::Sha384 => sha2::Sha384::digest(b).to_vec(),
            Algo::Sha512 => sha2::Sha512::digest(b).to_vec(),
        }
    }
    fn digest_len(self) -> i64 {
        match self {
            Algo::Md5 => 16,
            Algo::Sha1 => 20,
            Algo::Sha256 => 32,
            Algo::Sha384 => 48,
            Algo::Sha512 => 64,
        }
    }
    fn block_len(self) -> i64 {
        match self {
            Algo::Sha384 | Algo::Sha512 => 128,
            _ => 64,
        }
    }
}

fn algo_of(s: V) -> Option<Algo> {
    let mut c = real_class(s);
    while !c.is_null() {
        if let Some((_, a)) = CLASSES.get().iter().find(|(k, _)| *k == c) {
            return Some(*a);
        }
        c = unsafe { (*c).superclass };
    }
    None
}

pub fn hexencode(b: &[u8]) -> Vec<u8> {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut o = Vec::with_capacity(b.len() * 2);
    for &x in b {
        o.push(H[(x >> 4) as usize]);
        o.push(H[(x & 15) as usize]);
    }
    o
}
pub fn base64(b: &[u8]) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = Vec::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63]);
        o.push(T[(n >> 12) as usize & 63]);
        o.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] } else { b'=' });
        o.push(if ch.len() > 2 { T[n as usize & 63] } else { b'=' });
    }
    o
}
fn bin_str(b: Vec<u8>) -> V {
    let s = str_new(b);
    hdr(s).flags |= FL_BINARY;
    s
}
fn str_bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        Ok(rstr(v).b.clone())
    } else {
        Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
    }
}

// ---------- Digest::Instance (generic, via update/finish/reset) ----------

fn unimpl(s: V, m: &str) -> V {
    raise(rt().c.runtime_error, &format!("{} does not implement {}()", class_name(real_class(s)), m))
}
fn finish_of(s: V) -> V {
    let r = tri!(send0(s, "finish"));
    if !is_str(r) {
        return type_error("digest must be a String");
    }
    r
}
/// `digest(str = nil)`: with a string, reset+update+finish+reset; without, finish a clone.
fn inst_digest(s: V, a: &[V]) -> V {
    match a.first() {
        Some(&x) if x != QNIL => {
            tri!(send0(s, "reset"));
            tri!(send1(s, "update", x));
            let r = tri!(finish_of(s));
            tri!(send0(s, "reset"));
            r
        }
        _ => {
            let c = tri!(send0(s, "clone"));
            finish_of(c)
        }
    }
}
fn inst_digest_bang(s: V) -> V {
    let r = tri!(finish_of(s));
    tri!(send0(s, "reset"));
    r
}
fn hex_of(v: V) -> V {
    if v == UNDEF { UNDEF } else { str_new(hexencode(&rstr(v).b)) }
}
fn b64_of(v: V) -> V {
    if v == UNDEF { UNDEF } else { str_new(base64(&rstr(v).b)) }
}

fn init_instance(inst: *mut Class) {
    defn(inst, "update", 1, 1, |s, _, _| unimpl(s, "update"));
    defn(inst, "<<", 1, 1, |s, a, _| send1(s, "update", a[0]));
    defn(inst, "finish", 0, 0, |s, _, _| unimpl(s, "finish"));
    defn(inst, "reset", 0, 0, |s, _, _| unimpl(s, "reset"));
    defn(inst, "block_length", 0, 0, |s, _, _| unimpl(s, "block_length"));
    defn(inst, "new", 0, 0, |s, _, _| {
        let c = tri!(send0(s, "clone"));
        tri!(send0(c, "reset"));
        c
    });
    defn(inst, "digest", 0, 1, |s, a, _| inst_digest(s, a));
    defn(inst, "digest!", 0, 0, |s, _, _| inst_digest_bang(s));
    defn(inst, "hexdigest", 0, 1, |s, a, _| hex_of(inst_digest(s, a)));
    defn(inst, "hexdigest!", 0, 0, |s, _, _| hex_of(inst_digest_bang(s)));
    defn(inst, "to_s", 0, 0, |s, _, _| hex_of(inst_digest(s, &[])));
    defn(inst, "base64digest", 0, 1, |s, a, _| b64_of(inst_digest(s, a)));
    defn(inst, "base64digest!", 0, 0, |s, _, _| b64_of(inst_digest_bang(s)));
    defn(inst, "inspect", 0, 0, |s, _, _| {
        let h = tri!(hex_of(inst_digest(s, &[])));
        crate::kernel::new_str_v(format!("#<{}: {}>", class_name(real_class(s)), String::from_utf8_lossy(&rstr(h).b)))
    });
    defn(inst, "==", 1, 1, |s, a, _| {
        let o = a[0];
        if is_a(o, *INSTANCE.get()) {
            let x = tri!(inst_digest(s, &[]));
            let y = tri!(inst_digest(o, &[]));
            return bool_v(rstr(x).b == rstr(y).b);
        }
        if !is_str(o) {
            return QFALSE;
        }
        let x = tri!(hex_of(inst_digest(s, &[])));
        bool_v(rstr(x).b == rstr(o).b)
    });
    fn len(s: V) -> V {
        let d = tri!(inst_digest(s, &[]));
        mkfix(rstr(d).b.len() as i64)
    }
    defn(inst, "digest_length", 0, 0, |s, _, _| len(s));
    defn(inst, "length", 0, 0, |s, _, _| len(s));
    defn(inst, "size", 0, 0, |s, _, _| len(s));
    defn(inst, "file", 1, 1, |s, a, _| {
        let path = match crate::fmt::to_s(a[0]) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(()) => return UNDEF,
        };
        match std::fs::read(&path) {
            Ok(b) => {
                tri!(send1(s, "update", bin_str(b)));
                s
            }
            Err(_) => {
                let errno = const_lookup(rt().c.object, sym("Errno")).unwrap() as *mut Class;
                let enoent = const_get_under(errno, sym("ENOENT")).unwrap() as *mut Class;
                raise(enoent, &format!("No such file or directory @ rb_sysopen - {path}"))
            }
        }
    });
}

// ---------- Digest::Base ----------

fn buf(s: V) -> V {
    let b = ivar_get(s, sym("@__buf"));
    if is_str(b) {
        return b;
    }
    let n = bin_str(Vec::new());
    ivar_set(s, sym("@__buf"), n);
    n
}
fn base_algo(s: V) -> Result<Algo, V> {
    algo_of(s).ok_or_else(|| raise(rt().c.not_implemented_error, "Digest::Base is an abstract class"))
}

fn init_base(base: *mut Class) {
    fn update(s: V, a: &[V]) -> V {
        if let Err(e) = base_algo(s) {
            return e;
        }
        let add = match str_bytes(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        rstr(buf(s)).b.extend_from_slice(&add);
        s
    }
    defn(base, "update", 1, 1, |s, a, _| update(s, a));
    defn(base, "<<", 1, 1, |s, a, _| update(s, a));
    defn(base, "finish", 0, 0, |s, _, _| match base_algo(s) {
        Ok(al) => bin_str(al.hash(&rstr(buf(s)).b)),
        Err(e) => e,
    });
    defn(base, "reset", 0, 0, |s, _, _| {
        if let Err(e) = base_algo(s) {
            return e;
        }
        rstr(buf(s)).b.clear();
        s
    });
    // fast paths (no clone)
    defn(base, "digest", 0, 1, |s, a, _| match (a.first(), base_algo(s)) {
        (_, Err(e)) => e,
        (Some(&x), Ok(al)) if x != QNIL => match str_bytes(x) {
            Ok(b) => {
                rstr(buf(s)).b.clear();
                bin_str(al.hash(&b))
            }
            Err(e) => e,
        },
        (_, Ok(al)) => bin_str(al.hash(&rstr(buf(s)).b)),
    });
    defn(base, "digest_length", 0, 0, |s, _, _| match base_algo(s) {
        Ok(al) => mkfix(al.digest_len()),
        Err(e) => e,
    });
    defn(base, "length", 0, 0, |s, _, _| match base_algo(s) {
        Ok(al) => mkfix(al.digest_len()),
        Err(e) => e,
    });
    defn(base, "size", 0, 0, |s, _, _| match base_algo(s) {
        Ok(al) => mkfix(al.digest_len()),
        Err(e) => e,
    });
    defn(base, "block_length", 0, 0, |s, _, _| match base_algo(s) {
        Ok(al) => mkfix(al.block_len()),
        Err(e) => e,
    });
    // dup/clone only call a non-native initialize_copy: register it as a native proc body
    let p = proc_new_native(
        Box::new(|s, a, _| {
            let src = ivar_get(a[0], sym("@__buf"));
            let b = if is_str(src) { rstr(src).b.clone() } else { Vec::new() };
            ivar_set(s, sym("@__buf"), bin_str(b));
            s
        }),
        1,
        1,
    );
    add_method(base, sym("initialize_copy"), Body::Proc(p));
}

fn init_digest() {
    let r = rt();
    let dg = new_module("Digest");
    set_global_const("Digest", dg as V);
    const_set(dg, sym("VERSION"), str_from("3.2.0"));
    defs(dg, "hexencode", 1, 1, |_, a, _| match str_bytes(a[0]) {
        Ok(b) => str_new(hexencode(&b)),
        Err(e) => e,
    });
    let inst = new_module("Digest::Instance");
    const_set(dg, sym("Instance"), inst as V);
    *INSTANCE.get() = inst;
    init_instance(inst);

    let class = new_class("Digest::Class", r.c.object);
    const_set(dg, sym("Class"), class as V);
    crate::kernel::do_include(class, inst as V);
    defs(class, "digest", 1, -1, |s, a, _| {
        let o = tri!(send(s, sym("new"), &a[1..], QNIL));
        send1(o, "digest", a[0])
    });
    defs(class, "hexdigest", 1, -1, |s, a, _| {
        let o = tri!(send(s, sym("new"), &a[1..], QNIL));
        send1(o, "hexdigest", a[0])
    });
    defs(class, "base64digest", 1, -1, |s, a, _| {
        let o = tri!(send(s, sym("new"), &a[1..], QNIL));
        send1(o, "base64digest", a[0])
    });
    defs(class, "file", 1, -1, |s, a, _| {
        let o = tri!(send(s, sym("new"), &a[1..], QNIL));
        send1(o, "file", a[0])
    });

    let base = new_class("Digest::Base", class);
    const_set(dg, sym("Base"), base as V);
    init_base(base);
    for (name, al) in ALGOS {
        let k = new_class(&format!("Digest::{name}"), base);
        const_set(dg, sym(name), k as V);
        CLASSES.get().push((k, al));
    }
    // Digest(:SHA256) helper
    defn(r.c.kernel, "Digest", 1, 1, |_, a, _| {
        let name = match crate::kernel::sym_arg(a[0]) {
            Ok(id) => id,
            Err(e) => return e,
        };
        let dg = const_lookup(rt().c.object, sym("Digest")).unwrap() as *mut Class;
        match const_get_under(dg, name) {
            Some(v) => v,
            None => {
                let le = const_lookup(rt().c.object, sym("LoadError")).unwrap() as *mut Class;
                let n = sym_str(name);
                raise(le, &format!("library not found for class Digest::{n} -- digest/{}", n.to_lowercase()))
            }
        }
    });
}

pub fn init() {
    crate::compat::provide_feature(
        &["digest", "digest/md5", "digest/sha1", "digest/sha2", "digest/sha256", "digest/sha384", "digest/sha512", "digest/sha2/sha2"],
        Some(init_digest),
    );
}
