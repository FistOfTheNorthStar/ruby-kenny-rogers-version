//! The native part of the prism gem (`prism/prism`, a C extension in CRuby), on the libprism
//! that Kenny's compiler parses with (the ruby-prism-sys crate). It works as the gem's FFI
//! backend (prism/ffi.rb) does: libprism serializes its parse into a String and the gem's
//! pure-Ruby `Prism::Serialize` loads it. `Kenny::Prism` has the primitives; Kenny's
//! `prism/prism.rb` overlay (lib/kenny/prism/prism.rb) defines `Prism.parse`, `lex`, `dump` ...
//! on top of them. The serialized format changes with every prism release, so the Ruby library
//! must be the same version as libprism: `kenny build` checks that (crates/kenny/src/main.rs).
//!
//! Linked only into programs that load prism (or Ripper, which Kenny implements on
//! Prism::Translation::Ripper): `kenny build` then registers `Init_kenny_prism` as the extension
//! "kenny_prism"; other binaries don't carry libprism (the linker drops what nothing references).

use crate::core::*;
use std::ffi::{c_char, CStr};
// (the crate builds and links libprism; its functions are declared here)
use ruby_prism_sys as _;

#[repr(C)]
struct PmBuffer {
    length: usize,
    capacity: usize,
    value: *mut c_char,
}

unsafe extern "C" {
    fn pm_version() -> *const c_char;
    fn pm_buffer_init(buffer: *mut PmBuffer) -> bool;
    fn pm_buffer_free(buffer: *mut PmBuffer);
    fn pm_serialize_parse(buffer: *mut PmBuffer, source: *const u8, size: usize, data: *const c_char);
    fn pm_serialize_lex(buffer: *mut PmBuffer, source: *const u8, size: usize, data: *const c_char);
    fn pm_serialize_parse_comments(buffer: *mut PmBuffer, source: *const u8, size: usize, data: *const c_char);
    fn pm_serialize_parse_lex(buffer: *mut PmBuffer, source: *const u8, size: usize, data: *const c_char);
    fn pm_parse_success_p(source: *const u8, size: usize, data: *const c_char) -> bool;
    fn pm_string_query_local(source: *const u8, length: usize, encoding_name: *const c_char) -> i32;
    fn pm_string_query_constant(source: *const u8, length: usize, encoding_name: *const c_char) -> i32;
    fn pm_string_query_method_name(source: *const u8, length: usize, encoding_name: *const c_char) -> i32;
}

fn kenny_module() -> *mut Class {
    match const_lookup(rt().c.object, sym("Kenny")) {
        Some(m) => m as *mut Class,
        None => {
            let m = new_module("Kenny");
            set_global_const("Kenny", m as V);
            m
        }
    }
}

fn bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        Ok(rstr(v).b.to_vec())
    } else {
        Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
    }
}

/// The source with a NUL after it: libprism asserts a non-null source pointer, and an empty
/// Vec's isn't a real allocation.
fn source_bytes(v: V) -> Result<Vec<u8>, V> {
    let mut b = bytes(v)?;
    b.push(0);
    Ok(b)
}

/// `Kenny::Prism.serialize(kind, source, options)`: libprism's serialization of a parse (0), a
/// lex (1), the comments (2) or both a parse and a lex (3) of `source`, as a binary String.
/// `options` is the packed options string of prism/ffi.rb's `dump_options`.
fn serialize(_: V, a: &[V], _: V) -> V {
    let src = match source_bytes(a[1]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let opts = match bytes(a[2]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let f = match if is_fix(a[0]) { fix(a[0]) } else { -1 } {
        0 => pm_serialize_parse,
        1 => pm_serialize_lex,
        2 => pm_serialize_parse_comments,
        3 => pm_serialize_parse_lex,
        _ => return arg_error("invalid serialization kind"),
    };
    let mut buf = PmBuffer { length: 0, capacity: 0, value: std::ptr::null_mut() };
    let out = unsafe {
        if !pm_buffer_init(&mut buf) {
            return raise(rt().c.runtime_error, "failed to allocate memory");
        }
        f(&mut buf, src.as_ptr(), src.len() - 1, opts.as_ptr() as *const c_char);
        let out = if buf.value.is_null() { Vec::new() } else { std::slice::from_raw_parts(buf.value as *const u8, buf.length).to_vec() };
        pm_buffer_free(&mut buf);
        out
    };
    crate::pack::bin_str(out)
}

/// `Kenny::Prism.success?(source, options)`
fn success_p(_: V, a: &[V], _: V) -> V {
    let src = match source_bytes(a[0]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let opts = match bytes(a[1]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    bool_v(unsafe { pm_parse_success_p(src.as_ptr(), src.len() - 1, opts.as_ptr() as *const c_char) })
}

/// `Kenny::Prism.string_query(kind, string, encoding_name)`: whether `string` is a local variable
/// name (0), a constant name (1) or a method name (2): 1 yes, 0 no, -1 an encoding prism can't
/// check.
fn string_query(_: V, a: &[V], _: V) -> V {
    let s = match source_bytes(a[1]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let enc = match source_bytes(a[2]) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let f = match if is_fix(a[0]) { fix(a[0]) } else { -1 } {
        0 => pm_string_query_local,
        1 => pm_string_query_constant,
        2 => pm_string_query_method_name,
        _ => return arg_error("invalid string query"),
    };
    mkfix(unsafe { f(s.as_ptr(), s.len() - 1, enc.as_ptr() as *const c_char) } as i64)
}

/// libprism's version ("1.9.0").
pub fn version() -> String {
    unsafe { CStr::from_ptr(pm_version()) }.to_string_lossy().into_owned()
}

/// `require "kenny_prism"`
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "C" fn Init_kenny_prism() {
    let kenny = kenny_module();
    let m = new_module("Kenny::Prism");
    const_set(kenny, sym("Prism"), m as V);
    const_set(m, sym("LIBPRISM_VERSION"), freeze_str(str_from(&version())));
    defs(m, "serialize", 3, 3, serialize);
    defs(m, "success?", 2, 2, success_p);
    defs(m, "string_query", 3, 3, string_query);
}

fn freeze_str(v: V) -> V {
    hdr(v).flags |= FL_FROZEN;
    v
}
