//! Real IO: buffered file descriptors (IO, File, File::Stat), Dir and glob, pipes and
//! subprocesses (IO.popen, Kernel#system / ` / spawn / exec / fork), Process and
//! Process::Status, SystemCallError and the Errno classes.
//!
//! An IO object is a plain RObject whose hidden `@__io` ivar points to an RData owning the
//! Rust-side [`IoState`] (descriptors + buffers). When the GC frees that RData the state is
//! flushed and its descriptor closed, so unreferenced Files don't leak fds.
//! STDOUT writes go through `kernel::out` (the shared stdout buffer, flushed at exit and
//! before stderr writes / subprocesses) and STDERR through `kernel::err_out`, so ordering
//! with `puts` is preserved.
//!
//! There is one OS thread running Ruby code; blocking reads on pipes and `waitpid` poll and
//! yield to other green threads when there are any.

use crate::core::*;
use crate::fmt::{inspect, num_to_f, to_s};
use crate::kernel::{err_out, flush_out, new_str_v, out, puts_to};
use crate::gc::RVec;
use crate::tri;
use std::ffi::{c_void, CStr, CString, OsStr};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

macro_rules! ok {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

// ---------- globals ----------

static S_IO: G<u32> = G::new(0);
static GV_STDOUT: G<u32> = G::new(0);
static GV_STDERR: G<u32> = G::new(0);
static GV_STDIN: G<u32> = G::new(0);
static STDOUT_V: G<V> = G::new(0);
static STDERR_V: G<V> = G::new(0);
static STDIN_V: G<V> = G::new(0);
static C_FILE: G<*mut Class> = G::new(std::ptr::null_mut());
static C_STAT: G<*mut Class> = G::new(std::ptr::null_mut());
static C_DIR: G<*mut Class> = G::new(std::ptr::null_mut());
static C_STATUS: G<*mut Class> = G::new(std::ptr::null_mut());
static C_SCE: G<*mut Class> = G::new(std::ptr::null_mut());
static C_TMS: G<V> = G::new(QNIL);
static ERRNO_MAP: G<Vec<(i32, *mut Class)>> = G::new(Vec::new());
/// Every IoState that is still open (flushed before fork/exec/spawn and at exit).
static OPEN: crate::mt::L<Vec<usize>> = crate::mt::L::new(Vec::new());

const FNM_NOESCAPE: i64 = 1;
const FNM_PATHNAME: i64 = 2;
const FNM_DOTMATCH: i64 = 4;
const FNM_CASEFOLD: i64 = 8;
const FNM_EXTGLOB: i64 = 16;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}
fn bstr(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}
fn sv(b: Vec<u8>) -> V {
    str_new(b)
}

// ---------- SystemCallError / Errno ----------

const ERRNOS: &[(&str, i32)] = &[
    ("EPERM", libc::EPERM),
    ("ENOENT", libc::ENOENT),
    ("ESRCH", libc::ESRCH),
    ("EINTR", libc::EINTR),
    ("EIO", libc::EIO),
    ("ENXIO", libc::ENXIO),
    ("E2BIG", libc::E2BIG),
    ("ENOEXEC", libc::ENOEXEC),
    ("EBADF", libc::EBADF),
    ("ECHILD", libc::ECHILD),
    ("EAGAIN", libc::EAGAIN),
    ("ENOMEM", libc::ENOMEM),
    ("EACCES", libc::EACCES),
    ("EFAULT", libc::EFAULT),
    ("EBUSY", libc::EBUSY),
    ("EEXIST", libc::EEXIST),
    ("EXDEV", libc::EXDEV),
    ("ENODEV", libc::ENODEV),
    ("ENOTDIR", libc::ENOTDIR),
    ("EISDIR", libc::EISDIR),
    ("EINVAL", libc::EINVAL),
    ("ENFILE", libc::ENFILE),
    ("EMFILE", libc::EMFILE),
    ("ENOTTY", libc::ENOTTY),
    ("EFBIG", libc::EFBIG),
    ("ENOSPC", libc::ENOSPC),
    ("ESPIPE", libc::ESPIPE),
    ("EROFS", libc::EROFS),
    ("EMLINK", libc::EMLINK),
    ("EPIPE", libc::EPIPE),
    ("EDOM", libc::EDOM),
    ("ERANGE", libc::ERANGE),
    ("EDEADLK", libc::EDEADLK),
    ("ENAMETOOLONG", libc::ENAMETOOLONG),
    ("ENOTEMPTY", libc::ENOTEMPTY),
    ("ELOOP", libc::ELOOP),
    ("EWOULDBLOCK", libc::EWOULDBLOCK),
    ("ENOSYS", libc::ENOSYS),
    ("ENOTSUP", libc::ENOTSUP),
    ("EOPNOTSUPP", libc::EOPNOTSUPP),
    ("ENOTSOCK", libc::ENOTSOCK),
    ("EADDRINUSE", libc::EADDRINUSE),
    ("EADDRNOTAVAIL", libc::EADDRNOTAVAIL),
    ("ENETDOWN", libc::ENETDOWN),
    ("ENETUNREACH", libc::ENETUNREACH),
    ("ECONNABORTED", libc::ECONNABORTED),
    ("ECONNRESET", libc::ECONNRESET),
    ("ENOTCONN", libc::ENOTCONN),
    ("ETIMEDOUT", libc::ETIMEDOUT),
    ("ECONNREFUSED", libc::ECONNREFUSED),
    ("EHOSTUNREACH", libc::EHOSTUNREACH),
    ("EALREADY", libc::EALREADY),
    ("EINPROGRESS", libc::EINPROGRESS),
    ("EPROTO", libc::EPROTO),
];

fn strerror(e: i32) -> String {
    unsafe { CStr::from_ptr(libc::strerror(e)) }.to_string_lossy().into_owned()
}
fn errno_class(e: i32) -> *mut Class {
    ERRNO_MAP.get().iter().find(|(n, _)| *n == e).map(|(_, c)| *c).unwrap_or(*C_SCE.get())
}
/// Raise the Errno exception for `e`: "No such file or directory @ func - detail".
pub fn raise_errno(e: i32, func: Option<&str>, detail: &[u8]) -> V {
    let mut msg = strerror(e);
    if let Some(f) = func {
        msg.push_str(" @ ");
        msg.push_str(f);
    }
    if func.is_some() || !detail.is_empty() {
        msg.push_str(" - ");
        msg.push_str(&bstr(detail));
    }
    let ex = exc_new(errno_class(e), &msg);
    ivar_set(ex, sym("@__errno"), mkfix(e as i64));
    raise_exc(ex)
}
fn io_fail(e: &std::io::Error, func: Option<&str>, detail: &[u8]) -> V {
    raise_errno(e.raw_os_error().unwrap_or(libc::EIO), func, detail)
}
fn io_error(msg: &str) -> V {
    raise(rt().c.io_error, msg)
}
fn eof_error() -> V {
    let c = const_lookup(rt().c.object, sym("EOFError")).unwrap();
    raise(c as *mut Class, "end of file reached")
}

// ---------- argument helpers ----------

/// Split a trailing keyword hash off the arguments.
fn split_kw(a: &[V]) -> (&[V], Option<V>) {
    match a.last() {
        Some(&l) if is_hash(l) && hdr(l).flags & FL_KWHASH != 0 => (&a[..a.len() - 1], Some(l)),
        _ => (a, None),
    }
}
/// Split a trailing options hash (keyword or not) off the arguments.
fn split_opts(a: &[V]) -> (&[V], Option<V>) {
    match a.last() {
        Some(&l) if is_hash(l) => (&a[..a.len() - 1], Some(l)),
        _ => (a, None),
    }
}
fn kw_get(kw: Option<V>, name: &str) -> Option<V> {
    let h = kw?;
    rhash(h).m.get(&HKey(mksym(sym(name)))).copied()
}
fn kw_true(kw: Option<V>, name: &str) -> bool {
    kw_get(kw, name).is_some_and(truthy)
}
fn class_desc(v: V) -> String {
    match v {
        QNIL => "nil".into(),
        QTRUE => "true".into(),
        QFALSE => "false".into(),
        _ => class_name(real_class(v)),
    }
}
/// A path argument: String, or anything with to_path / to_str (Pathname, File).
pub fn path_bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    for m in ["to_path", "to_str"] {
        if is_heap(v) && respond_to(v, sym(m)) {
            let r = send(v, sym(m), &[], QNIL);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if is_str(r) {
                return Ok(rstr(r).b.clone());
            }
        }
    }
    Err(type_error(&format!("no implicit conversion of {} into String", class_desc(v))))
}
fn str_bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    if is_heap(v) && respond_to(v, sym("to_str")) {
        let r = send(v, sym("to_str"), &[], QNIL);
        if r == UNDEF {
            return Err(UNDEF);
        }
        if is_str(r) {
            return Ok(rstr(r).b.clone());
        }
    }
    Err(type_error(&format!("no implicit conversion of {} into String", class_desc(v))))
}
fn to_s_bytes(v: V) -> Result<Vec<u8>, V> {
    to_s(v).map_err(|_| UNDEF)
}
fn int_arg(v: V) -> Result<i64, V> {
    if is_fix(v) {
        return Ok(fix(v));
    }
    if let Some(f) = num_to_f(v) {
        return Ok(f as i64);
    }
    Err(type_error(&format!("no implicit conversion of {} into Integer", class_desc(v))))
}
fn cstr(b: &[u8]) -> Result<CString, V> {
    CString::new(b.to_vec()).map_err(|_| arg_error("path name contains null byte"))
}
fn osstr(b: &[u8]) -> &OsStr {
    OsStr::from_bytes(b)
}
/// The real file for path `p` (crate::vfs: the app's build directory maps to the run directory).
fn dpath(p: &[u8]) -> Result<CString, V> {
    cstr(&crate::vfs::disk(p))
}
/// Like `dpath`, for a path about to be created, written or removed.
fn wpath(p: &[u8]) -> Result<CString, V> {
    crate::vfs::prepare_write(p);
    dpath(p)
}
fn is_write_flags(flags: i32) -> bool {
    flags & libc::O_ACCMODE != libc::O_RDONLY || flags & (libc::O_CREAT | libc::O_TRUNC) != 0
}
fn cwd() -> Vec<u8> {
    if let Some(d) = crate::vfs::virtual_cwd() {
        return d;
    }
    std::env::current_dir().map(|d| d.as_os_str().as_bytes().to_vec()).unwrap_or_else(|_| b"/".to_vec())
}
fn set_gvar(name: &str, v: V) {
    rt().gvars.insert(sym(name), v);
}

// ---------- IoState ----------

pub struct IoState {
    pub fd: i32,
    /// Write descriptor (same as `fd` except for popen "r+").
    pub wfd: i32,
    /// 1 = STDOUT (writes go to kernel::out), 2 = STDERR (kernel::err_out), 0 = plain fd.
    pub std: u8,
    pub readable: bool,
    pub writable: bool,
    pub closed: bool,
    pub rbuf: Vec<u8>,
    pub rpos: usize,
    pub wbuf: Vec<u8>,
    pub sync: bool,
    pub path: Option<Vec<u8>>,
    pub lineno: i64,
    pub pid: i32,
    pub autoclose: bool,
    pub binmode: bool,
    /// A copy of STDOUT/STDERR: flush Kenny's stdout buffer before writing.
    pub stdio_copy: bool,
    /// A file embedded in the binary (crate::vfs), read from memory; `fd` is -1.
    pub mem: Option<&'static crate::vfs::Entry>,
    /// read position in `mem`
    pub mpos: usize,
    /// Several threads may use one IO (Action Cable writes to a socket from its worker
    /// threads while its event loop reads): the buffers change under these flags, so a race
    /// mixes up data at worst and never touches freed memory.
    wbusy: std::sync::atomic::AtomicBool,
    rbusy: std::sync::atomic::AtomicBool,
}

/// Holds an IO buffer flag (see IoState::wbusy) until dropped. Waiting yields to other
/// threads instead of blocking the OS thread (the holder may be in a slow write).
struct BufLock<'a>(&'a std::sync::atomic::AtomicBool);
impl<'a> BufLock<'a> {
    fn take(f: &'a std::sync::atomic::AtomicBool) -> Self {
        use std::sync::atomic::Ordering;
        let mut spins = 0u32;
        while f.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins < 64 {
                std::hint::spin_loop();
            } else if crate::thread::active() {
                crate::thread::pass();
                crate::gc::blocking(|| std::thread::sleep(Duration::from_micros(50)));
            } else {
                std::thread::yield_now();
            }
        }
        BufLock(f)
    }
}
impl Drop for BufLock<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn sys_write_all(fd: i32, mut b: &[u8]) -> Result<(), i32> {
    while !b.is_empty() {
        let n = unsafe { libc::write(fd, b.as_ptr() as *const c_void, b.len()) };
        if n < 0 {
            let e = errno();
            if e == libc::EINTR {
                continue;
            }
            if e == libc::EAGAIN {
                let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                unsafe { libc::poll(&mut p, 1, -1) };
                continue;
            }
            return Err(e);
        }
        b = &b[n as usize..];
    }
    Ok(())
}

/// Block until `fd` is readable, letting other green threads run meanwhile.
fn is_regular_file(fd: i32) -> bool {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    unsafe { libc::fstat(fd, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFREG }
}

fn wait_fd(fd: i32, events: i16) -> Result<(), V> {
    if is_regular_file(fd) {
        return Ok(());
    }
    poll_wait(&mut [libc::pollfd { fd, events, revents: 0 }], None).map(|_| ())
}

/// poll(2) on `pfds` until something is ready (returns poll's count) or `timeout` passes
/// (Ok(0)), without holding up this P's other threads or a collection: while the P has
/// other threads, poll without waiting and yield to them; alone, block in short slices
/// (counted as stopped for the GC), so a thread that comes to this P meanwhile (woken up,
/// or a sleeper whose time came) waits at most a slice.
fn poll_wait(pfds: &mut [libc::pollfd], timeout: Option<Duration>) -> Result<i32, V> {
    const SLICE_MS: i32 = 20;
    let start = std::time::Instant::now();
    loop {
        let remaining = timeout.map(|t| t.saturating_sub(start.elapsed()));
        let shared = crate::thread::idle_timeout().is_some();
        let ms = if shared { 0 } else { remaining.map_or(SLICE_MS, |r| (r.as_millis() as i32).min(SLICE_MS)) };
        let (ptr, len) = (pfds.as_mut_ptr() as usize, pfds.len() as libc::nfds_t);
        let n = if ms == 0 {
            unsafe { libc::poll(ptr as *mut libc::pollfd, len, 0) }
        } else {
            crate::gc::blocking(|| unsafe { libc::poll(ptr as *mut libc::pollfd, len, ms) })
        };
        if n > 0 || (n < 0 && errno() != libc::EINTR) {
            return Ok(n);
        }
        if remaining.is_some_and(|r| r.is_zero()) || timeout.is_some_and(|t| start.elapsed() >= t) {
            return Ok(0);
        }
        if shared {
            if crate::thread::sleep(Some(Duration::from_millis(2))) == UNDEF {
                return Err(UNDEF);
            }
        } else {
            crate::thread::pass();
            if pst().pending_is_set() {
                return Err(UNDEF);
            }
        }
    }
}

impl IoState {
    pub fn new(fd: i32, readable: bool, writable: bool) -> Box<IoState> {
        Box::new(IoState {
            fd,
            wfd: fd,
            std: 0,
            readable,
            writable,
            closed: false,
            rbuf: Vec::new(),
            rpos: 0,
            wbuf: Vec::new(),
            sync: false,
            path: None,
            lineno: 0,
            pid: 0,
            autoclose: true,
            binmode: false,
            stdio_copy: false,
            mem: None,
            mpos: 0,
            wbusy: std::sync::atomic::AtomicBool::new(false),
            rbusy: std::sync::atomic::AtomicBool::new(false),
        })
    }
    fn detail(&self) -> Vec<u8> {
        match &self.path {
            Some(p) => p.clone(),
            None => match self.std {
                1 => b"<STDOUT>".to_vec(),
                2 => b"<STDERR>".to_vec(),
                _ if self.fd == 0 => b"<STDIN>".to_vec(),
                _ => format!("fd {}", self.fd).into_bytes(),
            },
        }
    }
    fn check_open(&self) -> Result<(), V> {
        if self.closed { Err(io_error("closed stream")) } else { Ok(()) }
    }
    fn check_readable(&mut self) -> Result<(), V> {
        self.check_open()?;
        if !self.readable {
            return Err(io_error("not opened for reading"));
        }
        if !self.wbuf.is_empty() {
            self.flush()?;
        }
        Ok(())
    }
    fn check_writable(&mut self) -> Result<(), V> {
        self.check_open()?;
        if !self.writable {
            return Err(io_error("not opened for writing"));
        }
        if self.rpos < self.rbuf.len() && self.fd == self.wfd {
            // switching from reading to writing: give back the read-ahead
            let unread = (self.rbuf.len() - self.rpos) as i64;
            if unsafe { libc::lseek(self.fd, -unread as libc::off_t, libc::SEEK_CUR) } >= 0 {
                self.rbuf.clear();
                self.rpos = 0;
            }
        }
        Ok(())
    }
    fn flush_raw(&mut self) -> Result<(), i32> {
        if self.wbuf.is_empty() {
            return Ok(());
        }
        // held through the write: one flush's bytes stay together
        let wbusy = unsafe { &*(&self.wbusy as *const std::sync::atomic::AtomicBool) };
        let _g = BufLock::take(wbusy);
        let b = std::mem::take(&mut self.wbuf);
        match self.std {
            1 => {
                out(&b);
                Ok(())
            }
            2 => {
                err_out(&b);
                Ok(())
            }
            _ => {
                if self.wfd <= 2 || self.stdio_copy {
                    flush_out(); // keep ordering with Kenny's stdout buffer
                }
                let fd = self.wfd;
                if is_regular_file(fd) {
                    sys_write_all(fd, &b)
                } else {
                    // a socket or pipe may block until the other end reads
                    crate::gc::blocking(|| sys_write_all(fd, &b))
                }
            }
        }
    }
    pub fn flush(&mut self) -> Result<(), V> {
        if self.std == 1 {
            flush_out();
        }
        self.flush_raw().map_err(|e| raise_errno(e, None, &self.detail()))
    }
    pub fn write(&mut self, b: &[u8]) -> Result<(), V> {
        self.check_writable()?;
        match self.std {
            1 => {
                out(b);
                if self.sync {
                    flush_out();
                }
            }
            2 => err_out(b),
            _ => {
                let full = {
                    let wbusy = unsafe { &*(&self.wbusy as *const std::sync::atomic::AtomicBool) };
                    let _g = BufLock::take(wbusy);
                    self.wbuf.extend_from_slice(b);
                    self.wbuf.len() >= 8192
                };
                if self.sync || full {
                    self.flush()?;
                }
            }
        }
        Ok(())
    }
    fn avail(&self) -> usize {
        self.rbuf.len() - self.rpos
    }
    /// Read more bytes into the buffer (appending); returns how many (0 = EOF).
    fn fill_more(&mut self) -> Result<usize, V> {
        if self.rpos > 0 && self.rpos == self.rbuf.len() {
            self.rbuf.clear();
            self.rpos = 0;
        } else if self.rpos > (1 << 16) {
            self.rbuf.drain(..self.rpos);
            self.rpos = 0;
        }
        if let Some(e) = self.mem {
            let m = crate::vfs::contents(e);
            let from = self.mpos.min(m.len());
            let n = (m.len() - from).min(1 << 16);
            self.rbuf.extend_from_slice(&m[from..from + n]);
            self.mpos = from + n;
            return Ok(n);
        }
        if self.std == 0 && self.fd == 0 {
            flush_out();
        }
        wait_fd(self.fd, libc::POLLIN)?;
        let rbusy = unsafe { &*(&self.rbusy as *const std::sync::atomic::AtomicBool) };
        let _g = BufLock::take(rbusy);
        let old = self.rbuf.len();
        self.rbuf.resize(old + (1 << 16), 0);
        loop {
            // (wait_fd waited until a pipe, socket or terminal has data; regular files don't block)
            let n = unsafe { libc::read(self.fd, self.rbuf[old..].as_mut_ptr() as *mut c_void, 1 << 16) };
            if n < 0 {
                let e = errno();
                if e == libc::EINTR {
                    continue;
                }
                self.rbuf.truncate(old);
                if e == libc::EAGAIN {
                    wait_fd(self.fd, libc::POLLIN)?;
                    self.rbuf.resize(old + (1 << 16), 0);
                    continue;
                }
                return Err(raise_errno(e, Some("io_fread"), &self.detail()));
            }
            self.rbuf.truncate(old + n as usize);
            return Ok(n as usize);
        }
    }
    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.avail());
        let v = self.rbuf[self.rpos..self.rpos + n].to_vec();
        self.rpos += n;
        v
    }
    pub fn read_all(&mut self) -> Result<Vec<u8>, V> {
        self.check_readable()?;
        while self.fill_more()? > 0 {}
        Ok(self.take(self.avail()))
    }
    pub fn read_upto(&mut self, n: usize) -> Result<Vec<u8>, V> {
        self.check_readable()?;
        while self.avail() < n {
            if self.fill_more()? == 0 {
                break;
            }
        }
        Ok(self.take(n))
    }
    fn read_partial(&mut self, n: usize) -> Result<Vec<u8>, V> {
        self.check_readable()?;
        if self.avail() == 0 {
            self.fill_more()?;
        }
        Ok(self.take(n))
    }
    fn getbyte(&mut self) -> Result<Option<u8>, V> {
        self.check_readable()?;
        if self.avail() == 0 && self.fill_more()? == 0 {
            return Ok(None);
        }
        let b = self.rbuf[self.rpos];
        self.rpos += 1;
        Ok(Some(b))
    }
    fn getc(&mut self) -> Result<Option<Vec<u8>>, V> {
        self.check_readable()?;
        if self.avail() == 0 && self.fill_more()? == 0 {
            return Ok(None);
        }
        let b = self.rbuf[self.rpos];
        let len = if self.binmode {
            1
        } else if b >= 0xf0 {
            4
        } else if b >= 0xe0 {
            3
        } else if b >= 0xc0 {
            2
        } else {
            1
        };
        while self.avail() < len {
            if self.fill_more()? == 0 {
                break;
            }
        }
        Ok(Some(self.take(len)))
    }
    fn unget(&mut self, b: &[u8]) {
        if self.rpos >= b.len() {
            self.rpos -= b.len();
            self.rbuf[self.rpos..self.rpos + b.len()].copy_from_slice(b);
        } else {
            let rest = self.rbuf.split_off(self.rpos);
            self.rbuf.clear();
            self.rbuf.extend_from_slice(b);
            self.rbuf.extend_from_slice(&rest);
            self.rpos = 0;
        }
    }
    fn eof(&mut self) -> Result<bool, V> {
        self.check_readable()?;
        Ok(self.avail() == 0 && self.fill_more()? == 0)
    }
    pub fn gets(&mut self, o: &LineOpts) -> Result<Option<Vec<u8>>, V> {
        self.check_readable()?;
        if o.limit == Some(0) {
            return Ok(Some(Vec::new()));
        }
        let Some(sep) = &o.sep else {
            let all = match o.limit {
                Some(l) => self.read_upto(l)?,
                None => self.read_all()?,
            };
            if all.is_empty() {
                return Ok(None);
            }
            self.lineno += 1;
            return Ok(Some(all));
        };
        let paragraph = sep.is_empty();
        let sep: &[u8] = if paragraph { b"\n\n" } else { sep };
        if paragraph {
            loop {
                if self.avail() == 0 && self.fill_more()? == 0 {
                    break;
                }
                if self.rbuf[self.rpos] == b'\n' {
                    self.rpos += 1;
                } else {
                    break;
                }
            }
        }
        let mut scanned = 0usize; // relative to rpos
        let end_rel = loop {
            let hay = &self.rbuf[self.rpos..];
            if let Some(i) = crate::string::find(hay, sep, scanned) {
                break i + sep.len();
            }
            if let Some(l) = o.limit {
                if hay.len() >= l {
                    break l;
                }
            }
            scanned = hay.len().saturating_sub(sep.len() - 1);
            if self.fill_more()? == 0 {
                break self.avail();
            }
        };
        let mut end_rel = end_rel;
        if let Some(l) = o.limit {
            if end_rel > l {
                end_rel = l;
            }
            // don't split a UTF-8 character
            while end_rel < self.avail() && self.rbuf[self.rpos + end_rel] & 0xc0 == 0x80 {
                end_rel += 1;
            }
        }
        if end_rel == 0 {
            return Ok(None);
        }
        let mut line = self.take(end_rel);
        if paragraph {
            loop {
                if self.avail() == 0 && self.fill_more()? == 0 {
                    break;
                }
                if self.rbuf[self.rpos] == b'\n' {
                    self.rpos += 1;
                } else {
                    break;
                }
            }
        }
        if o.chomp {
            if line.ends_with(sep) {
                line.truncate(line.len() - sep.len());
                if sep == b"\n" && line.ends_with(b"\r") {
                    line.pop();
                }
            } else if paragraph && line.ends_with(b"\n") {
                line.pop();
            }
        }
        self.lineno += 1;
        Ok(Some(line))
    }
    fn tell(&mut self) -> Result<i64, V> {
        self.check_open()?;
        if self.std != 0 {
            return Ok(0);
        }
        if self.mem.is_some() {
            return Ok(self.mpos as i64 - self.avail() as i64);
        }
        let p = unsafe { libc::lseek(self.fd, 0, libc::SEEK_CUR) };
        if p < 0 {
            return Err(raise_errno(errno(), None, &self.detail()));
        }
        Ok(p as i64 - self.avail() as i64 + self.wbuf.len() as i64)
    }
    fn seek(&mut self, off: i64, whence: i32) -> Result<(), V> {
        self.check_open()?;
        self.flush()?;
        let off = if whence == libc::SEEK_CUR { off - self.avail() as i64 } else { off };
        if let Some(e) = self.mem {
            let base = match whence {
                libc::SEEK_CUR => self.mpos as i64,
                libc::SEEK_END => e.size as i64,
                _ => 0,
            };
            if base + off < 0 {
                return Err(raise_errno(libc::EINVAL, None, &self.detail()));
            }
            self.rbuf.clear();
            self.rpos = 0;
            self.mpos = (base + off) as usize;
            return Ok(());
        }
        self.rbuf.clear();
        self.rpos = 0;
        if unsafe { libc::lseek(self.fd, off as libc::off_t, whence) } < 0 {
            return Err(raise_errno(errno(), None, &self.detail()));
        }
        Ok(())
    }
    /// Flush and close the descriptors; for popen, reap the child and set `$?`.
    pub fn close(&mut self) -> Result<(), V> {
        if self.closed {
            return Ok(());
        }
        let r = self.flush_raw();
        self.close_fds();
        unregister(self);
        if self.pid > 0 {
            let pid = self.pid;
            self.pid = 0;
            if let Some((p, st)) = waitpid_v(pid, 0)? {
                set_last_status(p, st);
            }
        }
        r.map_err(|e| raise_errno(e, None, &self.detail()))
    }
    /// An embedded file that something needs a real descriptor for (`fileno`, `dup`, a
    /// child's redirection...): continue reading it from a temporary copy.
    fn materialize(&mut self) -> Result<(), V> {
        let Some(e) = self.mem else { return Ok(()) };
        let fd = embedded_fd(self.path.as_deref().unwrap_or_default(), e)?;
        unsafe { libc::lseek(fd, self.mpos as libc::off_t, libc::SEEK_SET) };
        self.fd = fd;
        self.wfd = fd;
        self.mem = None;
        Ok(())
    }
    /// `fstat` of the open file (an embedded file's own stat).
    fn fstat(&self) -> Result<libc::stat, i32> {
        if let Some(e) = self.mem {
            return Ok(crate::vfs::stat(e));
        }
        do_fstat(if self.std != 0 { self.std as i32 } else { self.fd })
    }
    fn close_fds(&mut self) {
        self.closed = true;
        self.readable = false;
        self.writable = false;
        if self.std == 0 && self.autoclose {
            if self.fd > 2 {
                unsafe { libc::close(self.fd) };
            }
            if self.wfd != self.fd && self.wfd > 2 {
                unsafe { libc::close(self.wfd) };
            }
        }
    }
}

