//! CRuby-compatible C API for native extensions.
//!
//! Extensions are compiled from source against Kenny's `ruby.h` (see `include/`).
//! VALUE uses CRuby's tagging (Fixnum, Symbol, nil/true/false), so most macros work
//! unchanged; struct-peeking macros (RSTRING_PTR, RARRAY_LEN, DATA_PTR) are functions.
//!
//! Functions that can raise are implemented here as `kn_capi_*` returning Qundef, and
//! wrapped by the C shim (`kenny_capi.c`), which longjmps to the nearest C-method
//! boundary, like CRuby. That way a longjmp never crosses a Rust frame.

#![allow(non_upper_case_globals, clippy::missing_safety_doc)]

use crate::core::*;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_long, c_void, CStr};

pub type CallCFunc = unsafe extern "C" fn(*const c_void, c_int, V, c_int, *const V) -> V;

/// The C-side trampoline (setjmp boundary), registered by the shim at startup.
pub static CFUNC_CALLER: G<Option<CallCFunc>> = G::new(None);
static EXTS: G<Vec<(String, unsafe extern "C" fn())>> = G::new(Vec::new());
static LOADED_EXTS: crate::mt::L<Vec<String>> = crate::mt::L::new(Vec::new());
type AllocMap = HashMap<usize, unsafe extern "C" fn(V) -> V>;
/// Classes' allocators from C extensions (`rb_define_alloc_func`). Read on every object
/// allocation from any thread, so readers take no lock: a change publishes a new map
/// (under `ALLOC_FUNCS_W`) and keeps the old one, which a reader may still be using.
/// Null while there are none.
static ALLOC_FUNCS: std::sync::atomic::AtomicPtr<AllocMap> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static ALLOC_FUNCS_W: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn alloc_funcs_change(f: impl FnOnce(&mut AllocMap)) {
    use std::sync::atomic::Ordering::{Acquire, Release};
    let _g = ALLOC_FUNCS_W.lock().unwrap_or_else(|e| e.into_inner());
    let old = ALLOC_FUNCS.load(Acquire);
    let mut m = if old.is_null() { AllocMap::new() } else { unsafe { (*old).clone() } };
    f(&mut m);
    let new = if m.is_empty() { std::ptr::null_mut() } else { Box::into_raw(Box::new(m)) };
    ALLOC_FUNCS.store(new, Release);
}
static GLOBAL_ROOTS: crate::mt::L<Vec<*const V>> = crate::mt::L::new(Vec::new());

#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_register_caller(f: CallCFunc) {
    *CFUNC_CALLER.get() = Some(f);
}
/// Called by each extension's generated constructor before main.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_ext(name: *const c_char, init: unsafe extern "C" fn()) {
    let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    EXTS.get().push((n, init));
}

/// `require "name"` of a linked-in extension: run Init_name once.
pub fn require_ext(name: &str) -> Option<V> {
    let base = name.rsplit('/').next().unwrap_or(name).trim_end_matches(".so").trim_end_matches(".bundle");
    let init = EXTS.get().iter().find(|(n, _)| n == base).map(|(_, f)| *f)?;
    if LOADED_EXTS.get().iter().any(|n| n == base) {
        return Some(QFALSE);
    }
    LOADED_EXTS.get().push(base.to_string());
    unsafe { init() };
    if !matches!(pst().pending, Throw::None) {
        return Some(UNDEF);
    }
    Some(QTRUE)
}

