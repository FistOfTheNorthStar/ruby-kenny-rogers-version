//! The native half of the `sqlite3` gem (1.x: `ext/sqlite3/*.c`), in Rust on top of the
//! bundled SQLite C library (`libsqlite3-sys`, the sys crate behind rusqlite).
//!
//! `require "sqlite3/sqlite3_native"` defines what the C extension defines — `SQLite3::Database`
//! (open_v2, close, prepare support, exec_batch, functions, aggregators, ...),
//! `SQLite3::Statement` (step/bind/reset/columns), `SQLite3::Backup`, `SQLite3::Blob` and
//! `SQLite3::Constants::Open` — and the gem's own Ruby files (database.rb, statement.rb,
//! resultset.rb, errors.rb, ...) load on top, exactly as with CRuby.
//!
//! Native handles live in hidden `@__db` / `@__st` / `@__bk` ivars holding a Data object whose
//! free function finalizes/closes the handle. Ruby callbacks (functions, aggregators, busy
//! handler, authorizer, trace, collations) run from inside SQLite; an exception raised there is
//! left pending, SQLite is told the callback failed, and the native method that drove SQLite
//! (step, prepare, exec_batch) then returns it — the C extension's `rb_errinfo` re-raise.

use crate::core::*;
use crate::gc::RVec;
use crate::tri;
use libsqlite3_sys as ffi;
use std::ffi::{CStr, CString, c_char, c_int, c_void};

// libsqlite3-sys leaves this one out of its bindings (rusqlite wraps it differently).
unsafe extern "C" {
    fn sqlite3_close_v2(db: *mut ffi::sqlite3) -> c_int;
}

static M_SQLITE3: G<*mut Class> = G::new(std::ptr::null_mut());
static C_DATABASE: G<*mut Class> = G::new(std::ptr::null_mut());
static C_STATEMENT: G<*mut Class> = G::new(std::ptr::null_mut());
static C_BLOB: G<*mut Class> = G::new(std::ptr::null_mut());

// ---------- native state ----------

struct Db {
    db: *mut ffi::sqlite3,
    /// The SQLite3::Database object (alive while SQLite can call back into it).
    self_v: V,
    /// A Ruby callback raised; the exception is pending in the runtime.
    exc: bool,
    /// busy_timeout in ms (see `timeout_cb`).
    busy_ms: i64,
    busy_since: Option<std::time::Instant>,
    /// Ruby code may run inside SQLite calls (functions, aggregates, trace, authorizer, busy
    /// handler blocks). Until then a query is plain C: it runs as a blocking call (gc::blocking),
    /// so a slow query holds up neither a collection nor the other threads of this P.
    ruby_cb: bool,
    /// statement_timeout= in ms (0: none) and the running statement's deadline (see `stmt_timeout_cb`)
    stmt_timeout_ms: i64,
    stmt_deadline: Option<std::time::Instant>,
    /// closed by `discard` (a forked child's inherited connection)
    discarded: bool,
}

struct Stmt {
    st: *mut ffi::sqlite3_stmt,
    done: bool,
}

struct Bk {
    p: *mut ffi::sqlite3_backup,
}

unsafe extern "C" fn free_db(p: *mut c_void) {
    let d = unsafe { Box::from_raw(p as *mut Db) };
    if !d.db.is_null() {
        // v2: becomes a zombie until statements still alive in the heap are finalized
        unsafe { sqlite3_close_v2(d.db) };
    }
}
unsafe extern "C" fn free_stmt(p: *mut c_void) {
    let s = unsafe { Box::from_raw(p as *mut Stmt) };
    if !s.st.is_null() {
        unsafe { ffi::sqlite3_finalize(s.st) };
    }
}
unsafe extern "C" fn free_bk(p: *mut c_void) {
    let b = unsafe { Box::from_raw(p as *mut Bk) };
    if !b.p.is_null() {
        unsafe { ffi::sqlite3_backup_finish(b.p) };
    }
}

fn wrap<T>(t: T, free: unsafe extern "C" fn(*mut c_void)) -> V {
    alloc(RData {
        h: Header { klass: rt().c.object, kind: Kind::Data, flags: 0 },
        data: Box::into_raw(Box::new(t)) as *mut c_void,
        dmark: None,
        dfree: Some(free),
        roots: Vec::new(),
    })
}
fn data_of<T>(v: V, ivar: &str) -> Option<&'static mut T> {
    let d = ivar_get(v, sym(ivar));
    if is_kind(d, Kind::Data) { Some(unsafe { &mut *((*(d as *mut RData)).data as *mut T) }) } else { None }
}
/// The Database's state, created on first use (like the C ext's alloc func).
fn db_of(v: V) -> &'static mut Db {
    if let Some(d) = data_of::<Db>(v, "@__db") {
        return d;
    }
    ivar_set(v, sym("@__db"), wrap(Db { db: std::ptr::null_mut(), self_v: v, exc: false, busy_ms: 0, busy_since: None, ruby_cb: false, stmt_timeout_ms: 0, stmt_deadline: None, discarded: false }, free_db));
    data_of::<Db>(v, "@__db").unwrap()
}
fn stmt_of(v: V) -> &'static mut Stmt {
    if let Some(s) = data_of::<Stmt>(v, "@__st") {
        return s;
    }
    ivar_set(v, sym("@__st"), wrap(Stmt { st: std::ptr::null_mut(), done: false }, free_stmt));
    data_of::<Stmt>(v, "@__st").unwrap()
}
fn bk_of(v: V) -> &'static mut Bk {
    if let Some(b) = data_of::<Bk>(v, "@__bk") {
        return b;
    }
    ivar_set(v, sym("@__bk"), wrap(Bk { p: std::ptr::null_mut() }, free_bk));
    data_of::<Bk>(v, "@__bk").unwrap()
}

// ---------- helpers ----------

fn sqlite_class(name: &str) -> *mut Class {
    match const_get_under(*M_SQLITE3.get(), sym(name)) {
        Some(c) if is_class(c) => c as *mut Class,
        _ => rt().c.runtime_error,
    }
}
fn sq_exception(msg: &str) -> V {
    raise(sqlite_class("Exception"), msg)
}
fn cstr_to_string(p: *const c_char) -> Option<String> {
    if p.is_null() { None } else { Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()) }
}
fn cstr_bytes(p: *const c_char) -> Option<Vec<u8>> {
    if p.is_null() { None } else { Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()) }
}
fn str_bytes(v: V) -> Vec<u8> {
    rstr(v).b.clone()
}
fn bin_str(b: Vec<u8>) -> V {
    let s = str_new(b);
    hdr(s).flags |= FL_BINARY;
    s
}
fn is_binary(v: V) -> bool {
    hdr(v).flags & FL_BINARY != 0
}
fn cstring(b: &[u8]) -> CString {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    CString::new(&b[..end]).unwrap()
}
/// `StringValue(v)`: a String, or `to_str` of it.
fn string_value(v: V) -> V {
    if is_str(v) {
        return v;
    }
    if respond_to(v, sym("to_str")) {
        let s = tri!(send(v, sym("to_str"), &[], QNIL));
        if is_str(s) {
            return s;
        }
    }
    type_error(&format!("no implicit conversion of {} into String", type_name(v)))
}
fn type_name(v: V) -> String {
    if v == QNIL {
        "nil".into()
    } else if v == QTRUE {
        "true".into()
    } else if v == QFALSE {
        "false".into()
    } else {
        class_name(real_class(v))
    }
}
/// `NUM2INT`.
fn num2int(v: V) -> Result<i64, V> {
    if is_fix(v) {
        return Ok(fix(v));
    }
    if is_float(v) {
        return Ok(float_val(v) as i64);
    }
    if v == QNIL {
        return Err(type_error("no implicit conversion from nil to integer"));
    }
    if is_big(v) {
        return Err(raise(rt().c.range_error, "bignum too big to convert into `int'"));
    }
    if respond_to(v, sym("to_int")) {
        let r = send(v, sym("to_int"), &[], QNIL);
        if r == UNDEF {
            return Err(UNDEF);
        }
        if is_fix(r) {
            return Ok(fix(r));
        }
    }
    Err(type_error(&format!("no implicit conversion of {} into Integer", type_name(v))))
}
fn int_v(i: i64) -> V {
    int_from_i128(i as i128)
}