fn unregister(st: &IoState) {
    let p = st as *const IoState as usize;
    let mut open = OPEN.get();
    if let Some(i) = open.iter().position(|x| *x == p) {
        open.swap_remove(i);
    }
}
/// Flush every open file's write buffer (and Kenny's stdout buffer).
pub fn flush_all() {
    flush_out();
    let open: Vec<usize> = OPEN.get().clone();
    for p in open {
        let st = unsafe { &mut *(p as *mut IoState) };
        let _ = st.flush_raw();
    }
    flush_out();
}

unsafe extern "C" fn io_free(p: *mut c_void) {
    let mut st = unsafe { Box::from_raw(p as *mut IoState) };
    if !st.closed {
        let _ = st.flush_raw();
        st.close_fds();
    }
    unregister(&st);
}

fn attach(obj: V, st: Box<IoState>) {
    let p = Box::into_raw(st);
    OPEN.get().push(p as usize);
    let d = alloc(RData {
        h: Header { klass: rt().c.object, kind: Kind::Data, flags: 0 },
        data: p as *mut c_void,
        dmark: None,
        dfree: Some(io_free),
        roots: Vec::new(),
    });
    ivar_set(obj, *S_IO.get(), d);
}
fn try_st(v: V) -> Option<&'static mut IoState> {
    if !is_kind(v, Kind::Object) {
        return None;
    }
    let d = ivar_get(v, *S_IO.get());
    if is_kind(d, Kind::Data) {
        let p = unsafe { (*(d as *mut RData)).data } as *mut IoState;
        if !p.is_null() {
            return Some(unsafe { &mut *p });
        }
    }
    None
}
fn st(v: V) -> Result<&'static mut IoState, V> {
    try_st(v).ok_or_else(|| io_error("uninitialized stream"))
}
fn new_io(cls: *mut Class, st: Box<IoState>) -> V {
    let o = object_new(cls);
    attach(o, st);
    o
}
/// The stream to use for Kernel#puts etc. None = the real STDOUT (fast path).
fn redirected(gv: u32, std: V) -> Option<V> {
    match rt().gvars.get(&gv) {
        Some(&v) if v != std && v != QNIL => Some(v),
        _ => None,
    }
}
/// Close `f` after a block ran; keeps a pending exception from the block.
fn close_after(f: V, r: V) -> V {
    if r == UNDEF {
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        if let Some(s) = try_st(f) {
            let _ = s.close();
        }
        pst().pending = saved;
        return UNDEF;
    }
    if let Some(s) = try_st(f) {
        if !s.closed {
            ok!(s.close());
        }
    } else {
        tri!(send(f, sym("close"), &[], QNIL));
    }
    r
}

// ---------- open modes ----------

struct Mode {
    flags: i32,
    r: bool,
    w: bool,
    bin: bool,
}
fn mode_from_flags(f: i32) -> Mode {
    let acc = f & libc::O_ACCMODE;
    Mode { flags: f, r: acc != libc::O_WRONLY, w: acc != libc::O_RDONLY, bin: false }
}
fn parse_mode_str(s: &[u8]) -> Result<Mode, V> {
    let main = s.split(|c| *c == b':').next().unwrap_or(b"");
    let bad = || arg_error(&format!("invalid access mode {}", bstr(s)));
    let Some(&first) = main.first() else { return Err(bad()) };
    let (mut flags, mut r, mut w) = match first {
        b'r' => (libc::O_RDONLY, true, false),
        b'w' => (libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, false, true),
        b'a' => (libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, false, true),
        _ => return Err(bad()),
    };
    let mut bin = false;
    for &c in &main[1..] {
        match c {
            b'+' => {
                flags = (flags & !libc::O_ACCMODE) | libc::O_RDWR;
                r = true;
                w = true;
            }
            b'b' => bin = true,
            b't' => {}
            b'x' if first == b'w' => flags |= libc::O_EXCL,
            _ => return Err(bad()),
        }
    }
    Ok(Mode { flags, r, w, bin })
}
fn parse_mode(m: Option<V>, kw: Option<V>, default: &[u8]) -> Result<Mode, V> {
    let m = m.filter(|x| *x != QNIL).or_else(|| kw_get(kw, "mode").filter(|x| *x != QNIL));
    let mut mode = match m {
        None => parse_mode_str(default)?,
        Some(x) if is_fix(x) => mode_from_flags(fix(x) as i32),
        Some(x) => parse_mode_str(&str_bytes(x)?)?,
    };
    if let Some(f) = kw_get(kw, "flags").filter(|x| is_fix(*x)) {
        mode = Mode { flags: mode.flags | fix(f) as i32, ..mode };
    }
    if kw_true(kw, "binmode") {
        mode.bin = true;
    }
    Ok(mode)
}
fn open_fd(path: &[u8], flags: i32, perm: u32) -> Result<i32, V> {
    if !is_write_flags(flags) {
        if let Some(e) = crate::vfs::find(path).filter(|e| !e.dir) {
            return embedded_fd(path, e);
        }
    }
    let c = if is_write_flags(flags) { wpath(path)? } else { dpath(path)? };
    loop {
        let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC, perm as libc::c_uint) };
        if fd >= 0 {
            return Ok(fd);
        }
        let e = errno();
        if e != libc::EINTR {
            return Err(raise_errno(e, Some("rb_sysopen"), path));
        }
    }
}
/// Open a file into a fresh (unregistered) state; used by the path-based class methods.
/// A descriptor for an embedded file (for code that needs a real fd): an unlinked temporary
/// file holding its contents.
fn embedded_fd(path: &[u8], e: &'static crate::vfs::Entry) -> Result<i32, V> {
    let mut t = tmpdir();
    t.extend_from_slice(format!("/kenny-embedded-{}-{}", std::process::id(), e.ino).as_bytes());
    let c = cstr(&t)?;
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        return Err(raise_errno(errno(), Some("rb_sysopen"), path));
    }
    unsafe { libc::unlink(c.as_ptr()) };
    let r = sys_write_all(fd, crate::vfs::contents(e));
    if let Err(x) = r {
        unsafe { libc::close(fd) };
        return Err(raise_errno(x, Some("rb_sysopen"), path));
    }
    unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    Ok(fd)
}
fn open_state(path: &[u8], mode: &Mode, perm: u32) -> Result<Box<IoState>, V> {
    if !is_write_flags(mode.flags) {
        if let Some(e) = crate::vfs::find(path).filter(|e| !e.dir) {
            let mut st = IoState::new(-1, true, false);
            st.mem = Some(e);
            st.path = Some(path.to_vec());
            st.binmode = mode.bin;
            return Ok(st);
        }
    }
    let fd = open_fd(path, mode.flags, perm)?;
    let mut st = IoState::new(fd, mode.r, mode.w);
    st.path = Some(path.to_vec());
    st.binmode = mode.bin;
    Ok(st)
}
fn with_file<T>(path: &[u8], mode: &Mode, perm: u32, f: impl FnOnce(&mut IoState) -> Result<T, V>) -> Result<T, V> {
    let mut st = open_state(path, mode, perm)?;
    let r = f(&mut st);
    if r.is_err() {
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        let _ = st.flush_raw();
        st.close_fds();
        pst().pending = saved;
        return r;
    }
    st.close()?;
    r
}

// ---------- line options ----------

pub struct LineOpts {
    sep: Option<Vec<u8>>,
    limit: Option<usize>,
    chomp: bool,
}
fn default_sep() -> Option<Vec<u8>> {
    match rt().gvars.get(&sym("$/")) {
        Some(&v) if is_str(v) => Some(rstr(v).b.clone()),
        Some(&QNIL) => None,
        _ => Some(b"\n".to_vec()),
    }
}
fn line_opts(a: &[V]) -> Result<LineOpts, V> {
    let (a, kw) = split_kw(a);
    let mut o = LineOpts { sep: default_sep(), limit: None, chomp: kw_true(kw, "chomp") };
    let lim = |v: V| -> Result<Option<usize>, V> {
        if v == QNIL {
            return Ok(None);
        }
        let n = int_arg(v)?;
        Ok(if n < 0 { None } else { Some(n as usize) })
    };
    match a.len() {
        0 => {}
        1 if is_fix(a[0]) => o.limit = lim(a[0])?,
        1 => o.sep = if a[0] == QNIL { None } else { Some(str_bytes(a[0])?) },
        _ => {
            o.sep = if a[0] == QNIL { None } else { Some(str_bytes(a[0])?) };
            o.limit = lim(a[1])?;
        }
    }
    Ok(o)
}
fn gets_v(s: V, a: &[V]) -> V {
    let o = ok!(line_opts(a));
    let st = ok!(st(s));
    let r = match ok!(st.gets(&o)) {
        Some(l) => sv(l),
        None => QNIL,
    };
    let n = st.lineno;
    set_gvar("$.", mkfix(n));
    r
}
fn each_line_v(s: V, a: &[V], b: V) -> V {
    if !is_proc(b) {
        return crate::misc::enum_for(s, "each_line", a);
    }
    let o = ok!(line_opts(a));
    loop {
        let l = match ok!(ok!(st(s)).gets(&o)) {
            Some(l) => sv(l),
            None => return s,
        };
        tri!(call_block(b, &[l]));
    }
}
fn readlines_v(s: V, a: &[V]) -> V {
    let o = ok!(line_opts(a));
    let st = ok!(st(s));
    // rooted: reading a pipe may switch green threads (and run the GC)
    let mut v = RVec::new();
    while let Some(l) = ok!(st.gets(&o)) {
        v.push(sv(l));
    }
    ary_new(v.take())
}

// ---------- File::Stat ----------

fn do_stat(path: &[u8], follow: bool) -> Result<Result<libc::stat, i32>, V> {
    if let Some(e) = crate::vfs::find(path) {
        return Ok(Ok(crate::vfs::stat(e)));
    }
    let c = dpath(path)?;
    let mut s: libc::stat = unsafe { std::mem::zeroed() };
    let r = unsafe { if follow { libc::stat(c.as_ptr(), &mut s) } else { libc::lstat(c.as_ptr(), &mut s) } };
    Ok(if r == 0 { Ok(s) } else { Err(errno()) })
}
fn do_fstat(fd: i32) -> Result<libc::stat, i32> {
    let mut s: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut s) } == 0 { Ok(s) } else { Err(errno()) }
}
/// stat for a path-or-IO argument; Ok(None) if it doesn't exist.
fn stat_arg(v: V, follow: bool) -> Result<Option<libc::stat>, V> {
    if let Some(s) = try_st(v) {
        return Ok(s.fstat().ok());
    }
    let p = path_bytes(v)?;
    Ok(do_stat(&p, follow)?.ok())
}
fn stat_or_raise(v: V, follow: bool, func: &str) -> Result<libc::stat, V> {
    if let Some(s) = try_st(v) {
        s.check_open()?;
        let _ = s.flush_raw();
        return s.fstat().map_err(|e| raise_errno(e, Some(func), &s.detail()));
    }
    let p = path_bytes(v)?;
    do_stat(&p, follow)?.map_err(|e| raise_errno(e, Some(func), &p))
}
fn ts(sec: i64, nsec: i64) -> f64 {
    sec as f64 + nsec as f64 * 1e-9
}
fn stat_new(s: &libc::stat) -> V {
    let o = object_new(*C_STAT.get());
    let set = |k: &str, v: V| {
        ivar_set(o, sym(k), v);
    };
    set("@__mode", mkfix(s.st_mode as i64));
    set("@__size", mkfix(s.st_size as i64));
    set("@__dev", mkfix(s.st_dev as i64));
    set("@__ino", int_from_i128(s.st_ino as i128));
    set("@__nlink", mkfix(s.st_nlink as i64));
    set("@__uid", mkfix(s.st_uid as i64));
    set("@__gid", mkfix(s.st_gid as i64));
    set("@__rdev", mkfix(s.st_rdev as i64));
    set("@__blksize", mkfix(s.st_blksize as i64));
    set("@__blocks", mkfix(s.st_blocks as i64));
    set("@__atime", float_new(ts(s.st_atime as i64, s.st_atime_nsec as i64)));
    set("@__mtime", float_new(ts(s.st_mtime as i64, s.st_mtime_nsec as i64)));
    set("@__ctime", float_new(ts(s.st_ctime as i64, s.st_ctime_nsec as i64)));
    #[cfg(target_os = "macos")]
    set("@__birthtime", float_new(ts(s.st_birthtime as i64, s.st_birthtime_nsec as i64)));
    o
}
fn sf(o: V, k: &str) -> i64 {
    let v = ivar_get(o, sym(k));
    if is_fix(v) { fix(v) } else { 0 }
}
fn ftype_of(mode: u32) -> &'static str {
    match mode & libc::S_IFMT as u32 {
        x if x == libc::S_IFREG as u32 => "file",
        x if x == libc::S_IFDIR as u32 => "directory",
        x if x == libc::S_IFLNK as u32 => "link",
        x if x == libc::S_IFCHR as u32 => "characterSpecial",
        x if x == libc::S_IFBLK as u32 => "blockSpecial",
        x if x == libc::S_IFIFO as u32 => "fifo",
        x if x == libc::S_IFSOCK as u32 => "socket",
        _ => "unknown",
    }
}
fn is_fmt(mode: u32, fmt: libc::mode_t) -> bool {
    mode & libc::S_IFMT as u32 == fmt as u32
}
/// Permission check against a stat (like CRuby's rb_stat_r/w/x).
fn stat_access(mode: u32, uid: u32, gid: u32, bits: (u32, u32, u32)) -> bool {
    let euid = unsafe { libc::geteuid() };
    if euid == 0 {
        // root: read/write always; execute if any x bit (or a directory)
        return bits.0 != libc::S_IXUSR as u32 || mode & 0o111 != 0 || is_fmt(mode, libc::S_IFDIR);
    }
    if uid == euid {
        return mode & bits.0 != 0;
    }
    if gid == unsafe { libc::getegid() } {
        return mode & bits.1 != 0;
    }
    mode & bits.2 != 0
}
fn time_v(secs: f64) -> V {
    crate::misc::time_new(secs, false)
}

// ---------- paths ----------

