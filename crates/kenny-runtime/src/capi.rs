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

/// `require "feature"` of a linked-in extension: run its Init function once. An extension
/// registered as "msgpack/msgpack" is found by that name, also under a directory
/// ("3.4/unf_ext.so", as gems that ship one binary per Ruby version ask for it).
pub fn require_ext(name: &str) -> Option<V> {
    let base = name.trim_end_matches(".so").trim_end_matches(".bundle");
    let (feature, init) = EXTS.get().iter().find(|(n, _)| base == n || base.strip_suffix(n.as_str()).is_some_and(|d| d.ends_with('/'))).map(|(n, f)| (n.clone(), *f))?;
    if LOADED_EXTS.get().iter().any(|n| *n == feature) {
        return Some(QFALSE);
    }
    LOADED_EXTS.get().push(feature);
    enter_frame(sym("require"), rt().main_obj, std::ptr::null_mut());
    if CFUNC_CALLER.get().is_some() {
        // (through the C shim: an exception the Init function raises longjmps back to it)
        let _ = crate::core::call_native_c(init as *const c_void, 0, QNIL, &[]);
    } else {
        // only the runtime's own native parts (kenny_assets, ...): Rust, no longjmp
        unsafe { init() };
    }
    leave_frame();
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
    rb_cBinding, rb_cComplex, rb_cData, rb_cDir, rb_cEncoding, rb_cEnumerator, rb_cFile, rb_cMatch, rb_cMethod, rb_cRandom, rb_cRational, rb_cRegexp, rb_cSet, rb_cThread, rb_cUnboundMethod, rb_eEncCompatError, rb_eEOFError, rb_eFatal, rb_eInterrupt, rb_eLoadError, rb_eMathDomainError, rb_eNoMatchingPatternError, rb_eNoMatchingPatternKeyError, rb_eRegexpError, rb_eSignal, rb_eSyntaxError, rb_eSystemCallError, rb_eSystemExit, rb_eThreadError, rb_mErrno, rb_mGC, rb_mMath, rb_mProcess, rb_eSysStackError,
);