/// `rb_sqlite3_raise`: the exception class for a result code, message from `sqlite3_errmsg`.
fn check(db: *mut ffi::sqlite3, status: c_int) -> V {
    if status == ffi::SQLITE_OK {
        return QNIL;
    }
    let name = match status & 0xff {
        ffi::SQLITE_ERROR => "SQLException",
        ffi::SQLITE_INTERNAL => "InternalException",
        ffi::SQLITE_PERM => "PermissionException",
        ffi::SQLITE_ABORT => "AbortException",
        ffi::SQLITE_BUSY => "BusyException",
        ffi::SQLITE_LOCKED => "LockedException",
        ffi::SQLITE_NOMEM => "MemoryException",
        ffi::SQLITE_READONLY => "ReadOnlyException",
        ffi::SQLITE_INTERRUPT => "InterruptException",
        ffi::SQLITE_IOERR => "IOException",
        ffi::SQLITE_CORRUPT => "CorruptException",
        ffi::SQLITE_NOTFOUND => "NotFoundException",
        ffi::SQLITE_FULL => "FullException",
        ffi::SQLITE_CANTOPEN => "CantOpenException",
        ffi::SQLITE_PROTOCOL => "ProtocolException",
        ffi::SQLITE_EMPTY => "EmptyException",
        ffi::SQLITE_SCHEMA => "SchemaChangedException",
        ffi::SQLITE_TOOBIG => "TooBigException",
        ffi::SQLITE_CONSTRAINT => "ConstraintException",
        ffi::SQLITE_MISMATCH => "MismatchException",
        ffi::SQLITE_MISUSE => "MisuseException",
        ffi::SQLITE_NOLFS => "UnsupportedException",
        ffi::SQLITE_AUTH => "AuthorizationException",
        ffi::SQLITE_FORMAT => "FormatException",
        ffi::SQLITE_RANGE => "RangeException",
        ffi::SQLITE_NOTADB => "NotADatabaseException",
        _ => "",
    };
    let cls = if name.is_empty() { rt().c.runtime_error } else { sqlite_class(name) };
    let msg = cstr_to_string(unsafe { ffi::sqlite3_errmsg(db) }).unwrap_or_else(|| "out of memory".into());
    let e = exc_new(cls, &msg);
    ivar_set(e, sym("@code"), mkfix(status as i64));
    raise_exc(e)
}

fn require_open_db(s: V) -> Result<&'static mut Db, V> {
    let d = db_of(s);
    if d.db.is_null() { Err(sq_exception("cannot use a closed database")) } else { Ok(d) }
}
macro_rules! open_db {
    ($s:expr) => {
        match require_open_db($s) {
            Ok(d) => d,
            Err(e) => return e,
        }
    };
}
fn require_open_stmt(s: V) -> Result<&'static mut Stmt, V> {
    let st = stmt_of(s);
    if st.st.is_null() { Err(sq_exception("cannot use a closed statement")) } else { Ok(st) }
}
macro_rules! open_stmt {
    ($s:expr) => {
        match require_open_stmt($s) {
            Ok(d) => d,
            Err(e) => return e,
        }
    };
}

/// A callback into Ruby raised earlier: hand that exception back (it is still pending).
fn take_exc(d: &mut Db) -> bool {
    if d.exc {
        d.exc = false;
        if pst().pending_is_set() {
            return true;
        }
    }
    false
}

// ---------- values between SQLite and Ruby ----------

/// `sqlite3val2rb` (function/aggregate arguments).
unsafe fn value_to_rb(val: *mut ffi::sqlite3_value) -> V {
    unsafe {
        match ffi::sqlite3_value_type(val) {
            ffi::SQLITE_INTEGER => int_v(ffi::sqlite3_value_int64(val)),
            ffi::SQLITE_FLOAT => float_new(ffi::sqlite3_value_double(val)),
            ffi::SQLITE_TEXT => {
                let p = ffi::sqlite3_value_text(val);
                let n = ffi::sqlite3_value_bytes(val) as usize;
                str_new(if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() })
            }
            ffi::SQLITE_BLOB => {
                let n = ffi::sqlite3_value_bytes(val) as usize;
                let p = ffi::sqlite3_value_blob(val) as *const u8;
                bin_str(if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() })
            }
            _ => QNIL,
        }
    }
}

/// `set_sqlite3_func_result`; Err(message) for an unsupported Ruby type.
unsafe fn set_result(ctx: *mut ffi::sqlite3_context, r: V) -> Result<(), String> {
    unsafe {
        if r == QNIL {
            ffi::sqlite3_result_null(ctx);
        } else if is_fix(r) {
            ffi::sqlite3_result_int64(ctx, fix(r));
        } else if is_big(r) || is_float(r) {
            let f = if is_float(r) {
                float_val(r)
            } else {
                use num_traits::ToPrimitive;
                rbig(r).n.to_f64().unwrap_or(f64::NAN)
            };
            ffi::sqlite3_result_double(ctx, f);
        } else if is_str(r) {
            let b = &rstr(r).b;
            if real_class(r) == *C_BLOB.get() || is_binary(r) {
                ffi::sqlite3_result_blob(ctx, b.as_ptr() as *const c_void, b.len() as c_int, ffi::SQLITE_TRANSIENT());
            } else {
                ffi::sqlite3_result_text(ctx, b.as_ptr() as *const c_char, b.len() as c_int, ffi::SQLITE_TRANSIENT());
            }
        } else {
            return Err(format!("can't return {}", class_name(real_class(r))));
        }
    }
    Ok(())
}

// ---------- callbacks ----------

/// A function's user data: the db (for exception bookkeeping) and the block.
struct FuncData {
    db: *mut Db,
    block: V,
}

unsafe extern "C" fn func_cb(ctx: *mut ffi::sqlite3_context, argc: c_int, argv: *mut *mut ffi::sqlite3_value) {
    unsafe {
        let fd = &*(ffi::sqlite3_user_data(ctx) as *const FuncData);
        let d = &mut *fd.db;
        if d.exc {
            ffi::sqlite3_result_error(ctx, c"Ruby Exception occurred".as_ptr(), -1);
            return;
        }
        let mut args = RVec::with_capacity(argc as usize);
        for i in 0..argc as usize {
            args.push(value_to_rb(*argv.add(i)));
        }
        let r = send(fd.block, sym("call"), &args, QNIL);
        if r == UNDEF {
            d.exc = true;
            ffi::sqlite3_result_error(ctx, c"Ruby Exception occurred".as_ptr(), -1);
            return;
        }
        if let Err(m) = set_result(ctx, r) {
            raise(rt().c.runtime_error, &m);
            d.exc = true;
            ffi::sqlite3_result_error(ctx, c"Ruby Exception occurred".as_ptr(), -1);
        }
    }
}
unsafe extern "C" fn free_funcdata(p: *mut c_void) {
    drop(unsafe { Box::from_raw(p as *mut FuncData) });
}