fn basename(p: &[u8]) -> Vec<u8> {
    if p.is_empty() {
        return Vec::new();
    }
    let mut end = p.len();
    while end > 0 && p[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return b"/".to_vec();
    }
    let t = &p[..end];
    match t.iter().rposition(|c| *c == b'/') {
        Some(i) => t[i + 1..].to_vec(),
        None => t.to_vec(),
    }
}
fn dirname(p: &[u8]) -> Vec<u8> {
    let abs = p.starts_with(b"/");
    let mut end = p.len();
    while end > 0 && p[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return if abs { b"/".to_vec() } else { b".".to_vec() };
    }
    match p[..end].iter().rposition(|c| *c == b'/') {
        None => b".".to_vec(),
        Some(i) => {
            let mut j = i;
            while j > 0 && p[j - 1] == b'/' {
                j -= 1;
            }
            if j == 0 { b"/".to_vec() } else { p[..j].to_vec() }
        }
    }
}
fn extname(p: &[u8]) -> Vec<u8> {
    let b = basename(p);
    let start = b.iter().position(|c| *c != b'.').unwrap_or(b.len());
    match b[start..].iter().rposition(|c| *c == b'.') {
        None => Vec::new(),
        Some(i) => b[start + i..].to_vec(),
    }
}
fn home_dir(user: &[u8]) -> Result<Vec<u8>, V> {
    if user.is_empty() {
        return match std::env::var_os("HOME") {
            Some(h) => Ok(h.as_bytes().to_vec()),
            None => Err(arg_error("couldn't find login name -- expanding '~'")),
        };
    }
    let c = cstr(user)?;
    let pw = unsafe { libc::getpwnam(c.as_ptr()) };
    if pw.is_null() {
        return Err(arg_error(&format!("user {} doesn't exist", bstr(user))));
    }
    Ok(unsafe { CStr::from_ptr((*pw).pw_dir) }.to_bytes().to_vec())
}
fn normalize(full: &[u8]) -> Vec<u8> {
    let mut segs: Vec<&[u8]> = Vec::new();
    for s in full.split(|c| *c == b'/') {
        match s {
            b"" | b"." => {}
            b".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let mut out = Vec::new();
    for s in segs {
        out.push(b'/');
        out.extend_from_slice(s);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}
fn expand(p: &[u8], dir: Option<&[u8]>, tilde: bool) -> Result<Vec<u8>, V> {
    let full = if tilde && p.starts_with(b"~") {
        let rest = &p[1..];
        let cut = rest.iter().position(|c| *c == b'/').unwrap_or(rest.len());
        let mut h = home_dir(&rest[..cut])?;
        h.extend_from_slice(&rest[cut..]);
        if !h.starts_with(b"/") {
            return Err(arg_error("non-absolute home"));
        }
        h
    } else if p.starts_with(b"/") {
        p.to_vec()
    } else {
        let mut base = match dir {
            Some(d) => expand(d, None, tilde)?,
            None => cwd(),
        };
        base.push(b'/');
        base.extend_from_slice(p);
        base
    };
    Ok(normalize(&full))
}
fn join_parts(v: V, out: &mut Vec<Vec<u8>>, depth: usize) -> Result<(), V> {
    if depth > 64 {
        return Err(arg_error("recursive array"));
    }
    if is_ary(v) {
        for x in crate::gc::Rooted::of(rary(v).v.clone()) {
            join_parts(x, out, depth + 1)?;
        }
        return Ok(());
    }
    out.push(path_bytes(v)?);
    Ok(())
}
fn file_join(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            out.extend_from_slice(p);
            continue;
        }
        let ends = out.last() == Some(&b'/');
        let starts = p.first() == Some(&b'/');
        if ends && starts {
            let k = p.iter().position(|c| *c != b'/').unwrap_or(p.len());
            out.extend_from_slice(&p[k..]);
        } else if ends || starts {
            out.extend_from_slice(p);
        } else {
            out.push(b'/');
            out.extend_from_slice(p);
        }
    }
    out
}

// ---------- fnmatch / glob ----------

fn upc(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}
fn at(v: &[char], i: usize) -> char {
    v.get(i).copied().unwrap_or('\0')
}
fn bracket(p: &[char], mut i: usize, c: char, flags: i64) -> Option<usize> {
    let nocase = flags & FNM_CASEFOLD != 0;
    let escape = flags & FNM_NOESCAPE == 0;
    if i >= p.len() {
        return None;
    }
    let (mut ok, mut not) = (false, false);
    if p[i] == '!' || p[i] == '^' {
        not = true;
        i += 1;
    }
    while at(p, i) != ']' {
        let mut t1 = i;
        if escape && at(p, t1) == '\\' {
            t1 += 1;
        }
        if t1 >= p.len() {
            return None;
        }
        i = t1 + 1;
        if i >= p.len() {
            return None;
        }
        if p[i] == '-' && at(p, i + 1) != ']' {
            let mut t2 = i + 1;
            if escape && at(p, t2) == '\\' {
                t2 += 1;
            }
            if t2 >= p.len() {
                return None;
            }
            i = t2 + 1;
            if ok {
                continue;
            }
            if p[t1] == c || p[t2] == c {
                ok = true;
                continue;
            }
            let f = |x: char| if nocase { upc(x) } else { x };
            let c1 = f(c);
            if c1 < f(p[t1]) || c1 > f(p[t2]) {
                continue;
            }
            ok = true;
        } else {
            if ok {
                continue;
            }
            if p[t1] == c || (nocase && upc(p[t1]) == upc(c)) {
                ok = true;
            }
        }
    }
    if ok == not { None } else { Some(i + 1) }
}
/// CRuby's fnmatch_helper: match one path segment (or the whole string without FNM_PATHNAME).
fn fnm_helper(p: &[char], pcur: &mut usize, s: &[char], scur: &mut usize, flags: i64) -> bool {
    let period = flags & FNM_DOTMATCH == 0;
    let pathname = flags & FNM_PATHNAME != 0;
    let escape = flags & FNM_NOESCAPE == 0;
    let nocase = flags & FNM_CASEFOLD != 0;
    let isend = |v: &[char], i: usize| i >= v.len() || (pathname && v[i] == '/');
    let unescape = |i: usize| if escape && at(p, i) == '\\' { i + 1 } else { i };
    let (mut pi, mut si) = (*pcur, *scur);
    let (mut ptmp, mut stmp): (Option<usize>, Option<usize>) = (None, None);
    macro_rules! ret {
        ($v:expr) => {{
            *pcur = pi;
            *scur = si;
            return $v;
        }};
    }
    if period && at(s, si) == '.' && at(p, unescape(pi)) != '.' {
        ret!(false);
    }
    'outer: loop {
        let failed = 'step: {
            match at(p, pi) {
                '*' => {
                    while at(p, pi) == '*' {
                        pi += 1;
                    }
                    if isend(p, unescape(pi)) {
                        pi = unescape(pi);
                        ret!(true);
                    }
                    if isend(s, si) {
                        ret!(false);
                    }
                    ptmp = Some(pi);
                    stmp = Some(si);
                    continue 'outer;
                }
                '?' => {
                    if isend(s, si) {
                        ret!(false);
                    }
                    pi += 1;
                    si += 1;
                    continue 'outer;
                }
                '[' => {
                    if isend(s, si) {
                        ret!(false);
                    }
                    if let Some(t) = bracket(p, pi + 1, s[si], flags) {
                        pi = t;
                        si += 1;
                        continue 'outer;
                    }
                    break 'step true;
                }
                _ => {}
            }
            let pu = unescape(pi);
            if isend(s, si) {
                pi = pu;
                ret!(isend(p, pu));
            }
            if isend(p, pu) {
                break 'step true;
            }
            if p[pu] == s[si] || (nocase && upc(p[pu]) == upc(s[si])) {
                pi = pu + 1;
                si += 1;
                continue 'outer;
            }
            true
        };
        if failed {
            if let (Some(pt), Some(st)) = (ptmp, stmp) {
                pi = pt;
                let st = st + 1;
                stmp = Some(st);
                si = st;
                continue;
            }
            ret!(false);
        }
    }
}
fn fnmatch_chars(p: &[char], s: &[char], flags: i64) -> bool {
    let period = flags & FNM_DOTMATCH == 0;
    let (mut pi, mut si) = (0usize, 0usize);
    if flags & FNM_PATHNAME == 0 {
        return fnm_helper(p, &mut pi, s, &mut si, flags);
    }
    let (mut ptmp, mut stmp): (Option<usize>, Option<usize>) = (None, None);
    loop {
        if at(p, pi) == '*' && at(p, pi + 1) == '*' && at(p, pi + 2) == '/' {
            while at(p, pi) == '*' && at(p, pi + 1) == '*' && at(p, pi + 2) == '/' {
                pi += 3;
            }
            ptmp = Some(pi);
            stmp = Some(si);
        }
        if fnm_helper(p, &mut pi, s, &mut si, flags) {
            while si < s.len() && s[si] != '/' {
                si += 1;
            }
            if pi < p.len() && si < s.len() {
                pi += 1;
                si += 1;
                continue;
            }
            if pi >= p.len() && si >= s.len() {
                return true;
            }
        }
        if let (Some(pt), Some(st)) = (ptmp, stmp) {
            if !(period && at(s, st) == '.') {
                let mut st = st;
                while st < s.len() && s[st] != '/' {
                    st += 1;
                }
                if st < s.len() {
                    pi = pt;
                    st += 1;
                    stmp = Some(st);
                    si = st;
                    continue;
                }
            }
        }
        return false;
    }
}
fn chars(b: &[u8]) -> Vec<char> {
    String::from_utf8_lossy(b).chars().collect()
}
pub fn fnmatch(pat: &[u8], s: &[u8], flags: i64) -> bool {
    let sc = chars(s);
    if flags & FNM_EXTGLOB != 0 {
        return brace_expand(pat, flags & FNM_NOESCAPE == 0).iter().any(|p| fnmatch_chars(&chars(p), &sc, flags));
    }
    fnmatch_chars(&chars(pat), &sc, flags)
}
fn brace_expand(p: &[u8], escape: bool) -> Vec<Vec<u8>> {
    let (mut i, mut depth) = (0usize, 0i32);
    let (mut l, mut r) = (None, None);
    while i < p.len() {
        let c = p[i];
        if c == b'\\' && escape {
            i += 2;
            continue;
        }
        if c == b'{' {
            if depth == 0 {
                l = Some(i);
            }
            depth += 1;
        } else if c == b'}' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                r = Some(i);
                break;
            }
        }
        i += 1;
    }
    let (Some(l), Some(r)) = (l, r) else { return vec![p.to_vec()] };
    let mut alts: Vec<&[u8]> = Vec::new();
    let (mut j, mut d, mut start) = (l + 1, 0i32, l + 1);
    while j < r {
        let c = p[j];
        if c == b'\\' && escape {
            j += 2;
            continue;
        }
        match c {
            b'{' => d += 1,
            b'}' => d -= 1,
            b',' if d == 0 => {
                alts.push(&p[start..j]);
                start = j + 1;
            }
            _ => {}
        }
        j += 1;
    }
    alts.push(&p[start..r]);
    let mut out = Vec::new();
    for a in alts {
        let mut s = p[..l].to_vec();
        s.extend_from_slice(a);
        s.extend_from_slice(&p[r + 1..]);
        out.extend(brace_expand(&s, escape));
    }
    out
}

#[derive(Clone)]
enum Seg {
    Rec,
    Lit(Vec<u8>),
    Pat(Vec<u8>),
    DirMark,
}
fn has_magic(s: &[u8], flags: i64) -> bool {
    s.iter().any(|c| matches!(c, b'*' | b'?' | b'[')) || (flags & FNM_NOESCAPE == 0 && s.contains(&b'\\'))
}
fn unescape_lit(s: &[u8], flags: i64) -> Vec<u8> {
    if flags & FNM_NOESCAPE != 0 {
        return s.to_vec();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && i + 1 < s.len() {
            i += 1;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}
fn split_glob(p: &[u8], flags: i64) -> (Vec<u8>, Vec<Seg>) {
    let root = if p.starts_with(b"/") { b"/".to_vec() } else { Vec::new() };
    let parts: Vec<&[u8]> = p.split(|c| *c == b'/').collect();
    let n = parts.len();
    let mut segs = Vec::new();
    for (i, s) in parts.iter().enumerate() {
        if s.is_empty() {
            if i == n - 1 && i > 0 && !segs.is_empty() {
                segs.push(Seg::DirMark);
            }
            continue;
        }
        if *s == b"**" && i + 1 < n {
            if !matches!(segs.last(), Some(Seg::Rec)) {
                segs.push(Seg::Rec);
            }
        } else if has_magic(s, flags) {
            segs.push(Seg::Pat(s.to_vec()));
        } else {
            segs.push(Seg::Lit(unescape_lit(s, flags)));
        }
    }
    (root, segs)
}
struct Globber<'a> {
    base: Option<&'a [u8]>,
    flags: i64,
    sort: bool,
    out: Vec<Vec<u8>>,
}
impl Globber<'_> {
    fn fs(&self, path: &[u8]) -> Vec<u8> {
        match self.base {
            Some(b) if !path.starts_with(b"/") => {
                let mut v = b.to_vec();
                if !path.is_empty() {
                    if !v.ends_with(b"/") {
                        v.push(b'/');
                    }
                    v.extend_from_slice(path);
                }
                v
            }
            _ => {
                if path.is_empty() { b".".to_vec() } else { path.to_vec() }
            }
        }
    }
    fn join(path: &[u8], name: &[u8]) -> Vec<u8> {
        let mut v = path.to_vec();
        if !v.is_empty() && !v.ends_with(b"/") {
            v.push(b'/');
        }
        v.extend_from_slice(name);
        v
    }
    fn exists(&self, path: &[u8]) -> bool {
        let p = self.fs(path);
        if crate::vfs::find(&p).is_some() {
            return true;
        }
        CString::new(crate::vfs::disk(&p).into_owned()).is_ok_and(|c| {
            let mut s: libc::stat = unsafe { std::mem::zeroed() };
            unsafe { libc::lstat(c.as_ptr(), &mut s) == 0 }
        })
    }
    fn is_dir(&self, path: &[u8], follow: bool) -> bool {
        let p = self.fs(path);
        if let Some(e) = crate::vfs::find(&p) {
            return e.dir;
        }
        CString::new(crate::vfs::disk(&p).into_owned()).is_ok_and(|c| {
            let mut s: libc::stat = unsafe { std::mem::zeroed() };
            let r = unsafe { if follow { libc::stat(c.as_ptr(), &mut s) } else { libc::lstat(c.as_ptr(), &mut s) } };
            r == 0 && is_fmt(s.st_mode as u32, libc::S_IFDIR)
        })
    }
    fn run(&mut self, path: &[u8], pats: &[&[Seg]]) {
        let mut rest: Vec<&[Seg]> = Vec::new();
        let (mut match_all, mut match_dir) = (false, false);
        for p in pats {
            let rec = matches!(p.first(), Some(Seg::Rec));
            let q = if rec { &p[1..] } else { p };
            if q.is_empty() {
                match_all = true;
            } else if matches!(q[0], Seg::DirMark) {
                match_dir = true;
            }
            if rec || !(q.is_empty() || matches!(q[0], Seg::DirMark)) {
                rest.push(p);
            }
        }
        if match_all && !path.is_empty() {
            self.out.push(path.to_vec());
        }
        if match_dir && !path.is_empty() && self.is_dir(path, true) {
            let mut v = path.to_vec();
            if !v.ends_with(b"/") {
                v.push(b'/');
            }
            self.out.push(v);
        }
        if rest.is_empty() {
            return;
        }
        let magical = rest.iter().any(|p| matches!(p[0], Seg::Rec | Seg::Pat(_)));
        if !magical {
            for p in rest {
                if let Seg::Lit(name) = &p[0] {
                    let np = Self::join(path, name);
                    if self.exists(&np) {
                        self.run(&np, &[&p[1..]]);
                    }
                }
            }
            return;
        }
        let dir = self.fs(path);
        let mut ents: Vec<(Vec<u8>, bool)> = if let Some(names) = crate::vfs::list_dir(&dir) {
            names
                .into_iter()
                .map(|n| {
                    let d = self.is_dir(&Self::join(path, &n), false);
                    (n, d)
                })
                .collect()
        } else {
            let Ok(rd) = std::fs::read_dir(osstr(&crate::vfs::disk(&dir))) else { return };
            rd.filter_map(|e| e.ok())
                .map(|e| {
                    let d = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    (e.file_name().as_bytes().to_vec(), d)
                })
                .collect()
        };
        let dotmatch = self.flags & FNM_DOTMATCH != 0;
        if dotmatch && path.is_empty() {
            ents.push((b".".to_vec(), false)); // Ruby 3.1+: "." (never "..") with FNM_DOTMATCH
        }
        if self.sort {
            ents.sort();
        }
        let mflags = self.flags | FNM_PATHNAME;
        for (name, isdir) in ents {
            let hidden = name.starts_with(b".");
            let mut next: Vec<&[Seg]> = Vec::new();
            for p in &rest {
                let mut p: &[Seg] = p;
                if matches!(p[0], Seg::Rec) {
                    if isdir && (!hidden || dotmatch) {
                        next.push(p);
                    }
                    p = &p[1..];
                }
                match &p[0] {
                    Seg::Lit(n) if *n == name => next.push(&p[1..]),
                    Seg::Pat(pt) if fnmatch(pt, &name, mflags) => next.push(&p[1..]),
                    _ => {}
                }
            }
            if !next.is_empty() {
                let np = Self::join(path, &name);
                self.run(&np, &next);
            }
        }
    }
}
fn glob(patterns: &[Vec<u8>], flags: i64, base: Option<&[u8]>, sort: bool) -> Vec<Vec<u8>> {
    let mut g = Globber { base, flags, sort, out: Vec::new() };
    for pat in patterns {
        for p in brace_expand(pat, flags & FNM_NOESCAPE == 0) {
            let (root, segs) = split_glob(&p, flags);
            if segs.is_empty() {
                if !root.is_empty() {
                    g.out.push(root);
                }
                continue;
            }
            g.run(&root, &[&segs[..]]);
        }
    }
    g.out
}
fn dir_glob(a: &[V], blk: V, default_flags: i64) -> V {
    let (a, kw) = split_kw(a);
    let mut pats = Vec::new();
    let mut flags = default_flags;
    for (i, &x) in a.iter().enumerate() {
        if i > 0 && is_fix(x) {
            flags |= fix(x);
            continue;
        }
        if is_ary(x) {
            for y in crate::gc::Rooted::of(rary(x).v.clone()) {
                pats.push(ok!(path_bytes(y)));
            }
        } else {
            pats.push(ok!(path_bytes(x)));
        }
    }
    if let Some(f) = kw_get(kw, "flags").filter(|x| is_fix(*x)) {
        flags |= fix(f);
    }
    let base = match kw_get(kw, "base").filter(|x| *x != QNIL) {
        Some(b) => Some(ok!(path_bytes(b))),
        None => None,
    };
    let sort = !kw_get(kw, "sort").is_some_and(|v| v == QFALSE);
    let base = base.filter(|b| !b.is_empty());
    let res = glob(&pats, flags, base.as_deref(), sort);
    if is_proc(blk) {
        for r in res {
            tri!(call_block(blk, &[sv(r)]));
        }
        return QNIL;
    }
    ary_new(res.into_iter().map(sv).collect())
}

// ---------- processes ----------