pub fn init() {
    crate::gc::heap().extra_roots.push(static_roots);
    let c = rt().c;
    let get = |n: &str| const_lookup(c.object, sym(n)).unwrap_or(c.standard_error as V);
    // (a class the runtime lacks is Object, so a C extension's reference to it still links)
    let path = |p: &str| {
        let mut k = c.object as V;
        for n in p.split("::") {
            match const_lookup(cls(k), sym(n)) {
                Some(v) if is_class(v) => k = v,
                _ => return c.object as V,
            }
        }
        k
    };
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
        rb_cBinding = path("Binding");
        rb_cComplex = path("Complex");
        rb_cData = path("Object");
        rb_cDir = path("Dir");
        rb_cEncoding = path("Encoding");
        rb_cEnumerator = path("Enumerator");
        rb_cFile = path("File");
        rb_cMatch = path("MatchData");
        rb_cMethod = path("Method");
        rb_cRandom = path("Random");
        rb_cRational = path("Rational");
        rb_cRegexp = path("Regexp");
        rb_cSet = path("Set");
        rb_cThread = path("Thread");
        rb_cUnboundMethod = path("UnboundMethod");
        rb_eEncCompatError = path("Encoding::CompatibilityError");
        rb_eEOFError = path("EOFError");
        rb_eFatal = path("Exception");
        rb_eInterrupt = path("Interrupt");
        rb_eLoadError = path("LoadError");
        rb_eMathDomainError = path("Math::DomainError");
        rb_eNoMatchingPatternError = path("NoMatchingPatternError");
        rb_eNoMatchingPatternKeyError = path("NoMatchingPatternKeyError");
        rb_eRegexpError = path("RegexpError");
        rb_eSignal = path("SignalException");
        rb_eSyntaxError = path("SyntaxError");
        rb_eSysStackError = path("SystemStackError");
        rb_eSystemCallError = path("SystemCallError");
        rb_eSystemExit = path("SystemExit");
        rb_eThreadError = path("ThreadError");
        rb_mErrno = path("Errno");
        rb_mGC = path("GC");
        rb_mMath = path("Math");
        rb_mProcess = path("Process");
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
pub extern "C" fn rb_get_alloc_func(klass: V) -> Option<unsafe extern "C" fn(V) -> V> {
    alloc_func(cls(klass))
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
    if is_flonum(v) {
        return T_FLOAT;
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
    // as `obj.respond_to?(name)`: private methods don't count (ox's SAX handler base class
    // has every callback as a private no-op)
    let m = find_method(class_of(v), id as u32);
    if !m.is_null() && unsafe { (*m).vis } != VIS_PRIVATE && !crate::kernel::is_user_method(v, "respond_to?") {
        return 1;
    }
    let r = send(v, sym("respond_to?"), &[mksym(id as u32)], QNIL);
    if r == UNDEF {
        pst().pending = Throw::None;
        return 0;
    }
    truthy(r) as c_int
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
    if is_float(v) {
        let f = float_val(v);
        if f >= -9.223372036854776e18 && f < 9.223372036854776e18 {
            return f as i64;
        }
        raise(rt().c.range_error, &format!("float {} out of range of integer", crate::fmt::inspect_lossy(v)));
        unsafe { *ok = 0 };
        return 0;
    }
    match num_to_int(v) {
        Some(i) if is_fix(i) => return fix(i),
        Some(i) => {
            if let Some(i) = rbig(i).n.to_i64() {
                return i;
            }
            raise(rt().c.range_error, "bignum too big to convert into 'long'");
        }
        None => {}
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

unsafe fn bytes_of<'a>(p: *const c_char, len: c_long) -> &'a [u8] {
    if p.is_null() || len <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(p as *const u8, len as usize) } }
}
/// A new String of `len` bytes from `p` (zeros if `p` is null) in encoding `idx`.
unsafe fn str_with(p: *const c_char, len: c_long, idx: c_int) -> V {
    let s = if p.is_null() { str_new(vec![0; len.max(0) as usize]) } else { str_new(unsafe { bytes_of(p, len) }.to_vec()) };
    set_enc_index(s, idx);
    s
}
fn cstr_len(p: *const c_char) -> c_long {
    if p.is_null() { 0 } else { unsafe { CStr::from_ptr(p) }.to_bytes().len() as c_long }
}
/// rb_str_new: an ASCII-8BIT String, as in CRuby (rb_utf8_str_new for UTF-8).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_BINARY) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_BINARY) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_utf8_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_utf8_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_usascii_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_USASCII) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_usascii_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_USASCII) }
}
// (the default external, locale and filesystem encodings are UTF-8)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_external_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_external_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_locale_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_locale_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_filesystem_str_new(p: *const c_char, len: c_long) -> V {
    unsafe { str_with(p, len, ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_filesystem_str_new_cstr(p: *const c_char) -> V {
    unsafe { str_with(p, cstr_len(p), ENC_UTF8) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_str_new(p: *const c_char, len: c_long, enc: *const KnEncoding) -> V {
    unsafe { str_with(p, len, if enc.is_null() { ENC_BINARY } else { (*enc).index }) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_str_new_cstr(p: *const c_char, enc: *const KnEncoding) -> V {
    unsafe { rb_enc_str_new(p, cstr_len(p), enc) }
}
/// A frozen String (CRuby's are deduplicated; Kenny's `-"str"` doesn't deduplicate either).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_interned_str(p: *const c_char, len: c_long, enc: *const KnEncoding) -> V {
    let s = unsafe { rb_enc_str_new(p, len, enc) };
    hdr(s).flags |= FL_FROZEN;
    s
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_interned_str_cstr(p: *const c_char, enc: *const KnEncoding) -> V {
    unsafe { rb_enc_interned_str(p, cstr_len(p), enc) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_interned_str(p: *const c_char, len: c_long) -> V {
    let s = unsafe { str_with(p, len, ENC_USASCII) };
    if !rstr(s).b.is_ascii() {
        set_enc_index(s, ENC_BINARY);
    }
    hdr(s).flags |= FL_FROZEN;
    s
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_interned_str_cstr(p: *const c_char) -> V {
    unsafe { rb_interned_str(p, cstr_len(p)) }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_fstring(s: V) -> V {
    if is_frozen(s) {
        return s;
    }
    let d = rb_str_dup(s);
    hdr(d).flags |= FL_FROZEN;
    d
}
/// (rb_str_cat: the C side checks that the String isn't frozen)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_str_cat(s: V, p: *const c_char, len: c_long) -> V {
    rstr(s).b.extend_from_slice(unsafe { bytes_of(p, len) });
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
/// A copy with the same encoding and class.
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_dup(s: V) -> V {
    let d = str_new(rstr(s).b.clone());
    crate::transcode::set_enc(d, crate::transcode::enc_of(s));
    let k = real_class(s);
    if k != rt().c.string {
        hdr(d).klass = k;
    }
    d
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_new_frozen(s: V) -> V {
    rb_fstring(s)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_str_new_with_class(obj: V, p: *const c_char, len: c_long) -> V {
    let s = unsafe { rb_str_new(p, len) };
    if is_str(obj) {
        crate::transcode::set_enc(s, crate::transcode::enc_of(obj));
        hdr(s).klass = real_class(obj);
    }
    s
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_capacity(s: V) -> usize {
    // (C code that asks may write up to it: see `filling`)
    filling(s, true);
    rstr(s).b.capacity()
}
/// (CRuby: a byte range of the String, sharing its buffer; here a copy)
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_subseq(s: V, beg: c_long, len: c_long) -> V {
    let b = &rstr(s).b;
    let beg = (beg.max(0) as usize).min(b.len());
    let end = (beg + len.max(0) as usize).min(b.len());
    let d = str_new(b[beg..end].to_vec());
    crate::transcode::set_enc(d, crate::transcode::enc_of(s));
    d
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_cmp(a: V, b: V) -> c_int {
    match rstr(a).b.as_slice().cmp(rstr(b).b.as_slice()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_hash(s: V) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    rstr(s).b.as_slice().hash(&mut h);
    h.finish() as usize
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_str_reserve(s: V, extra: c_long) {
    rstr(s).b.reserve(extra.max(0) as usize);
    filling(s, true);
}

/// Strings C code is filling past their length (after rb_str_modify_expand or
/// rb_str_buf_new, until rb_str_set_len): RSTRING_PTR then leaves the bytes after the length
/// alone instead of writing the terminator there. (The string, and its buffer: an entry whose
/// buffer moved is stale.)
static FILLING: std::sync::Mutex<Option<rustc_hash::FxHashMap<V, usize>>> = std::sync::Mutex::new(None);
static FILLING_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn filling(s: V, on: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    if !on && FILLING_N.load(Relaxed) == 0 {
        return;
    }
    let mut g = FILLING.lock().unwrap();
    let m = g.get_or_insert_with(Default::default);
    if on {
        m.insert(s, rstr(s).b.as_ptr() as usize);
    } else {
        m.remove(&s);
    }
    FILLING_N.store(m.len(), Relaxed);
}
fn is_filling(s: V) -> bool {
    if FILLING_N.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return false;
    }
    let g = FILLING.lock().unwrap();
    g.as_ref().and_then(|m| m.get(&s)).is_some_and(|&p| p == rstr(s).b.as_ptr() as usize)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_resize(s: V, len: c_long) -> V {
    filling(s, false);
    rstr(s).b.resize(len.max(0) as usize, 0);
    s
}
/// The new length of a String whose buffer C code wrote into: what is there stays (within
/// its capacity; growing past it zero-fills).
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_set_len(s: V, len: c_long) {
    filling(s, false);
    let b = &mut rstr(s).b;
    let len = len.max(0) as usize;
    if len <= b.capacity() {
        b.set_len_in_capacity(len);
    } else {
        b.resize(len, 0);
    }
    // (the terminator, as CRuby's)
    b.reserve(1);
    unsafe { *b.as_mut_ptr().add(b.len()) = 0 };
}
/// An empty ASCII-8BIT String with room for `cap` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn rb_str_buf_new(cap: c_long) -> V {
    let s = str_new(Vec::with_capacity(cap.max(0) as usize + 1));
    set_enc_index(s, ENC_BINARY);
    filling(s, true);
    s
}
/// RSTRING_PTR: a NUL-terminated view of the bytes (capacity reserved for the terminator).
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_rstring_ptr(s: V) -> *mut c_char {
    if is_filling(s) && rstr(s).b.len() < rstr(s).b.capacity() {
        return rstr(s).b.as_mut_ptr() as *mut c_char;
    }
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
/// The pairs of a Hash as a flat Array [k0, v0, k1, v1 ...] (rb_hash_foreach iterates over
/// it on the C side: its callback may raise, and that unwinds through C frames only).
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_hash_pairs(h: V) -> V {
    ary_new(rhash(h).m.iter().flat_map(|(k, v)| [k.0, *v]).collect())
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_new_capa(_capa: c_long) -> V {
    hash_new()
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_size_num(h: V) -> usize {
    rhash(h).m.len()
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_values(h: V) -> V {
    ary_new(rhash(h).m.values().copied().collect())
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_ifnone(h: V) -> V {
    rhash(h).default
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_hash_set_ifnone(h: V, v: V) -> V {
    rhash(h).default = v;
    h
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
    ivar_set(v, type_ivar(), mkfix(ty as i64 >> 1));
    v
}
fn type_ivar() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| sym("@__type"))
}
/// The rb_data_type_t of a TypedData object (null for anything else).
#[unsafe(no_mangle)]
pub extern "C" fn rb_kn_typeddata_type(v: V) -> *const RbDataType {
    if !is_heap(v) || !is_kind(v, Kind::Data) {
        return std::ptr::null();
    }
    let t = ivar_get(v, type_ivar());
    if is_fix(t) { (fix(t) << 1) as usize as *const RbDataType } else { std::ptr::null() }
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
pub unsafe extern "C" fn kn_capi_check_typeddata(v: V, ty: *const RbDataType, ok: *mut c_int) -> *mut c_void {
    unsafe { *ok = 1 };
    let mut t = rb_kn_typeddata_type(v);
    let actual = t;
    while !t.is_null() && t != ty {
        t = unsafe { (*t).parent };
    }
    if t.is_null() {
        let what = if actual.is_null() {
            match v {
                QNIL => "nil".to_string(),
                QTRUE => "true".to_string(),
                QFALSE => "false".to_string(),
                _ => class_name(real_class(v)),
            }
        } else {
            cstr(unsafe { (*actual).wrap_struct_name })
        };
        type_error(&format!("wrong argument type {what} (expected {})", cstr(unsafe { (*ty).wrap_struct_name })));
        unsafe { *ok = 0 };
        return std::ptr::null_mut();
    }
    unsafe { (*(v as *mut RData)).data }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_mark(v: V) {
    crate::gc::mark_from_extension(v);
}
/// (a word that may not be an object: marked only if it is one)
#[unsafe(no_mangle)]
pub extern "C" fn rb_gc_mark_maybe(v: V) {
    if !is_heap(v) || crate::gc::is_object(v as usize) {
        crate::gc::mark_from_extension(v);
    }
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
pub extern "C" fn rb_gc_unregister_address(p: *const V) {
    GLOBAL_ROOTS.get().retain(|x| *x != p);
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

// ---------- the running C method (rb_frame_this_func, rb_current_receiver) ----------

thread_local! {
    static FRAMES: std::cell::RefCell<Vec<(u32, V, *mut Class)>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// A C method is called (its name, receiver and owner; the receiver is also on the caller's
/// stack, the owner is a class).
pub fn enter_frame(name: u32, recv: V, owner: *mut Class) {
    FRAMES.with(|f| f.borrow_mut().push((name, recv, owner)));
}
pub fn leave_frame() {
    FRAMES.with(|f| f.borrow_mut().pop());
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_frame_this_func() -> V {
    FRAMES.with(|f| f.borrow().last().map_or(0, |x| x.0 as V))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_frame_callee() -> V {
    rb_frame_this_func()
}
/// `super` from the running C method (with its arguments and block).
#[unsafe(no_mangle)]
pub extern "C" fn rb_call_super(argc: c_int, argv: *const V) -> V {
    let Some((mid, recv, owner)) = FRAMES.with(|f| f.borrow().last().copied()) else {
        return raise(rt().c.runtime_error, "super called outside of method");
    };
    let args: &[V] = if argc <= 0 || argv.is_null() { &[] } else { unsafe { std::slice::from_raw_parts(argv, argc as usize) } };
    let m = crate::core::find_super_method(recv, owner, mid);
    if m.is_null() || owner.is_null() {
        let d = crate::core::describe_recv(recv);
        return raise(rt().c.no_method_error, &format!("super: no superclass method '{}' for {d}", sym_str(mid)));
    }
    let blk = pst().current_block.last().copied().unwrap_or(QNIL);
    crate::core::invoke(m, recv, args, blk)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_current_receiver() -> V {
    FRAMES.with(|f| f.borrow().last().map_or(rt().main_obj, |x| x.1))
}

// ---------- encodings ----------
//
// rb_encoding is a small struct per encoding (name, index, min/max character length), at
// CRuby's indexes for the three Kenny strings carry natively: 0 ASCII-8BIT, 1 UTF-8,
// 2 US-ASCII; then every other encoding CRuby has (enc_table.rs).

#[repr(C)]
pub struct KnEncoding {
    pub name: *const c_char,
    pub index: c_int,
    pub min_len: c_int,
    pub max_len: c_int,
    pub ascii_compat: c_int,
    pub dummy: c_int,
}
unsafe impl Sync for KnEncoding {}
unsafe impl Send for KnEncoding {}
pub const ENC_BINARY: c_int = 0;
pub const ENC_UTF8: c_int = 1;
pub const ENC_USASCII: c_int = 2;

fn encodings() -> &'static [KnEncoding] {
    static T: std::sync::OnceLock<Vec<KnEncoding>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut names: Vec<(&'static str, bool, bool)> = vec![("ASCII-8BIT", true, false), ("UTF-8", true, false), ("US-ASCII", true, false)];
        for &(n, _, _, dummy, ascii) in crate::enc_table::ENCODINGS {
            names.push((n, ascii, dummy));
        }
        names
            .into_iter()
            .enumerate()
            .map(|(i, (n, ascii, dummy))| {
                let up = n.to_ascii_uppercase();
                let single = i == 0
                    || i == 2
                    || ["ISO-8859", "WINDOWS-125", "IBM", "CP85", "CP86", "KOI8", "MAC", "TIS-620", "ASCII"].iter().any(|p| up.starts_with(p));
                let (min, max) = if up.starts_with("UTF-16") {
                    (2, 4)
                } else if up.starts_with("UTF-32") {
                    (4, 4)
                } else if single {
                    (1, 1)
                } else {
                    (if ascii { 1 } else { 2 }, if up.contains("UTF") || up.contains("CESU") || up.contains("GB18030") || up.contains("EUC") { 4 } else { 2 })
                };
                let name = Box::leak(std::ffi::CString::new(n).unwrap().into_boxed_c_str()).as_ptr();
                KnEncoding { name, index: i as c_int, min_len: min, max_len: max, ascii_compat: ascii as c_int, dummy: dummy as c_int }
            })
            .collect()
    })
}
fn enc_ptr(i: c_int) -> *const KnEncoding {
    encodings().get(i.max(0) as usize).map_or(std::ptr::null(), |e| e as *const KnEncoding)
}
fn enc_index_of(e: crate::transcode::Enc) -> c_int {
    if e.codec == crate::transcode::Codec::Binary {
        return ENC_BINARY;
    }
    encodings().iter().position(|x| cstr(x.name) == e.name).map_or(ENC_BINARY, |i| i as c_int)
}
fn enc_at(i: c_int) -> Option<crate::transcode::Enc> {
    let e = encodings().get(usize::try_from(i).ok()?)?;
    crate::transcode::lookup(&cstr(e.name))
}
/// Give String `s` the encoding at index `idx`.
pub fn set_enc_index(s: V, idx: c_int) {
    if is_str(s) {
        if let Some(e) = enc_at(idx) {
            crate::transcode::set_enc(s, e);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rb_ascii8bit_encoding() -> *const KnEncoding {
    enc_ptr(ENC_BINARY)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_utf8_encoding() -> *const KnEncoding {
    enc_ptr(ENC_UTF8)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_usascii_encoding() -> *const KnEncoding {
    enc_ptr(ENC_USASCII)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_locale_encoding() -> *const KnEncoding {
    enc_ptr(ENC_UTF8)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_filesystem_encoding() -> *const KnEncoding {
    enc_ptr(ENC_UTF8)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_ascii8bit_encindex() -> c_int {
    ENC_BINARY
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_utf8_encindex() -> c_int {
    ENC_UTF8
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_usascii_encindex() -> c_int {
    ENC_USASCII
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_locale_encindex() -> c_int {
    ENC_UTF8
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_filesystem_encindex() -> c_int {
    ENC_UTF8
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_from_index(i: c_int) -> *const KnEncoding {
    if i < 0 { std::ptr::null() } else { enc_ptr(i) }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_find_index(name: *const c_char) -> c_int {
    crate::transcode::lookup(&cstr(name)).map_or(-1, enc_index_of)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_find(name: *const c_char) -> *const KnEncoding {
    rb_enc_from_index(rb_enc_find_index(name))
}
/// The encoding index of a String or Symbol (-1 for objects without one).
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_get_index(v: V) -> c_int {
    if is_str(v) {
        enc_index_of(crate::transcode::enc_of(v))
    } else if is_sym(v) {
        if sym_name(sym_id(v)).is_ascii() { ENC_USASCII } else { ENC_UTF8 }
    } else {
        -1
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_get(v: V) -> *const KnEncoding {
    rb_enc_from_index(rb_enc_get_index(v))
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_set_index(v: V, i: c_int) {
    set_enc_index(v, i)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_associate_index(v: V, i: c_int) -> V {
    set_enc_index(v, i);
    v
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_associate(v: V, e: *const KnEncoding) -> V {
    set_enc_index(v, if e.is_null() { ENC_BINARY } else { unsafe { (*e).index } });
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_copy(dst: V, src: V) {
    if is_str(dst) && is_str(src) {
        crate::transcode::set_enc(dst, crate::transcode::enc_of(src));
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_enc_from_encoding(e: *const KnEncoding) -> V {
    if e.is_null() {
        return QNIL;
    }
    match enc_at(unsafe { (*e).index }) {
        Some(x) => crate::transcode::encoding_obj(x),
        None => QNIL,
    }
}
/// An Encoding object or name → its index (-1: not an encoding).
#[unsafe(no_mangle)]
pub extern "C" fn rb_to_encoding_index(v: V) -> c_int {
    if !is_str(v) && !is_heap(v) {
        return -1;
    }
    match crate::transcode::enc_arg(v) {
        Ok(e) => enc_index_of(e),
        Err(_) => -1,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_str_coderange(s: V) -> c_int {
    const CR_7BIT: c_int = 1 << 20;
    const CR_VALID: c_int = 1 << 21;
    const CR_BROKEN: c_int = CR_7BIT | CR_VALID;
    let b = &rstr(s).b;
    if b.is_ascii() {
        return CR_7BIT;
    }
    match rb_enc_get_index(s) {
        ENC_UTF8 => {
            if std::str::from_utf8(b).is_ok() {
                CR_VALID
            } else {
                CR_BROKEN
            }
        }
        ENC_USASCII => CR_BROKEN,
        _ => {
            if send(s, sym("valid_encoding?"), &[], QNIL) == QFALSE {
                CR_BROKEN
            } else {
                CR_VALID
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_str_asciionly_p(s: V) -> c_int {
    rstr(s).b.is_ascii() as c_int
}
/// The encoding two Strings combine to, or null (rb_enc_check raises then).
#[unsafe(no_mangle)]
pub extern "C" fn rb_enc_compatible(a: V, b: V) -> *const KnEncoding {
    let (ia, ib) = (rb_enc_get_index(a), rb_enc_get_index(b));
    if ia < 0 || ib < 0 {
        return std::ptr::null();
    }
    if ia == ib {
        return enc_ptr(ia);
    }
    let ascii = |v: V| !is_str(v) || rstr(v).b.is_ascii();
    let compat = |i: c_int| encodings().get(i as usize).is_some_and(|e| e.ascii_compat != 0);
    if is_str(b) && rstr(b).b.is_empty() {
        return enc_ptr(ia);
    }
    if is_str(a) && rstr(a).b.is_empty() {
        return enc_ptr(if compat(ia) && ascii(b) { ia } else { ib });
    }
    if !compat(ia) || !compat(ib) {
        return std::ptr::null();
    }
    if ascii(b) {
        return enc_ptr(ia);
    }
    if ascii(a) {
        return enc_ptr(ib);
    }
    std::ptr::null()
}

// ---------- symbols ----------

/// The name of a Symbol as a frozen String (US-ASCII when it is ASCII, as CRuby's).
#[unsafe(no_mangle)]
pub extern "C" fn rb_sym2str(s: V) -> V {
    rb_id2str(sym_id(s) as V)
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_id2str(id: V) -> V {
    let b = sym_name(id as u32).to_vec();
    let ascii = b.is_ascii();
    let s = str_new(b);
    set_enc_index(s, if ascii { ENC_USASCII } else { ENC_UTF8 });
    hdr(s).flags |= FL_FROZEN;
    s
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_intern_str(s: V) -> V {
    intern(&rstr(s).b.clone()) as V
}

// ---------- numbers ----------

fn num_to_int(v: V) -> Option<V> {
    if is_int(v) {
        return Some(v);
    }
    if is_float(v) {
        let f = float_val(v);
        if f.is_nan() || f.is_infinite() {
            raise(rt().c.float_domain_error, &crate::fmt::inspect_lossy(v));
            return None;
        }
        return Some(int_from_f64(f.trunc()));
    }
    if v == QNIL {
        type_error("no implicit conversion from nil to integer");
        return None;
    }
    if is_heap(v) && !is_str(v) && respond_to(v, sym("to_int")) {
        let r = send0(v, "to_int");
        if r == UNDEF {
            return None;
        }
        if is_int(r) {
            return Some(r);
        }
        type_error(&format!("can't convert {} to Integer", class_name(real_class(v))));
        return None;
    }
    type_error(&format!("no implicit conversion of {} into Integer", describe_type(v)));
    None
}
fn describe_type(v: V) -> String {
    match v {
        QNIL => "nil".into(),
        QTRUE => "true".into(),
        QFALSE => "false".into(),
        _ => class_name(real_class(v)),
    }
}
fn int_from_f64(f: f64) -> V {
    if f.abs() < 4.0e18 {
        mkfix(f as i64)
    } else {
        use num_traits::FromPrimitive;
        match num_bigint::BigInt::from_f64(f) {
            Some(b) => int_from_big(b),
            None => mkfix(0),
        }
    }
}
/// NUM2ULONG: 0..ULONG_MAX, and negative values down to LONG_MIN (wrapped), as CRuby.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_num2ulong(v: V, ok: *mut c_int) -> u64 {
    use num_traits::ToPrimitive;
    unsafe { *ok = 1 };
    let Some(i) = num_to_int(v) else {
        unsafe { *ok = 0 };
        return 0;
    };
    if is_fix(i) {
        return fix(i) as u64;
    }
    let n = &rbig(i).n;
    if let Some(u) = n.to_u64() {
        return u;
    }
    if let Some(s) = n.to_i64() {
        return s as u64;
    }
    raise(rt().c.range_error, "bignum out of range of unsigned long");
    unsafe { *ok = 0 };
    0
}
/// 1 for a positive (or zero) Integer, 0 for a negative one.
#[unsafe(no_mangle)]
pub extern "C" fn rb_big_sign(v: V) -> c_int {
    if is_fix(v) {
        return (fix(v) >= 0) as c_int;
    }
    (rbig(v).n.sign() != num_bigint::Sign::Minus) as c_int
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_big2ull(v: V, ok: *mut c_int) -> u64 {
    use num_traits::ToPrimitive;
    unsafe { *ok = 1 };
    if is_fix(v) {
        return fix(v) as u64;
    }
    if is_big(v) {
        let n = &rbig(v).n;
        if let Some(u) = n.to_u64() {
            return u;
        }
        if let Some(s) = n.to_i64() {
            return s as u64;
        }
    }
    raise(rt().c.range_error, "bignum too big to convert into 'unsigned long long'");
    unsafe { *ok = 0 };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_capi_big2ll(v: V, ok: *mut c_int) -> i64 {
    use num_traits::ToPrimitive;
    unsafe { *ok = 1 };
    if is_fix(v) {
        return fix(v);
    }
    if let Some(s) = is_big(v).then(|| rbig(v).n.to_i64()).flatten() {
        return s;
    }
    raise(rt().c.range_error, "bignum too big to convert into 'long long'");
    unsafe { *ok = 0 };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_big2dbl(v: V) -> f64 {
    use num_traits::ToPrimitive;
    if is_fix(v) {
        return fix(v) as f64;
    }
    rbig(v).n.to_f64().unwrap_or(f64::INFINITY)
}
/// The bytes |v| takes, and (in *nlz) the unused high bits of the top byte.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rb_absint_size(v: V, nlz: *mut c_int) -> usize {
    let bits = if is_fix(v) { 64 - fix(v).unsigned_abs().leading_zeros() as u64 } else { rbig(v).n.bits() };
    let bytes = bits.div_ceil(8);
    if !nlz.is_null() {
        unsafe { *nlz = (bytes * 8 - bits) as c_int };
    }
    bytes as usize
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_absint_singlebit_p(v: V) -> c_int {
    if is_fix(v) {
        return fix(v).unsigned_abs().is_power_of_two() as c_int;
    }
    let m = rbig(v).n.magnitude();
    (m.count_ones() == 1) as c_int
}

// ---------- classes, methods, objects ----------

#[unsafe(no_mangle)]
pub extern "C" fn rb_singleton_class(v: V) -> V {
    singleton_class(v) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_class_real(c: V) -> V {
    let mut k = cls(c);
    while !k.is_null() && unsafe { (*k).is_singleton } {
        k = unsafe { (*k).superclass };
    }
    k as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_class_new(sup: V) -> V {
    new_class("", cls(sup)) as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_module_new() -> V {
    new_module("") as V
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_define_method_id(klass: V, id: V, f: *const c_void, argc: c_int) {
    add_method(cls(klass), id as u32, Body::CFunc(f, argc));
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_undef(klass: V, id: V) {
    add_method(cls(klass), id as u32, Body::Undefined);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_undef_method(klass: V, name: *const c_char) {
    add_method(cls(klass), sym(&cstr(name)), Body::Undefined);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_alias(klass: V, new: V, old: V) {
    crate::abi::alias(cls(klass), new as u32, old as u32);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_attr(klass: V, id: V, read: c_int, write: c_int, _ex: c_int) {
    let n = String::from_utf8_lossy(sym_name(id as u32)).into_owned();
    let c = std::ffi::CString::new(n).unwrap_or_default();
    rb_define_attr(klass, c.as_ptr(), read, write);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_const_set(klass: V, id: V, v: V) {
    const_set(cls(klass), id as u32, v);
}
#[unsafe(no_mangle)]
pub extern "C" fn rb_obj_respond_to(v: V, id: V, _private: c_int) -> c_int {
    respond_to(v, id as u32) as c_int
}
/// Whether `v` is a Hash passed as keyword arguments.
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_is_kwhash(v: V) -> c_int {
    (is_hash(v) && hdr(v).flags & FL_KWHASH != 0) as c_int
}
/// Mark a Hash as keyword arguments (rb_funcallv_kw and friends).
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_kwhash(v: V) -> V {
    if is_hash(v) {
        hdr(v).flags |= FL_KWHASH;
    }
    v
}

// ---------- blocks ----------

/// A Proc whose body is the C function `func` (rb_block_call_func: yielded_arg, data2, argc,
/// argv, blockarg), called through the C trampoline.
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_proc_new(func: *const c_void, data2: V) -> V {
    let f = func as usize;
    let p = proc_new_native_rooted(
        Box::new(move |_s, args, blk| {
            let mut a = args.to_vec();
            a.push(blk);
            crate::core::call_native_c(f as *const c_void, -3, data2, &a)
        }),
        0,
        -1,
        vec![data2],
    );
    rproc(p).autosplat = false;
    p
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_push_block(b: V) {
    pst().current_block.push(b);
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_pop_block() {
    pst().current_block.pop();
}

/// rb_protect / rb_ensure: set aside a non-exception unwind (break, throw, ...) in flight, as a
/// handle; kn_capi_restore_throw resumes it (rb_jump_tag).
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_save_throw() -> V {
    match std::mem::replace(&mut pst().pending, Throw::None) {
        Throw::None => 0,
        t => crate::gc::save_throw(t) as V,
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_restore_throw(h: V) {
    if h != 0 {
        let t = crate::gc::take_throw(h as u64);
        if !matches!(t, Throw::None) {
            pst().pending = t;
        }
    }
}

// ---------- Integer <-> words (rb_integer_pack / rb_integer_unpack) ----------

fn int_of(v: V) -> Option<num_bigint::BigInt> {
    if is_fix(v) {
        Some(num_bigint::BigInt::from(fix(v)))
    } else if is_big(v) {
        Some(rbig(v).n.clone())
    } else {
        None
    }
}
const IP_MSWORD_FIRST: c_int = 0x01;
const IP_MSBYTE_FIRST: c_int = 0x10;
const IP_NATIVE: c_int = 0x40;
const IP_2COMP: c_int = 0x80;
const IP_NEGATIVE: c_int = 0x200;
/// Index in the buffer of little-endian byte `i` of the whole number.
fn ip_pos(i: usize, wordsize: usize, numwords: usize, flags: c_int) -> usize {
    let (w, b) = (i / wordsize, i % wordsize);
    let w = if flags & IP_MSWORD_FIRST != 0 { numwords - 1 - w } else { w };
    let msbyte = flags & IP_MSBYTE_FIRST != 0 || (flags & IP_NATIVE != 0 && cfg!(target_endian = "big"));
    let b = if msbyte { wordsize - 1 - b } else { b };
    w * wordsize + b
}
/// CRuby's validate_integer_pack_format: an ArgumentError (pending) when the flags don't say
/// the word and byte order the sizes need.
fn ip_check(numwords: usize, wordsize: usize, flags: c_int) -> bool {
    let wo = flags & (IP_MSWORD_FIRST | 0x02);
    let bo = flags & (IP_MSBYTE_FIRST | 0x20 | IP_NATIVE);
    let msg = if wo == (IP_MSWORD_FIRST | 0x02) {
        "unexpected word order"
    } else if wo == 0 && numwords > 1 {
        "word order not specified"
    } else if bo.count_ones() > 1 {
        "unexpected byte order"
    } else if bo == 0 && wordsize > 1 {
        "byte order not specified"
    } else if wordsize == 0 {
        "too small wordsize"
    } else {
        return true;
    };
    arg_error(msg);
    false
}
/// rb_integer_pack (kenny_capi.c): *ok = 0 with an exception pending.
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_integer_pack(val: V, words: *mut u8, numwords: usize, wordsize: usize, _nails: usize, flags: c_int, ok: *mut c_int) -> c_int {
    use num_bigint::Sign;
    unsafe { *ok = 0 };
    if !ip_check(numwords, wordsize, flags) {
        return 0;
    }
    // (the C side has converted it with rb_to_int)
    let Some(n) = int_of(val) else { return 0 };
    unsafe { *ok = 1 };
    let total = numwords * wordsize;
    let sign: c_int = match n.sign() {
        Sign::Minus => -1,
        Sign::NoSign => 0,
        Sign::Plus => 1,
    };
    let mag = n.magnitude().to_bytes_le();
    let mut bytes = vec![0u8; total];
    let mut overflow = mag.len() > total && !(mag.len() == 1 && mag[0] == 0);
    for (i, &b) in mag.iter().enumerate().take(total) {
        bytes[i] = b;
    }
    if flags & IP_2COMP != 0 && sign < 0 {
        // two's complement: invert and add one
        let mut carry = 1u16;
        for b in bytes.iter_mut() {
            let x = (!*b) as u16 + carry;
            *b = x as u8;
            carry = x >> 8;
        }
        // fits when -2**(8*total-1) <= n
        if !overflow && total > 0 && bytes[total - 1] & 0x80 == 0 && mag.iter().any(|&b| b != 0) {
            overflow = true;
        }
    }
    let out = unsafe { std::slice::from_raw_parts_mut(words, total) };
    for (i, &b) in bytes.iter().enumerate() {
        out[ip_pos(i, wordsize, numwords, flags)] = b;
    }
    if overflow { sign * 2 } else { sign }
}
/// rb_integer_unpack (kenny_capi.c): UNDEF with an exception pending.
#[unsafe(no_mangle)]
pub extern "C" fn kn_capi_integer_unpack(words: *const u8, numwords: usize, wordsize: usize, _nails: usize, flags: c_int) -> V {
    if !ip_check(numwords, wordsize, flags) {
        return UNDEF;
    }
    let total = numwords * wordsize;
    let src = unsafe { std::slice::from_raw_parts(words, total) };
    let mut bytes = vec![0u8; total];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = src[ip_pos(i, wordsize, numwords, flags)];
    }
    let mut n = num_bigint::BigInt::from_bytes_le(num_bigint::Sign::Plus, &bytes);
    if flags & IP_2COMP != 0 {
        if total > 0 && bytes[total - 1] & 0x80 != 0 {
            n -= num_bigint::BigInt::from(1) << (8 * total);
        } else if flags & IP_NEGATIVE != 0 && n == num_bigint::BigInt::from(0) {
            n = -(num_bigint::BigInt::from(1) << (8 * total));
        }
    } else if flags & IP_NEGATIVE != 0 {
        n = -n;
    }
    int_from_big(n)
}