/// define_aggregator2's wrapper: the handler class, and its live instances (rooted through the
/// Database's `@__aggregators` array, which holds `instances`).
struct AggData {
    db: *mut Db,
    klass: V,
    instances: V,
}
unsafe extern "C" fn free_aggdata(p: *mut c_void) {
    drop(unsafe { Box::from_raw(p as *mut AggData) });
}
/// The aggregate context holds a Ruby array [handler instance, failed?] (rooted in `instances`).
unsafe fn agg_instance(ctx: *mut ffi::sqlite3_context) -> Option<V> {
    unsafe {
        let ad = &*(ffi::sqlite3_user_data(ctx) as *const AggData);
        let slot = ffi::sqlite3_aggregate_context(ctx, std::mem::size_of::<V>() as c_int) as *mut V;
        if slot.is_null() {
            return None;
        }
        if *slot == 0 {
            let d = &mut *ad.db;
            let inst = if d.exc { UNDEF } else { send(ad.klass, sym("new"), &[], QNIL) };
            let rec = if inst == UNDEF {
                d.exc = true;
                ary_new(vec![QNIL, QTRUE])
            } else {
                ary_new(vec![inst, QFALSE])
            };
            rary(ad.instances).v.push(rec);
            *slot = rec;
        }
        Some(*slot)
    }
}
unsafe extern "C" fn agg_step(ctx: *mut ffi::sqlite3_context, argc: c_int, argv: *mut *mut ffi::sqlite3_value) {
    unsafe {
        let Some(rec) = agg_instance(ctx) else { return };
        let ad = &*(ffi::sqlite3_user_data(ctx) as *const AggData);
        let d = &mut *ad.db;
        if truthy(rary(rec).v[1]) || d.exc {
            return;
        }
        let mut args = RVec::with_capacity(argc as usize);
        for i in 0..argc as usize {
            args.push(value_to_rb(*argv.add(i)));
        }
        let r = send(rary(rec).v[0], sym("step"), &args, QNIL);
        if r == UNDEF {
            d.exc = true;
            rary(rec).v[1] = QTRUE;
        }
    }
}
unsafe extern "C" fn agg_final(ctx: *mut ffi::sqlite3_context) {
    unsafe {
        let ad = &*(ffi::sqlite3_user_data(ctx) as *const AggData);
        let d = &mut *ad.db;
        let Some(rec) = agg_instance(ctx) else { return };
        let mut failed = truthy(rary(rec).v[1]) || d.exc;
        if !failed {
            let r = send(rary(rec).v[0], sym("finalize"), &[], QNIL);
            if r == UNDEF {
                d.exc = true;
                failed = true;
            } else if let Err(m) = set_result(ctx, r) {
                raise(rt().c.runtime_error, &m);
                d.exc = true;
                failed = true;
            }
        }
        if failed {
            ffi::sqlite3_result_error(ctx, c"Ruby Exception occurred".as_ptr(), -1);
        }
        let inst = &mut rary(ad.instances).v;
        if let Some(i) = inst.iter().position(|&x| x == rec) {
            inst.remove(i);
        }
    }
}