fn status_new(pid: i32, raw: i32) -> V {
    let o = object_new(*C_STATUS.get());
    ivar_set(o, sym("@__pid"), mkfix(pid as i64));
    ivar_set(o, sym("@__st"), mkfix(raw as i64));
    o
}
fn set_last_status(pid: i32, raw: i32) -> V {
    let s = status_new(pid, raw);
    set_gvar("$?", s);
    s
}
/// waitpid that lets green threads run; Ok(None) = WNOHANG and nothing exited.
fn waitpid_v(pid: i32, flags: i32) -> Result<Option<(i32, i32)>, V> {
    loop {
        let threads = crate::thread::idle_timeout().is_some();
        let f = if threads { flags | libc::WNOHANG } else { flags };
        let mut status = 0;
        let r = unsafe { libc::waitpid(pid, &mut status, f) };
        if r > 0 {
            return Ok(Some((r, status)));
        }
        if r < 0 {
            let e = errno();
            if e == libc::EINTR {
                continue;
            }
            return Err(raise_errno(e, None, b""));
        }
        if flags & libc::WNOHANG != 0 {
            return Ok(None);
        }
        if crate::thread::sleep(Some(Duration::from_millis(2))) == UNDEF {
            return Err(UNDEF);
        }
    }
}
fn mkpipe() -> Result<(i32, i32), V> {
    let mut f = [0i32; 2];
    if unsafe { libc::pipe(f.as_mut_ptr()) } != 0 {
        return Err(raise_errno(errno(), None, b""));
    }
    for fd in f {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok((f[0], f[1]))
}

const SH_WORDS: &[&str] = &[
    "!", ".", ":", "break", "case", "continue", "do", "done", "elif", "else", "esac", "eval", "exec", "exit", "export", "fi",
    "for", "if", "in", "readonly", "return", "set", "shift", "then", "times", "trap", "unset", "until", "while",
];
fn needs_shell(s: &[u8]) -> bool {
    if s.iter().any(|c| b"*?{}[]<>()~&|\\$;'`\"\n#".contains(c)) {
        return true;
    }
    let first: &[u8] = s.split(|c| c.is_ascii_whitespace()).find(|w| !w.is_empty()).unwrap_or(b"");
    first.contains(&b'=') || SH_WORDS.iter().any(|w| w.as_bytes() == first)
}

enum Redir {
    Fd(i32),
    Path(Vec<u8>, i32, u32),
    Null,
    Child(usize),
}
struct SpawnSpec {
    cmd: Command,
    label: Vec<u8>,
    exception: bool,
    redirs: [Option<Redir>; 3],
}
fn redir_targets(k: V) -> Result<Vec<usize>, V> {
    if is_ary(k) {
        let mut v = Vec::new();
        for x in crate::gc::Rooted::of(rary(k).v.clone()) {
            v.extend(redir_targets(x)?);
        }
        return Ok(v);
    }
    if is_sym(k) {
        return Ok(match sym_str(sym_id(k)).as_str() {
            "in" => vec![0],
            "out" => vec![1],
            "err" => vec![2],
            _ => vec![],
        });
    }
    if is_fix(k) && (0..=2).contains(&fix(k)) {
        return Ok(vec![fix(k) as usize]);
    }
    if let Some(s) = try_st(k) {
        s.materialize()?;
        let fd = if s.std != 0 { s.std as i32 } else { s.fd };
        if (0..=2).contains(&fd) {
            return Ok(vec![fd as usize]);
        }
    }
    Ok(vec![])
}
fn redir_value(v: V, target: usize) -> Result<Redir, V> {
    let default_flags = if target == 0 { libc::O_RDONLY } else { libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC };
    if is_sym(v) {
        return Ok(match sym_str(sym_id(v)).as_str() {
            "close" => Redir::Null,
            "in" => Redir::Fd(0),
            "out" => Redir::Fd(1),
            "err" => Redir::Fd(2),
            other => return Err(arg_error(&format!("wrong exec redirect action: {other}"))),
        });
    }
    if is_fix(v) {
        return Ok(Redir::Fd(fix(v) as i32));
    }
    if is_ary(v) {
        let items = rary(v).v.clone();
        if items.len() == 2 && is_sym(items[0]) && sym_str(sym_id(items[0])) == "child" {
            return Ok(Redir::Child(redir_targets(items[1])?.first().copied().unwrap_or(1)));
        }
        let path = path_bytes(*items.first().unwrap_or(&QNIL))?;
        let flags = match items.get(1) {
            Some(&m) if m != QNIL => parse_mode(Some(m), None, b"r")?.flags,
            _ => default_flags,
        };
        let perm = items.get(2).filter(|x| is_fix(**x)).map(|x| fix(*x) as u32).unwrap_or(0o644);
        return Ok(Redir::Path(path, flags, perm));
    }
    if let Some(s) = try_st(v) {
        s.check_open()?;
        let _ = s.flush_raw();
        s.materialize()?;
        return Ok(Redir::Fd(match s.std {
            1 | 2 => s.std as i32,
            _ if target == 0 => s.fd,
            _ => s.wfd,
        }));
    }
    Ok(Redir::Path(path_bytes(v)?, default_flags, 0o644))
}
/// Parse `[env,] command... [, options]` (Kernel#system / spawn / exec, IO.popen).
fn spawn_spec(args: &[V]) -> Result<SpawnSpec, V> {
    let mut a = args;
    let mut env = None;
    if let Some(&f) = a.first() {
        if is_hash(f) {
            env = Some(f);
            a = &a[1..];
        }
    }
    let (a, opts) = if a.len() > 1 { split_opts(a) } else { (a, None) };
    if a.is_empty() {
        return Err(arg_error("wrong number of arguments (given 0, expected 1+)"));
    }
    let (mut cmd, label) = if a.len() == 1 && !is_ary(a[0]) {
        let s = str_bytes(a[0])?;
        if needs_shell(&s) {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(osstr(&s));
            (c, s)
        } else {
            let words: Vec<&[u8]> = s.split(|c| c.is_ascii_whitespace()).filter(|w| !w.is_empty()).collect();
            if words.is_empty() {
                return Err(raise_errno(libc::ENOENT, None, &s));
            }
            let mut c = Command::new(osstr(words[0]));
            c.args(words[1..].iter().map(|w| osstr(w)));
            (c, words[0].to_vec())
        }
    } else {
        let (prog, argv0) = if is_ary(a[0]) {
            let pa = rary(a[0]).v.clone();
            (str_bytes(*pa.first().unwrap_or(&QNIL))?, Some(str_bytes(*pa.get(1).unwrap_or(&QNIL))?))
        } else {
            (str_bytes(a[0])?, None)
        };
        let mut c = Command::new(osstr(&prog));
        if let Some(a0) = argv0 {
            c.arg0(osstr(&a0));
        }
        for &x in &a[1..] {
            c.arg(osstr(&str_bytes(x)?));
        }
        (c, prog)
    };
    let mut spec_redirs: [Option<Redir>; 3] = [None, None, None];
    let mut exception = false;
    if let Some(o) = opts {
        if kw_true(Some(o), "unsetenv_others") {
            cmd.env_clear();
        }
        for (k, v) in crate::hash::pairs(o) {
            if is_sym(k) {
                match sym_str(sym_id(k)).as_str() {
                    "chdir" => {
                        cmd.current_dir(osstr(&crate::vfs::disk(&path_bytes(v)?)));
                        continue;
                    }
                    "exception" => {
                        exception = truthy(v);
                        continue;
                    }
                    "pgroup" | "process_group" => {
                        if v == QTRUE || v == mkfix(0) {
                            cmd.process_group(0);
                        } else if is_fix(v) {
                            cmd.process_group(fix(v) as i32);
                        }
                        continue;
                    }
                    "unsetenv_others" | "umask" | "close_others" | "new_pgroup" | "rlimit_core" => continue,
                    _ => {}
                }
            }
            // [:out, :err] => target: open once, share between the streams
            let targets = redir_targets(k)?;
            if let Some(&first) = targets.first() {
                spec_redirs[first] = Some(redir_value(v, first)?);
                for &t in &targets[1..] {
                    spec_redirs[t] = Some(Redir::Child(first));
                }
            }
        }
    }
    if let Some(e) = env {
        for (k, v) in crate::hash::pairs(e) {
            let k = str_bytes(k)?;
            if v == QNIL {
                cmd.env_remove(osstr(&k));
            } else {
                cmd.env(osstr(&k), osstr(&str_bytes(v)?));
            }
        }
    }
    Ok(SpawnSpec { cmd, label, exception, redirs: spec_redirs })
}
struct Spawned {
    pid: i32,
    rfd: i32,
    wfd: i32,
}
/// Start the child. Ok(Err(errno)) if it could not be executed.
fn spawn_child(spec: SpawnSpec, capture_in: bool, capture_out: bool) -> Result<Result<Spawned, i32>, V> {
    flush_all();
    let SpawnSpec { mut cmd, redirs, .. } = spec;
    let mut fds: [Option<i32>; 3] = [None, None, None];
    let mut cleanup: Vec<i32> = Vec::new();
    let mut fail = None;
    for (i, r) in redirs.iter().enumerate() {
        match r {
            Some(Redir::Fd(n)) => fds[i] = Some(unsafe { libc::dup(*n) }),
            Some(Redir::Path(p, flags, perm)) => match open_fd(p, *flags, *perm) {
                Ok(fd) => fds[i] = Some(fd),
                Err(e) => {
                    fail = Some(e);
                    break;
                }
            },
            Some(Redir::Null) => match open_fd(b"/dev/null", if i == 0 { libc::O_RDONLY } else { libc::O_WRONLY }, 0) {
                Ok(fd) => fds[i] = Some(fd),
                Err(e) => {
                    fail = Some(e);
                    break;
                }
            },
            _ => {}
        }
    }
    let close_all = |fds: &[Option<i32>; 3], extra: &[i32]| {
        for fd in fds.iter().flatten() {
            unsafe { libc::close(*fd) };
        }
        for fd in extra {
            unsafe { libc::close(*fd) };
        }
    };
    if let Some(e) = fail {
        close_all(&fds, &[]);
        return Err(e);
    }
    let (mut prd, mut pwr) = (-1, -1);
    if capture_out {
        let (r, w) = mkpipe()?;
        if let Some(old) = fds[1].replace(w) {
            unsafe { libc::close(old) };
        }
        prd = r;
        cleanup.push(r);
    }
    if capture_in {
        let (r, w) = mkpipe()?;
        if let Some(old) = fds[0].replace(r) {
            unsafe { libc::close(old) };
        }
        pwr = w;
        cleanup.push(w);
    }
    for (i, r) in redirs.iter().enumerate() {
        if let Some(Redir::Child(n)) = r {
            let src = fds[*n].unwrap_or(*n as i32);
            fds[i] = Some(unsafe { libc::dup(src) });
        }
    }
    for (i, fd) in fds.iter().enumerate() {
        if let Some(fd) = *fd {
            let s = Stdio::from(unsafe { OwnedFd::from_raw_fd(fd) });
            match i {
                0 => cmd.stdin(s),
                1 => cmd.stdout(s),
                _ => cmd.stderr(s),
            };
        }
    }
    let res = cmd.spawn();
    drop(cmd); // closes the parent's copies of the child's ends
    match res {
        Ok(child) => Ok(Ok(Spawned { pid: child.id() as i32, rfd: prd, wfd: pwr })),
        Err(e) => {
            close_all(&[None, None, None], &cleanup);
            Ok(Err(e.raw_os_error().unwrap_or(libc::ENOENT)))
        }
    }
}
fn read_fd_all(fd: i32) -> Result<Vec<u8>, V> {
    let mut st = IoState::new(fd, true, false);
    st.autoclose = false;
    st.read_all()
}
fn k_system(a: &[V]) -> V {
    let spec = ok!(spawn_spec(a));
    let (exc, label) = (spec.exception, spec.label.clone());
    match ok!(spawn_child(spec, false, false)) {
        Err(e) => {
            if exc {
                return raise_errno(e, None, &label);
            }
            set_last_status(0, 127 << 8);
            QNIL
        }
        Ok(sp) => {
            let Some((pid, raw)) = ok!(waitpid_v(sp.pid, 0)) else { return QNIL };
            set_last_status(pid, raw);
            let success = libc::WIFEXITED(raw) && libc::WEXITSTATUS(raw) == 0;
            if exc && !success {
                let why = if libc::WIFEXITED(raw) {
                    format!("exit {}", libc::WEXITSTATUS(raw))
                } else {
                    format!("signal {}", libc::WTERMSIG(raw))
                };
                return raise(rt().c.runtime_error, &format!("Command failed with {why}: {}", bstr(&label)));
            }
            bool_v(success)
        }
    }
}
fn k_backtick(a: &[V]) -> V {
    let spec = ok!(spawn_spec(&a[..1]));
    let label = spec.label.clone();
    match ok!(spawn_child(spec, false, true)) {
        Err(e) => raise_errno(e, None, &label),
        Ok(sp) => {
            let data = read_fd_all(sp.rfd);
            unsafe { libc::close(sp.rfd) };
            let data = ok!(data);
            if let Some((pid, raw)) = ok!(waitpid_v(sp.pid, 0)) {
                set_last_status(pid, raw);
            }
            sv(data)
        }
    }
}
fn k_spawn(a: &[V]) -> V {
    let spec = ok!(spawn_spec(a));
    let label = spec.label.clone();
    match ok!(spawn_child(spec, false, false)) {
        Err(e) => raise_errno(e, None, &label),
        Ok(sp) => mkfix(sp.pid as i64),
    }
}
fn k_exec(a: &[V]) -> V {
    let mut spec = ok!(spawn_spec(a));
    flush_all();
    for (i, r) in spec.redirs.iter().enumerate() {
        let fd = match r {
            Some(Redir::Fd(n)) => unsafe { libc::dup(*n) },
            Some(Redir::Path(p, f, perm)) => ok!(open_fd(p, *f, *perm)),
            _ => continue,
        };
        let s = Stdio::from(unsafe { OwnedFd::from_raw_fd(fd) });
        match i {
            0 => spec.cmd.stdin(s),
            1 => spec.cmd.stdout(s),
            _ => spec.cmd.stderr(s),
        };
    }
    let e = spec.cmd.exec();
    io_fail(&e, None, &spec.label)
}
fn print_uncaught() {
    if let Throw::Raise(e) = pst().pending {
        pst().pending = Throw::None;
        let m = send(e, sym("message"), &[], QNIL);
        let msg = if is_str(m) { bstr(&rstr(m).b) } else { String::new() };
        err_out(format!("{} ({})\n", msg, class_name(real_class(e))).as_bytes());
    }
}
fn k_fork(blk: V) -> V {
    flush_all();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return raise_errno(errno(), None, b"fork(2)");
    }
    if pid == 0 {
        set_gvar("$$", mkfix(std::process::id() as i64));
        if is_proc(blk) {
            let r = call_block(blk, &[]);
            let mut code = 0;
            if r == UNDEF {
                if matches!(pst().pending, Throw::Raise(_)) {
                    code = 1;
                }
                print_uncaught();
                pst().pending = Throw::None;
            }
            crate::abi::run_at_exit();
            flush_all();
            std::process::exit(code);
        }
        return QNIL;
    }
    mkfix(pid as i64)
}
fn signo(v: V) -> Result<i32, V> {
    if is_fix(v) {
        return Ok(fix(v) as i32);
    }
    let name = if is_sym(v) { sym_str(sym_id(v)) } else { bstr(&str_bytes(v)?) };
    let n = name.trim_start_matches('-');
    let n = n.strip_prefix("SIG").unwrap_or(n);
    SIGNALS
        .iter()
        .find(|(s, _)| *s == n)
        .map(|(_, x)| *x)
        .ok_or_else(|| arg_error(&format!("unsupported signal 'SIG{n}'")))
}
const SIGNALS: &[(&str, i32)] = &[
    ("EXIT", 0),
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    ("IOT", libc::SIGIOT),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("BUS", libc::SIGBUS),
    ("SEGV", libc::SIGSEGV),
    ("SYS", libc::SIGSYS),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("URG", libc::SIGURG),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("CONT", libc::SIGCONT),
    ("CHLD", libc::SIGCHLD),
    ("CLD", libc::SIGCHLD),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("IO", libc::SIGIO),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("USR1", libc::SIGUSR1),
    ("USR2", libc::SIGUSR2),
];

/// IO.popen(cmd, mode = "r", opts) { |io| }
fn io_popen(cls: V, a: &[V], blk: V) -> V {
    let (a, kw) = split_opts(a);
    let mut i = 0;
    let env = if a.first().is_some_and(|x| is_hash(*x)) {
        i = 1;
        Some(a[0])
    } else {
        None
    };
    let Some(&cmd) = a.get(i) else { return arg_error("wrong number of arguments (given 0, expected 1+)") };
    let mode = ok!(parse_mode(a.get(i + 1).copied(), kw, b"r"));
    let mut sargs = RVec::new();
    if let Some(e) = env {
        sargs.push(e);
    }
    if is_ary(cmd) {
        sargs.extend(rary(cmd).v.iter().copied());
    } else {
        let s = ok!(str_bytes(cmd));
        if s == b"-" {
            return raise(rt().c.not_implemented_error, "IO.popen(\"-\") (fork) is not supported by Kenny");
        }
        sargs.push(cmd);
    }
    if let Some(o) = kw {
        // spawn options (mode: etc. are ignored by spawn_spec)
        let h = hash_new();
        for (k, v) in crate::hash::pairs(o) {
            if is_sym(k) && matches!(sym_str(sym_id(k)).as_str(), "mode" | "flags" | "binmode" | "textmode" | "encoding" | "external_encoding" | "internal_encoding" | "autoclose") {
                continue;
            }
            crate::hash::hset(h, k, v);
        }
        sargs.push(h);
    }
    let spec = ok!(spawn_spec(&sargs));
    let label = spec.label.clone();
    let sp = match ok!(spawn_child(spec, mode.w, mode.r)) {
        Ok(sp) => sp,
        Err(e) => return raise_errno(e, None, &label),
    };
    let fd = if mode.r { sp.rfd } else { sp.wfd };
    let mut stt = IoState::new(fd, mode.r, mode.w);
    stt.wfd = if mode.w { sp.wfd } else { fd };
    stt.pid = sp.pid;
    stt.sync = true;
    stt.binmode = mode.bin;
    let c = if is_class(cls) { cls as *mut Class } else { rt().c.io };
    let io = new_io(c, stt);
    if is_proc(blk) {
        let r = call_block(blk, &[io]);
        return close_after(io, r);
    }
    io
}

// ---------- Dir helpers ----------