pub fn alloc_func(c: *mut Class) -> Option<unsafe extern "C" fn(V) -> V> {
    let p = ALLOC_FUNCS.load(std::sync::atomic::Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    let m = unsafe { &*p };
    // allocators belong to classes, so the superclass chain is enough (no ancestors list)
    let mut k = c;
    while !k.is_null() {
        if let Some(f) = m.get(&(k as usize)) {
            return Some(*f);
        }
        k = unsafe { (*k).superclass };
    }
    None
}

pub fn call_cfunc(f: *const c_void, arity: i32, recv: V, args: &[V]) -> V {
    if arity >= 0 && args.len() as i32 != arity {
        return arity_error(args.len() as i32, arity, arity);
    }
    match *CFUNC_CALLER.get() {
        Some(caller) => unsafe { caller(f, arity, recv, args.len() as c_int, args.as_ptr()) },
        None => raise(rt().c.not_implemented_error, "C extension support was not linked in"),
    }
}

fn static_roots(out: &mut Vec<V>) {
    for p in GLOBAL_ROOTS.raw().iter() {
        out.push(unsafe { **p });
    }
}

// ---------- exported class / exception globals (CRuby names) ----------

macro_rules! globals {
    ($($name:ident),* $(,)?) => { $( #[unsafe(no_mangle)] pub static mut $name: V = 0; )* };
}
globals!(
    rb_cObject, rb_cBasicObject, rb_cModule, rb_cClass, rb_mKernel, rb_mComparable, rb_mEnumerable, rb_cString,
    rb_cSymbol, rb_cArray, rb_cHash, rb_cInteger, rb_cFloat, rb_cNumeric, rb_cRange, rb_cProc, rb_cNilClass,
    rb_cTrueClass, rb_cFalseClass, rb_cStruct, rb_cTime, rb_cIO, rb_eException, rb_eStandardError, rb_eRuntimeError,
    rb_eArgError, rb_eTypeError, rb_eNameError, rb_eNoMethodError, rb_eIndexError, rb_eKeyError, rb_eRangeError,
    rb_eZeroDivError, rb_eNotImpError, rb_eIOError, rb_eStopIteration, rb_eFrozenError, rb_eNoMemError,
    rb_eScriptError, rb_eFloatDomainError, rb_eEncodingError, rb_eLocalJumpError, rb_eSecurityError,
);

pub fn init() {
    crate::gc::heap().extra_roots.push(static_roots);
    let c = rt().c;
    let get = |n: &str| const_lookup(c.object, sym(n)).unwrap_or(c.standard_error as V);
    unsafe {
        rb_cObject = c.object as V;
        rb_cBasicObject = c.basic_object as V;
        rb_cModule = c.module as V;
        rb_cClass = c.class as V;
        rb_mKernel = c.kernel as V;
        rb_mComparable = c.comparable as V;
        rb_mEnumerable = c.enumerable as V;
        rb_cString = c.string as V;
        rb_cSymbol = c.symbol as V;
        rb_cArray = c.array as V;
        rb_cHash = c.hash as V;
        rb_cInteger = c.integer as V;
        rb_cFloat = c.float as V;
        rb_cNumeric = c.numeric as V;
        rb_cRange = c.range as V;
        rb_cProc = c.proc_ as V;
        rb_cNilClass = c.nil as V;
        rb_cTrueClass = c.true_ as V;
        rb_cFalseClass = c.false_ as V;
        rb_cStruct = c.struct_ as V;
        rb_cTime = c.time as V;
        rb_cIO = c.io as V;
        rb_eException = c.exception as V;
        rb_eStandardError = c.standard_error as V;
        rb_eRuntimeError = c.runtime_error as V;
        rb_eArgError = c.argument_error as V;
        rb_eTypeError = c.type_error as V;
        rb_eNameError = c.name_error as V;
        rb_eNoMethodError = c.no_method_error as V;
        rb_eIndexError = c.index_error as V;
        rb_eKeyError = c.key_error as V;
        rb_eRangeError = c.range_error as V;
        rb_eZeroDivError = c.zero_div_error as V;
        rb_eNotImpError = c.not_implemented_error as V;
        rb_eIOError = c.io_error as V;
        rb_eStopIteration = c.stop_iteration as V;
        rb_eFrozenError = c.frozen_error as V;
        rb_eNoMemError = c.standard_error as V;
        rb_eScriptError = c.script_error as V;
        rb_eFloatDomainError = c.float_domain_error as V;
        rb_eEncodingError = get("EncodingError");
        rb_eLocalJumpError = c.local_jump_error as V;
        rb_eSecurityError = get("SecurityError");
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}
fn cls(v: V) -> *mut Class {
    if is_class(v) { v as *mut Class } else { rt().c.object }
}

// ---------- definitions ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_define_module(name: *const c_char) -> V {
    let n = cstr(name);
    if let Some(v) = const_lookup(rt().c.object, sym(&n)) {
        return v;
    }
    let m = new_module("");
    const_set(rt().c.object, sym(&n), m as V);
    m as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_module_under(outer: V, name: *const c_char) -> V {
    let n = cstr(name);
    if let Some(&v) = unsafe { (*cls(outer)).consts.get(&sym(&n)) } {
        return v;
    }
    let m = new_module("");
    const_set(cls(outer), sym(&n), m as V);
    m as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_class(name: *const c_char, sup: V) -> V {
    rb_define_class_under(rt().c.object as V, name, sup)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_class_under(outer: V, name: *const c_char, sup: V) -> V {
    let n = cstr(name);
    if let Some(&v) = unsafe { (*cls(outer)).consts.get(&sym(&n)) } {
        return v;
    }
    let c = new_class("", cls(sup));
    unsafe { (*c).lexical_parent = cls(outer) };
    const_set(cls(outer), sym(&n), c as V);
    c as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_method(klass: V, name: *const c_char, f: *const c_void, argc: c_int) {
    add_method(cls(klass), sym(&cstr(name)), Body::CFunc(f, argc));
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_private_method(klass: V, name: *const c_char, f: *const c_void, argc: c_int) {
    rb_define_method(klass, name, f, argc)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_protected_method(klass: V, name: *const c_char, f: *const c_void, argc: c_int) {
    rb_define_method(klass, name, f, argc)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_singleton_method(obj: V, name: *const c_char, f: *const c_void, argc: c_int) {
    add_method(singleton_class(obj), sym(&cstr(name)), Body::CFunc(f, argc));
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_module_function(m: V, name: *const c_char, f: *const c_void, argc: c_int) {
    rb_define_method(m, name, f, argc);
    rb_define_singleton_method(m, name, f, argc);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_global_function(name: *const c_char, f: *const c_void, argc: c_int) {
    rb_define_method(rt().c.kernel as V, name, f, argc);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_alias(klass: V, new: *const c_char, old: *const c_char) {
    crate::abi::alias(cls(klass), sym(&cstr(new)), sym(&cstr(old)));
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_attr(klass: V, name: *const c_char, read: c_int, write: c_int) {
    let n = cstr(name);
    let iv = sym(&format!("@{n}"));
    if read != 0 {
        add_method(cls(klass), sym(&n), Body::AttrReader(iv));
    }
    if write != 0 {
        add_method(cls(klass), sym(&format!("{n}=")), Body::AttrWriter(iv));
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_const(klass: V, name: *const c_char, v: V) {
    const_set(cls(klass), sym(&cstr(name)), v);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_global_const(name: *const c_char, v: V) {
    const_set(rt().c.object, sym(&cstr(name)), v);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_alloc_func(klass: V, f: unsafe extern "C" fn(V) -> V) {
    alloc_funcs_change(|m| {
        m.insert(klass as usize, f);
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_undef_alloc_func(klass: V) {
    alloc_funcs_change(|m| {
        m.remove(&(klass as usize));
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_include_module(klass: V, m: V) {
    crate::kernel::do_include(cls(klass), m);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_extend_object(obj: V, m: V) {
    crate::kernel::do_include(singleton_class(obj), m);
}

// ---------- symbols ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_intern(name: *const c_char) -> V {
    sym(&cstr(name)) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_intern2(name: *const c_char, len: c_long) -> V {
    intern(unsafe { std::slice::from_raw_parts(name as *const u8, len as usize) }) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_id2sym(id: V) -> V {
    mksym(id as u32)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_sym2id(s: V) -> V {
    sym_id(s) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_id2name(id: V) -> *const c_char {
    let mut b = sym_name(id as u32).to_vec();
    b.push(0);
    Box::leak(b.into_boxed_slice()).as_ptr() as *const c_char
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_to_id(v: V) -> V {
    if is_sym(v) { sym_id(v) as V } else if is_str(v) { intern(&rstr(v).b.clone()) as V } else { 0 }
}

// ---------- type queries ----------

pub const T_NONE: c_int = 0x00;
pub const T_OBJECT: c_int = 0x01;
pub const T_CLASS: c_int = 0x02;
pub const T_MODULE: c_int = 0x03;
pub const T_FLOAT: c_int = 0x04;
pub const T_STRING: c_int = 0x05;
pub const T_REGEXP: c_int = 0x06;
pub const T_ARRAY: c_int = 0x07;
pub const T_HASH: c_int = 0x08;
pub const T_STRUCT: c_int = 0x09;
pub const T_BIGNUM: c_int = 0x0a;
pub const T_DATA: c_int = 0x0c;
pub const T_NIL: c_int = 0x11;
pub const T_TRUE: c_int = 0x12;
pub const T_FALSE: c_int = 0x13;
pub const T_SYMBOL: c_int = 0x14;
pub const T_FIXNUM: c_int = 0x15;

#[unsafe(no_mangle)]
pub extern "C" fn rb_type(v: V) -> c_int {
    if is_fix(v) {
        return T_FIXNUM;
    }
    match v {
        QNIL => return T_NIL,
        QTRUE => return T_TRUE,
        QFALSE => return T_FALSE,
        _ => {}
    }
    if is_sym(v) {
        return T_SYMBOL;
    }
    match kind(v) {
        Some(Kind::String) => T_STRING,
        Some(Kind::Array) => T_ARRAY,
        Some(Kind::Hash) => T_HASH,
        Some(Kind::Float) => T_FLOAT,
        Some(Kind::Bignum) => T_BIGNUM,
        Some(Kind::Class) => {
            if rclass(v).is_module {
                T_MODULE
            } else {
                T_CLASS
            }
        }
        Some(Kind::Data) => {
            if crate::regex::is_regexp(v) {
                T_REGEXP
            } else {
                T_DATA
            }
        }
        Some(Kind::Object) => {
            if is_a(v, rt().c.struct_) {
                T_STRUCT
            } else {
                T_OBJECT
            }
        }
        _ => T_OBJECT,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_check_type(v: V, t: c_int) -> V {
    let actual = rb_type(v);
    if actual == t || (t == T_DATA && actual == T_REGEXP) {
        return QNIL;
    }
    let want = match t {
        T_STRING => "String",
        T_ARRAY => "Array",
        T_HASH => "Hash",
        T_FLOAT => "Float",
        T_FIXNUM => "Integer",
        T_SYMBOL => "Symbol",
        T_DATA => "Data",
        T_CLASS => "Class",
        T_MODULE => "Module",
        _ => "?",
    };
    type_error(&format!("wrong argument type {} (expected {})", class_name(real_class(v)), want))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_class(v: V) -> V {
    real_class(v) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_class_of(v: V) -> V {
    class_of(v) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_classname(v: V) -> *const c_char {
    let mut b = class_name(real_class(v)).into_bytes();
    b.push(0);
    Box::leak(b.into_boxed_slice()).as_ptr() as *const c_char
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_class2name(c: V) -> *const c_char {
    let mut b = class_name(cls(c)).into_bytes();
    b.push(0);
    Box::leak(b.into_boxed_slice()).as_ptr() as *const c_char
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_is_kind_of(v: V, c: V) -> V {
    bool_v(is_class(c) && is_a(v, c as *mut Class))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_is_instance_of(v: V, c: V) -> V {
    bool_v(real_class(v) as V == c)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_respond_to(v: V, id: V) -> c_int {
    respond_to(v, id as u32) as c_int
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_freeze(v: V) -> V {
    if is_heap(v) {
        hdr(v).flags |= FL_FROZEN;
    }
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_frozen_p(v: V) -> V {
    bool_v(is_frozen(v))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_check_frozen(v: V) -> V {
    check_frozen(v)
}

// ---------- numbers ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_int2inum(n: isize) -> V {
    int_from_i128(n as i128)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_uint2inum(n: usize) -> V {
    int_from_i128(n as i128)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ll2inum(n: i64) -> V {
    int_from_i128(n as i128)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ull2inum(n: u64) -> V {
    int_from_i128(n as i128)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_float_new(f: f64) -> V {
    float_new(f)
}
/// Integer conversion; *ok = 0 when a TypeError/RangeError is pending.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_num2long(v: V, ok: *mut c_int) -> i64 {
    use num_traits::ToPrimitive;
    unsafe { *ok = 1 };
    if is_fix(v) {
        return fix(v);
    }
    if is_big(v) {
        if let Some(i) = rbig(v).n.to_i64() {
            return i;
        }
        raise(rt().c.range_error, "bignum too big to convert into 'long'");
    } else if is_float(v) {
        return float_val(v) as i64;
    } else if v == QNIL {
        type_error("no implicit conversion from nil to integer");
    } else {
        type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v))));
    }
    unsafe { *ok = 0 };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_num2dbl(v: V, ok: *mut c_int) -> f64 {
    unsafe { *ok = 1 };
    if let Some(f) = crate::fmt::num_to_f(v) {
        return f;
    }
    type_error(&format!("no implicit conversion to float from {}", crate::fmt::inspect_lossy(v)));
    unsafe { *ok = 0 };
    0.0
}

// ---------- strings ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_new(p: *const c_char, len: c_long) -> V {
    if p.is_null() {
        return str_new(vec![0; len.max(0) as usize]);
    }
    str_new(unsafe { std::slice::from_raw_parts(p as *const u8, len.max(0) as usize) }.to_vec())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_new_cstr(p: *const c_char) -> V {
    str_new(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_utf8_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { rb_str_new(p, len) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_utf8_str_new_cstr(p: *const c_char) -> V {
    unsafe { rb_str_new_cstr(p) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_cat(s: V, p: *const c_char, len: c_long) -> V {
    rstr(s).b.extend_from_slice(unsafe { std::slice::from_raw_parts(p as *const u8, len.max(0) as usize) });
    s
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_cat_cstr(s: V, p: *const c_char) -> V {
    rstr(s).b.extend_from_slice(unsafe { CStr::from_ptr(p) }.to_bytes());
    s
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_append(s: V, o: V) -> V {
    if is_str(o) {
        let b = rstr(o).b.clone();
        rstr(s).b.extend_from_slice(&b);
    }
    s
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_dup(s: V) -> V {
    str_new(rstr(s).b.clone())
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_resize(s: V, len: c_long) -> V {
    rstr(s).b.resize(len.max(0) as usize, 0);
    s
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_set_len(s: V, len: c_long) {
    rstr(s).b.resize(len.max(0) as usize, 0);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_buf_new(cap: c_long) -> V {
    str_new(Vec::with_capacity(cap.max(0) as usize))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_modify(s: V) {
    let _ = s;
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_modify_expand(s: V, extra: c_long) {
    rstr(s).b.reserve(extra.max(0) as usize);
}
/// RSTRING_PTR: a NUL-terminated view of the bytes (capacity reserved for the terminator).
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_rstring_ptr(s: V) -> *mut c_char {
    let b = &mut rstr(s).b;
    b.reserve(1);
    unsafe { *b.as_mut_ptr().add(b.len()) = 0 };
    b.as_mut_ptr() as *mut c_char
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_rstring_len(s: V) -> c_long {
    rstr(s).b.len() as c_long
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_string_value(v: V) -> V {
    if is_str(v) {
        return v;
    }
    if is_heap(v) && respond_to(v, sym("to_str")) {
        return send0(v, "to_str");
    }
    type_error(&format!("no implicit conversion of {} into String", if v == QNIL { "nil".to_string() } else { class_name(real_class(v)) }))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_as_string(v: V) -> V {
    match crate::fmt::to_s(v) {
        Ok(b) => str_new(b),
        Err(()) => UNDEF,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_inspect(v: V) -> V {
    match crate::fmt::inspect(v) {
        Ok(b) => str_new(b),
        Err(()) => UNDEF,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_equal(a: V, b: V) -> V {
    bool_v(is_str(a) && is_str(b) && rstr(a).b == rstr(b).b)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_intern(s: V) -> V {
    mksym(intern(&rstr(s).b.clone()))
}

// ---------- arrays ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_new() -> V {
    ary_new(vec![])
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_new_capa(n: c_long) -> V {
    ary_new(Vec::with_capacity(n.max(0) as usize))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_ary_new_from_values(n: c_long, p: *const V) -> V {
    ary_new(unsafe { std::slice::from_raw_parts(p, n.max(0) as usize) }.to_vec())
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_push(a: V, v: V) -> V {
    rary(a).v.push(v);
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_pop(a: V) -> V {
    rary(a).v.pop().unwrap_or(QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_shift(a: V) -> V {
    let v = &mut rary(a).v;
    if v.is_empty() { QNIL } else { v.remove(0) }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_unshift(a: V, x: V) -> V {
    rary(a).v.insert(0, x);
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_entry(a: V, i: c_long) -> V {
    crate::abi::kn_ary_ref(a, i as i64)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_store(a: V, i: c_long, v: V) {
    let vec = &mut rary(a).v;
    let i = if i < 0 { i + vec.len() as c_long } else { i };
    if i < 0 {
        return;
    }
    let i = i as usize;
    if i >= vec.len() {
        vec.resize(i + 1, QNIL);
    }
    vec[i] = v;
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_rarray_len(a: V) -> c_long {
    rary(a).v.len() as c_long
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_rarray_ptr(a: V) -> *mut V {
    rary(a).v.as_mut_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_dup(a: V) -> V {
    ary_new(rary(a).v.clone())
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_clear(a: V) -> V {
    rary(a).v.clear();
    a
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ary_join(a: V, sep: V) -> V {
    send1(a, "join", sep)
}

// ---------- hashes ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_new() -> V {
    hash_new()
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_hash_aref(h: V, k: V) -> V {
    crate::hash::hget(h, k)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_lookup(h: V, k: V) -> V {
    rhash(h).m.get(&HKey(k)).copied().unwrap_or(QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_lookup2(h: V, k: V, def: V) -> V {
    rhash(h).m.get(&HKey(k)).copied().unwrap_or(def)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_aset(h: V, k: V, v: V) -> V {
    crate::hash::hset(h, k, v)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_delete(h: V, k: V) -> V {
    rhash(h).m.shift_remove(&HKey(k)).unwrap_or(QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_size(h: V) -> V {
    mkfix(rhash(h).m.len() as i64)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_keys(h: V) -> V {
    ary_new(rhash(h).m.keys().map(|k| k.0).collect())
}
/// rb_hash_foreach: callback returns 0 (ST_CONTINUE), 1 (ST_STOP), 2 (ST_DELETE).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_hash_foreach(h: V, f: unsafe extern "C" fn(V, V, V) -> c_int, arg: V) {
    for (k, v) in crate::hash::pairs(h) {
        match unsafe { f(k, v, arg) } {
            1 => break,
            2 => {
                rhash(h).m.shift_remove(&HKey(k));
            }
            _ => {}
        }
    }
}

// ---------- calling Ruby ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_funcallv(recv: V, mid: V, argc: c_int, argv: *const V) -> V {
    let args = unsafe { args_slice(argc as u64, argv) };
    send(recv, mid as u32, args, QNIL)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_funcall_with_block(recv: V, mid: V, argc: c_int, argv: *const V, blk: V) -> V {
    let args = unsafe { args_slice(argc as u64, argv) };
    send(recv, mid as u32, args, blk)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_yield_values(argc: c_int, argv: *const V) -> V {
    let blk = pst().current_block.last().copied().unwrap_or(QNIL);
    call_block(blk, unsafe { args_slice(argc as u64, argv) })
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_block_given_p() -> c_int {
    pst().current_block.last().is_some_and(|b| is_proc(*b)) as c_int
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_block_proc() -> V {
    pst().current_block.last().copied().unwrap_or(QNIL)
}
/// Block of the innermost C method call (for rb_yield / rb_block_given_p).

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_class_new_instance(argc: c_int, argv: *const V, klass: V) -> V {
    crate::kernel::class_new_instance(klass, unsafe { args_slice(argc as u64, argv) }, QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_const_get(klass: V, id: V) -> V {
    match const_lookup(cls(klass), id as u32) {
        Some(v) => v,
        None => crate::abi::const_missing(cls(klass), id as u32),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_path2class(path: *const c_char) -> V {
    let p = cstr(path);
    let mut cur = rt().c.object as V;
    for part in p.split("::").filter(|s| !s.is_empty()) {
        match const_get_under(cls(cur), sym(part)) {
            Some(v) => cur = v,
            None => return raise(rt().c.name_error, &format!("uninitialized constant {p}")),
        }
    }
    cur
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ivar_get(obj: V, id: V) -> V {
    ivar_get(obj, id as u32)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ivar_set(obj: V, id: V, v: V) -> V {
    ivar_set(obj, id as u32, v)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_iv_get(obj: V, name: *const c_char) -> V {
    ivar_get(obj, sym(&cstr(name)))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_iv_set(obj: V, name: *const c_char, v: V) -> V {
    ivar_set(obj, sym(&cstr(name)), v)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gv_get(name: *const c_char) -> V {
    let n = cstr(name);
    let n = if n.starts_with('$') { n } else { format!("${n}") };
    rt().gvars.get(&sym(&n)).copied().unwrap_or(QNIL)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gv_set(name: *const c_char, v: V) -> V {
    let n = cstr(name);
    let n = if n.starts_with('$') { n } else { format!("${n}") };
    rt().gvars.insert(sym(&n), v);
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_equal(a: V, b: V) -> V {
    equal(a, b)
}

// ---------- exceptions ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_raise(klass: V, msg: *const c_char) -> V {
    raise(cls(klass), &cstr(msg))
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_exc_raise(e: V) -> V {
    raise_exc(e)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_pending() -> c_int {
    !matches!(pst().pending, Throw::None) as c_int
}
/// rb_protect support: take the pending exception (into $!) and return it.
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_take_pending() -> V {
    match std::mem::replace(&mut pst().pending, Throw::None) {
        Throw::Raise(e) => {
            pst().errinfo = e;
            e
        }
        other => {
            pst().pending = other;
            QNIL
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_errinfo() -> V {
    pst().errinfo
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_set_errinfo(e: V) {
    pst().errinfo = e;
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_warn(msg: *const c_char) {
    crate::kernel::err_out(format!("warning: {}\n", cstr(msg)).as_bytes());
}

// ---------- data objects ----------

#[repr(C)]
pub struct RbDataType {
    pub wrap_struct_name: *const c_char,
    pub dmark: Option<unsafe extern "C" fn(*mut c_void)>,
    pub dfree: Option<unsafe extern "C" fn(*mut c_void)>,
    pub dsize: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    pub dcompact: Option<unsafe extern "C" fn(*mut c_void)>,
    pub reserved: [*mut c_void; 1],
    pub parent: *const RbDataType,
    pub data: *mut c_void,
    pub flags: V,
}

unsafe extern "C" fn default_free(p: *mut c_void) {
    unsafe { libc::free(p) };
}
fn data_new(klass: V, ptr: *mut c_void, mark: Option<unsafe extern "C" fn(*mut c_void)>, free: Option<unsafe extern "C" fn(*mut c_void)>) -> V {
    // RUBY_DEFAULT_FREE is (RUBY_DATA_FUNC)-1
    let free = match free {
        Some(f) if f as usize == usize::MAX => Some(default_free as unsafe extern "C" fn(*mut c_void)),
        other => other,
    };
    let k = if is_class(klass) { klass as *mut Class } else { rt().c.object };
    let v = alloc(RData { h: Header { klass: k, kind: Kind::Data, flags: 0 }, data: ptr, dmark: mark, dfree: free, roots: Vec::new() });
    if mark.is_some() {
        crate::gc::register_marked_data(v);
    }
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_data_object_wrap(klass: V, ptr: *mut c_void, mark: Option<unsafe extern "C" fn(*mut c_void)>, free: Option<unsafe extern "C" fn(*mut c_void)>) -> V {
    data_new(klass, ptr, mark, free)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_data_typed_object_wrap(klass: V, ptr: *mut c_void, ty: *const RbDataType) -> V {
    let t = unsafe { &*ty };
    let v = data_new(klass, ptr, t.dmark, t.dfree);
    ivar_set(v, sym("@__type"), mkfix(ty as i64 >> 1));
    v
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_data_typed_object_zalloc(klass: V, size: usize, ty: *const RbDataType) -> V {
    let p = unsafe { libc::calloc(1, size.max(1)) };
    unsafe { rb_data_typed_object_wrap(klass, p, ty) }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_data_ptr_ref(v: V) -> *mut *mut c_void {
    unsafe { &mut (*(v as *mut RData)).data }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_check_typeddata(v: V, ty: *const RbDataType) -> *mut c_void {
    if !is_kind(v, Kind::Data) {
        type_error(&format!("wrong argument type {} (expected {})", class_name(real_class(v)), cstr(unsafe { (*ty).wrap_struct_name })));
        return std::ptr::null_mut();
    }
    unsafe { (*(v as *mut RData)).data }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_mark(v: V) {
    crate::gc::mark_from_extension(v);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_mark_maybe(v: V) {
    crate::gc::mark_from_extension(v);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_register_address(p: *const V) {
    GLOBAL_ROOTS.get().push(p);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_global_variable(p: *const V) {
    GLOBAL_ROOTS.get().push(p);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_register_mark_object(v: V) {
    let b = Box::leak(Box::new(v));
    GLOBAL_ROOTS.get().push(b as *const V);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc() {
    crate::gc::collect_now();
}

#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_obj_alloc(klass: V) -> V {
    crate::kernel::alloc_instance(cls(klass))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_const_defined(klass: V, id: V) -> c_int {
    const_lookup(cls(klass), id as u32).is_some() as c_int
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_attr_get(obj: V, id: V) -> V {
    ivar_get(obj, id as u32)
}