unsafe extern "C" fn trace_cb(p: *mut c_void, sql: *const c_char) {
    unsafe {
        let d = &mut *(p as *mut Db);
        if d.exc {
            return;
        }
        let f = ivar_get(d.self_v, sym("@tracefunc"));
        let s = str_new(cstr_bytes(sql).unwrap_or_default());
        if send(f, sym("call"), &[s], QNIL) == UNDEF {
            d.exc = true;
        }
    }
}
unsafe extern "C" fn busy_cb(p: *mut c_void, count: c_int) -> c_int {
    unsafe {
        let d = &mut *(p as *mut Db);
        if d.exc {
            return 0;
        }
        let h = ivar_get(d.self_v, sym("@busy_handler"));
        let r = send(h, sym("call"), &[mkfix(count as i64)], QNIL);
        if r == UNDEF {
            d.exc = true;
            return 0;
        }
        if r == QFALSE { 0 } else { 1 }
    }
}
unsafe extern "C" fn auth_cb(p: *mut c_void, action: c_int, a: *const c_char, b: *const c_char, c: *const c_char, e: *const c_char) -> c_int {
    unsafe {
        let d = &mut *(p as *mut Db);
        if d.exc {
            return ffi::SQLITE_DENY;
        }
        let s = |x: *const c_char| cstr_bytes(x).map(str_new).unwrap_or(QNIL);
        let args = RVec::from(vec![mkfix(action as i64), s(a), s(b), s(c), s(e)]);
        let cb = ivar_get(d.self_v, sym("@authorizer"));
        let r = send(cb, sym("call"), &args, QNIL);
        if r == UNDEF {
            d.exc = true;
            return ffi::SQLITE_DENY;
        }
        if is_fix(r) {
            fix(r) as c_int
        } else if r == QTRUE {
            ffi::SQLITE_OK
        } else if r == QFALSE {
            ffi::SQLITE_DENY
        } else {
            ffi::SQLITE_IGNORE
        }
    }
}
/// `busy_timeout=`: like sqlite3_busy_timeout, but waiting with Kenny's scheduler-aware sleep, so
/// other Ruby threads of this process (perhaps the one holding the lock) keep running.
unsafe extern "C" fn timeout_cb(p: *mut c_void, count: c_int) -> c_int {
    let d = unsafe { &mut *(p as *mut Db) };
    let now = std::time::Instant::now();
    if count == 0 || d.busy_since.is_none() {
        d.busy_since = Some(now);
    }
    let waited = now.duration_since(d.busy_since.unwrap()).as_millis() as i64;
    if waited >= d.busy_ms || d.exc {
        return 0;
    }
    // SQLite's own schedule: 1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100 ms
    const DELAYS: [i64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
    let delay = DELAYS[(count as usize).min(DELAYS.len() - 1)].min(d.busy_ms - waited).max(1);
    if crate::thread::sleep(Some(std::time::Duration::from_millis(delay as u64))) == UNDEF {
        d.exc = true;
        return 0;
    }
    1
}

struct CollData {
    db: *mut Db,
    cmp: V,
}
/// Progress handler for `statement_timeout=`: interrupts a statement running past its deadline.
/// Pure Rust (no Ruby), so it may run inside a blocking call.
unsafe extern "C" fn stmt_timeout_cb(p: *mut c_void) -> c_int {
    let d = unsafe { &mut *(p as *mut Db) };
    let now = std::time::Instant::now();
    match d.stmt_deadline {
        None => {
            d.stmt_deadline = Some(now + std::time::Duration::from_millis(d.stmt_timeout_ms.max(0) as u64));
            0
        }
        Some(t) => (now > t) as c_int,
    }
}

unsafe extern "C" fn coll_cb(p: *mut c_void, alen: c_int, a: *const c_void, blen: c_int, b: *const c_void) -> c_int {
    unsafe {
        let cd = &*(p as *const CollData);
        let d = &mut *cd.db;
        if d.exc {
            return 0;
        }
        let sa = str_new(std::slice::from_raw_parts(a as *const u8, alen as usize).to_vec());
        let sb = str_new(std::slice::from_raw_parts(b as *const u8, blen as usize).to_vec());
        let r = send(cd.cmp, sym("compare"), &[sa, sb], QNIL);
        if r == UNDEF {
            d.exc = true;
            return 0;
        }
        if is_fix(r) { fix(r) as c_int } else { 0 }
    }
}
unsafe extern "C" fn free_colldata(p: *mut c_void) {
    drop(unsafe { Box::from_raw(p as *mut CollData) });
}

/// `rb_define_class_under` / `rb_define_module_under`: reuse an existing constant, else create
/// it with its lexical parent set (constant lookup inside a reopened `class Database` body
/// walks lexical parents).
fn define_under(m: *mut Class, name: &str, superclass: Option<*mut Class>) -> *mut Class {
    if let Some(c) = const_get_under(m, sym(name)) {
        if is_class(c) {
            return c as *mut Class;
        }
    }
    let full = format!("{}::{}", unsafe { &(*m).name }, name);
    let c = match superclass {
        Some(sup) => new_class(&full, sup),
        None => new_module(&full),
    };
    unsafe { (*c).lexical_parent = m };
    const_set(m, sym(name), c as V);
    c
}

// ---------- SQLite3::Database ----------

fn open_v2(s: V, a: &[V]) -> V {
    let file = tri!(string_value(a[0]));
    let mode = match num2int(a[1]) {
        Ok(m) => m as c_int,
        Err(e) => return e,
    };
    let vfs = if a[2] == QNIL {
        None
    } else {
        let z = tri!(string_value(a[2]));
        Some(cstring(&rstr(z).b))
    };
    let d = db_of(s);
    if !d.db.is_null() {
        unsafe { sqlite3_close_v2(d.db) };
        d.db = std::ptr::null_mut();
    }
    let path = cstring(&db_path(&rstr(file).b));
    let status = unsafe { ffi::sqlite3_open_v2(path.as_ptr(), &mut d.db, mode, vfs.as_ref().map_or(std::ptr::null(), |v| v.as_ptr())) };
    after_open(d);
    tri!(check(d.db, status));
    s
}
/// The real file of a database path (crate::vfs: under the app's directory it lives in the
/// run directory; an embedded database is copied there first).
fn db_path(p: &[u8]) -> Vec<u8> {
    if p.is_empty() || p == b":memory:" || p.starts_with(b"file:") {
        return p.to_vec();
    }
    crate::vfs::write_path(p)
}
/// The bundled library defaults foreign keys to ON; the gem's (and SQLite's) default is OFF.
fn after_open(d: &mut Db) {
    if !d.db.is_null() {
        unsafe { ffi::sqlite3_db_config(d.db, ffi::SQLITE_DBCONFIG_ENABLE_FKEY, 0 as c_int, std::ptr::null_mut::<c_int>()) };
    }
}

fn exec_batch(s: V, a: &[V]) -> V {
    let d = open_db!(s);
    let sql = tri!(string_value(a[0]));
    let as_hash = a[1] == QTRUE;
    let sql_c = cstring(&rstr(sql).b);
    // Rows collected into a Ruby array (no Ruby code runs during sqlite3_exec).
    struct Acc {
        out: V,
        hash: bool,
    }
    unsafe extern "C" fn cb(p: *mut c_void, n: c_int, data: *mut *mut c_char, cols: *mut *mut c_char) -> c_int {
        unsafe {
            let acc = &*(p as *const Acc);
            if acc.hash {
                let h = hash_new();
                for i in 0..n as usize {
                    let k = str_new(cstr_bytes(*cols.add(i)).unwrap_or_default());
                    let v = cstr_bytes(*data.add(i)).map(str_new).unwrap_or(QNIL);
                    crate::hash::hset(h, k, v);
                }
                rary(acc.out).v.push(h);
            } else {
                let row: Vec<V> = (0..n as usize).map(|i| cstr_bytes(*data.add(i)).map(str_new).unwrap_or(QNIL)).collect();
                rary(acc.out).v.push(ary_new(row));
            }
        }
        0
    }
    let acc = Acc { out: ary_new(Vec::new()), hash: as_hash };
    let mut err: *mut c_char = std::ptr::null_mut();
    let status = unsafe { ffi::sqlite3_exec(d.db, sql_c.as_ptr(), Some(cb), &acc as *const Acc as *mut c_void, &mut err) };
    if take_exc(d) {
        if !err.is_null() {
            unsafe { ffi::sqlite3_free(err as *mut c_void) };
        }
        return UNDEF;
    }
    if status != ffi::SQLITE_OK {
        let msg = cstr_to_string(err).unwrap_or_default();
        if !err.is_null() {
            unsafe { ffi::sqlite3_free(err as *mut c_void) };
        }
        return raise(rt().c.runtime_error, &msg);
    }
    acc.out
}

fn db_encoding(s: V) -> V {
    let d = open_db!(s);
    let enc = ivar_get(s, sym("@encoding"));
    if enc != QNIL {
        return enc;
    }
    let mut name = String::from("UTF-8");
    unsafe {
        let mut st: *mut ffi::sqlite3_stmt = std::ptr::null_mut();
        if ffi::sqlite3_prepare_v2(d.db, c"PRAGMA encoding".as_ptr(), -1, &mut st, std::ptr::null_mut()) == ffi::SQLITE_OK {
            if ffi::sqlite3_step(st) == ffi::SQLITE_ROW {
                if let Some(n) = cstr_to_string(ffi::sqlite3_column_text(st, 0) as *const c_char) {
                    name = n;
                }
            }
            ffi::sqlite3_finalize(st);
        }
    }
    if take_exc(d) {
        return UNDEF;
    }
    let encc = const_lookup(rt().c.object, sym("Encoding")).unwrap_or(QNIL);
    let e = send(encc, sym("find"), &[str_from(&name)], QNIL);
    let e = if e == UNDEF {
        pst().pending = Throw::None;
        tri!(send(encc, sym("find"), &[str_from("UTF-8")], QNIL))
    } else {
        e
    };
    ivar_set(s, sym("@encoding"), e);
    e
}

fn define_function_with_flags(s: V, name: V, flags: V, blk: V) -> V {
    let d = open_db!(s);
    if !is_proc(blk) {
        return arg_error("tried to create Proc object without a block");
    }
    let name = tri!(string_value(name));
    let flags = match num2int(flags) {
        Ok(f) => f as c_int,
        Err(e) => return e,
    };
    let arity = tri!(send(blk, sym("arity"), &[], QNIL));
    let arity = if is_fix(arity) { fix(arity) as c_int } else { -1 };
    d.ruby_cb = true;
    let fd = Box::into_raw(Box::new(FuncData { db: d as *mut Db, block: blk }));
    let n = cstring(&rstr(name).b);
    let status = unsafe {
        ffi::sqlite3_create_function_v2(d.db, n.as_ptr(), arity, flags, fd as *mut c_void, Some(func_cb), None, None, Some(free_funcdata))
    };
    tri!(check(d.db, status));
    let fns = ivar_get(s, sym("@functions"));
    if is_hash(fns) {
        crate::hash::hset(fns, name, blk);
    } else {
        // keep the block alive even if @functions isn't set up
        let keep = ivar_get(s, sym("@__keep"));
        let keep = if is_ary(keep) { keep } else { let a = ary_new(Vec::new()); ivar_set(s, sym("@__keep"), a); a };
        rary(keep).v.push(blk);
    }
    s
}

fn define_aggregator2(s: V, aggregator: V, name: V) -> V {
    let d = open_db!(s);
    let arity = if respond_to(aggregator, sym("arity")) {
        let r = tri!(send(aggregator, sym("arity"), &[], QNIL));
        match num2int(r) {
            Ok(a) => a,
            Err(e) => return e,
        }
    } else {
        -1
    };
    if !(-1..=127).contains(&arity) {
        let ins = tri!(send(s, sym("inspect"), &[], QNIL));
        return arg_error(&format!("{} arity={} out of range -1..127", String::from_utf8_lossy(&rstr(ins).b), arity));
    }
    let name = tri!(string_value(name));
    let instances = ary_new(Vec::new());
    let aggs = ivar_get(s, sym("@__aggregators"));
    let aggs = if is_ary(aggs) { aggs } else { let a = ary_new(Vec::new()); ivar_set(s, sym("@__aggregators"), a); a };
    rary(aggs).v.push(aggregator);
    rary(aggs).v.push(instances);
    d.ruby_cb = true;
    let ad = Box::into_raw(Box::new(AggData { db: d as *mut Db, klass: aggregator, instances }));
    let n = cstring(&rstr(name).b);
    let status = unsafe {
        ffi::sqlite3_create_function_v2(d.db, n.as_ptr(), arity as c_int, ffi::SQLITE_UTF8, ad as *mut c_void, None, Some(agg_step), Some(agg_final), Some(free_aggdata))
    };
    tri!(check(d.db, status));
    s
}

fn init_database(m: *mut Class) {
    let c = define_under(m, "Database", Some(rt().c.object));
    *C_DATABASE.get() = c;
    let private = |name: &str| {
        let mm = unsafe { (*c).methods.get(&sym(name)).copied() };
        if let Some(mm) = mm {
            unsafe { (*(mm as *mut Method)).vis = VIS_PRIVATE };
        }
    };

    defn(c, "statement_timeout=", 1, 1, |s, a, _| {
        let d = db_of(s);
        let ms = match num2int(a[0]) {
            Ok(i) => i,
            Err(e) => return e,
        };
        d.stmt_timeout_ms = ms;
        if !d.db.is_null() {
            let n = if ms == 0 { -1 } else { 1000 };
            unsafe { ffi::sqlite3_progress_handler(d.db, n, Some(stmt_timeout_cb), d as *mut Db as *mut c_void) };
        }
        s
    });
    // ForkSafety: a forked child drops its inherited connection without touching the parent's
    // files' locks (sqlite3_close would); Kenny never forks with live connections, so this closes.
    defn(c, "discard", 0, 0, |s, _, _| {
        let d = db_of(s);
        if !d.db.is_null() {
            unsafe { ffi::sqlite3_db_release_memory(d.db) };
            unsafe { sqlite3_close_v2(d.db) };
            d.db = std::ptr::null_mut();
        }
        d.discarded = true;
        s
    });
    defn(c, "load_extension_internal", 1, 1, |_, _, _| {
        raise(rt().c.not_implemented_error, "loading SQLite extensions is not supported by Kenny (a Kenny binary doesn't load shared libraries)")
    });
    defn(c, "open_v2", 3, 3, |s, a, _| open_v2(s, a));
    defn(c, "open16", 1, 1, |s, a, _| {
        // Kenny strings are UTF-8 (or binary); open the path as given.
        let file = tri!(string_value(a[0]));
        let d = db_of(s);
        let path = cstring(&db_path(&rstr(file).b));
        let status = unsafe { ffi::sqlite3_open(path.as_ptr(), &mut d.db) };
        after_open(d);
        tri!(check(d.db, status));
        mkfix(status as i64)
    });
    defn(c, "disable_quirk_mode", 0, 0, |s, _, _| {
        let d = db_of(s);
        if d.db.is_null() {
            return QFALSE;
        }
        unsafe {
            ffi::sqlite3_db_config(d.db, ffi::SQLITE_DBCONFIG_DQS_DDL, 0 as c_int, std::ptr::null_mut::<c_void>());
            ffi::sqlite3_db_config(d.db, ffi::SQLITE_DBCONFIG_DQS_DML, 0 as c_int, std::ptr::null_mut::<c_void>());
        }
        QTRUE
    });
    defn(c, "close", 0, 0, |s, _, _| {
        let d = db_of(s);
        let status = unsafe { ffi::sqlite3_close(d.db) };
        tri!(check(d.db, status));
        d.db = std::ptr::null_mut();
        ivar_set(s, sym("@__aggregators"), QNIL);
        s
    });
    defn(c, "closed?", 0, 0, |s, _, _| bool_v(db_of(s).db.is_null()));
    defn(c, "total_changes", 0, 0, |s, _, _| {
        let d = open_db!(s);
        int_v(unsafe { ffi::sqlite3_total_changes64(d.db) })
    });
    defn(c, "trace", 0, 1, |s, a, b| {
        let d = open_db!(s);
        let mut blk = a.first().copied().unwrap_or(QNIL);
        if blk == QNIL && is_proc(b) {
            blk = b;
        }
        ivar_set(s, sym("@tracefunc"), blk);
        d.ruby_cb |= blk != QNIL;
        #[allow(deprecated)]
        unsafe {
            ffi::sqlite3_trace(d.db, if blk == QNIL { None } else { Some(trace_cb) }, d as *mut Db as *mut c_void)
        };
        s
    });
    defn(c, "last_insert_row_id", 0, 0, |s, _, _| {
        let d = open_db!(s);
        int_v(unsafe { ffi::sqlite3_last_insert_rowid(d.db) })
    });
    defn(c, "define_function", 1, 1, |s, a, b| define_function_with_flags(s, a[0], mkfix(ffi::SQLITE_UTF8 as i64), b));
    defn(c, "define_function_with_flags", 2, 2, |s, a, b| define_function_with_flags(s, a[0], a[1], b));
    defn(c, "define_aggregator2", 2, 2, |s, a, _| define_aggregator2(s, a[0], a[1]));
    defn(c, "interrupt", 0, 0, |s, _, _| {
        let d = open_db!(s);
        unsafe { ffi::sqlite3_interrupt(d.db) };
        s
    });
    defn(c, "errmsg", 0, 0, |s, _, _| {
        let d = open_db!(s);
        str_new(cstr_bytes(unsafe { ffi::sqlite3_errmsg(d.db) }).unwrap_or_default())
    });
    defn(c, "errcode", 0, 0, |s, _, _| {
        let d = open_db!(s);
        mkfix(unsafe { ffi::sqlite3_errcode(d.db) } as i64)
    });
    defn(c, "complete?", 1, 1, |_, a, _| {
        let sql = tri!(string_value(a[0]));
        let cs = cstring(&rstr(sql).b);
        bool_v(unsafe { ffi::sqlite3_complete(cs.as_ptr()) } != 0)
    });
    defn(c, "changes", 0, 0, |s, _, _| {
        let d = open_db!(s);
        int_v(unsafe { ffi::sqlite3_changes64(d.db) })
    });
    defn(c, "authorizer=", 1, 1, |s, a, _| {
        let d = open_db!(s);
        d.ruby_cb |= a[0] != QNIL;
        let status = unsafe { ffi::sqlite3_set_authorizer(d.db, if a[0] == QNIL { None } else { Some(auth_cb) }, d as *mut Db as *mut c_void) };
        tri!(check(d.db, status));
        ivar_set(s, sym("@authorizer"), a[0]);
        a[0]
    });
    defn(c, "busy_handler", 0, 1, |s, a, b| {
        let d = open_db!(s);
        let mut blk = a.first().copied().unwrap_or(QNIL);
        if blk == QNIL && is_proc(b) {
            blk = b;
        }
        ivar_set(s, sym("@busy_handler"), blk);
        d.ruby_cb |= blk != QNIL;
        let status = unsafe { ffi::sqlite3_busy_handler(d.db, if blk == QNIL { None } else { Some(busy_cb) }, d as *mut Db as *mut c_void) };
        tri!(check(d.db, status));
        s
    });
    defn(c, "busy_timeout=", 1, 1, |s, a, _| {
        let d = open_db!(s);
        let ms = match num2int(a[0]) {
            Ok(m) => m as c_int,
            Err(e) => return e,
        };
        d.busy_ms = ms as i64;
        d.busy_since = None;
        let status = unsafe { ffi::sqlite3_busy_handler(d.db, if ms > 0 { Some(timeout_cb) } else { None }, d as *mut Db as *mut c_void) };
        tri!(check(d.db, status));
        a[0]
    });
    defn(c, "extended_result_codes=", 1, 1, |s, a, _| {
        let d = open_db!(s);
        tri!(check(d.db, unsafe { ffi::sqlite3_extended_result_codes(d.db, if truthy(a[0]) { 1 } else { 0 }) }));
        a[0]
    });
    defn(c, "transaction_active?", 0, 0, |s, _, _| {
        let d = open_db!(s);
        bool_v(unsafe { ffi::sqlite3_get_autocommit(d.db) } == 0)
    });
    defn(c, "exec_batch", 2, 2, |s, a, _| exec_batch(s, a));
    defn(c, "db_filename", 1, 1, |s, a, _| {
        let d = open_db!(s);
        let n = tri!(string_value(a[0]));
        let cs = cstring(&rstr(n).b);
        match cstr_bytes(unsafe { ffi::sqlite3_db_filename(d.db, cs.as_ptr()) }) {
            Some(b) => str_new(b),
            None => QNIL,
        }
    });
    defn(c, "collation", 2, 2, |s, a, _| {
        let d = open_db!(s);
        let name = tri!(string_value(a[0]));
        let cs = cstring(&rstr(name).b);
        let status = if a[1] == QNIL {
            unsafe { ffi::sqlite3_create_collation_v2(d.db, cs.as_ptr(), ffi::SQLITE_UTF8, std::ptr::null_mut(), None, None) }
        } else {
            let cd = Box::into_raw(Box::new(CollData { db: d as *mut Db, cmp: a[1] }));
            unsafe { ffi::sqlite3_create_collation_v2(d.db, cs.as_ptr(), ffi::SQLITE_UTF8, cd as *mut c_void, Some(coll_cb), Some(free_colldata)) }
        };
        tri!(check(d.db, status));
        let colls = ivar_get(s, sym("@collations"));
        if is_hash(colls) {
            crate::hash::hset(colls, name, a[1]);
        }
        s
    });
    defn(c, "load_extension", 1, 1, |s, a, _| {
        let d = open_db!(s);
        let f = tri!(string_value(a[0]));
        let cs = cstring(&rstr(f).b);
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe { ffi::sqlite3_load_extension(d.db, cs.as_ptr(), std::ptr::null(), &mut err) };
        if status != ffi::SQLITE_OK {
            let msg = cstr_to_string(err).unwrap_or_default();
            unsafe { ffi::sqlite3_free(err as *mut c_void) };
            return raise(rt().c.runtime_error, &msg);
        }
        s
    });
    defn(c, "enable_load_extension", 1, 1, |s, a, _| {
        let d = open_db!(s);
        let on = if a[0] == QTRUE {
            1
        } else if a[0] == QFALSE {
            0
        } else {
            match num2int(a[0]) {
                Ok(v) => v as c_int,
                Err(e) => return e,
            }
        };
        tri!(check(d.db, unsafe { ffi::sqlite3_enable_load_extension(d.db, on) }));
        s
    });
    defn(c, "encoding", 0, 0, |s, _, _| db_encoding(s));
    for p in ["open_v2", "open16", "define_aggregator2", "disable_quirk_mode", "exec_batch", "db_filename", "discard", "load_extension_internal"] {
        private(p);
    }
}

// ---------- SQLite3::Statement ----------

fn stmt_initialize(s: V, db: V, sql: V) -> V {
    let sql = tri!(string_value(sql));
    if !is_a(db, *C_DATABASE.get()) {
        return type_error(&format!("wrong argument type {} (expected SQLite3::Database)", type_name(db)));
    }
    let d = db_of(db);
    if d.db.is_null() {
        return arg_error("prepare called on a closed database");
    }
    d.stmt_deadline = None;
    let st = stmt_of(s);
    let bytes = str_bytes(sql);
    let (dbp, sql_p, sql_n) = (d.db as usize, bytes.as_ptr() as usize, bytes.len() as c_int);
    let prepare = move || {
        let mut raw: *mut ffi::sqlite3_stmt = std::ptr::null_mut();
        let mut tail: *const c_char = std::ptr::null();
        let status = unsafe { ffi::sqlite3_prepare_v2(dbp as *mut ffi::sqlite3, sql_p as *const c_char, sql_n, &mut raw, &mut tail) };
        (status, raw as usize, tail as usize)
    };
    let (status, raw, t) = if d.ruby_cb { prepare() } else { crate::gc::blocking(prepare) };
    st.st = raw as *mut ffi::sqlite3_stmt;
    let tail = t as *const c_char;
    if take_exc(d) {
        return UNDEF;
    }
    tri!(check(d.db, status));
    let rest = if tail.is_null() {
        Vec::new()
    } else {
        let off = (tail as usize).saturating_sub(bytes.as_ptr() as usize).min(bytes.len());
        // rb_str_new2: up to the first NUL
        let r = &bytes[off..];
        r[..r.iter().position(|&c| c == 0).unwrap_or(r.len())].to_vec()
    };
    ivar_set(s, sym("@connection"), db);
    ivar_set(s, sym("@remainder"), str_new(rest));
    ivar_set(s, sym("@columns"), QNIL);
    ivar_set(s, sym("@types"), QNIL);
    s
}

fn stmt_step(s: V) -> V {
    let st = open_stmt!(s);
    if st.done {
        return QNIL;
    }
    let conn = ivar_get(s, sym("@connection"));
    let stmt = st.st;
    if is_heap(conn) && unsafe { ffi::sqlite3_stmt_busy(stmt) } == 0 {
        db_of(conn).stmt_deadline = None;
    }
    let quiet = is_heap(conn) && !db_of(conn).ruby_cb;
    let value = if quiet {
        let p = stmt as usize;
        crate::gc::blocking(move || unsafe { ffi::sqlite3_step(p as *mut ffi::sqlite3_stmt) })
    } else {
        unsafe { ffi::sqlite3_step(stmt) }
    };
    if is_heap(conn) {
        let d = db_of(conn);
        if take_exc(d) {
            unsafe { ffi::sqlite3_reset(stmt) };
            st.done = false;
            return UNDEF;
        }
    }
    match value {
        ffi::SQLITE_ROW => {
            let n = unsafe { ffi::sqlite3_column_count(stmt) };
            let mut row = Vec::with_capacity(n as usize);
            for i in 0..n {
                let v = unsafe {
                    match ffi::sqlite3_column_type(stmt, i) {
                        ffi::SQLITE_INTEGER => int_v(ffi::sqlite3_column_int64(stmt, i)),
                        ffi::SQLITE_FLOAT => float_new(ffi::sqlite3_column_double(stmt, i)),
                        ffi::SQLITE_TEXT => {
                            let p = ffi::sqlite3_column_text(stmt, i);
                            let len = ffi::sqlite3_column_bytes(stmt, i) as usize;
                            str_new(if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, len).to_vec() })
                        }
                        ffi::SQLITE_BLOB => {
                            let p = ffi::sqlite3_column_blob(stmt, i) as *const u8;
                            let len = ffi::sqlite3_column_bytes(stmt, i) as usize;
                            bin_str(if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, len).to_vec() })
                        }
                        _ => QNIL,
                    }
                };
                row.push(v);
            }
            ary_new(row)
        }
        ffi::SQLITE_DONE => {
            st.done = true;
            QNIL
        }
        _ => {
            unsafe { ffi::sqlite3_reset(stmt) };
            st.done = false;
            check(unsafe { ffi::sqlite3_db_handle(stmt) }, value)
        }
    }
}