fn read_dir_names(p: &[u8], func: &str) -> Result<Vec<Vec<u8>>, V> {
    if let Some(names) = crate::vfs::list_dir(p) {
        return Ok(names);
    }
    match std::fs::read_dir(osstr(&crate::vfs::disk(p))) {
        Ok(rd) => Ok(rd.filter_map(|e| e.ok()).map(|e| e.file_name().as_bytes().to_vec()).collect()),
        Err(e) => Err(io_fail(&e, Some(func), p)),
    }
}
fn tmpdir() -> Vec<u8> {
    for k in ["TMPDIR", "TMP", "TEMP"] {
        if let Some(v) = std::env::var_os(k) {
            let b = v.as_bytes();
            if !b.is_empty() && std::path::Path::new(&v).is_dir() {
                let mut b = b.to_vec();
                while b.len() > 1 && b.ends_with(b"/") {
                    b.pop();
                }
                return b;
            }
        }
    }
    b"/tmp".to_vec()
}
fn tmpname(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let mut r = crate::kernel::rand_u64();
    let mut rnd = String::new();
    for _ in 0..6 {
        rnd.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(r % 36) as usize] as char);
        r /= 36;
    }
    let mut v = prefix.to_vec();
    v.extend_from_slice(format!("{:04}{:02}{:02}-{}-{}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, std::process::id(), rnd).as_bytes());
    v.extend_from_slice(suffix);
    v
}
fn prefix_suffix(v: Option<V>, default: &[u8]) -> Result<(Vec<u8>, Vec<u8>), V> {
    match v {
        None | Some(QNIL) => Ok((default.to_vec(), Vec::new())),
        Some(x) if is_ary(x) => {
            let items = rary(x).v.clone();
            let p = match items.first() {
                Some(&y) if y != QNIL => str_bytes(y)?,
                _ => default.to_vec(),
            };
            let s = match items.get(1) {
                Some(&y) if y != QNIL => str_bytes(y)?,
                _ => Vec::new(),
            };
            Ok((p, s))
        }
        Some(x) => Ok((str_bytes(x)?, Vec::new())),
    }
}
fn dir_state(d: V) -> Result<(V, usize), V> {
    let ents = ivar_get(d, sym("@__ents"));
    if !is_ary(ents) {
        return Err(io_error("closed directory"));
    }
    let pos = ivar_get(d, sym("@__pos"));
    Ok((ents, if is_fix(pos) { fix(pos) as usize } else { 0 }))
}

// ---------- init ----------

pub fn init() {
    let r = rt();
    let c = r.c;
    *S_IO.get() = sym("@__io");
    *GV_STDOUT.get() = sym("$stdout");
    *GV_STDERR.get() = sym("$stderr");
    *GV_STDIN.get() = sym("$stdin");
    let object = c.object;

    // ---- SystemCallError / Errno ----
    let sce = match const_lookup(object, sym("SystemCallError")) {
        Some(v) => v as *mut Class,
        None => {
            let k = new_class("SystemCallError", c.standard_error);
            set_global_const("SystemCallError", k as V);
            k
        }
    };
    *C_SCE.get() = sce;
    let errno_mod = const_lookup(object, sym("Errno")).unwrap() as *mut Class;
    for &(name, n) in ERRNOS {
        if let Some(&(_, k)) = ERRNO_MAP.get().iter().find(|(x, _)| *x == n) {
            const_set(errno_mod, sym(name), k as V);
            continue;
        }
        let k = match unsafe { (*errno_mod).consts.get(&sym(name)) } {
            Some(&k) => k as *mut Class,
            None => {
                let k = new_class(&format!("Errno::{name}"), sce);
                const_set(errno_mod, sym(name), k as V);
                k
            }
        };
        const_set(k, sym("Errno"), mkfix(n as i64));
        ERRNO_MAP.get().push((n, k));
    }
    defn(sce, "initialize", 0, 2, |s, a, _| {
        let own = const_get_under(real_class(s), sym("Errno")).filter(|x| is_fix(*x)).map(|x| fix(x) as i32);
        let e = own.or_else(|| a.get(1).filter(|x| is_fix(**x)).map(|x| fix(*x) as i32));
        let base = match e {
            Some(e) => strerror(e),
            None => "unknown error".to_string(),
        };
        let msg = match a.first() {
            Some(&m) if m != QNIL => format!("{base} - {}", bstr(&ok!(to_s_bytes(m)))),
            _ => base,
        };
        ivar_set(s, sym("@__message"), new_str_v(msg));
        if let Some(e) = e {
            ivar_set(s, sym("@__errno"), mkfix(e as i64));
        }
        QNIL
    });
    defn(sce, "errno", 0, 0, |s, _, _| {
        let e = ivar_get(s, sym("@__errno"));
        if is_fix(e) { e } else { const_get_under(real_class(s), sym("Errno")).filter(|x| is_fix(*x)).unwrap_or(QNIL) }
    });

    // ---- IO ----
    let io = c.io;
    unsafe {
        if !(*io).includes.contains(&c.enumerable) {
            (*io).includes.push(c.enumerable);
        }
    }
    bump_serial();
    let fconst = new_module("File::Constants");
    for (name, v) in [
        ("RDONLY", libc::O_RDONLY),
        ("WRONLY", libc::O_WRONLY),
        ("RDWR", libc::O_RDWR),
        ("APPEND", libc::O_APPEND),
        ("CREAT", libc::O_CREAT),
        ("EXCL", libc::O_EXCL),
        ("NONBLOCK", libc::O_NONBLOCK),
        ("TRUNC", libc::O_TRUNC),
        ("NOCTTY", libc::O_NOCTTY),
        ("SYNC", libc::O_SYNC),
        ("NOFOLLOW", libc::O_NOFOLLOW),
        ("DIRECTORY", libc::O_DIRECTORY),
        ("BINARY", 0),
        ("SHARE_DELETE", 0),
        ("LOCK_SH", libc::LOCK_SH),
        ("LOCK_EX", libc::LOCK_EX),
        ("LOCK_NB", libc::LOCK_NB),
        ("LOCK_UN", libc::LOCK_UN),
        ("FNM_NOESCAPE", FNM_NOESCAPE as i32),
        ("FNM_PATHNAME", FNM_PATHNAME as i32),
        ("FNM_DOTMATCH", FNM_DOTMATCH as i32),
        ("FNM_CASEFOLD", FNM_CASEFOLD as i32),
        ("FNM_EXTGLOB", FNM_EXTGLOB as i32),
        ("FNM_SYSCASE", 0),
        ("FNM_SHORTNAME", 0),
    ] {
        const_set(fconst, sym(name), mkfix(v as i64));
    }
    const_set(fconst, sym("NULL"), str_from("/dev/null"));
    unsafe { (*io).includes.push(fconst) };
    const_set(io, sym("SEEK_SET"), mkfix(libc::SEEK_SET as i64));
    const_set(io, sym("SEEK_CUR"), mkfix(libc::SEEK_CUR as i64));
    const_set(io, sym("SEEK_END"), mkfix(libc::SEEK_END as i64));
    for name in ["WaitReadable", "WaitWritable"] {
        let m = new_module(&format!("IO::{name}"));
        const_set(io, sym(name), m as V);
    }

    let mk_std = |fd: i32, std: u8, r: bool, w: bool| {
        let mut s = IoState::new(fd, r, w);
        s.std = std;
        s.autoclose = false;
        new_io(io, s)
    };
    let stdin = mk_std(0, 0, true, false);
    let stdout = mk_std(1, 1, false, true);
    let stderr = mk_std(2, 2, false, true);
    if let Some(s) = try_st(stderr) {
        s.sync = true;
    }
    *STDIN_V.get() = stdin;
    *STDOUT_V.get() = stdout;
    *STDERR_V.get() = stderr;
    set_global_const("STDIN", stdin);
    set_global_const("STDOUT", stdout);
    set_global_const("STDERR", stderr);
    r.gvars.insert(sym("$stdin"), stdin);
    r.gvars.insert(sym("$stdout"), stdout);
    r.gvars.insert(sym("$stderr"), stderr);
    r.gvars.insert(sym("$>"), stdout);
    r.gvars.insert(sym("$<"), stdin);
    r.gvars.insert(sym("$$"), mkfix(std::process::id() as i64));

    defn(io, "initialize", 1, -1, |s, a, _| {
        let (a, kw) = split_kw(a);
        let Some(&fdv) = a.first() else { return arity_error(0, 1, 2) };
        let fd = ok!(int_arg(fdv)) as i32;
        let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if fl < 0 {
            return raise_errno(errno(), None, b"");
        }
        let mode = if a.get(1).is_some_and(|x| *x != QNIL) || kw_get(kw, "mode").is_some() {
            ok!(parse_mode(a.get(1).copied(), kw, b"r"))
        } else {
            mode_from_flags(fl)
        };
        let mut stt = IoState::new(fd, mode.r, mode.w);
        stt.binmode = mode.bin;
        if kw_get(kw, "autoclose") == Some(QFALSE) {
            stt.autoclose = false;
        }
        if fd == 1 || fd == 2 {
            stt.std = fd as u8;
            stt.autoclose = false;
        }
        attach(s, stt);
        QNIL
    });
    defs(io, "for_fd", 1, -1, |s, a, b| send(s, sym("new"), a, b));
    defs(io, "open", 0, -1, |s, a, b| {
        let f = tri!(send(s, sym("new"), a, QNIL));
        if !is_proc(b) {
            return f;
        }
        let r = call_block(b, &[f]);
        close_after(f, r)
    });
    defs(io, "sysopen", 1, 3, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let m = ok!(parse_mode(a.get(1).copied(), None, b"r"));
        let perm = a.get(2).filter(|x| is_fix(**x)).map(|x| fix(*x) as u32).unwrap_or(0o666);
        mkfix(ok!(open_fd(&p, m.flags, perm)) as i64)
    });
    defn(io, "write", 0, -1, |s, a, _| {
        let mut n = 0;
        let mut buf = Vec::new();
        for &x in a {
            let b = ok!(to_s_bytes(x));
            n += b.len();
            buf.extend_from_slice(&b);
        }
        ok!(ok!(st(s)).write(&buf));
        mkfix(n as i64)
    });
    defn(io, "syswrite", 1, 1, |s, a, _| {
        let b = ok!(to_s_bytes(a[0]));
        let stt = ok!(st(s));
        ok!(stt.write(&b));
        ok!(stt.flush());
        mkfix(b.len() as i64)
    });
    defn(io, "write_nonblock", 1, 2, |s, a, _| {
        let b = ok!(to_s_bytes(a[0]));
        let stt = ok!(st(s));
        ok!(stt.write(&b));
        ok!(stt.flush());
        mkfix(b.len() as i64)
    });
    defn(io, "print", 0, -1, |s, a, _| {
        let mut buf = Vec::new();
        for &x in a {
            buf.extend_from_slice(&ok!(to_s_bytes(x)));
        }
        ok!(ok!(st(s)).write(&buf));
        QNIL
    });
    defn(io, "puts", 0, -1, |s, a, _| {
        let mut buf = Vec::new();
        tri!(puts_to(a, &mut |b| buf.extend_from_slice(b)));
        ok!(ok!(st(s)).write(&buf));
        QNIL
    });
    defn(io, "<<", 1, 1, |s, a, _| {
        let b = ok!(to_s_bytes(a[0]));
        ok!(ok!(st(s)).write(&b));
        s
    });
    defn(io, "printf", 1, -1, |s, a, _| {
        let f = tri!(send(rt().main_obj, sym("format"), a, QNIL));
        ok!(ok!(st(s)).write(&rstr(f).b.clone()));
        QNIL
    });
    defn(io, "putc", 1, 1, |s, a, _| {
        let b = if is_fix(a[0]) {
            vec![fix(a[0]) as u8]
        } else {
            let b = ok!(str_bytes(a[0]));
            let n = b.first().map(|&c| if c >= 0xf0 { 4 } else if c >= 0xe0 { 3 } else if c >= 0xc0 { 2 } else { 1 }).unwrap_or(0);
            b[..n.min(b.len())].to_vec()
        };
        ok!(ok!(st(s)).write(&b));
        a[0]
    });
    defn(io, "flush", 0, 0, |s, _, _| {
        ok!(ok!(st(s)).flush());
        s
    });
    defn(io, "fsync", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        ok!(stt.flush());
        if stt.std == 0 {
            unsafe { libc::fsync(stt.wfd) };
        }
        mkfix(0)
    });
    defn(io, "fdatasync", 0, 0, |s, _, _| send(s, sym("fsync"), &[], QNIL));
    defn(io, "sync", 0, 0, |s, _, _| bool_v(ok!(st(s)).sync));
    defn(io, "sync=", 1, 1, |s, a, _| {
        let stt = ok!(st(s));
        stt.sync = truthy(a[0]);
        if stt.sync {
            ok!(stt.flush());
        }
        a[0]
    });
    defn(io, "fileno", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        ok!(stt.check_open());
        ok!(stt.materialize());
        mkfix(if stt.std != 0 { stt.std as i64 } else { stt.fd as i64 })
    });
    defn(io, "to_i", 0, 0, |s, _, _| send(s, sym("fileno"), &[], QNIL));
    defn(io, "to_io", 0, 0, |s, _, _| s);
    defn(io, "tty?", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        let fd = if stt.std != 0 { stt.std as i32 } else { stt.fd };
        bool_v(!stt.closed && unsafe { libc::isatty(fd) } == 1)
    });
    defn(io, "isatty", 0, 0, |s, _, _| send(s, sym("tty?"), &[], QNIL));
    defn(io, "pid", 0, 0, |s, _, _| {
        let p = ok!(st(s)).pid;
        if p > 0 { mkfix(p as i64) } else { QNIL }
    });
    defn(io, "read", 0, 2, |s, a, _| {
        let stt = ok!(st(s));
        let r = match a.first() {
            Some(&n) if n != QNIL => {
                let n = ok!(int_arg(n));
                if n < 0 {
                    return arg_error(&format!("negative length {n} given"));
                }
                let b = ok!(stt.read_upto(n as usize));
                if b.is_empty() && n > 0 {
                    return QNIL;
                }
                b
            }
            _ => ok!(stt.read_all()),
        };
        if let Some(&buf) = a.get(1).filter(|x| is_str(**x)) {
            rstr(buf).b.set(r);
            return buf;
        }
        sv(r)
    });
    fn readpartial(s: V, a: &[V], nonblock: bool) -> V {
        let (a, kw) = split_kw(a);
        let n = ok!(int_arg(*a.first().unwrap_or(&mkfix(0))));
        let stt = ok!(st(s));
        ok!(stt.check_readable());
        let exc = kw_get(kw, "exception") != Some(QFALSE);
        if nonblock && stt.avail() == 0 && stt.mem.is_none() {
            let mut p = libc::pollfd { fd: stt.fd, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut p, 1, 0) } == 0 {
                if !exc {
                    return mksym(sym("wait_readable"));
                }
                return raise_errno(libc::EAGAIN, None, b"read would block");
            }
        }
        let b = ok!(stt.read_partial(n.max(0) as usize));
        if b.is_empty() && n > 0 {
            if nonblock && !exc {
                return QNIL;
            }
            return eof_error();
        }
        if let Some(&buf) = a.get(1).filter(|x| is_str(**x)) {
            rstr(buf).b.set(b);
            return buf;
        }
        sv(b)
    }
    defn(io, "readpartial", 1, 2, |s, a, _| readpartial(s, a, false));
    defn(io, "sysread", 1, 2, |s, a, _| readpartial(s, a, false));
    defn(io, "read_nonblock", 1, 3, |s, a, _| readpartial(s, a, true));
    defn(io, "gets", 0, 3, |s, a, _| {
        let l = gets_v(s, a);
        if l != UNDEF {
            set_gvar("$_", l);
        }
        l
    });
    defn(io, "readline", 0, 3, |s, a, _| {
        let l = tri!(gets_v(s, a));
        if l == QNIL { eof_error() } else { l }
    });
    defn(io, "readlines", 0, 3, |s, a, _| readlines_v(s, a));
    defn(io, "to_a", 0, 3, |s, a, _| readlines_v(s, a));
    defn(io, "each_line", 0, 3, |s, a, b| each_line_v(s, a, b));
    defn(io, "each", 0, 3, |s, a, b| each_line_v(s, a, b));
    defn(io, "getc", 0, 0, |s, _, _| match ok!(ok!(st(s)).getc()) {
        Some(b) => sv(b),
        None => QNIL,
    });
    defn(io, "readchar", 0, 0, |s, _, _| match ok!(ok!(st(s)).getc()) {
        Some(b) => sv(b),
        None => eof_error(),
    });
    defn(io, "getbyte", 0, 0, |s, _, _| match ok!(ok!(st(s)).getbyte()) {
        Some(b) => mkfix(b as i64),
        None => QNIL,
    });
    defn(io, "readbyte", 0, 0, |s, _, _| match ok!(ok!(st(s)).getbyte()) {
        Some(b) => mkfix(b as i64),
        None => eof_error(),
    });
    defn(io, "ungetc", 1, 1, |s, a, _| {
        let b = if is_fix(a[0]) {
            let mut buf = [0u8; 4];
            char::from_u32(fix(a[0]) as u32).unwrap_or('?').encode_utf8(&mut buf).as_bytes().to_vec()
        } else {
            ok!(str_bytes(a[0]))
        };
        let stt = ok!(st(s));
        ok!(stt.check_readable());
        stt.unget(&b);
        QNIL
    });
    defn(io, "ungetbyte", 1, 1, |s, a, _| {
        let b = if is_fix(a[0]) { vec![fix(a[0]) as u8] } else { ok!(str_bytes(a[0])) };
        let stt = ok!(st(s));
        ok!(stt.check_readable());
        stt.unget(&b);
        QNIL
    });
    defn(io, "each_char", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each_char", &[]);
        }
        while let Some(c) = ok!(ok!(st(s)).getc()) {
            tri!(call_block(b, &[sv(c)]));
        }
        s
    });
    defn(io, "each_byte", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each_byte", &[]);
        }
        while let Some(c) = ok!(ok!(st(s)).getbyte()) {
            tri!(call_block(b, &[mkfix(c as i64)]));
        }
        s
    });
    defn(io, "eof?", 0, 0, |s, _, _| bool_v(ok!(ok!(st(s)).eof())));
    defn(io, "eof", 0, 0, |s, _, _| bool_v(ok!(ok!(st(s)).eof())));
    defn(io, "pos", 0, 0, |s, _, _| mkfix(ok!(ok!(st(s)).tell())));
    defn(io, "tell", 0, 0, |s, _, _| mkfix(ok!(ok!(st(s)).tell())));
    defn(io, "pos=", 1, 1, |s, a, _| {
        ok!(ok!(st(s)).seek(ok!(int_arg(a[0])), libc::SEEK_SET));
        a[0]
    });
    defn(io, "seek", 1, 2, |s, a, _| {
        let whence = match a.get(1) {
            None => libc::SEEK_SET,
            Some(&w) if is_fix(w) => fix(w) as i32,
            Some(&w) if is_sym(w) => match sym_str(sym_id(w)).as_str() {
                "SET" => libc::SEEK_SET,
                "CUR" => libc::SEEK_CUR,
                "END" => libc::SEEK_END,
                o => return arg_error(&format!("unknown whence: {o}")),
            },
            Some(_) => return type_error("whence must be an Integer or Symbol"),
        };
        ok!(ok!(st(s)).seek(ok!(int_arg(a[0])), whence));
        mkfix(0)
    });
    defn(io, "rewind", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        ok!(stt.seek(0, libc::SEEK_SET));
        stt.lineno = 0;
        mkfix(0)
    });
    defn(io, "lineno", 0, 0, |s, _, _| mkfix(ok!(st(s)).lineno));
    defn(io, "lineno=", 1, 1, |s, a, _| {
        ok!(st(s)).lineno = ok!(int_arg(a[0]));
        a[0]
    });
    defn(io, "close", 0, 0, |s, _, _| {
        ok!(ok!(st(s)).close());
        QNIL
    });
    defn(io, "closed?", 0, 0, |s, _, _| bool_v(ok!(st(s)).closed));
    defn(io, "close_read", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        if stt.closed {
            return QNIL;
        }
        if !stt.writable || stt.wfd == stt.fd {
            ok!(stt.close());
            return QNIL;
        }
        if stt.fd > 2 {
            unsafe { libc::close(stt.fd) };
        }
        stt.fd = stt.wfd;
        stt.readable = false;
        QNIL
    });
    defn(io, "close_write", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        if stt.closed {
            return QNIL;
        }
        if !stt.readable || stt.wfd == stt.fd {
            ok!(stt.close());
            return QNIL;
        }
        let _ = stt.flush_raw();
        if stt.wfd > 2 {
            unsafe { libc::close(stt.wfd) };
        }
        stt.wfd = stt.fd;
        stt.writable = false;
        QNIL
    });
    defn(io, "autoclose?", 0, 0, |s, _, _| bool_v(ok!(st(s)).autoclose));
    defn(io, "autoclose=", 1, 1, |s, a, _| {
        ok!(st(s)).autoclose = truthy(a[0]);
        a[0]
    });
    defn(io, "close_on_exec?", 0, 0, |_, _, _| QTRUE);
    defn(io, "close_on_exec=", 1, 1, |_, a, _| a[0]);
    defn(io, "binmode", 0, 0, |s, _, _| {
        ok!(st(s)).binmode = true;
        s
    });
    defn(io, "binmode?", 0, 0, |s, _, _| bool_v(ok!(st(s)).binmode));
    defn(io, "set_encoding", 0, -1, |s, _, _| s);
    defn(io, "set_encoding_by_bom", 0, 0, |_, _, _| QNIL);
    defn(io, "external_encoding", 0, 0, |s, _, _| {
        let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap() as *mut Class;
        let name = if ok!(st(s)).binmode { "BINARY" } else { "UTF_8" };
        const_get_under(enc, sym(name)).unwrap_or(QNIL)
    });
    defn(io, "internal_encoding", 0, 0, |_, _, _| QNIL);
    defn(io, "fcntl", 1, 2, |_, _, _| mkfix(0));
    defn(io, "advise", 1, 3, |_, _, _| QNIL);
    defn(io, "wait_readable", 0, 1, |s, _, _| s);
    defn(io, "wait_writable", 0, 1, |s, _, _| s);
    defn(io, "stat", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        ok!(stt.check_open());
        let _ = stt.flush_raw();
        match stt.fstat() {
            Ok(x) => stat_new(&x),
            Err(e) => raise_errno(e, None, &stt.detail()),
        }
    });
    defn(io, "reopen", 1, 2, |s, a, _| {
        let target = ok!(st(s));
        if target.closed && target.std == 0 {
            // reopening a closed IO: give it a fresh descriptor
            let (nfd, path, r, w) = if let Some(o) = try_st(a[0]) {
                ok!(o.check_open());
                let _ = o.flush_raw();
                ok!(o.materialize());
                (unsafe { libc::dup(if o.std != 0 { o.std as i32 } else { o.fd }) }, o.path.clone(), o.readable, o.writable)
            } else {
                let p = ok!(path_bytes(a[0]));
                let m = ok!(parse_mode(a.get(1).copied(), None, b"r"));
                (ok!(open_fd(&p, m.flags, 0o666)), Some(p), m.r, m.w)
            };
            target.fd = nfd;
            target.wfd = nfd;
            target.closed = false;
            target.readable = r;
            target.writable = w;
            target.rbuf.clear();
            target.rpos = 0;
            target.path = path;
            OPEN.get().push(target as *const IoState as usize);
            return s;
        }
        ok!(target.check_open());
        let _ = target.flush_raw();
        if target.std == 1 {
            flush_out();
        }
        let tfd = if target.std != 0 { target.std as i32 } else { target.fd };
        let (nfd, close_new, path) = if let Some(o) = try_st(a[0]) {
            let _ = o.flush_raw();
            ok!(o.materialize());
            (if o.std != 0 { o.std as i32 } else { o.fd }, false, o.path.clone())
        } else {
            let p = ok!(path_bytes(a[0]));
            let default: &[u8] = if target.writable && !target.readable { b"w" } else { b"r" };
            let m = ok!(parse_mode(a.get(1).copied(), None, default));
            target.readable = m.r;
            target.writable = m.w;
            (ok!(open_fd(&p, m.flags, 0o666)), true, Some(p))
        };
        if unsafe { libc::dup2(nfd, tfd) } < 0 {
            return raise_errno(errno(), None, &target.detail());
        }
        if close_new {
            unsafe { libc::close(nfd) };
        }
        target.rbuf.clear();
        target.rpos = 0;
        if target.std == 0 {
            target.path = path;
        }
        s
    });
    fn io_dup(s: V) -> V {
        let src = ok!(st(s));
        ok!(src.check_open());
        let _ = src.flush_raw();
        ok!(src.materialize());
        let fd0 = if src.std != 0 { src.std as i32 } else { src.fd };
        let fd = unsafe { libc::fcntl(fd0, libc::F_DUPFD_CLOEXEC, 3) };
        if fd < 0 {
            return raise_errno(errno(), None, &src.detail());
        }
        let wfd = if src.std == 0 && src.wfd != src.fd { unsafe { libc::fcntl(src.wfd, libc::F_DUPFD_CLOEXEC, 3) } } else { fd };
        let mut n = IoState::new(fd, src.readable, src.writable);
        n.wfd = wfd;
        n.path = src.path.clone();
        n.sync = src.sync;
        n.binmode = src.binmode;
        n.lineno = src.lineno;
        n.stdio_copy = fd0 <= 2 || src.stdio_copy;
        new_io(real_class(s), n)
    }
    defn(io, "dup", 0, 0, |s, _, _| io_dup(s));
    defn(io, "clone", 0, 1, |s, _, _| io_dup(s));
    defn(io, "inspect", 0, 0, |s, _, _| {
        let cn = class_name(real_class(s));
        let Some(stt) = try_st(s) else { return new_str_v(format!("#<{cn}:(uninitialized)>")) };
        let what = match (&stt.path, stt.std, stt.fd) {
            (Some(p), _, _) => bstr(p),
            (None, 1, _) => "<STDOUT>".into(),
            (None, 2, _) => "<STDERR>".into(),
            (None, _, 0) => "<STDIN>".into(),
            (None, _, fd) => format!("fd {fd}"),
        };
        new_str_v(format!("#<{cn}:{what}{}>", if stt.closed { " (closed)" } else { "" }))
    });

    // IO class methods (path based)
    fn s_read(a: &[V], bin: bool) -> V {
        let (a, kw) = split_kw(a);
        let p = ok!(path_bytes(a[0]));
        let mut m = ok!(parse_mode(None, kw, b"r"));
        m.bin |= bin;
        let len = match a.get(1) {
            Some(&n) if n != QNIL => {
                let n = ok!(int_arg(n));
                if n < 0 {
                    return arg_error(&format!("negative length {n} given"));
                }
                Some(n as usize)
            }
            _ => None,
        };
        let off = a.get(2).filter(|x| **x != QNIL).map(|x| int_arg(*x));
        let off = match off {
            Some(Ok(o)) => Some(o),
            Some(Err(e)) => return e,
            None => None,
        };
        let r = with_file(&p, &m, 0o666, |s| {
            if let Some(o) = off {
                s.seek(o, libc::SEEK_SET)?;
            }
            match len {
                Some(n) => s.read_upto(n).map(Some).map(|b| b.filter(|x| !x.is_empty() || n == 0)),
                None => s.read_all().map(Some),
            }
        });
        match ok!(r) {
            Some(b) => sv(b),
            None => QNIL,
        }
    }
    fn s_write(a: &[V], bin: bool) -> V {
        let (a, kw) = split_kw(a);
        if a.len() < 2 {
            return arity_error(a.len() as i32, 2, 3);
        }
        let p = ok!(path_bytes(a[0]));
        let data = ok!(to_s_bytes(a[1]));
        let off = match a.get(2).filter(|x| **x != QNIL) {
            Some(&o) => Some(ok!(int_arg(o))),
            None => None,
        };
        let default: &[u8] = if off.is_some() { b"r+" } else { b"w" };
        let mut m = ok!(parse_mode(None, kw, default));
        if off.is_some() && kw_get(kw, "mode").is_none() {
            m.flags |= libc::O_CREAT;
        }
        m.bin |= bin;
        let perm = kw_get(kw, "perm").filter(|x| is_fix(*x)).map(|x| fix(x) as u32).unwrap_or(0o666);
        ok!(with_file(&p, &m, perm, |s| {
            if let Some(o) = off {
                s.seek(o, libc::SEEK_SET)?;
            }
            s.write(&data)
        }));
        mkfix(data.len() as i64)
    }
    fn s_readlines(a: &[V]) -> V {
        let (pa, kw) = split_kw(a);
        let p = ok!(path_bytes(pa[0]));
        let o = ok!(line_opts(&a[1..]));
        let m = ok!(parse_mode(None, kw, b"r"));
        let r = with_file(&p, &m, 0o666, |s| {
            let mut v = RVec::new();
            while let Some(l) = s.gets(&o)? {
                v.push(sv(l));
            }
            Ok(v.take())
        });
        ary_new(ok!(r))
    }
    fn s_foreach(cls: V, a: &[V], b: V) -> V {
        if !is_proc(b) {
            return crate::misc::enum_for(cls, "foreach", a);
        }
        let (pa, kw) = split_kw(a);
        let p = ok!(path_bytes(pa[0]));
        let o = ok!(line_opts(&a[1..]));
        let m = ok!(parse_mode(None, kw, b"r"));
        ok!(with_file(&p, &m, 0o666, |s| {
            while let Some(l) = s.gets(&o)? {
                if call_block(b, &[sv(l)]) == UNDEF {
                    return Err(UNDEF);
                }
            }
            Ok(())
        }));
        QNIL
    }
    defs(io, "read", 1, 4, |_, a, _| s_read(a, false));
    defs(io, "binread", 1, 3, |_, a, _| s_read(a, true));
    defs(io, "write", 2, 4, |_, a, _| s_write(a, false));
    defs(io, "binwrite", 2, 4, |_, a, _| s_write(a, true));
    defs(io, "readlines", 1, 4, |_, a, _| s_readlines(a));
    defs(io, "foreach", 1, 4, |s, a, b| s_foreach(s, a, b));
    defs(io, "popen", 1, -1, io_popen);
    defs(io, "pipe", 0, -1, |s, _, b| {
        let (r, w) = ok!(mkpipe());
        let c = s as *mut Class;
        let rio = new_io(c, IoState::new(r, true, false));
        let mut ws = IoState::new(w, false, true);
        ws.sync = true;
        let wio = new_io(c, ws);
        if is_proc(b) {
            let res = call_block(b, &[rio, wio]);
            let res2 = close_after(wio, res);
            return close_after(rio, res2);
        }
        ary_new(vec![rio, wio])
    });
    defs(io, "copy_stream", 2, 4, |_, a, _| {
        let (src, dst) = (a[0], a[1]);
        let len = match a.get(2).filter(|x| **x != QNIL) {
            Some(&n) => Some(ok!(int_arg(n)) as usize),
            None => None,
        };
        let off = match a.get(3).filter(|x| **x != QNIL) {
            Some(&n) => Some(ok!(int_arg(n))),
            None => None,
        };
        let data = if let Some(s) = try_st(src) {
            if let Some(o) = off {
                ok!(s.seek(o, libc::SEEK_SET));
            }
            match len {
                Some(n) => ok!(s.read_upto(n)),
                None => ok!(s.read_all()),
            }
        } else if is_str(src) || respond_to(src, sym("to_path")) {
            let p = ok!(path_bytes(src));
            let m = Mode { flags: libc::O_RDONLY, r: true, w: false, bin: true };
            ok!(with_file(&p, &m, 0, |s| {
                if let Some(o) = off {
                    s.seek(o, libc::SEEK_SET)?;
                }
                match len {
                    Some(n) => s.read_upto(n),
                    None => s.read_all(),
                }
            }))
        } else {
            let lv = match len {
                Some(n) => vec![mkfix(n as i64)],
                None => vec![],
            };
            let r = tri!(send(src, sym("read"), &lv, QNIL));
            if r == QNIL { Vec::new() } else { ok!(str_bytes(r)) }
        };
        if let Some(d) = try_st(dst) {
            ok!(d.write(&data));
        } else if is_str(dst) || respond_to(dst, sym("to_path")) {
            let p = ok!(path_bytes(dst));
            let m = Mode { flags: libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, r: false, w: true, bin: true };
            ok!(with_file(&p, &m, 0o666, |s| s.write(&data)));
        } else {
            tri!(send(dst, sym("write"), &[str_new(data.clone())], QNIL));
        }
        mkfix(data.len() as i64)
    });
    defs(io, "select", 1, 4, |_, a, _| {
        let list = |v: Option<&V>| -> Vec<V> {
            match v {
                Some(&x) if is_ary(x) => rary(x).v.clone(),
                _ => vec![],
            }
        };
        let (rs, ws) = (list(a.first()), list(a.get(1)));
        let timeout = a.get(3).and_then(|t| num_to_f(*t)).map(|t| Duration::from_secs_f64(t.max(0.0)));
        let mut ready_r = Vec::new();
        let mut ready_w = Vec::new();
        let mut pfds = Vec::new();
        for &x in &rs {
            let s = ok!(st(x));
            if s.avail() > 0 || s.mem.is_some() {
                ready_r.push(x);
            }
            // an embedded file (fd -1) is always readable; poll skips negative fds
            pfds.push(libc::pollfd { fd: if s.std != 0 { s.std as i32 } else { s.fd }, events: libc::POLLIN, revents: 0 });
        }
        for &x in &ws {
            let s = ok!(st(x));
            pfds.push(libc::pollfd { fd: if s.std != 0 { s.std as i32 } else { s.wfd }, events: libc::POLLOUT, revents: 0 });
        }
        let n = if ready_r.is_empty() {
            match poll_wait(&mut pfds, timeout) {
                Ok(n) => n,
                Err(e) => return e,
            }
        } else {
            unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 0) }
        };
        if n > 0 {
            for (i, p) in pfds.iter().enumerate() {
                if p.revents != 0 {
                    if i < rs.len() {
                        if !ready_r.contains(&rs[i]) {
                            ready_r.push(rs[i]);
                        }
                    } else {
                        ready_w.push(ws[i - rs.len()]);
                    }
                }
            }
        }
        if ready_r.is_empty() && ready_w.is_empty() {
            return QNIL;
        }
        ary_new(vec![ary_new(ready_r), ary_new(ready_w), ary_new(vec![])])
    });
    defs(io, "try_convert", 1, 1, |_, a, _| if try_st(a[0]).is_some() { a[0] } else { QNIL });

    // ---- File ----
    let file = match const_lookup(object, sym("File")) {
        Some(f) => f as *mut Class,
        None => {
            let f = new_class("File", io);
            set_global_const("File", f as V);
            f
        }
    };
    *C_FILE.get() = file;
    const_set(file, sym("Constants"), fconst as V);
    const_set(file, sym("SEPARATOR"), str_from("/"));
    const_set(file, sym("Separator"), str_from("/"));
    const_set(file, sym("ALT_SEPARATOR"), QNIL);
    const_set(file, sym("PATH_SEPARATOR"), str_from(":"));
    defn(file, "initialize", 1, -1, |s, a, _| {
        let (a, kw) = split_kw(a);
        if is_fix(a[0]) {
            let mut args = a.to_vec();
            if let Some(k) = kw {
                args.push(k);
            }
            let m = find_method(rt().c.io, sym("initialize"));
            return invoke(m, s, &args, QNIL);
        }
        let p = ok!(path_bytes(a[0]));
        let m = ok!(parse_mode(a.get(1).copied(), kw, b"r"));
        let perm = a
            .get(2)
            .copied()
            .or_else(|| kw_get(kw, "perm"))
            .filter(|x| is_fix(*x))
            .map(|x| fix(x) as u32)
            .unwrap_or(0o666);
        let stt = ok!(open_state(&p, &m, perm));
        attach(s, stt);
        QNIL
    });
    defs(file, "open", 0, -1, |s, a, b| {
        let f = tri!(send(s, sym("new"), a, QNIL));
        if !is_proc(b) {
            return f;
        }
        let r = call_block(b, &[f]);
        close_after(f, r)
    });
    defn(file, "path", 0, 0, |s, _, _| match ok!(st(s)).path.clone() {
        Some(p) => sv(p),
        None => QNIL,
    });
    defn(file, "to_path", 0, 0, |s, _, _| send(s, sym("path"), &[], QNIL));
    defn(file, "size", 0, 0, |s, _, _| {
        let stt = ok!(st(s));
        ok!(stt.check_open());
        ok!(stt.flush());
        match stt.fstat() {
            Ok(x) => mkfix(x.st_size as i64),
            Err(e) => raise_errno(e, None, &stt.detail()),
        }
    });
    defn(file, "truncate", 1, 1, |s, a, _| {
        let stt = ok!(st(s));
        ok!(stt.check_writable());
        ok!(stt.flush());
        if unsafe { libc::ftruncate(stt.fd, ok!(int_arg(a[0])) as libc::off_t) } != 0 {
            return raise_errno(errno(), None, &stt.detail());
        }
        mkfix(0)
    });
    defn(file, "flock", 1, 1, |s, a, _| {
        let stt = ok!(st(s));
        ok!(stt.check_open());
        let op = ok!(int_arg(a[0])) as i32;
        if stt.mem.is_some() {
            return mkfix(0); // an embedded file: nobody else writes it
        }
        loop {
            let r = unsafe { libc::flock(stt.fd, op) };
            if r == 0 {
                return mkfix(0);
            }
            let e = errno();
            if e == libc::EINTR {
                continue;
            }
            if e == libc::EWOULDBLOCK && op & libc::LOCK_NB != 0 {
                return QFALSE;
            }
            return raise_errno(e, None, &stt.detail());
        }
    });
    defn(file, "chmod", 1, 1, |s, a, _| {
        let stt = ok!(st(s));
        ok!(stt.check_open());
        if unsafe { libc::fchmod(stt.fd, ok!(int_arg(a[0])) as libc::mode_t) } != 0 {
            return raise_errno(errno(), None, &stt.detail());
        }
        mkfix(0)
    });
    defn(file, "chown", 2, 2, |s, a, _| {
        let stt = ok!(st(s));
        let id = |v: V| if is_fix(v) { fix(v) as u32 } else { u32::MAX };
        unsafe { libc::fchown(stt.fd, id(a[0]), id(a[1])) };
        mkfix(0)
    });
    for (name, field) in [("atime", "@__atime"), ("mtime", "@__mtime"), ("ctime", "@__ctime"), ("birthtime", "@__birthtime")] {
        let _ = field;
        let f: NativeFn = match name {
            "atime" => |s, _, _| {
                let x = tri!(send(s, sym("stat"), &[], QNIL));
                send(x, sym("atime"), &[], QNIL)
            },
            "mtime" => |s, _, _| {
                let x = tri!(send(s, sym("stat"), &[], QNIL));
                send(x, sym("mtime"), &[], QNIL)
            },
            "ctime" => |s, _, _| {
                let x = tri!(send(s, sym("stat"), &[], QNIL));
                send(x, sym("ctime"), &[], QNIL)
            },
            _ => |s, _, _| {
                let x = tri!(send(s, sym("stat"), &[], QNIL));
                send(x, sym("birthtime"), &[], QNIL)
            },
        };
        add_method(file, sym(name), Body::Native(f, 0, 0));
    }
    defn(file, "lstat", 0, 0, |s, _, _| {
        let p = ok!(st(s)).path.clone().unwrap_or_default();
        match ok!(do_stat(&p, false)) {
            Ok(x) => stat_new(&x),
            Err(e) => raise_errno(e, Some("rb_file_lstat"), &p),
        }
    });

    // File class methods
    defs(file, "path", 1, 1, |_, a, _| sv(ok!(path_bytes(a[0]))));
    defs(file, "exist?", 1, 1, |_, a, _| bool_v(ok!(stat_arg(a[0], true)).is_some()));
    macro_rules! pred {
        ($name:expr, $follow:expr, $f:expr) => {
            defs(file, $name, 1, 1, |_, a, _| {
                let f: fn(&libc::stat) -> bool = $f;
                bool_v(ok!(stat_arg(a[0], $follow)).is_some_and(|s| f(&s)))
            });
        };
    }
    pred!("file?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFREG));
    pred!("directory?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFDIR));
    pred!("symlink?", false, |s| is_fmt(s.st_mode as u32, libc::S_IFLNK));
    pred!("pipe?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFIFO));
    pred!("socket?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFSOCK));
    pred!("blockdev?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFBLK));
    pred!("chardev?", true, |s| is_fmt(s.st_mode as u32, libc::S_IFCHR));
    pred!("zero?", true, |s| s.st_size == 0);
    pred!("empty?", true, |s| s.st_size == 0);
    pred!("setuid?", true, |s| s.st_mode as u32 & libc::S_ISUID as u32 != 0);
    pred!("setgid?", true, |s| s.st_mode as u32 & libc::S_ISGID as u32 != 0);
    pred!("sticky?", true, |s| s.st_mode as u32 & libc::S_ISVTX as u32 != 0);
    pred!("owned?", true, |s| s.st_uid == unsafe { libc::geteuid() });
    pred!("grpowned?", true, |s| s.st_gid == unsafe { libc::getegid() });
    macro_rules! access {
        ($name:expr, $m:expr) => {
            defs(file, $name, 1, 1, |_, a, _| {
                let p = ok!(path_bytes(a[0]));
                if let Some(e) = crate::vfs::find(&p) {
                    // embedded files can be read; executable if their mode says so
                    match $m {
                        libc::R_OK => return QTRUE,
                        libc::X_OK => return bool_v(e.dir || e.mode & 0o111 != 0),
                        _ => {}
                    }
                }
                let c = ok!(dpath(&p));
                bool_v(unsafe { libc::access(c.as_ptr(), $m) } == 0)
            });
        };
    }
    access!("readable?", libc::R_OK);
    access!("readable_real?", libc::R_OK);
    access!("writable?", libc::W_OK);
    access!("writable_real?", libc::W_OK);
    access!("executable?", libc::X_OK);
    access!("executable_real?", libc::X_OK);
    defs(file, "world_readable?", 1, 1, |_, a, _| match ok!(stat_arg(a[0], true)) {
        Some(s) if s.st_mode as u32 & libc::S_IROTH as u32 != 0 => mkfix((s.st_mode as u32 & 0o777) as i64),
        _ => QNIL,
    });
    defs(file, "world_writable?", 1, 1, |_, a, _| match ok!(stat_arg(a[0], true)) {
        Some(s) if s.st_mode as u32 & libc::S_IWOTH as u32 != 0 => mkfix((s.st_mode as u32 & 0o777) as i64),
        _ => QNIL,
    });
    defs(file, "size?", 1, 1, |_, a, _| match ok!(stat_arg(a[0], true)) {
        Some(s) if s.st_size > 0 => mkfix(s.st_size as i64),
        _ => QNIL,
    });
    defs(file, "size", 1, 1, |_, a, _| mkfix(ok!(stat_or_raise(a[0], true, "rb_file_s_size")).st_size as i64));
    defs(file, "stat", 1, 1, |_, a, _| stat_new(&ok!(stat_or_raise(a[0], true, "rb_file_s_stat"))));
    defs(file, "lstat", 1, 1, |_, a, _| stat_new(&ok!(stat_or_raise(a[0], false, "rb_file_s_lstat"))));
    defs(file, "ftype", 1, 1, |_, a, _| str_from(ftype_of(ok!(stat_or_raise(a[0], false, "rb_file_s_ftype")).st_mode as u32)));
    defs(file, "mtime", 1, 1, |_, a, _| {
        let s = ok!(stat_or_raise(a[0], true, "rb_file_s_mtime"));
        time_v(ts(s.st_mtime as i64, s.st_mtime_nsec as i64))
    });
    defs(file, "atime", 1, 1, |_, a, _| {
        let s = ok!(stat_or_raise(a[0], true, "rb_file_s_atime"));
        time_v(ts(s.st_atime as i64, s.st_atime_nsec as i64))
    });
    defs(file, "ctime", 1, 1, |_, a, _| {
        let s = ok!(stat_or_raise(a[0], true, "rb_file_s_ctime"));
        time_v(ts(s.st_ctime as i64, s.st_ctime_nsec as i64))
    });
    defs(file, "birthtime", 1, 1, |_, a, _| {
        let x = stat_new(&ok!(stat_or_raise(a[0], true, "rb_file_s_birthtime")));
        send(x, sym("birthtime"), &[], QNIL)
    });
    defs(file, "identical?", 2, 2, |_, a, _| {
        match (ok!(stat_arg(a[0], true)), ok!(stat_arg(a[1], true))) {
            (Some(x), Some(y)) => bool_v(x.st_dev == y.st_dev && x.st_ino == y.st_ino),
            _ => QFALSE,
        }
    });
    defs(file, "basename", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let mut b = basename(&p);
        if let Some(&sfx) = a.get(1) {
            let sfx = ok!(str_bytes(sfx));
            if sfx == b".*" {
                if let Some(i) = b.iter().rposition(|c| *c == b'.') {
                    if i > 0 {
                        b.truncate(i);
                    }
                }
            } else if b.len() > sfx.len() && b.ends_with(&sfx) {
                b.truncate(b.len() - sfx.len());
            }
        }
        sv(b)
    });
    defs(file, "dirname", 1, 2, |_, a, _| {
        let mut p = ok!(path_bytes(a[0]));
        let n = match a.get(1) {
            Some(&n) => ok!(int_arg(n)),
            None => 1,
        };
        if n < 0 {
            return arg_error(&format!("negative level: {n}"));
        }
        for _ in 0..n {
            let d = dirname(&p);
            if d == p {
                break;
            }
            p = d;
        }
        sv(p)
    });
    defs(file, "extname", 1, 1, |_, a, _| sv(extname(&ok!(path_bytes(a[0])))));
    defs(file, "split", 1, 1, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        ary_new(vec![sv(dirname(&p)), sv(basename(&p))])
    });
    defs(file, "join", 0, -1, |_, a, _| {
        let mut parts = Vec::new();
        for &x in a {
            ok!(join_parts(x, &mut parts, 0));
        }
        sv(file_join(&parts))
    });
    defs(file, "expand_path", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let d = match a.get(1).filter(|x| **x != QNIL) {
            Some(&d) => Some(ok!(path_bytes(d))),
            None => None,
        };
        sv(ok!(expand(&p, d.as_deref(), true)))
    });
    defs(file, "absolute_path", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let d = match a.get(1).filter(|x| **x != QNIL) {
            Some(&d) => Some(ok!(path_bytes(d))),
            None => None,
        };
        sv(ok!(expand(&p, d.as_deref(), false)))
    });
    defs(file, "absolute_path?", 1, 1, |_, a, _| bool_v(ok!(path_bytes(a[0])).starts_with(b"/")));
    defs(file, "realpath", 1, 2, |_, a, _| {
        let mut p = ok!(path_bytes(a[0]));
        if let Some(&d) = a.get(1).filter(|x| **x != QNIL) {
            if !p.starts_with(b"/") {
                let mut base = ok!(path_bytes(d));
                base.push(b'/');
                base.extend_from_slice(&p);
                p = base;
            }
        }
        if let Some(r) = crate::vfs::realpath(&p) {
            return sv(r);
        }
        match std::fs::canonicalize(osstr(&crate::vfs::disk(&p))) {
            Ok(r) => sv(r.as_os_str().as_bytes().to_vec()),
            Err(e) => io_fail(&e, Some("rb_check_realpath_internal"), &p),
        }
    });
    defs(file, "realdirpath", 1, 2, |_, a, _| {
        let mut p = ok!(path_bytes(a[0]));
        if let Some(&d) = a.get(1).filter(|x| **x != QNIL) {
            if !p.starts_with(b"/") {
                let mut base = ok!(path_bytes(d));
                base.push(b'/');
                base.extend_from_slice(&p);
                p = base;
            }
        }
        if let Some(r) = crate::vfs::realpath(&p) {
            return sv(r);
        }
        if let Ok(r) = std::fs::canonicalize(osstr(&crate::vfs::disk(&p))) {
            return sv(r.as_os_str().as_bytes().to_vec());
        }
        let parent = dirname(&p);
        if let Some(mut v) = crate::vfs::realpath(&parent) {
            if !v.ends_with(b"/") {
                v.push(b'/');
            }
            v.extend_from_slice(&basename(&p));
            return sv(v);
        }
        match std::fs::canonicalize(osstr(&crate::vfs::disk(&parent))) {
            Ok(r) => {
                let mut v = r.as_os_str().as_bytes().to_vec();
                if !v.ends_with(b"/") {
                    v.push(b'/');
                }
                v.extend_from_slice(&basename(&p));
                sv(v)
            }
            Err(e) => io_fail(&e, Some("rb_check_realpath_internal"), &parent),
        }
    });
    defs(file, "readlink", 1, 1, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        if crate::vfs::find(&p).is_some() {
            return raise_errno(libc::EINVAL, Some("rb_readlink"), &p);
        }
        match std::fs::read_link(osstr(&crate::vfs::disk(&p))) {
            Ok(r) => sv(r.as_os_str().as_bytes().to_vec()),
            Err(e) => io_fail(&e, Some("rb_readlink"), &p),
        }
    });
    fn unlink_all(a: &[V]) -> V {
        for &x in a {
            let p = ok!(path_bytes(x));
            let embedded = crate::vfs::find(&p).is_some_and(|e| !e.dir);
            let c = ok!(wpath(&p));
            if unsafe { libc::unlink(c.as_ptr()) } != 0 && !(embedded && errno() == libc::ENOENT) {
                return raise_errno(errno(), Some("apply2files"), &p);
            }
        }
        mkfix(a.len() as i64)
    }
    defs(file, "delete", 0, -1, |_, a, _| unlink_all(a));
    defs(file, "unlink", 0, -1, |_, a, _| unlink_all(a));
    defs(file, "rename", 2, 2, |_, a, _| {
        let (x, y) = (ok!(path_bytes(a[0])), ok!(path_bytes(a[1])));
        let (cx, cy) = (ok!(wpath(&x)), ok!(wpath(&y)));
        if unsafe { libc::rename(cx.as_ptr(), cy.as_ptr()) } != 0 {
            return raise_errno(errno(), Some("rb_file_s_rename"), format!("({}, {})", bstr(&x), bstr(&y)).as_bytes());
        }
        mkfix(0)
    });
    defs(file, "symlink", 2, 2, |_, a, _| {
        let (x, y) = (ok!(path_bytes(a[0])), ok!(path_bytes(a[1])));
        let (cx, cy) = (ok!(cstr(&x)), ok!(wpath(&y)));
        if unsafe { libc::symlink(cx.as_ptr(), cy.as_ptr()) } != 0 {
            return raise_errno(errno(), Some("rb_file_s_symlink"), format!("({}, {})", bstr(&x), bstr(&y)).as_bytes());
        }
        mkfix(0)
    });
    defs(file, "link", 2, 2, |_, a, _| {
        let (x, y) = (ok!(path_bytes(a[0])), ok!(path_bytes(a[1])));
        let (cx, cy) = (ok!(dpath(&x)), ok!(wpath(&y)));
        if unsafe { libc::link(cx.as_ptr(), cy.as_ptr()) } != 0 {
            return raise_errno(errno(), Some("rb_file_s_link"), format!("({}, {})", bstr(&x), bstr(&y)).as_bytes());
        }
        mkfix(0)
    });
    fn chmod_all(a: &[V], follow: bool) -> V {
        let mode = ok!(int_arg(a[0])) as libc::mode_t;
        for &x in &a[1..] {
            let p = ok!(path_bytes(x));
            let c = ok!(wpath(&p));
            let r = unsafe {
                if follow { libc::chmod(c.as_ptr(), mode) } else { libc::fchmodat(libc::AT_FDCWD, c.as_ptr(), mode, libc::AT_SYMLINK_NOFOLLOW) }
            };
            if r != 0 {
                return raise_errno(errno(), Some("apply2files"), &p);
            }
        }
        mkfix(a.len() as i64 - 1)
    }
    defs(file, "chmod", 1, -1, |_, a, _| chmod_all(a, true));
    defs(file, "lchmod", 1, -1, |_, a, _| chmod_all(a, false));
    defs(file, "chown", 2, -1, |_, a, _| {
        let id = |v: V| if is_fix(v) { fix(v) as u32 } else { u32::MAX };
        for &x in &a[2..] {
            let p = ok!(path_bytes(x));
            let c = ok!(wpath(&p));
            if unsafe { libc::chown(c.as_ptr(), id(a[0]), id(a[1])) } != 0 {
                return raise_errno(errno(), Some("apply2files"), &p);
            }
        }
        mkfix(a.len() as i64 - 2)
    });
    defs(file, "lchown", 2, -1, |_, a, _| mkfix(a.len() as i64 - 2));
    fn utime_all(a: &[V]) -> V {
        let tv = |v: V| -> Result<libc::timeval, V> {
            let secs = if v == QNIL {
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
            } else if let Some(f) = num_to_f(v) {
                f
            } else {
                let f = send(v, sym("to_f"), &[], QNIL);
                if f == UNDEF {
                    return Err(UNDEF);
                }
                num_to_f(f).unwrap_or(0.0)
            };
            Ok(libc::timeval { tv_sec: secs.floor() as libc::time_t, tv_usec: ((secs - secs.floor()) * 1e6) as libc::suseconds_t })
        };
        let times = [ok!(tv(a[0])), ok!(tv(a[1]))];
        for &x in &a[2..] {
            let p = ok!(path_bytes(x));
            let c = ok!(wpath(&p));
            if unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) } != 0 {
                return raise_errno(errno(), Some("apply2files"), &p);
            }
        }
        mkfix(a.len() as i64 - 2)
    }
    defs(file, "utime", 2, -1, |_, a, _| utime_all(a));
    defs(file, "lutime", 2, -1, |_, a, _| utime_all(a));
    defs(file, "truncate", 2, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let c = ok!(wpath(&p));
        if unsafe { libc::truncate(c.as_ptr(), ok!(int_arg(a[1])) as libc::off_t) } != 0 {
            return raise_errno(errno(), Some("rb_file_s_truncate"), &p);
        }
        mkfix(0)
    });
    defs(file, "mkfifo", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let c = ok!(wpath(&p));
        let mode = a.get(1).filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0o666);
        if unsafe { libc::mkfifo(c.as_ptr(), mode as libc::mode_t) } != 0 {
            return raise_errno(errno(), Some("rb_file_s_mkfifo"), &p);
        }
        mkfix(0)
    });
    defs(file, "umask", 0, 1, |_, a, _| {
        let old = match a.first() {
            Some(&m) => unsafe { libc::umask(ok!(int_arg(m)) as libc::mode_t) },
            None => unsafe {
                let o = libc::umask(0);
                libc::umask(o);
                o
            },
        };
        mkfix(old as i64)
    });
    defs(file, "fnmatch", 2, 3, |_, a, _| {
        let p = ok!(str_bytes(a[0]));
        let s = ok!(path_bytes(a[1]));
        let f = a.get(2).filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0);
        bool_v(fnmatch(&p, &s, f))
    });
    defs(file, "fnmatch?", 2, 3, |_, a, _| {
        let p = ok!(str_bytes(a[0]));
        let s = ok!(path_bytes(a[1]));
        let f = a.get(2).filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0);
        bool_v(fnmatch(&p, &s, f))
    });

    // ---- File::Stat ----
    let stc = new_class("File::Stat", object);
    const_set(file, sym("Stat"), stc as V);
    *C_STAT.get() = stc;
    unsafe { (*stc).includes.push(c.comparable) };
    defn(stc, "initialize", 1, 1, |s, a, _| {
        let x = ok!(stat_or_raise(a[0], true, "rb_file_s_stat"));
        let tmp = stat_new(&x);
        for (k, v) in ivar_list_all(tmp) {
            ivar_set(s, k, v);
        }
        QNIL
    });
    fn ivar_list_all(v: V) -> Vec<(u32, V)> {
        let o = obj(v);
        let shape = unsafe { &*o.shape };
        o.ivars.iter().enumerate().filter(|(i, _)| *i < shape.ivar_names.len()).map(|(i, &x)| (shape.ivar_names[i], x)).collect()
    }
    macro_rules! stat_int {
        ($name:expr, $k:expr) => {
            defn(stc, $name, 0, 0, |s, _, _| ivar_get(s, sym($k)));
        };
    }
    stat_int!("mode", "@__mode");
    stat_int!("size", "@__size");
    stat_int!("dev", "@__dev");
    stat_int!("ino", "@__ino");
    stat_int!("nlink", "@__nlink");
    stat_int!("uid", "@__uid");
    stat_int!("gid", "@__gid");
    stat_int!("rdev", "@__rdev");
    stat_int!("blksize", "@__blksize");
    stat_int!("blocks", "@__blocks");
    defn(stc, "dev_major", 0, 0, |s, _, _| mkfix((sf(s, "@__dev") >> 24) & 0xff));
    defn(stc, "dev_minor", 0, 0, |s, _, _| mkfix(sf(s, "@__dev") & 0xffffff));
    macro_rules! stat_time {
        ($name:expr, $k:expr) => {
            defn(stc, $name, 0, 0, |s, _, _| {
                let t = ivar_get(s, sym($k));
                if t == QNIL {
                    return raise(rt().c.not_implemented_error, concat!($name, "() function is unimplemented on this machine"));
                }
                time_v(num_to_f(t).unwrap_or(0.0))
            });
        };
    }
    stat_time!("atime", "@__atime");
    stat_time!("mtime", "@__mtime");
    stat_time!("ctime", "@__ctime");
    stat_time!("birthtime", "@__birthtime");
    macro_rules! stat_pred {
        ($name:expr, $f:expr) => {
            defn(stc, $name, 0, 0, |s, _, _| {
                let f: fn(V, u32) -> bool = $f;
                bool_v(f(s, sf(s, "@__mode") as u32))
            });
        };
    }
    stat_pred!("file?", |_, m| is_fmt(m, libc::S_IFREG));
    stat_pred!("directory?", |_, m| is_fmt(m, libc::S_IFDIR));
    stat_pred!("symlink?", |_, m| is_fmt(m, libc::S_IFLNK));
    stat_pred!("pipe?", |_, m| is_fmt(m, libc::S_IFIFO));
    stat_pred!("socket?", |_, m| is_fmt(m, libc::S_IFSOCK));
    stat_pred!("blockdev?", |_, m| is_fmt(m, libc::S_IFBLK));
    stat_pred!("chardev?", |_, m| is_fmt(m, libc::S_IFCHR));
    stat_pred!("zero?", |s, _| sf(s, "@__size") == 0);
    stat_pred!("setuid?", |_, m| m & libc::S_ISUID as u32 != 0);
    stat_pred!("setgid?", |_, m| m & libc::S_ISGID as u32 != 0);
    stat_pred!("sticky?", |_, m| m & libc::S_ISVTX as u32 != 0);
    stat_pred!("owned?", |s, _| sf(s, "@__uid") as u32 == unsafe { libc::geteuid() });
    stat_pred!("grpowned?", |s, _| sf(s, "@__gid") as u32 == unsafe { libc::getegid() });
    stat_pred!("readable?", |s, m| stat_access(
        m,
        sf(s, "@__uid") as u32,
        sf(s, "@__gid") as u32,
        (libc::S_IRUSR as u32, libc::S_IRGRP as u32, libc::S_IROTH as u32)
    ));
    stat_pred!("writable?", |s, m| stat_access(
        m,
        sf(s, "@__uid") as u32,
        sf(s, "@__gid") as u32,
        (libc::S_IWUSR as u32, libc::S_IWGRP as u32, libc::S_IWOTH as u32)
    ));
    stat_pred!("executable?", |s, m| stat_access(
        m,
        sf(s, "@__uid") as u32,
        sf(s, "@__gid") as u32,
        (libc::S_IXUSR as u32, libc::S_IXGRP as u32, libc::S_IXOTH as u32)
    ));
    defn(stc, "size?", 0, 0, |s, _, _| if sf(s, "@__size") == 0 { QNIL } else { ivar_get(s, sym("@__size")) });
    defn(stc, "ftype", 0, 0, |s, _, _| str_from(ftype_of(sf(s, "@__mode") as u32)));
    defn(stc, "world_readable?", 0, 0, |s, _, _| {
        let m = sf(s, "@__mode");
        if m & libc::S_IROTH as i64 != 0 { mkfix(m & 0o777) } else { QNIL }
    });
    defn(stc, "world_writable?", 0, 0, |s, _, _| {
        let m = sf(s, "@__mode");
        if m & libc::S_IWOTH as i64 != 0 { mkfix(m & 0o777) } else { QNIL }
    });
    defn(stc, "<=>", 1, 1, |s, a, _| {
        if !is_heap(a[0]) || real_class(a[0]) != *C_STAT.get() {
            return QNIL;
        }
        let x = num_to_f(ivar_get(s, sym("@__mtime"))).unwrap_or(0.0);
        let y = num_to_f(ivar_get(a[0], sym("@__mtime"))).unwrap_or(0.0);
        mkfix(x.partial_cmp(&y).map(|o| o as i64).unwrap_or(0))
    });
    defn(stc, "inspect", 0, 0, |s, _, _| {
        new_str_v(format!(
            "#<File::Stat dev=0x{:x}, ino={}, mode=0{:o}, nlink={}, uid={}, gid={}, size={}>",
            sf(s, "@__dev"),
            crate::fmt::inspect_lossy(ivar_get(s, sym("@__ino"))),
            sf(s, "@__mode"),
            sf(s, "@__nlink"),
            sf(s, "@__uid"),
            sf(s, "@__gid"),
            sf(s, "@__size")
        ))
    });

    // ---- Dir ----
    let dir = match const_lookup(object, sym("Dir")) {
        Some(d) => d as *mut Class,
        None => {
            let d = new_class("Dir", object);
            set_global_const("Dir", d as V);
            d
        }
    };
    *C_DIR.get() = dir;
    unsafe { (*dir).includes.push(c.enumerable) };
    bump_serial();
    defs(dir, "pwd", 0, 0, |_, _, _| sv(cwd()));
    defs(dir, "getwd", 0, 0, |_, _, _| sv(cwd()));
    defs(dir, "chdir", 0, 1, |_, a, b| {
        let p = match a.first() {
            Some(&p) => ok!(path_bytes(p)),
            None => ok!(home_dir(b"")),
        };
        let old = cwd();
        if let Err(e) = crate::vfs::chdir(&p) {
            return io_fail(&e, Some("dir_s_chdir"), &p);
        }
        if !is_proc(b) {
            return mkfix(0);
        }
        let r = call_block(b, &[sv(p)]);
        let _ = crate::vfs::chdir(&old);
        r
    });
    defs(dir, "mkdir", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        if crate::vfs::find(&p).is_some() {
            return raise_errno(libc::EEXIST, Some("dir_s_mkdir"), &p);
        }
        let c = ok!(wpath(&p));
        let mode = a.get(1).filter(|x| is_fix(**x)).map(|x| fix(*x)).unwrap_or(0o777);
        if unsafe { libc::mkdir(c.as_ptr(), mode as libc::mode_t) } != 0 {
            return raise_errno(errno(), Some("dir_s_mkdir"), &p);
        }
        mkfix(0)
    });
    fn rmdir(a: &[V]) -> V {
        let p = ok!(path_bytes(a[0]));
        let c = ok!(dpath(&p));
        if unsafe { libc::rmdir(c.as_ptr()) } != 0 {
            return raise_errno(errno(), Some("dir_s_rmdir"), &p);
        }
        mkfix(0)
    }
    defs(dir, "rmdir", 1, 1, |_, a, _| rmdir(a));
    defs(dir, "delete", 1, 1, |_, a, _| rmdir(a));
    defs(dir, "unlink", 1, 1, |_, a, _| rmdir(a));
    defs(dir, "exist?", 1, 1, |_, a, _| bool_v(ok!(stat_arg(a[0], true)).is_some_and(|s| is_fmt(s.st_mode as u32, libc::S_IFDIR))));
    defs(dir, "empty?", 1, 1, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        if let Some(names) = crate::vfs::list_dir(&p) {
            return bool_v(names.is_empty());
        }
        match std::fs::read_dir(osstr(&crate::vfs::disk(&p))) {
            Ok(mut rd) => bool_v(rd.next().is_none()),
            Err(_) => QFALSE,
        }
    });
    defs(dir, "entries", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        let mut v = vec![sv(b".".to_vec()), sv(b"..".to_vec())];
        v.extend(ok!(read_dir_names(&p, "dir_initialize")).into_iter().map(sv));
        ary_new(v)
    });
    defs(dir, "children", 1, 2, |_, a, _| {
        let p = ok!(path_bytes(a[0]));
        ary_new(ok!(read_dir_names(&p, "dir_initialize")).into_iter().map(sv).collect())
    });
    defs(dir, "each_child", 1, 2, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each_child", a);
        }
        let p = ok!(path_bytes(a[0]));
        for n in ok!(read_dir_names(&p, "dir_initialize")) {
            tri!(call_block(b, &[sv(n)]));
        }
        QNIL
    });
    defs(dir, "foreach", 1, 2, |s, a, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "foreach", a);
        }
        let p = ok!(path_bytes(a[0]));
        let mut names = vec![b".".to_vec(), b"..".to_vec()];
        names.extend(ok!(read_dir_names(&p, "dir_initialize")));
        for n in names {
            tri!(call_block(b, &[sv(n)]));
        }
        QNIL
    });
    defs(dir, "home", 0, 1, |_, a, _| {
        let u = match a.first().filter(|x| **x != QNIL) {
            Some(&u) => ok!(str_bytes(u)),
            None => Vec::new(),
        };
        sv(ok!(home_dir(&u)))
    });
    defs(dir, "glob", 1, -1, |_, a, b| dir_glob(a, b, 0));
    defs(dir, "[]", 0, -1, |_, a, _| {
        let (pa, kw) = split_kw(a);
        let mut args = RVec::from(vec![ary_new(pa.to_vec())]);
        if let Some(k) = kw {
            args.push(k);
        }
        dir_glob(&args, QNIL, 0)
    });
    defs(dir, "tmpdir", 0, 0, |_, _, _| sv(tmpdir()));
    defs(dir, "mktmpdir", 0, -1, |_, a, b| {
        let (a, _kw) = split_kw(a);
        let (pre, suf) = ok!(prefix_suffix(a.first().copied(), b"d"));
        let base = match a.get(1).filter(|x| **x != QNIL) {
            Some(&d) => ok!(path_bytes(d)),
            None => tmpdir(),
        };
        let path = loop {
            let mut p = base.clone();
            p.push(b'/');
            p.extend_from_slice(&tmpname(&pre, &suf));
            let c = ok!(cstr(&p));
            if unsafe { libc::mkdir(c.as_ptr(), 0o700) } == 0 {
                break p;
            }
            let e = errno();
            if e != libc::EEXIST {
                return raise_errno(e, Some("dir_s_mkdir"), &p);
            }
        };
        if !is_proc(b) {
            return sv(path);
        }
        let r = call_block(b, &[sv(path.clone())]);
        let _ = std::fs::remove_dir_all(osstr(&path));
        r
    });
    // Dir instances
    defn(dir, "initialize", 1, 2, |s, a, _| {
        let p = ok!(path_bytes(a[0]));
        let mut v = vec![sv(b".".to_vec()), sv(b"..".to_vec())];
        v.extend(ok!(read_dir_names(&p, "dir_initialize")).into_iter().map(sv));
        ivar_set(s, sym("@__path"), sv(p));
        ivar_set(s, sym("@__ents"), ary_new(v));
        ivar_set(s, sym("@__pos"), mkfix(0));
        QNIL
    });
    defs(dir, "open", 1, 2, |s, a, b| {
        let d = tri!(send(s, sym("new"), a, QNIL));
        if !is_proc(b) {
            return d;
        }
        let r = call_block(b, &[d]);
        ivar_set(d, sym("@__ents"), QNIL);
        r
    });
    defn(dir, "path", 0, 0, |s, _, _| ivar_get(s, sym("@__path")));
    defn(dir, "to_path", 0, 0, |s, _, _| ivar_get(s, sym("@__path")));
    defn(dir, "read", 0, 0, |s, _, _| {
        let (ents, pos) = ok!(dir_state(s));
        let v = rary(ents).v.get(pos).copied().unwrap_or(QNIL);
        if v != QNIL {
            ivar_set(s, sym("@__pos"), mkfix(pos as i64 + 1));
        }
        v
    });
    defn(dir, "each", 0, 0, |s, _, b| {
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each", &[]);
        }
        loop {
            let (ents, pos) = ok!(dir_state(s));
            let Some(&v) = rary(ents).v.get(pos) else { return s };
            ivar_set(s, sym("@__pos"), mkfix(pos as i64 + 1));
            tri!(call_block(b, &[v]));
        }
    });
    defn(dir, "children", 0, 0, |s, _, _| {
        let (ents, _) = ok!(dir_state(s));
        ary_new(rary(ents).v.iter().copied().filter(|x| !matches!(rstr(*x).b.as_slice(), b"." | b"..")).collect())
    });
    defn(dir, "each_child", 0, 0, |s, _, b| {
        let (ents, _) = ok!(dir_state(s));
        let items = RVec::from(rary(ents).v.iter().copied().filter(|x| !matches!(rstr(*x).b.as_slice(), b"." | b"..")).collect());
        if !is_proc(b) {
            return crate::misc::enum_for(s, "each_child", &[]);
        }
        for &v in items.iter() {
            tri!(call_block(b, &[v]));
        }
        s
    });
    defn(dir, "pos", 0, 0, |s, _, _| ivar_get(s, sym("@__pos")));
    defn(dir, "tell", 0, 0, |s, _, _| ivar_get(s, sym("@__pos")));
    defn(dir, "rewind", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__pos"), mkfix(0));
        s
    });
    defn(dir, "close", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__ents"), QNIL);
        QNIL
    });
    defn(dir, "fileno", 0, 0, |_, _, _| mkfix(-1));
    defn(dir, "inspect", 0, 0, |s, _, _| {
        let p = ivar_get(s, sym("@__path"));
        new_str_v(format!("#<Dir:{}>", if is_str(p) { bstr(&rstr(p).b) } else { String::new() }))
    });

    // ---- Process ----
    let pr = const_lookup(object, sym("Process")).unwrap() as *mut Class;
    const_set(pr, sym("WNOHANG"), mkfix(libc::WNOHANG as i64));
    const_set(pr, sym("WUNTRACED"), mkfix(libc::WUNTRACED as i64));
    const_set(pr, sym("RLIMIT_NOFILE"), mkfix(libc::RLIMIT_NOFILE as i64));
    const_set(pr, sym("RLIMIT_CORE"), mkfix(libc::RLIMIT_CORE as i64));
    const_set(pr, sym("RLIMIT_STACK"), mkfix(libc::RLIMIT_STACK as i64));
    const_set(pr, sym("RLIMIT_CPU"), mkfix(libc::RLIMIT_CPU as i64));
    const_set(pr, sym("RLIM_INFINITY"), int_from_i128(libc::RLIM_INFINITY as i128));
    let stc = new_class("Process::Status", object);
    const_set(pr, sym("Status"), stc as V);
    *C_STATUS.get() = stc;
    fn raw(s: V) -> i32 {
        sf(s, "@__st") as i32
    }
    defn(stc, "pid", 0, 0, |s, _, _| ivar_get(s, sym("@__pid")));
    defn(stc, "to_i", 0, 0, |s, _, _| ivar_get(s, sym("@__st")));
    defn(stc, "exitstatus", 0, 0, |s, _, _| {
        let r = raw(s);
        if libc::WIFEXITED(r) { mkfix(libc::WEXITSTATUS(r) as i64) } else { QNIL }
    });
    defn(stc, "success?", 0, 0, |s, _, _| {
        let r = raw(s);
        if libc::WIFEXITED(r) { bool_v(libc::WEXITSTATUS(r) == 0) } else { QNIL }
    });
    defn(stc, "exited?", 0, 0, |s, _, _| bool_v(libc::WIFEXITED(raw(s))));
    defn(stc, "signaled?", 0, 0, |s, _, _| bool_v(libc::WIFSIGNALED(raw(s))));
    defn(stc, "stopped?", 0, 0, |s, _, _| bool_v(libc::WIFSTOPPED(raw(s))));
    defn(stc, "coredump?", 0, 0, |s, _, _| bool_v(libc::WCOREDUMP(raw(s))));
    defn(stc, "termsig", 0, 0, |s, _, _| {
        let r = raw(s);
        if libc::WIFSIGNALED(r) { mkfix(libc::WTERMSIG(r) as i64) } else { QNIL }
    });
    defn(stc, "stopsig", 0, 0, |s, _, _| {
        let r = raw(s);
        if libc::WIFSTOPPED(r) { mkfix(libc::WSTOPSIG(r) as i64) } else { QNIL }
    });
    defn(stc, "==", 1, 1, |s, a, _| {
        if is_fix(a[0]) {
            return bool_v(fix(a[0]) == raw(s) as i64);
        }
        bool_v(s == a[0])
    });
    defn(stc, "&", 1, 1, |s, a, _| mkfix(raw(s) as i64 & ok!(int_arg(a[0]))));
    defn(stc, ">>", 1, 1, |s, a, _| mkfix((raw(s) as i64) >> ok!(int_arg(a[0]))));
    fn status_desc(s: V) -> String {
        let r = raw(s);
        let pid = sf(s, "@__pid");
        if libc::WIFEXITED(r) {
            format!("pid {pid} exit {}", libc::WEXITSTATUS(r))
        } else if libc::WIFSIGNALED(r) {
            let n = libc::WTERMSIG(r);
            let name = SIGNALS.iter().find(|(_, x)| *x == n).map(|(s, _)| format!("SIG{s}")).unwrap_or_default();
            format!("pid {pid} {name} (signal {n}){}", if libc::WCOREDUMP(r) { " (core dumped)" } else { "" })
        } else if libc::WIFSTOPPED(r) {
            format!("pid {pid} stopped signal {}", libc::WSTOPSIG(r))
        } else {
            format!("pid {pid} status {r}")
        }
    }
    defn(stc, "to_s", 0, 0, |s, _, _| new_str_v(status_desc(s)));
    defn(stc, "inspect", 0, 0, |s, _, _| new_str_v(format!("#<Process::Status: {}>", status_desc(s))));

    defs(pr, "pid", 0, 0, |_, _, _| mkfix(std::process::id() as i64));
    defs(pr, "ppid", 0, 0, |_, _, _| mkfix(unsafe { libc::getppid() } as i64));
    defs(pr, "uid", 0, 0, |_, _, _| mkfix(unsafe { libc::getuid() } as i64));
    defs(pr, "euid", 0, 0, |_, _, _| mkfix(unsafe { libc::geteuid() } as i64));
    defs(pr, "gid", 0, 0, |_, _, _| mkfix(unsafe { libc::getgid() } as i64));
    defs(pr, "egid", 0, 0, |_, _, _| mkfix(unsafe { libc::getegid() } as i64));
    defs(pr, "getpgrp", 0, 0, |_, _, _| mkfix(unsafe { libc::getpgrp() } as i64));
    defs(pr, "getpgid", 1, 1, |_, a, _| {
        let r = unsafe { libc::getpgid(ok!(int_arg(a[0])) as i32) };
        if r < 0 { raise_errno(errno(), None, b"") } else { mkfix(r as i64) }
    });
    defs(pr, "setsid", 0, 0, |_, _, _| {
        let r = unsafe { libc::setsid() };
        if r < 0 { raise_errno(errno(), None, b"") } else { mkfix(r as i64) }
    });
    defs(pr, "setpgid", 2, 2, |_, a, _| {
        if unsafe { libc::setpgid(ok!(int_arg(a[0])) as i32, ok!(int_arg(a[1])) as i32) } < 0 {
            return raise_errno(errno(), None, b"");
        }
        mkfix(0)
    });
    defs(pr, "getrlimit", 1, 1, |_, a, _| {
        let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
        let res = if is_fix(a[0]) {
            fix(a[0]) as i32
        } else {
            match const_get_under(const_lookup(rt().c.object, sym("Process")).unwrap() as *mut Class, sym(&format!("RLIMIT_{}", bstr(&ok!(to_s_bytes(a[0]))).to_uppercase()))) {
                Some(v) if is_fix(v) => fix(v) as i32,
                _ => return arg_error("invalid resource name"),
            }
        };
        if unsafe { libc::getrlimit(res as _, &mut rl) } != 0 {
            return raise_errno(errno(), None, b"");
        }
        ary_new(vec![int_from_i128(rl.rlim_cur as i128), int_from_i128(rl.rlim_max as i128)])
    });
    defs(pr, "setrlimit", 2, 3, |_, a, _| {
        let cur = ok!(int_arg(a[1]));
        let max = match a.get(2) {
            Some(&m) => ok!(int_arg(m)),
            None => cur,
        };
        let rl = libc::rlimit { rlim_cur: cur as libc::rlim_t, rlim_max: max as libc::rlim_t };
        if unsafe { libc::setrlimit(ok!(int_arg(a[0])) as _, &rl) } != 0 {
            return raise_errno(errno(), None, b"");
        }
        QNIL
    });
    defs(pr, "kill", 1, -1, |_, a, _| {
        let sig = ok!(signo(a[0]));
        let neg = is_str(a[0]) && rstr(a[0]).b.starts_with(b"-");
        for &p in &a[1..] {
            let pid = ok!(int_arg(p)) as i32;
            let r = unsafe { if neg { libc::killpg(pid, sig) } else { libc::kill(pid, sig) } };
            if r != 0 {
                return raise_errno(errno(), None, b"");
            }
        }
        mkfix(a.len() as i64 - 1)
    });
    fn wait_impl(a: &[V]) -> Result<Option<(i32, i32)>, V> {
        let pid = match a.first() {
            Some(&p) if p != QNIL => int_arg(p)? as i32,
            _ => -1,
        };
        let flags = match a.get(1) {
            Some(&f) if f != QNIL => int_arg(f)? as i32,
            _ => 0,
        };
        let r = waitpid_v(pid, flags)?;
        if let Some((p, st)) = r {
            set_last_status(p, st);
        }
        Ok(r)
    }
    defs(pr, "wait", 0, 2, |_, a, _| match ok!(wait_impl(a)) {
        Some((p, _)) => mkfix(p as i64),
        None => QNIL,
    });
    defs(pr, "waitpid", 0, 2, |_, a, _| match ok!(wait_impl(a)) {
        Some((p, _)) => mkfix(p as i64),
        None => QNIL,
    });
    fn wait2(a: &[V]) -> V {
        match ok!(wait_impl(a)) {
            Some((p, st)) => ary_new(vec![mkfix(p as i64), status_new(p, st)]),
            None => QNIL,
        }
    }
    defs(pr, "wait2", 0, 2, |_, a, _| wait2(a));
    defs(pr, "waitpid2", 0, 2, |_, a, _| wait2(a));
    defs(pr, "waitall", 0, 0, |_, _, _| {
        let mut v = RVec::new();
        loop {
            let mut status = 0;
            let r = unsafe { libc::waitpid(-1, &mut status, 0) };
            if r <= 0 {
                break;
            }
            set_last_status(r, status);
            v.push(ary_new(vec![mkfix(r as i64), status_new(r, status)]));
        }
        ary_new(v.take())
    });
    defs(pr, "last_status", 0, 0, |_, _, _| rt().gvars.get(&sym("$?")).copied().unwrap_or(QNIL));
    defs(pr, "detach", 1, 1, |_, a, _| {
        let pid = ok!(int_arg(a[0])) as i32;
        let body = proc_new_native(
            Box::new(move |_, _, _| match waitpid_v(pid, 0) {
                Ok(Some((p, st))) => status_new(p, st),
                Ok(None) => QNIL,
                Err(e) => e,
            }),
            0,
            -1,
        );
        let thread_cls = const_lookup(rt().c.object, sym("Thread")).unwrap();
        let t = tri!(send(thread_cls, sym("new"), &[], body));
        ivar_set(t, sym("@__pid"), mkfix(pid as i64));
        add_method(singleton_class(t), sym("pid"), Body::Native(|s, _, _| ivar_get(s, sym("@__pid")), 0, 0));
        t
    });
    defs(pr, "fork", 0, 0, |_, _, b| k_fork(b));
    defs(pr, "spawn", 1, -1, |_, a, _| k_spawn(a));
    defs(pr, "exec", 1, -1, |_, a, _| k_exec(a));
    defs(pr, "exit", 0, 1, |_, a, _| send(rt().main_obj, sym("exit"), a, QNIL));
    defs(pr, "exit!", 0, 1, |_, a, _| send(rt().main_obj, sym("exit!"), a, QNIL));
    defs(pr, "abort", 0, 1, |_, a, _| send(rt().main_obj, sym("abort"), a, QNIL));
    defs(pr, "argv0", 0, 0, |_, _, _| rt().gvars.get(&sym("$0")).copied().unwrap_or(QNIL));
    defs(pr, "setproctitle", 1, 1, |_, a, _| a[0]);
    defs(pr, "daemon", 0, 2, |_, _, _| raise(rt().c.not_implemented_error, "Process.daemon is not supported by Kenny"));
    if let Some(sc) = const_lookup(object, sym("Struct")) {
        let names: Vec<V> = ["utime", "stime", "cutime", "cstime"].iter().map(|n| mksym(sym(n))).collect();
        let tms = send(sc, sym("new"), &names, QNIL);
        if tms != UNDEF && is_class(tms) {
            const_set(pr, sym("Tms"), tms);
            *C_TMS.get() = tms;
        } else {
            pst().pending = Throw::None;
        }
    }
    defs(pr, "times", 0, 0, |_, _, _| {
        let tv = |t: libc::timeval| float_new(t.tv_sec as f64 + t.tv_usec as f64 * 1e-6);
        let mut me: libc::rusage = unsafe { std::mem::zeroed() };
        let mut ch: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe {
            libc::getrusage(libc::RUSAGE_SELF, &mut me);
            libc::getrusage(libc::RUSAGE_CHILDREN, &mut ch);
        }
        let vals = [tv(me.ru_utime), tv(me.ru_stime), tv(ch.ru_utime), tv(ch.ru_stime)];
        let tms = *C_TMS.get();
        if tms == QNIL {
            return ary_new(vals.to_vec());
        }
        send(tms, sym("new"), &vals, QNIL)
    });

    // ---- Signal ----
    if let Some(sig) = const_lookup(object, sym("Signal")) {
        let sig = sig as *mut Class;
        defs(sig, "list", 0, 0, |_, _, _| {
            let h = hash_new();
            for (n, v) in SIGNALS {
                crate::hash::hset(h, str_from(n), mkfix(*v as i64));
            }
            h
        });
        defs(sig, "signame", 1, 1, |_, a, _| {
            let n = ok!(int_arg(a[0])) as i32;
            match SIGNALS.iter().find(|(_, x)| *x == n) {
                Some((s, _)) => str_from(s),
                None => QNIL,
            }
        });
    }

    // ---- Kernel ----
    let k = c.kernel;
    defn(k, "puts", 0, -1, |_, a, _| match redirected(*GV_STDOUT.get(), *STDOUT_V.get()) {
        None => puts_to(a, &mut |b| out(b)),
        Some(o) => send(o, sym("puts"), a, QNIL),
    });
    defn(k, "print", 0, -1, |_, a, _| match redirected(*GV_STDOUT.get(), *STDOUT_V.get()) {
        None => {
            for &x in a {
                out(&ok!(to_s_bytes(x)));
            }
            QNIL
        }
        Some(o) => send(o, sym("print"), a, QNIL),
    });
    defn(k, "p", 0, -1, |_, a, _| {
        let mut buf = Vec::new();
        for &x in a {
            match inspect(x) {
                Ok(b) => buf.extend_from_slice(&b),
                Err(()) => return UNDEF,
            }
            buf.push(b'\n');
        }
        match redirected(*GV_STDOUT.get(), *STDOUT_V.get()) {
            None => out(&buf),
            Some(o) => {
                tri!(send(o, sym("write"), &[str_new(buf)], QNIL));
            }
        }
        match a.len() {
            0 => QNIL,
            1 => a[0],
            _ => ary_new(a.to_vec()),
        }
    });
    defn(k, "printf", 0, -1, |_, a, _| {
        if a.is_empty() {
            return QNIL;
        }
        if try_st(a[0]).is_some() || (is_heap(a[0]) && !is_str(a[0]) && respond_to(a[0], sym("write"))) {
            return send(a[0], sym("printf"), &a[1..], QNIL);
        }
        let f = tri!(send(rt().main_obj, sym("format"), a, QNIL));
        match redirected(*GV_STDOUT.get(), *STDOUT_V.get()) {
            None => out(&rstr(f).b.clone()),
            Some(o) => {
                tri!(send(o, sym("write"), &[f], QNIL));
            }
        }
        QNIL
    });
    defn(k, "putc", 1, 1, |_, a, _| {
        let o = redirected(*GV_STDOUT.get(), *STDOUT_V.get()).unwrap_or(*STDOUT_V.get());
        send(o, sym("putc"), a, QNIL)
    });
    defn(k, "warn", 0, -1, |_, a, _| {
        let (a, _) = split_kw(a);
        if a.is_empty() {
            return QNIL;
        }
        let mut buf = Vec::new();
        tri!(puts_to(a, &mut |b| buf.extend_from_slice(b)));
        match redirected(*GV_STDERR.get(), *STDERR_V.get()) {
            None => err_out(&buf),
            Some(o) => {
                tri!(send(o, sym("write"), &[str_new(buf)], QNIL));
            }
        }
        QNIL
    });
    fn stdin_obj() -> V {
        redirected(*GV_STDIN.get(), *STDIN_V.get()).unwrap_or(*STDIN_V.get())
    }
    defn(k, "gets", 0, 3, |_, a, _| {
        let l = tri!(send(stdin_obj(), sym("gets"), a, QNIL));
        set_gvar("$_", l);
        l
    });
    defn(k, "readline", 0, 3, |_, a, _| {
        let l = tri!(send(stdin_obj(), sym("gets"), a, QNIL));
        set_gvar("$_", l);
        if l == QNIL { eof_error() } else { l }
    });
    defn(k, "readlines", 0, 3, |_, a, _| send(stdin_obj(), sym("readlines"), a, QNIL));
    defn(k, "open", 1, -1, |_, a, b| {
        if is_str(a[0]) && rstr(a[0]).b.starts_with(b"|") {
            let mut args = RVec::from(a.to_vec());
            args[0] = str_new(rstr(a[0]).b[1..].to_vec());
            return io_popen(rt().c.io as V, &args, b);
        }
        if is_heap(a[0]) && !is_str(a[0]) && respond_to(a[0], sym("to_open")) {
            let o = tri!(send(a[0], sym("to_open"), &a[1..], QNIL));
            return if is_proc(b) { call_block(b, &[o]) } else { o };
        }
        send(*C_FILE.get() as V, sym("open"), a, b)
    });
    defn(k, "system", 1, -1, |_, a, _| k_system(a));
    defn(k, "`", 1, 1, |_, a, _| k_backtick(a));
    defn(k, "spawn", 1, -1, |_, a, _| k_spawn(a));
    defn(k, "exec", 1, -1, |_, a, _| k_exec(a));
    defn(k, "fork", 0, 0, |_, _, b| k_fork(b));
    defn(k, "select", 1, 4, |_, a, _| send(rt().c.io as V, sym("select"), a, QNIL));
    defn(k, "trap", 1, 2, |_, a, b| {
        let sig = const_lookup(rt().c.object, sym("Signal")).unwrap();
        send(sig, sym("trap"), a, b)
    });
    defn(k, "test", 2, 3, |_, a, _| {
        let cmd = if is_fix(a[0]) { fix(a[0]) as u8 } else { ok!(str_bytes(a[0])).first().copied().unwrap_or(0) };
        let f = *C_FILE.get() as V;
        let m = match cmd {
            b'e' => "exist?",
            b'f' => "file?",
            b'd' => "directory?",
            b'r' => "readable?",
            b'w' => "writable?",
            b'x' => "executable?",
            b's' => "size?",
            b'z' => "zero?",
            b'l' => "symlink?",
            b'M' => "mtime",
            _ => return arg_error("unknown command"),
        };
        send(f, sym(m), &a[1..2], QNIL)
    });

    // flush open files at exit (registered first, so it runs after user at_exit blocks)
    let flusher = proc_new_native(
        Box::new(|_, _, _| {
            flush_all();
            QNIL
        }),
        0,
        -1,
    );
    crate::abi::AT_EXIT.get().push(flusher);
    init_stdlib_support();
}

