//! Builds the core class hierarchy and registers the built-in methods.

use crate::core::*;

fn make_meta(c: *mut Class, sup: *mut Class) {
    let meta = new_class_raw("", sup, false);
    unsafe {
        (*meta).is_singleton = true;
        (*meta).attached = c as V;
        (*meta).h.klass = rt().c.class;
        (*c).h.klass = meta;
    }
}

fn class(name: &str, sup: *mut Class) -> *mut Class {
    let c = new_class(name, sup);
    set_global_const(name, c as V);
    c
}
fn module(name: &str) -> *mut Class {
    let m = new_module(name);
    set_global_const(name, m as V);
    m
}

pub fn boot() {
    crate::gc::init_stats();
    let r = rt();
    let null = std::ptr::null_mut();
    let bo = new_class_raw("BasicObject", null, false);
    let ob = new_class_raw("Object", bo, false);
    let md = new_class_raw("Module", ob, false);
    let cl = new_class_raw("Class", md, false);
    r.c.basic_object = bo;
    r.c.object = ob;
    r.c.module = md;
    r.c.class = cl;
    make_meta(bo, cl);
    make_meta(ob, unsafe { (*bo).h.klass });
    make_meta(md, unsafe { (*ob).h.klass });
    make_meta(cl, unsafe { (*md).h.klass });
    for c in [bo, ob, md, cl] {
        set_global_const(&unsafe { (*c).name.clone() }, c as V);
    }

    r.c.kernel = module("Kernel");
    unsafe { (*ob).includes.push(r.c.kernel) };
    r.c.comparable = module("Comparable");
    r.c.enumerable = module("Enumerable");

    r.c.nil = class("NilClass", ob);
    r.c.true_ = class("TrueClass", ob);
    r.c.false_ = class("FalseClass", ob);
    r.c.numeric = class("Numeric", ob);
    unsafe { (*r.c.numeric).includes.push(r.c.comparable) };
    r.c.integer = class("Integer", r.c.numeric);
    r.c.float = class("Float", r.c.numeric);
    r.c.string = class("String", ob);
    unsafe {
        (*r.c.string).alloc = Kind::String;
        (*r.c.string).includes.push(r.c.comparable);
    }
    r.c.symbol = class("Symbol", ob);
    unsafe { (*r.c.symbol).includes.push(r.c.comparable) };
    r.c.array = class("Array", ob);
    unsafe {
        (*r.c.array).alloc = Kind::Array;
        (*r.c.array).includes.push(r.c.enumerable);
    }
    r.c.hash = class("Hash", ob);
    unsafe {
        (*r.c.hash).alloc = Kind::Hash;
        (*r.c.hash).includes.push(r.c.enumerable);
    }
    r.c.range = class("Range", ob);
    unsafe { (*r.c.range).includes.push(r.c.enumerable) };
    r.c.proc_ = class("Proc", ob);
    r.c.enumerator = class("Enumerator", ob);
    unsafe { (*r.c.enumerator).includes.push(r.c.enumerable) };

    r.c.exception = class("Exception", ob);
    r.c.script_error = class("ScriptError", r.c.exception);
    r.c.not_implemented_error = class("NotImplementedError", r.c.script_error);
    class("SyntaxError", r.c.script_error);
    class("LoadError", r.c.script_error);
    r.c.standard_error = class("StandardError", r.c.exception);
    let nmp = class("NoMatchingPatternError", r.c.standard_error);
    r.c.runtime_error = class("RuntimeError", r.c.standard_error);
    r.c.frozen_error = class("FrozenError", r.c.runtime_error);
    r.c.argument_error = class("ArgumentError", r.c.standard_error);
    r.c.name_error = class("NameError", r.c.standard_error);
    r.c.no_method_error = class("NoMethodError", r.c.name_error);
    r.c.type_error = class("TypeError", r.c.standard_error);
    r.c.zero_div_error = class("ZeroDivisionError", r.c.standard_error);
    r.c.index_error = class("IndexError", r.c.standard_error);
    r.c.key_error = class("KeyError", r.c.index_error);
    r.c.stop_iteration = class("StopIteration", r.c.index_error);
    r.c.range_error = class("RangeError", r.c.standard_error);
    r.c.float_domain_error = class("FloatDomainError", r.c.range_error);
    r.c.local_jump_error = class("LocalJumpError", r.c.standard_error);
    r.c.io_error = class("IOError", r.c.standard_error);
    class("EOFError", r.c.io_error);
    r.c.uncaught_throw_error = class("UncaughtThrowError", r.c.argument_error);
    class("NoMatchingPatternKeyError", nmp);
    class("FiberError", r.c.standard_error);
    class("ThreadError", r.c.standard_error);
    class("EncodingError", r.c.standard_error);
    class("SecurityError", r.c.exception);
    class("NoMemoryError", r.c.exception);
    let signal_exception = class("SignalException", r.c.exception);
    class("Interrupt", signal_exception);
    class("SystemExit", r.c.exception);
    r.c.system_stack_error = class("SystemStackError", r.c.exception);
    let errno = module("Errno");
    let sce = class("SystemCallError", r.c.standard_error);
    let enoent = new_class("Errno::ENOENT", sce);
    const_set(errno, sym("ENOENT"), enoent as V);

    r.c.struct_ = class("Struct", ob);
    unsafe { (*r.c.struct_).includes.push(r.c.enumerable) };
    r.c.time = class("Time", ob);
    unsafe { (*r.c.time).includes.push(r.c.comparable) };
    r.c.io = class("IO", ob);
    r.c.mutex = class("Mutex", ob);

    let main = object_new(ob);
    r.main_obj = main;

    crate::kernel::init();
    crate::numeric::init();
    crate::string::init();
    crate::array::init();
    crate::hash::init();
    crate::misc::init();
    crate::io::init();
    crate::server::init();
    crate::regex::init();
    crate::lazy::init();
    crate::thread::init();
    crate::capi::init();
    crate::compat::init();
    crate::openssl::init();
    crate::json::init();
    crate::strscan::init();
    crate::digest::init();
    crate::zlib::init();
    crate::date::init();
    crate::yaml::init();
    crate::bigdecimal::init();
    crate::nokogiri::init();
    crate::pack::init();
    crate::extra::init();
    crate::methodobj::init();
    crate::visibility::init();
    crate::rational::init();
    crate::rational::init_complex();
    crate::marshal::init();
    crate::cgi_escape::init();
    crate::sqlite::init();
    crate::socket::init();
    crate::misc::blockless_enumerators();
    crate::opt::init();
    crate::gc::arm_malloc_limit();
}