fn bind_param(s: V, key: V, value: V) -> V {
    let st = open_stmt!(s);
    let index: c_int = if is_sym(key) || is_str(key) {
        let mut name = if is_sym(key) { sym_name(sym_id(key)).to_vec() } else { str_bytes(key) };
        if name.first() != Some(&b':') {
            name.insert(0, b':');
        }
        let cs = cstring(&name);
        unsafe { ffi::sqlite3_bind_parameter_index(st.st, cs.as_ptr()) }
    } else {
        match num2int(key) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        }
    };
    if index == 0 {
        return sq_exception("no such bind parameter");
    }
    let status = unsafe {
        if is_str(value) {
            let b = &rstr(value).b;
            if real_class(value) == *C_BLOB.get() || is_binary(value) {
                ffi::sqlite3_bind_blob(st.st, index, b.as_ptr() as *const c_void, b.len() as c_int, ffi::SQLITE_TRANSIENT())
            } else {
                ffi::sqlite3_bind_text(st.st, index, b.as_ptr() as *const c_char, b.len() as c_int, ffi::SQLITE_TRANSIENT())
            }
        } else if is_big(value) {
            use num_traits::ToPrimitive;
            let n = &rbig(value).n;
            match n.to_i64() {
                Some(i) => ffi::sqlite3_bind_int64(st.st, index, i),
                None => ffi::sqlite3_bind_double(st.st, index, n.to_f64().unwrap_or(f64::NAN)),
            }
        } else if is_float(value) {
            ffi::sqlite3_bind_double(st.st, index, float_val(value))
        } else if is_fix(value) {
            ffi::sqlite3_bind_int64(st.st, index, fix(value))
        } else if value == QNIL {
            ffi::sqlite3_bind_null(st.st, index)
        } else {
            return raise(rt().c.runtime_error, &format!("can't prepare {}", class_name(real_class(value))));
        }
    };
    tri!(check(unsafe { ffi::sqlite3_db_handle(st.st) }, status));
    s
}