/// Small core methods the pure-Ruby stdlib file helpers (fileutils, tmpdir, tempfile) rely on.
pub fn init_stdlib_support() {
    let c = rt().c;
    // define only what nobody else has defined yet
    fn missing(k: *mut Class, name: &str, singleton: bool) -> bool {
        let k = if singleton { singleton_class(k as V) } else { k };
        find_method(k, sym(name)).is_null()
    }
    if missing(c.module, "ruby2_keywords", false) {
        defn(c.module, "ruby2_keywords", 0, -1, |_, _, _| QNIL);
    }
    if missing(c.proc_, "ruby2_keywords", false) {
        defn(c.proc_, "ruby2_keywords", 0, 0, |s, _, _| s);
    }
    init_etc();
    if !missing(c.array, "try_convert", true) {
        return;
    }
    defs(c.array, "try_convert", 1, 1, |_, a, _| {
        if is_ary(a[0]) {
            return a[0];
        }
        if is_heap(a[0]) && respond_to(a[0], sym("to_ary")) {
            return send(a[0], sym("to_ary"), &[], QNIL);
        }
        QNIL
    });
    defs(c.hash, "try_convert", 1, 1, |_, a, _| {
        if is_hash(a[0]) {
            return a[0];
        }
        if is_heap(a[0]) && respond_to(a[0], sym("to_hash")) {
            return send(a[0], sym("to_hash"), &[], QNIL);
        }
        QNIL
    });
    defs(c.string, "try_convert", 1, 1, |_, a, _| {
        if is_str(a[0]) {
            return a[0];
        }
        if is_heap(a[0]) && respond_to(a[0], sym("to_str")) {
            return send(a[0], sym("to_str"), &[], QNIL);
        }
        QNIL
    });
}