fn init_statement(m: *mut Class) {
    let c = define_under(m, "Statement", Some(rt().c.object));
    *C_STATEMENT.get() = c;
    defn(c, "initialize", 2, 2, |s, a, _| stmt_initialize(s, a[0], a[1]));
    defn(c, "close", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        unsafe { ffi::sqlite3_finalize(st.st) };
        st.st = std::ptr::null_mut();
        s
    });
    defn(c, "closed?", 0, 0, |s, _, _| bool_v(stmt_of(s).st.is_null()));
    defn(c, "bind_param", 2, 2, |s, a, _| bind_param(s, a[0], a[1]));
    defn(c, "reset!", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        unsafe { ffi::sqlite3_reset(st.st) };
        st.done = false;
        s
    });
    defn(c, "clear_bindings!", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        unsafe { ffi::sqlite3_clear_bindings(st.st) };
        st.done = false;
        s
    });
    // sqlite3 2.x: Statement#initialize is Ruby and calls this; returns the unparsed remainder
    defn(c, "prepare", 2, 2, |s, a, _| {
        tri!(stmt_initialize(s, a[0], a[1]));
        ivar_get(s, sym("@remainder"))
    });

    defn(c, "sql", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        let v = cstr_bytes(unsafe { ffi::sqlite3_sql(st.st) }).map(str_new).unwrap_or(QNIL);
        if is_heap(v) {
            hdr(v).flags |= FL_FROZEN;
        }
        v
    });
    defn(c, "expanded_sql", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        let p = unsafe { ffi::sqlite3_expanded_sql(st.st) };
        let v = cstr_bytes(p).map(str_new).unwrap_or(QNIL);
        unsafe { ffi::sqlite3_free(p as *mut c_void) };
        if is_heap(v) {
            hdr(v).flags |= FL_FROZEN;
        }
        v
    });
    defn(c, "named_params", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        let n = unsafe { ffi::sqlite3_bind_parameter_count(st.st) };
        let mut out = Vec::new();
        for i in 1..=n {
            if let Some(name) = cstr_bytes(unsafe { ffi::sqlite3_bind_parameter_name(st.st, i) }) {
                if name.first().is_some_and(|&c| c != b'?') {
                    out.push(str_new(name[1..].to_vec()));
                }
            }
        }
        let a = ary_new(out);
        hdr(a).flags |= FL_FROZEN;
        a
    });
    defn(c, "memused", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        mkfix(unsafe { ffi::sqlite3_stmt_status(st.st, ffi::SQLITE_STMTSTATUS_MEMUSED, 0) } as i64)
    });
    const STATS: [(&str, c_int); 8] = [
        ("fullscan_steps", ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP),
        ("sorts", ffi::SQLITE_STMTSTATUS_SORT),
        ("autoindexes", ffi::SQLITE_STMTSTATUS_AUTOINDEX),
        ("vm_steps", ffi::SQLITE_STMTSTATUS_VM_STEP),
        ("reprepares", ffi::SQLITE_STMTSTATUS_REPREPARE),
        ("runs", ffi::SQLITE_STMTSTATUS_RUN),
        ("filter_misses", ffi::SQLITE_STMTSTATUS_FILTER_MISS),
        ("filter_hits", ffi::SQLITE_STMTSTATUS_FILTER_HIT),
    ];
    defn(c, "stat_for", 1, 1, |s, a, _| {
        let st = open_stmt!(s);
        if !is_sym(a[0]) {
            return type_error("non-symbol given");
        }
        let name = sym_name(sym_id(a[0]));
        let op = STATS.iter().find(|(n, _)| n.as_bytes() == &name[..]).map(|(_, op)| *op);
        mkfix(op.map_or(0, |op| unsafe { ffi::sqlite3_stmt_status(st.st, op, 0) }) as i64)
    });
    defn(c, "stats_as_hash", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        let h = hash_new();
        for (n, op) in STATS {
            crate::hash::hset(h, mksym(sym(n)), mkfix(unsafe { ffi::sqlite3_stmt_status(st.st, op, 0) } as i64));
        }
        h
    });
    defn(c, "column_decltype_downcased", 1, 1, |s, a, _| {
        let st = open_stmt!(s);
        let i = match num2int(a[0]) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        };
        cstr_bytes(unsafe { ffi::sqlite3_column_decltype(st.st, i) }).map(|b| str_new(b.to_ascii_lowercase())).unwrap_or(QNIL)
    });
    for p in ["prepare", "stat_for", "stats_as_hash", "column_decltype_downcased"] {
        let mm = unsafe { (*c).methods.get(&sym(p)).copied() };
        if let Some(mm) = mm {
            unsafe { (*(mm as *mut Method)).vis = VIS_PRIVATE };
        }
    }
    defn(c, "step", 0, 0, |s, _, _| stmt_step(s));
    defn(c, "done?", 0, 0, |s, _, _| bool_v(stmt_of(s).done));
    defn(c, "column_count", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        mkfix(unsafe { ffi::sqlite3_column_count(st.st) } as i64)
    });
    defn(c, "column_name", 1, 1, |s, a, _| {
        let st = open_stmt!(s);
        let i = match num2int(a[0]) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        };
        cstr_bytes(unsafe { ffi::sqlite3_column_name(st.st, i) }).map(str_new).unwrap_or(QNIL)
    });
    defn(c, "column_decltype", 1, 1, |s, a, _| {
        let st = open_stmt!(s);
        let i = match num2int(a[0]) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        };
        cstr_bytes(unsafe { ffi::sqlite3_column_decltype(st.st, i) }).map(str_new).unwrap_or(QNIL)
    });
    defn(c, "bind_parameter_count", 0, 0, |s, _, _| {
        let st = open_stmt!(s);
        mkfix(unsafe { ffi::sqlite3_bind_parameter_count(st.st) } as i64)
    });
    defn(c, "database_name", 1, 1, |s, a, _| {
        let st = open_stmt!(s);
        let i = match num2int(a[0]) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        };
        cstr_bytes(unsafe { ffi::sqlite3_column_database_name(st.st, i) }).map(str_new).unwrap_or(QNIL)
    });
}