static C_PASSWD: G<V> = G::new(QNIL);

fn passwd_v(pw: *const libc::passwd) -> V {
    if pw.is_null() || *C_PASSWD.get() == QNIL {
        return QNIL;
    }
    let p = unsafe { &*pw };
    let cs = |c: *const libc::c_char| if c.is_null() { str_from("") } else { str_new(unsafe { CStr::from_ptr(c) }.to_bytes().to_vec()) };
    let vals = [cs(p.pw_name), cs(p.pw_passwd), mkfix(p.pw_uid as i64), mkfix(p.pw_gid as i64), cs(p.pw_gecos), cs(p.pw_dir), cs(p.pw_shell)];
    send(*C_PASSWD.get(), sym("new"), &vals, QNIL)
}

/// The `etc` extension (CRuby's is C): systmpdir, nprocessors, getlogin, getpwuid, uname...
fn init_etc() {
    let object = rt().c.object;
    if const_lookup(object, sym("Etc")).is_some() {
        return;
    }
    let etc = new_module("Etc");
    set_global_const("Etc", etc as V);
    if let Some(sc) = const_lookup(object, sym("Struct")) {
        let names: Vec<V> = ["name", "passwd", "uid", "gid", "gecos", "dir", "shell"].iter().map(|n| mksym(sym(n))).collect();
        let k = send(sc, sym("new"), &names, QNIL);
        if k != UNDEF && is_class(k) {
            const_set(etc, sym("Passwd"), k);
            *C_PASSWD.get() = k;
        } else {
            pst().pending = Throw::None;
        }
    }
    defs(etc, "systmpdir", 0, 0, |_, _, _| str_from("/tmp"));
    defs(etc, "sysconfdir", 0, 0, |_, _, _| str_from("/etc"));
    defs(etc, "nprocessors", 0, 0, |_, _, _| mkfix(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as i64));
    defs(etc, "getlogin", 0, 0, |_, _, _| {
        let p = unsafe { libc::getlogin() };
        if !p.is_null() {
            return str_new(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec());
        }
        match std::env::var("USER") {
            Ok(u) => new_str_v(u),
            Err(_) => QNIL,
        }
    });
    defs(etc, "getpwuid", 0, 1, |_, a, _| {
        let uid = match a.first() {
            Some(&u) if is_fix(u) => fix(u) as libc::uid_t,
            _ => unsafe { libc::getuid() },
        };
        passwd_v(unsafe { libc::getpwuid(uid) })
    });
    defs(etc, "getpwnam", 1, 1, |_, a, _| {
        let n = ok!(cstr(&ok!(str_bytes(a[0]))));
        let pw = unsafe { libc::getpwnam(n.as_ptr()) };
        if pw.is_null() {
            return arg_error(&format!("can't find user for {}", bstr(n.as_bytes())));
        }
        passwd_v(pw)
    });
    defs(etc, "uname", 0, 0, |_, _, _| {
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        unsafe { libc::uname(&mut u) };
        let h = hash_new();
        let f = |x: &[libc::c_char]| str_new(unsafe { CStr::from_ptr(x.as_ptr()) }.to_bytes().to_vec());
        for (k, v) in [("sysname", f(&u.sysname)), ("nodename", f(&u.nodename)), ("release", f(&u.release)), ("version", f(&u.version)), ("machine", f(&u.machine))] {
            crate::hash::hset(h, mksym(sym(k)), v);
        }
        h
    });
}