// ---------- SQLite3::Backup ----------

fn init_backup(m: *mut Class) {
    let c = define_under(m, "Backup", Some(rt().c.object));
    defn(c, "initialize", 4, 4, |s, a, _| {
        let (dst, dname, src, sname) = (a[0], a[1], a[2], a[3]);
        let dname = tri!(string_value(dname));
        let sname = tri!(string_value(sname));
        let dd = db_of(dst);
        let sd = db_of(src);
        if sd.db.is_null() {
            return arg_error("cannot backup from a closed database");
        }
        if dd.db.is_null() {
            return arg_error("cannot backup to a closed database");
        }
        let dn = cstring(&rstr(dname).b);
        let sn = cstring(&rstr(sname).b);
        let p = unsafe { ffi::sqlite3_backup_init(dd.db, dn.as_ptr(), sd.db, sn.as_ptr()) };
        if p.is_null() {
            return check(dd.db, unsafe { ffi::sqlite3_errcode(dd.db) });
        }
        bk_of(s).p = p;
        s
    });
    fn open_bk(s: V) -> Result<*mut ffi::sqlite3_backup, V> {
        let b = bk_of(s);
        if b.p.is_null() { Err(sq_exception("cannot use a closed backup")) } else { Ok(b.p) }
    }
    defn(c, "step", 1, 1, |s, a, _| {
        let p = match open_bk(s) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let n = match num2int(a[0]) {
            Ok(n) => n as c_int,
            Err(e) => return e,
        };
        mkfix(unsafe { ffi::sqlite3_backup_step(p, n) } as i64)
    });
    defn(c, "finish", 0, 0, |s, _, _| {
        let p = match open_bk(s) {
            Ok(p) => p,
            Err(e) => return e,
        };
        unsafe { ffi::sqlite3_backup_finish(p) };
        bk_of(s).p = std::ptr::null_mut();
        QNIL
    });
    defn(c, "remaining", 0, 0, |s, _, _| match open_bk(s) {
        Ok(p) => mkfix(unsafe { ffi::sqlite3_backup_remaining(p) } as i64),
        Err(e) => e,
    });
    defn(c, "pagecount", 0, 0, |s, _, _| match open_bk(s) {
        Ok(p) => mkfix(unsafe { ffi::sqlite3_backup_pagecount(p) } as i64),
        Err(e) => e,
    });
}

// ---------- module ----------

fn init_sqlite3_native() {
    unsafe { ffi::sqlite3_initialize() };
    let m = match const_lookup(rt().c.object, sym("SQLite3")) {
        Some(m) if is_class(m) => m as *mut Class,
        _ => {
            let m = new_module("SQLite3");
            set_global_const("SQLite3", m as V);
            m
        }
    };
    *M_SQLITE3.get() = m;
    let blob = define_under(m, "Blob", Some(rt().c.string));
    *C_BLOB.get() = blob;

    // Constants::Open
    let consts = define_under(m, "Constants", None);
    let open = define_under(consts, "Open", None);
    for (n, v) in [
        ("READONLY", ffi::SQLITE_OPEN_READONLY),
        ("READWRITE", ffi::SQLITE_OPEN_READWRITE),
        ("CREATE", ffi::SQLITE_OPEN_CREATE),
        ("DELETEONCLOSE", ffi::SQLITE_OPEN_DELETEONCLOSE),
        ("EXCLUSIVE", ffi::SQLITE_OPEN_EXCLUSIVE),
        ("MAIN_DB", ffi::SQLITE_OPEN_MAIN_DB),
        ("TEMP_DB", ffi::SQLITE_OPEN_TEMP_DB),
        ("TRANSIENT_DB", ffi::SQLITE_OPEN_TRANSIENT_DB),
        ("MAIN_JOURNAL", ffi::SQLITE_OPEN_MAIN_JOURNAL),
        ("TEMP_JOURNAL", ffi::SQLITE_OPEN_TEMP_JOURNAL),
        ("SUBJOURNAL", ffi::SQLITE_OPEN_SUBJOURNAL),
        ("MASTER_JOURNAL", ffi::SQLITE_OPEN_SUPER_JOURNAL),
        ("NOMUTEX", ffi::SQLITE_OPEN_NOMUTEX),
        ("FULLMUTEX", ffi::SQLITE_OPEN_FULLMUTEX),
        ("AUTOPROXY", ffi::SQLITE_OPEN_AUTOPROXY),
        ("SHAREDCACHE", ffi::SQLITE_OPEN_SHAREDCACHE),
        ("PRIVATECACHE", ffi::SQLITE_OPEN_PRIVATECACHE),
        ("WAL", ffi::SQLITE_OPEN_WAL),
        ("URI", ffi::SQLITE_OPEN_URI),
        ("MEMORY", ffi::SQLITE_OPEN_MEMORY),
    ] {
        const_set(open, sym(n), mkfix(v as i64));
    }

    init_database(m);
    init_statement(m);
    init_backup(m);

    defs(m, "status", 1, 2, |_, a, _| {
        let op = match num2int(a[0]) {
            Ok(i) => i as c_int,
            Err(e) => return e,
        };
        let reset = a.get(1).is_some_and(|v| truthy(*v)) as c_int;
        let (mut cur, mut high): (c_int, c_int) = (0, 0);
        unsafe { ffi::sqlite3_status(op, &mut cur, &mut high, reset) };
        let h = hash_new();
        crate::hash::hset(h, mksym(sym("current")), mkfix(cur as i64));
        crate::hash::hset(h, mksym(sym("highwater")), mkfix(high as i64));
        h
    });
    // the SQLite library is compiled into the runtime, as with the gem's precompiled builds
    const_set(m, sym("SQLITE_PACKAGED_LIBRARIES"), QTRUE);
    const_set(m, sym("SQLITE_PRECOMPILED_LIBRARIES"), QTRUE);
    defs(m, "sqlcipher?", 0, 0, |_, _, _| QFALSE);
    defs(m, "libversion", 0, 0, |_, _, _| mkfix(unsafe { ffi::sqlite3_libversion_number() } as i64));
    defs(m, "threadsafe", 0, 0, |_, _, _| mkfix(unsafe { ffi::sqlite3_threadsafe() } as i64));
    let ver = ffi::SQLITE_VERSION.to_string_lossy().into_owned();
    const_set(m, sym("SQLITE_VERSION"), str_from(&ver));
    const_set(m, sym("SQLITE_VERSION_NUMBER"), mkfix(ffi::SQLITE_VERSION_NUMBER as i64));
    let loaded = cstr_to_string(unsafe { ffi::sqlite3_libversion() }).unwrap_or(ver);
    const_set(m, sym("SQLITE_LOADED_VERSION"), str_from(&loaded));
}

pub fn init() {
    // lib/sqlite3.rb first tries the fat-binary path "sqlite3/<ruby major.minor>/sqlite3_native"
    crate::compat::provide_feature(&["sqlite3/sqlite3_native", "sqlite3_native", "sqlite3/3.4/sqlite3_native"], Some(init_sqlite3_native));
}
