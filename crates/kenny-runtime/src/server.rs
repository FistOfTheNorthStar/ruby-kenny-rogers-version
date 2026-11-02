//! The built-in HTTP server (like Go's net/http, but for Rack apps).
//!
//! A `config.ru` compiled with Kenny calls `run app` (and optionally `use Middleware`).
//! When the program's top level finishes and an app was registered, the binary starts
//! serving HTTP/1.1. No Puma/Unicorn needed.
//!
//! Concurrency: an event loop on every P (one per core), all accepting from the same socket,
//! reads, parses and writes the connections; the requests run on a pool of request threads
//! (lightweight Threads, KENNY_SERVER_THREADS) that take them from one queue and hand the
//! responses back to the connection's loop. A connection runs one request at a time.

use crate::core::*;
use crate::kernel::{err_out, flush_out};
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static APP: G<Option<V>> = G::new(None);

/// After this long without any request on any P, P 0's loop runs a full collection that may
/// move objects to empty sparse pages and return memory (gc::idle_compact).
const IDLE_TIDY: std::time::Duration = std::time::Duration::from_secs(1);
/// When some P last had network activity (ms since process start).
static LAST_ACTIVITY: AtomicStamp = AtomicStamp::new(0);
struct Stamp;
impl Stamp {
    fn now() -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
    }
}
struct AtomicStamp(std::sync::atomic::AtomicU64);
impl AtomicStamp {
    const fn new(v: u64) -> Self {
        AtomicStamp(std::sync::atomic::AtomicU64::new(v))
    }
    fn load(&self, o: std::sync::atomic::Ordering) -> StampVal {
        StampVal(self.0.load(o))
    }
    fn store(&self, v: u64, o: std::sync::atomic::Ordering) {
        self.0.store(v, o)
    }
}
struct StampVal(u64);
impl StampVal {
    fn elapsed_ms(&self) -> u64 {
        Stamp::now().saturating_sub(self.0)
    }
}
static MIDDLEWARE: G<Vec<(V, Vec<V>, V)>> = G::new(Vec::new());

pub fn static_roots() -> Vec<V> {
    let mut v: Vec<V> = APP.get().iter().copied().collect();
    for (m, args, b) in MIDDLEWARE.get().iter() {
        v.push(*m);
        v.extend(args.iter().copied());
        v.push(*b);
    }
    v
}

// ---------- StringIO (needed for rack.input) ----------

fn sio_buf(s: V) -> V {
    ivar_get(s, sym("@__buf"))
}
fn sio_pos(s: V) -> usize {
    let p = ivar_get(s, sym("@__pos"));
    if is_fix(p) { fix(p) as usize } else { 0 }
}
fn sio_set_pos(s: V, p: usize) {
    ivar_set(s, sym("@__pos"), mkfix(p as i64));
}
pub fn stringio_new(bytes: Vec<u8>) -> V {
    let c = const_lookup(rt().c.object, sym("StringIO")).unwrap();
    let o = object_new(c as *mut Class);
    ivar_set(o, sym("@__buf"), str_new(bytes));
    sio_set_pos(o, 0);
    o
}

fn init_stringio() {
    let c = new_class("StringIO", rt().c.object);
    set_global_const("StringIO", c as V);
    defn(c, "initialize", 0, 2, |s, a, _| {
        let b = a.first().filter(|x| is_str(**x)).map(|x| rstr(*x).b.clone()).unwrap_or_default();
        ivar_set(s, sym("@__buf"), str_new(b));
        sio_set_pos(s, 0);
        QNIL
    });
    defn(c, "string", 0, 0, |s, _, _| sio_buf(s));
    defn(c, "read", 0, 2, |s, a, _| {
        let b = rstr(sio_buf(s)).b.clone();
        let pos = sio_pos(s).min(b.len());
        let end = match a.first() {
            Some(n) if is_fix(*n) => (pos + fix(*n) as usize).min(b.len()),
            _ => b.len(),
        };
        // (`read(n, buffer)` fills the buffer and returns it, as IO#read does)
        let out = a.get(1).copied().filter(|x| is_str(*x));
        if let Some(o) = out {
            crate::tri!(check_frozen(o));
        }
        if a.first().is_some_and(|n| is_fix(*n)) && pos >= b.len() {
            if let Some(o) = out {
                rstr(o).b.clear();
            }
            return QNIL;
        }
        sio_set_pos(s, end);
        match out {
            Some(o) => {
                let ob = &mut rstr(o).b;
                ob.clear();
                ob.extend_from_slice(&b[pos..end]);
                o
            }
            None => str_new(b[pos..end].to_vec()),
        }
    });
    defn(c, "gets", 0, 1, |s, _, _| {
        let b = rstr(sio_buf(s)).b.clone();
        let pos = sio_pos(s);
        if pos >= b.len() {
            return QNIL;
        }
        let end = b[pos..].iter().position(|c| *c == b'\n').map(|i| pos + i + 1).unwrap_or(b.len());
        sio_set_pos(s, end);
        str_new(b[pos..end].to_vec())
    });
    defn(c, "each", 0, 1, |s, _, blk| {
        loop {
            let l = send0(s, "gets");
            if l == QNIL {
                return s;
            }
            crate::tri!(call_block(blk, &[l]));
        }
    });
    defn(c, "rewind", 0, 0, |s, _, _| {
        sio_set_pos(s, 0);
        mkfix(0)
    });
    defn(c, "pos", 0, 0, |s, _, _| mkfix(sio_pos(s) as i64));
    defn(c, "eof?", 0, 0, |s, _, _| bool_v(sio_pos(s) >= rstr(sio_buf(s)).b.len()));
    defn(c, "size", 0, 0, |s, _, _| mkfix(rstr(sio_buf(s)).b.len() as i64));
    fn write(s: V, a: &[V]) -> V {
        let mut n = 0;
        for &x in a {
            match crate::fmt::to_s(x) {
                Ok(b) => {
                    n += b.len();
                    rstr(sio_buf(s)).b.extend_from_slice(&b);
                }
                Err(()) => return UNDEF,
            }
        }
        sio_set_pos(s, rstr(sio_buf(s)).b.len());
        mkfix(n as i64)
    }
    defn(c, "write", 0, -1, |s, a, _| write(s, a));
    defn(c, "print", 0, -1, |s, a, _| {
        crate::tri!(write(s, a));
        QNIL
    });
    defn(c, "<<", 1, 1, |s, a, _| {
        crate::tri!(write(s, a));
        s
    });
    defn(c, "puts", 0, -1, |s, a, _| {
        let buf = sio_buf(s);
        let r = crate::kernel::puts_to(a, &mut |b| rstr(buf).b.extend_from_slice(b));
        sio_set_pos(s, rstr(buf).b.len());
        r
    });
    defn(c, "close", 0, 0, |_, _, _| QNIL);
    defn(c, "binmode", 0, 0, |s, _, _| s);
    defn(c, "set_encoding", 0, -1, |s, _, _| s);
}

pub fn init() {
    init_stringio();
    let k = rt().c.kernel;
    // Rack::Builder DSL, available at the top level of a config.ru
    defn(k, "run", 0, 1, |_, a, b| {
        let app = a.first().copied().unwrap_or(b);
        *APP.get() = Some(app);
        app
    });
    defn(k, "use", 1, -1, |_, a, b| {
        MIDDLEWARE.get().push((a[0], a[1..].to_vec(), b));
        QNIL
    });
}

// ---------- serving ----------

fn reason(code: i64) -> &'static str {
    match code {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn report(prefix: &str) {
    let pending = std::mem::replace(&mut pst().pending, Throw::None);
    if let Throw::Raise(e) = pending {
        let m = send0(e, "message");
        let m = if is_str(m) { String::from_utf8_lossy(&rstr(m).b).into_owned() } else { String::new() };
        let mut out = format!("{prefix}: {} ({})\n", m, class_name(real_class(e)));
        let bt = crate::backtrace::exc_backtrace(e);
        if is_ary(bt) {
            for l in rary(bt).v.iter().filter(|x| is_str(**x)) {
                out.push_str(&format!("\tfrom {}\n", String::from_utf8_lossy(&rstr(*l).b)));
            }
        }
        err_out(out.as_bytes());
    } else {
        err_out(format!("{prefix}: unexpected non-local exit\n").as_bytes());
    }
}

/// Called after the program's top level ran. Returns an exit code if a server ran.
pub fn maybe_serve() -> Option<i32> {
    let mut app = (*APP.get())?;
    // `kenny build` precompiles a Rails app's assets with the app's own binary: the app is
    // loaded, then lib/kenny/kenny/assets_precompile.rb runs instead of the server
    if std::env::var_os("KENNY_ASSETS_PRECOMPILE").is_some() {
        let r = send(rt().main_obj, sym("require"), &[str_from("kenny/assets_precompile")], QNIL);
        if r == UNDEF || !matches!(pst().pending, Throw::None) {
            report("assets:precompile failed");
            return Some(1);
        }
        return Some(0);
    }
    for (mw, args, blk) in MIDDLEWARE.get().clone().into_iter().rev() {
        let mut a = vec![app];
        a.extend(args);
        app = send(mw, sym("new"), &a, blk);
        if app == UNDEF {
            report("failed to build middleware");
            return Some(1);
        }
    }
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let listener = match std::net::TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            err_out(format!("kenny: can't listen on {host}:{port}: {e}\n").as_bytes());
            return Some(1);
        }
    };
    let _ = listener.set_nonblocking(true);
    // One process using every core: an event loop on each of KENNY_THREADS OS threads, all
    // accepting from the same socket. WEB_CONCURRENCY=n additionally forks n processes.
    let procs: usize = std::env::var("WEB_CONCURRENCY").ok().and_then(|w| w.parse().ok()).unwrap_or(1).max(1);
    let loops = crate::thread::max_ps();
    println_flush(&format!(
        "Kenny serving http://{host}:{port} ({loops} thread{}{})",
        if loops == 1 { "" } else { "s" },
        if procs > 1 { format!(" x {procs} processes") } else { String::new() }
    ));
    MULTIPROCESS.store(procs > 1, Ordering::Relaxed);
    if procs <= 1 {
        return Some(serve_on_all_ps(listener, app, loops));
    }
    let mut children = Vec::new();
    for _ in 0..procs {
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // (threads don't survive fork: each child starts its own)
            let code = serve_on_all_ps(listener, app, loops);
            flush_out();
            unsafe { libc::_exit(code) };
        }
        if pid > 0 {
            children.push(pid);
        }
    }
    let mut status = 0;
    for _ in &children {
        unsafe { libc::waitpid(-1, &mut status, 0) };
    }
    Some(0)
}

static MULTIPROCESS: AtomicBool = AtomicBool::new(false);

/// Run an event loop on every P (this one included), and start the request threads; returns
/// when this P's loop ends.
fn serve_on_all_ps(listener: std::net::TcpListener, app: V, loops: usize) -> i32 {
    let workers = request_threads(loops);
    REQUEST_THREADS.store(workers, Ordering::Relaxed);
    for i in 0..workers {
        crate::thread::spawn_native_unpinned_on(i % loops, Box::new(move || request_thread(app)));
    }
    for p in 1..loops {
        let l = match listener.try_clone() {
            Ok(l) => l,
            Err(e) => {
                err_out(format!("kenny: listener: {e}\n").as_bytes());
                break;
            }
        };
        crate::thread::spawn_native_on(
            p,
            Box::new(move || {
                let code = worker_loop(l, app);
                mkfix(code as i64)
            }),
        );
    }
    worker_loop(listener, app)
}

/// How many threads run requests: KENNY_SERVER_THREADS (0: each event loop runs its requests
/// itself, one at a time), else 4 per event loop. A request that waits (on the database, a
/// socket, `sleep`) leaves its core to the others.
fn request_threads(loops: usize) -> usize {
    std::env::var("KENNY_SERVER_THREADS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(loops * 4).min(4096)
}
static REQUEST_THREADS: AtomicUsize = AtomicUsize::new(0);
/// Requests running now, on any loop or request thread (the idle tidy-up waits for none).
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// A parsed request waiting for a request thread, and the mailbox of the event loop that
/// owns its connection.
struct Job {
    req: Request,
    token: Token,
    fd: i32,
    peer: String,
    port: u16,
    reply: Arc<Mailbox>,
}
/// What a request thread hands back: the response bytes, keep-alive, hijacked.
struct Done {
    token: Token,
    resp: Vec<u8>,
    keep: bool,
    hijacked: bool,
}
/// An event loop's finished requests. A request thread wakes the loop's poller only when the
/// loop waits there (`polling`): when it is running it takes them before it next polls.
struct Mailbox {
    done: Mutex<Vec<Done>>,
    polling: AtomicBool,
    waker: Option<Arc<mio::Waker>>,
}
impl Mailbox {
    fn post(&self, d: Done) {
        self.done.lock().unwrap_or_else(|e| e.into_inner()).push(d);
        if self.polling.load(Ordering::SeqCst) {
            if let Some(w) = &self.waker {
                let _ = w.wake();
            }
        }
    }
    fn take(&self) -> Vec<Done> {
        std::mem::take(&mut *self.done.lock().unwrap_or_else(|e| e.into_inner()))
    }
    fn is_empty(&self) -> bool {
        self.done.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

/// Requests waiting for a request thread (all event loops of the process share them).
static JOBS: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
/// What idle request threads wait on (an address no object has).
fn jobs_key() -> usize {
    &JOBS as *const _ as usize
}
fn jobs_waiting() -> bool {
    !JOBS.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// A request thread: run the requests the event loops queue, one at a time.
fn request_thread(app: V) -> V {
    loop {
        let job = JOBS.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        let Some(job) = job else {
            if !crate::thread::wait_until(jobs_key(), jobs_waiting) {
                // (a Thread#raise / kill from the app: request threads keep serving)
                pst().pending = Throw::None;
            }
            continue;
        };
        let (resp, keep, hijacked) = handle(app, job.req, job.port, &job.peer, job.fd);
        pst().pending = Throw::None;
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        job.reply.post(Done { token: job.token, resp, keep, hijacked });
    }
}

fn println_flush(s: &str) {
    crate::kernel::out(s.as_bytes());
    crate::kernel::out(b"\n");
    flush_out();
}

// ---------- limits and timeouts ----------

/// The request line and headers may take this much (Puma's limit): 431 above it.
const MAX_HEAD: usize = 112 * 1024;
/// Headers per request (431 above it).
const MAX_HEADERS: usize = 128;
/// A request body larger than this goes to a temporary file instead of memory.
const SPOOL_AT: usize = 1 << 20;
/// What one connection's readiness reads at most before the loop turns to the others (the
/// rest is read on the loop's next round).
const READ_BUDGET: usize = 256 * 1024;
/// How often the loop looks for connections that timed out.
const SWEEP_EVERY: Duration = Duration::from_secs(1);

/// The connections' limits, from the environment.
struct Limits {
    /// KENNY_MAX_BODY (bytes, or with a K, M or G suffix): the largest request body taken
    /// (413 above it). None by default, as in Puma: bodies over 1 MB go to a temporary file.
    max_body: Option<u64>,
    /// KENNY_READ_TIMEOUT (seconds, default 30; 0: none): a request's headers must arrive
    /// within it, and its body upload, or a response the client reads, may not stall longer.
    read: Option<Duration>,
    /// KENNY_KEEPALIVE_TIMEOUT (seconds, default 20; 0: none): how long a keep-alive
    /// connection may wait idle for its next request.
    keepalive: Option<Duration>,
}

fn limits() -> &'static Limits {
    static L: std::sync::OnceLock<Limits> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let secs = |name: &str, default: f64| {
            let v = std::env::var(name).ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default);
            (v > 0.0).then(|| Duration::from_secs_f64(v))
        };
        Limits { max_body: std::env::var("KENNY_MAX_BODY").ok().and_then(|v| parse_size(&v)), read: secs("KENNY_READ_TIMEOUT", 30.0), keepalive: secs("KENNY_KEEPALIVE_TIMEOUT", 20.0) }
    })
}

/// "1048576", "512K", "10M", "1G" (powers of 1024).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.as_bytes().last()?.to_ascii_uppercase() {
        b'K' => (&s[..s.len() - 1], 1u64 << 10),
        b'M' => (&s[..s.len() - 1], 1 << 20),
        b'G' => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    num.trim().parse::<u64>().ok()?.checked_mul(mult)
}

// ---------- connections ----------

struct Conn {
    stream: TcpStream,
    input: Vec<u8>,
    output: Vec<u8>,
    close_after: bool,
    /// the client closed its side (requests it sent before still get their responses)
    eof: bool,
    /// a read or write failed: close once no request of it is running
    broken: bool,
    /// a request thread is running one of its requests: the loop doesn't read from it
    /// meanwhile (the app may hijack the socket), and its next request waits
    busy: bool,
    peer: String,
    /// taken over by the app through `rack.hijack`
    hijacked: bool,
    /// the request whose body is arriving (its headers were read)
    incoming: Option<Incoming>,
    /// it asked for `100 Continue` before sending its body: sent once the body is awaited
    expect: bool,
    /// the last time bytes moved either way (or a request finished): stalls count from here
    active_at: Instant,
    /// when the first bytes of the headers being read arrived
    head_at: Option<Instant>,
    /// it has served a request: idle, it is a keep-alive connection
    served: bool,
    /// the last read stopped at READ_BUDGET with the socket not drained
    more: bool,
}

impl Conn {
    fn new(stream: TcpStream, peer: String) -> Conn {
        Conn {
            stream,
            input: Vec::new(),
            output: Vec::new(),
            close_after: false,
            eof: false,
            broken: false,
            busy: false,
            peer,
            hijacked: false,
            incoming: None,
            expect: false,
            active_at: Instant::now(),
            head_at: None,
            served: false,
            more: false,
        }
    }
    /// Read what the socket has (it is edge-triggered: until it would block), up to
    /// READ_BUDGET (then `more` is set and the loop comes back to it).
    fn read_all(&mut self) {
        self.more = false;
        let mut buf = [0u8; 16384];
        let mut got = 0;
        loop {
            if got >= READ_BUDGET {
                self.more = true;
                break;
            }
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => {
                    self.input.extend_from_slice(&buf[..n]);
                    got += n;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.broken = true;
                    break;
                }
            }
        }
        if got > 0 {
            self.active_at = Instant::now();
        }
    }
    /// Write what output the socket takes.
    fn write_out(&mut self) {
        let before = self.output.len();
        while !self.output.is_empty() && !self.broken {
            match self.stream.write(&self.output) {
                Ok(0) => self.broken = true,
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => self.broken = true,
            }
        }
        if self.output.len() < before {
            self.active_at = Instant::now();
        }
    }
    /// A request of this connection finished.
    fn finish(&mut self, resp: Vec<u8>, keep: bool, hijacked: bool) {
        self.busy = false;
        self.served = true;
        self.active_at = Instant::now();
        if hijacked {
            // the app owns the socket now (WebSockets: Action Cable)
            self.hijacked = true;
            return;
        }
        self.output.extend_from_slice(&resp);
        if !keep {
            self.close_after = true;
        }
    }
    /// Answer with an error and close (the request couldn't be read).
    fn fail(&mut self, code: i64) {
        self.output.extend_from_slice(&error_response(code, false));
        self.close_after = true;
        self.incoming = None;
        self.expect = false;
    }
    /// Why the connection should be closed now (the read timeout's 408 code, or 0 for no
    /// answer), if it timed out.
    fn timed_out(&self, now: Instant, l: &Limits) -> Option<i64> {
        if self.busy {
            return None; // (a running request isn't timed: that is the app's business)
        }
        let over = |since: Instant, limit: Option<Duration>| limit.is_some_and(|d| now.saturating_duration_since(since) > d);
        if !self.output.is_empty() {
            // the client doesn't read its response
            return over(self.active_at, l.read).then_some(0);
        }
        if self.incoming.is_some() {
            return over(self.active_at, l.read).then_some(408);
        }
        if let Some(h) = self.head_at {
            return over(h, l.read).then_some(408);
        }
        if self.served { over(self.active_at, l.keepalive) } else { over(self.active_at, l.read) }.then_some(0)
    }
}

const LISTENER: Token = Token(0);
/// Another P gave this one work (a thread to run, see thread::set_wake_hook), or a request
/// thread finished one of this loop's requests.
const WAKER: Token = Token(usize::MAX);

/// One P's event loop: accepts connections, reads and parses their requests, queues them
/// for the request threads, and writes the responses back.
fn worker_loop(std_listener: std::net::TcpListener, app: V) -> i32 {
    let mut poll = match Poll::new() {
        Ok(p) => p,
        Err(e) => {
            err_out(format!("kenny: poll: {e}\n").as_bytes());
            return 1;
        }
    };
    let mut listener = TcpListener::from_std(std_listener.try_clone().expect("listener"));
    poll.registry().register(&mut listener, LISTENER, Interest::READABLE).expect("register listener");
    let waker = mio::Waker::new(poll.registry(), WAKER).ok().map(Arc::new);
    if let Some(w) = &waker {
        let w = w.clone();
        crate::thread::set_wake_hook(Box::new(move || {
            let _ = w.wake();
        }));
    }
    // without a waker no request thread could tell this loop it finished: run requests here
    let pooled = waker.is_some() && REQUEST_THREADS.load(Ordering::Relaxed) > 0;
    let mailbox = Arc::new(Mailbox { done: Mutex::new(Vec::new()), polling: AtomicBool::new(false), waker });
    let mut conns: HashMap<Token, Conn> = HashMap::new();
    let mut next = 1usize;
    let mut events = Events::with_capacity(1024);
    let port = std_listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let me = crate::core::pst().thr.idx;
    // the loop on P 0 tidies the heap once the server has been idle for a second
    let tidier = me == 0;
    let mut ctx = LoopCtx { app, port, pooled, mailbox: mailbox.clone() };
    // connections whose reads stopped at READ_BUDGET
    let mut again: Vec<Token> = Vec::new();
    let mut next_sweep = Instant::now() + SWEEP_EVERY;
    loop {
        let mut timeout = crate::thread::idle_timeout();
        if tidier && crate::gc::idle_compaction_wanted() {
            timeout = Some(timeout.map_or(IDLE_TIDY, |t| t.min(IDLE_TIDY)));
        }
        if !conns.is_empty() {
            let to_sweep = next_sweep.saturating_duration_since(Instant::now());
            timeout = Some(timeout.map_or(to_sweep, |t| t.min(to_sweep)));
        }
        mailbox.polling.store(true, Ordering::SeqCst);
        if !mailbox.is_empty() || !again.is_empty() {
            timeout = Some(Duration::ZERO);
        }
        // waiting for the network doesn't hold up a collection on another thread
        let r = crate::gc::blocking(|| poll.poll(&mut events, timeout));
        mailbox.polling.store(false, Ordering::SeqCst);
        if let Err(e) = r {
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            err_out(format!("kenny: poll: {e}\n").as_bytes());
            return 1;
        }
        if events.is_empty() {
            if tidier && LAST_ACTIVITY.load(Ordering::Relaxed).elapsed_ms() >= IDLE_TIDY.as_millis() as u64 && IN_FLIGHT.load(Ordering::SeqCst) == 0 {
                crate::gc::idle_compact();
            }
        } else {
            LAST_ACTIVITY.store(Stamp::now(), Ordering::Relaxed);
        }
        for ev in events.iter() {
            if ev.token() == WAKER {
                continue;
            }
            if ev.token() == LISTENER {
                loop {
                    match listener.accept() {
                        Ok((mut stream, addr)) => {
                            let t = Token(next);
                            next += 1;
                            let _ = stream.set_nodelay(true);
                            if poll.registry().register(&mut stream, t, Interest::READABLE).is_ok() {
                                conns.insert(t, Conn::new(stream, addr.ip().to_string()));
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
                continue;
            }
            let t = ev.token();
            if let Some(c) = conns.get_mut(&t) {
                if ev.is_readable() && !c.busy {
                    c.read_all();
                }
                drive(&mut ctx, &mut poll, &mut conns, t, &mut again);
            }
        }
        for t in std::mem::take(&mut again) {
            if let Some(c) = conns.get_mut(&t) {
                if !c.busy && !c.hijacked && !c.broken {
                    c.read_all();
                }
                drive(&mut ctx, &mut poll, &mut conns, t, &mut again);
            }
        }
        // the requests the request threads finished meanwhile; then let this P's threads
        // (request threads among them) run before polling again; a few rounds while they
        // keep finishing this loop's requests, then back to the network. (Without request
        // threads too: the app's own threads on this P run only here.)
        for _ in 0..4 {
            for d in mailbox.take() {
                if let Some(c) = conns.get_mut(&d.token) {
                    c.finish(d.resp, d.keep, d.hijacked);
                    // what came while the request ran (a pipelined request, the client closing)
                    if !c.hijacked && !c.broken {
                        c.read_all();
                    }
                    drive(&mut ctx, &mut poll, &mut conns, d.token, &mut again);
                }
            }
            flush_out();
            if !crate::thread::active() {
                break;
            }
            crate::thread::pass();
            pst().pending = Throw::None;
            if !pooled || mailbox.is_empty() {
                break;
            }
        }
        let now = Instant::now();
        if now >= next_sweep {
            next_sweep = now + SWEEP_EVERY;
            sweep(&mut poll, &mut conns, now);
        }
    }
}

/// Close the connections that timed out (with a 408 to those in the middle of sending a
/// request).
fn sweep(poll: &mut Poll, conns: &mut HashMap<Token, Conn>, now: Instant) {
    let l = limits();
    let expired: Vec<(Token, i64)> = conns.iter().filter_map(|(t, c)| c.timed_out(now, l).map(|code| (*t, code))).collect();
    for (t, code) in expired {
        if let Some(mut c) = conns.remove(&t) {
            if code != 0 {
                // (best effort: the connection is closed whatever the socket takes)
                let _ = c.stream.write(&error_response(code, false));
            }
            let _ = poll.registry().deregister(&mut c.stream);
        }
    }
}

/// What `drive` needs of its event loop.
struct LoopCtx {
    app: V,
    port: u16,
    pooled: bool,
    mailbox: Arc<Mailbox>,
}

/// Move connection `t` along: read its next request's headers and body, start the request
/// unless one is running (on a request thread, or right here without them), write what
/// output it can, close it when it is done.
fn drive(ctx: &mut LoopCtx, poll: &mut Poll, conns: &mut HashMap<Token, Conn>, t: Token, again: &mut Vec<Token>) {
    let Some(c) = conns.get_mut(&t) else { return };
    while !c.busy && !c.close_after && !c.broken && !c.hijacked {
        if c.incoming.is_none() {
            if c.input.is_empty() {
                break;
            }
            match parse_head(&c.input) {
                Head::Partial => {
                    c.head_at.get_or_insert_with(Instant::now);
                    break;
                }
                Head::Fail(code) => {
                    c.fail(code);
                    break;
                }
                Head::Done(inc, hlen, expect) => {
                    c.input.drain(..hlen);
                    c.head_at = None;
                    c.incoming = Some(inc);
                    c.expect = expect;
                }
            }
        }
        let inc = c.incoming.as_mut().unwrap();
        match inc.feed(&mut c.input) {
            Feed::More => {
                if std::mem::take(&mut c.expect) {
                    c.output.extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
                }
                break;
            }
            Feed::Fail(code) => {
                c.fail(code);
                break;
            }
            Feed::Done => {}
        }
        c.expect = false;
        let req = match c.incoming.take().unwrap().into_request() {
            Ok(r) => r,
            Err(_) => {
                c.fail(500);
                break;
            }
        };
        let fd = c.stream.as_raw_fd();
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        if ctx.pooled {
            c.busy = true;
            let job = Job { req, token: t, fd, peer: c.peer.clone(), port: ctx.port, reply: ctx.mailbox.clone() };
            JOBS.lock().unwrap_or_else(|e| e.into_inner()).push_back(job);
            crate::thread::notify_one(jobs_key(), Some(crate::core::pst().thr.idx));
        } else {
            let (resp, keep, hijacked) = handle(ctx.app, req, ctx.port, &c.peer, fd);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            c.finish(resp, keep, hijacked);
        }
    }
    if !c.hijacked {
        c.write_out();
    }
    // (a connection with a request running stays open: the request thread has its socket)
    let done = !c.busy && (c.hijacked || c.broken || (c.output.is_empty() && (c.close_after || c.eof)));
    if !done {
        if c.more && !c.busy {
            again.push(t);
        }
        let interest = if c.output.is_empty() { Interest::READABLE } else { Interest::READABLE | Interest::WRITABLE };
        let _ = poll.registry().reregister(&mut c.stream, t, interest);
        return;
    }
    if let Some(mut c) = conns.remove(&t) {
        // (a hijacked socket stays open: the app's IO has its own descriptor for it)
        let _ = poll.registry().deregister(&mut c.stream);
    }
}

// ---------- requests ----------

struct Request {
    method: String,
    target: String,
    version: u8,
    headers: Vec<(String, Vec<u8>)>,
    body: Body,
}

/// A request body: in memory, or in a temporary file (already unlinked) when it is large.
enum Body {
    Mem(Vec<u8>),
    File(std::fs::File),
}

/// Where a body goes while it arrives: memory, then a temporary file past SPOOL_AT.
struct Sink {
    mem: Vec<u8>,
    file: Option<std::fs::File>,
    len: u64,
}
impl Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<()> {
        if b.is_empty() {
            return Ok(());
        }
        // (file writes are blocking calls: the P's other threads may run elsewhere meanwhile,
        // and a collection doesn't wait for them)
        if self.file.is_none() && self.mem.len() + b.len() > SPOOL_AT {
            let mem = std::mem::take(&mut self.mem);
            self.file = Some(crate::gc::blocking(|| {
                let mut f = spool_file()?;
                f.write_all(&mem)?;
                Ok::<_, std::io::Error>(f)
            })?);
        }
        match &mut self.file {
            Some(f) => crate::gc::blocking(|| f.write_all(b))?,
            None => self.mem.extend_from_slice(b),
        }
        self.len += b.len() as u64;
        Ok(())
    }
}

/// A new temporary file, removed from its directory at once (it lives while it is open).
fn spool_file() -> std::io::Result<std::fs::File> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir();
    let mut last = None;
    for _ in 0..16 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let path = dir.join(format!("kenny-body-{}-{}-{nanos}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path) {
            Ok(f) => {
                let _ = std::fs::remove_file(&path);
                return Ok(f);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no temporary file")))
}

/// How the body is delimited.
enum Framing {
    /// Content-Length: the bytes still to come
    Length(u64),
    Chunked(Chunk),
}
/// Where a chunked body's decoding is.
enum Chunk {
    /// at a chunk's size line
    Size,
    /// in a chunk's data: the bytes still to come
    Data(u64),
    /// at the CRLF after a chunk's data
    DataEnd,
    /// after the last chunk: trailer lines until an empty one
    Trailer,
}

/// A request whose headers were read, while its body arrives.
struct Incoming {
    req: Request,
    framing: Framing,
    sink: Sink,
    /// bytes of trailer lines so far (a chunked body's: they count against MAX_HEAD)
    trailer: usize,
}

enum Feed {
    More,
    Done,
    Fail(i64),
}

impl Incoming {
    /// Take the body bytes at the start of `input` (and only those: the next pipelined
    /// request stays).
    fn feed(&mut self, input: &mut Vec<u8>) -> Feed {
        let max = limits().max_body;
        let Incoming { framing, sink, trailer, .. } = self;
        let mut pos = 0;
        let r = loop {
            let rest = &input[pos..];
            match framing {
                Framing::Length(left) => {
                    let n = (*left).min(rest.len() as u64) as usize;
                    if sink.write(&rest[..n]).is_err() {
                        break Feed::Fail(500);
                    }
                    pos += n;
                    *left -= n as u64;
                    break if *left == 0 { Feed::Done } else { Feed::More };
                }
                Framing::Chunked(st) => match st {
                    Chunk::Size | Chunk::Trailer => {
                        let Some(i) = rest.windows(2).position(|w| w == b"\r\n") else {
                            let too_long = if matches!(st, Chunk::Trailer) { *trailer + rest.len() > MAX_HEAD } else { rest.len() > 4096 };
                            break if too_long { Feed::Fail(if matches!(st, Chunk::Trailer) { 431 } else { 400 }) } else { Feed::More };
                        };
                        let line = &rest[..i];
                        pos += i + 2;
                        if matches!(st, Chunk::Trailer) {
                            if line.is_empty() {
                                break Feed::Done;
                            }
                            *trailer += i + 2;
                            if *trailer > MAX_HEAD {
                                break Feed::Fail(431);
                            }
                            continue;
                        }
                        // (chunk extensions after a `;` are ignored)
                        let hex = line.split(|b| *b == b';').next().unwrap_or(b"");
                        let hex = hex.trim_ascii();
                        if hex.is_empty() || hex.len() > 15 || !hex.iter().all(u8::is_ascii_hexdigit) {
                            break Feed::Fail(400);
                        }
                        let n = u64::from_str_radix(std::str::from_utf8(hex).unwrap_or("0"), 16).unwrap_or(0);
                        if max.is_some_and(|m| sink.len + n > m) {
                            break Feed::Fail(413);
                        }
                        *st = if n == 0 { Chunk::Trailer } else { Chunk::Data(n) };
                    }
                    Chunk::Data(left) => {
                        let n = (*left).min(rest.len() as u64) as usize;
                        if sink.write(&rest[..n]).is_err() {
                            break Feed::Fail(500);
                        }
                        pos += n;
                        *left -= n as u64;
                        if *left > 0 {
                            break Feed::More;
                        }
                        *st = Chunk::DataEnd;
                    }
                    Chunk::DataEnd => {
                        if rest.len() < 2 {
                            break Feed::More;
                        }
                        if &rest[..2] != b"\r\n" {
                            break Feed::Fail(400);
                        }
                        pos += 2;
                        *st = Chunk::Size;
                    }
                },
            }
        };
        input.drain(..pos);
        r
    }

    /// The request, its body complete. A chunked body is given to the app as one of known
    /// length (CONTENT_LENGTH, no Transfer-Encoding), as Puma does.
    fn into_request(self) -> std::io::Result<Request> {
        let mut req = self.req;
        if matches!(self.framing, Framing::Chunked(_)) {
            req.headers.push(("Content-Length".into(), self.sink.len.to_string().into_bytes()));
        }
        req.body = match self.sink.file {
            Some(mut f) => {
                use std::io::Seek;
                f.seek(std::io::SeekFrom::Start(0))?;
                Body::File(f)
            }
            None => Body::Mem(self.sink.mem),
        };
        Ok(req)
    }
}

enum Head {
    Partial,
    Fail(i64),
    /// the request (its body to come), the length of its head, whether it expects `100 Continue`
    Done(Incoming, usize, bool),
}

/// Read a request's line and headers from the start of `buf`.
fn parse_head(buf: &[u8]) -> Head {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    let hlen = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return if buf.len() > MAX_HEAD { Head::Fail(431) } else { Head::Partial },
        Err(httparse::Error::TooManyHeaders) => return Head::Fail(431),
        Err(_) => return Head::Fail(400),
    };
    if hlen > MAX_HEAD {
        return Head::Fail(431);
    }
    let version = req.version.unwrap_or(1);
    let mut clen: Option<u64> = None;
    let mut chunked = false;
    let mut expect = false;
    let mut hs = Vec::new();
    for h in req.headers.iter() {
        if h.name.eq_ignore_ascii_case("content-length") {
            // digits only (no sign, no list): disagreeing lengths are how requests get smuggled
            let v = h.value.trim_ascii();
            if v.is_empty() || v.len() > 18 || !v.iter().all(u8::is_ascii_digit) {
                return Head::Fail(400);
            }
            let n: u64 = std::str::from_utf8(v).unwrap_or("0").parse().unwrap_or(0);
            if clen.is_some_and(|c| c != n) {
                return Head::Fail(400);
            }
            clen = Some(n);
        } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
            if !h.value.trim_ascii().eq_ignore_ascii_case(b"chunked") || chunked {
                // (gzip, deflate ... bodies: not supported, as in Puma)
                return Head::Fail(501);
            }
            chunked = true;
            continue;
        } else if h.name.eq_ignore_ascii_case("expect") && h.value.trim_ascii().eq_ignore_ascii_case(b"100-continue") {
            expect = version == 1;
        }
        hs.push((h.name.to_string(), h.value.to_vec()));
    }
    if chunked && (clen.is_some() || version == 0) {
        return Head::Fail(400);
    }
    if let (Some(max), Some(n)) = (limits().max_body, clen) {
        if n > max {
            return Head::Fail(413);
        }
    }
    let framing = if chunked { Framing::Chunked(Chunk::Size) } else { Framing::Length(clen.unwrap_or(0)) };
    let cap = clen.unwrap_or(0).min(SPOOL_AT as u64) as usize;
    let req = Request {
        method: req.method.unwrap_or("GET").to_string(),
        target: req.path.unwrap_or("/").to_string(),
        version,
        headers: hs,
        body: Body::Mem(Vec::new()),
    };
    let body_comes = chunked || clen.unwrap_or(0) > 0;
    Head::Done(Incoming { req, framing, sink: Sink { mem: Vec::with_capacity(cap), file: None, len: 0 }, trailer: 0 }, hlen, expect && body_comes)
}

fn hset(h: V, k: &str, v: V) {
    rhash(h).m.insert(HKey(str_from(k)), v);
}

/// The connection's socket as a Ruby IO (blocking, like the sockets other Rack servers hand
/// out), for `rack.hijack`. The IO gets a descriptor of its own: the event loop closes the
/// connection's once the request is over, and the socket stays open while the app's is.
fn hijack_io(fd: i32) -> V {
    let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return raise(rt().c.io_error, &format!("rack.hijack: {}", std::io::Error::last_os_error()));
    }
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
    }
    send(rt().c.io as V, sym("new"), &[mkfix(fd as i64), str_from("r+")], QNIL)
}

/// Returns (response bytes, keep-alive, hijacked). A hijacked connection (full hijack:
/// `env["rack.hijack"].call`; partial: a `rack.hijack` response header) belongs to the app.
fn handle(app: V, mut req: Request, port: u16, peer: &str, fd: i32) -> (Vec<u8>, bool, bool) {
    let mut keep = req.version == 1;
    let env = hash_new();
    let (path, query) = match req.target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (req.target.clone(), String::new()),
    };
    hset(env, "REQUEST_METHOD", str_from(&req.method));
    hset(env, "SCRIPT_NAME", str_from(""));
    hset(env, "PATH_INFO", str_from(&path));
    hset(env, "QUERY_STRING", str_from(&query));
    hset(env, "SERVER_PROTOCOL", str_from(if req.version == 1 { "HTTP/1.1" } else { "HTTP/1.0" }));
    hset(env, "SERVER_PORT", str_from(&port.to_string()));
    hset(env, "REMOTE_ADDR", str_from(peer));
    hset(env, "rack.url_scheme", str_from("http"));
    // (a body in a temporary file is an IO on it, closed after the response, as in Puma)
    let mut input_file = QNIL;
    let input = match std::mem::replace(&mut req.body, Body::Mem(Vec::new())) {
        Body::Mem(b) => stringio_new(b),
        Body::File(f) => {
            use std::os::fd::IntoRawFd;
            input_file = send(rt().c.io as V, sym("new"), &[mkfix(f.into_raw_fd() as i64), str_from("rb")], QNIL);
            if input_file == UNDEF {
                report("rack.input");
                return (error_response(500, false), false, false);
            }
            input_file
        }
    };
    hset(env, "rack.input", input);
    hset(env, "rack.errors", const_lookup(rt().c.object, sym("STDERR")).unwrap_or(QNIL));
    hset(env, "rack.multithread", bool_v(crate::thread::max_ps() > 1 || REQUEST_THREADS.load(Ordering::Relaxed) > 1));
    hset(env, "rack.multiprocess", bool_v(MULTIPROCESS.load(Ordering::Relaxed)));
    hset(env, "rack.run_once", QFALSE);
    hset(env, "rack.hijack?", QTRUE);
    // (Arc: the proc may be freed by a collection on another OS thread)
    let hijacked = Arc::new(AtomicBool::new(false));
    let flag = hijacked.clone();
    let hijack = proc_new_native_rooted(
        Box::new(move |_, _, _| {
            if flag.load(Ordering::Acquire) {
                return rhash(env).m.get(&HKey(str_from("rack.hijack_io"))).copied().unwrap_or(QNIL);
            }
            let io = hijack_io(fd);
            if io == UNDEF {
                return UNDEF;
            }
            hset(env, "rack.hijack_io", io);
            flag.store(true, Ordering::Release);
            io
        }),
        0,
        -1,
        vec![env],
    );
    hset(env, "rack.hijack", hijack);
    let mut server_name = "localhost".to_string();
    for (name, value) in &req.headers {
        let v = str_new(value.clone());
        let lname = name.to_ascii_lowercase();
        match lname.as_str() {
            "content-type" => hset(env, "CONTENT_TYPE", v),
            "content-length" => hset(env, "CONTENT_LENGTH", v),
            _ => {
                if lname == "host" {
                    let hv = String::from_utf8_lossy(value).into_owned();
                    server_name = hv.split(':').next().unwrap_or("localhost").to_string();
                }
                if lname == "connection" {
                    let cv = String::from_utf8_lossy(value).to_ascii_lowercase();
                    if cv.contains("close") {
                        keep = false;
                    } else if cv.contains("keep-alive") {
                        keep = true;
                    }
                }
                let key = format!("HTTP_{}", name.to_ascii_uppercase().replace('-', "_"));
                hset(env, &key, v);
            }
        }
    }
    hset(env, "SERVER_NAME", str_from(&server_name));

    let resp = send(app, sym("call"), &[env], QNIL);
    if hijacked.load(Ordering::Acquire) {
        // full hijack: the response is ignored (Rack: the app wrote its own), but its body
        // is still closed, like Puma does: Rails' executor completes the request there
        // (otherwise the request keeps its share of the autoload interlock)
        pst().pending = Throw::None;
        close_body(resp);
        return (Vec::new(), false, true);
    }
    if resp == UNDEF {
        report(&format!("{} {}", req.method, req.target));
        close_input(input_file);
        return (error_response(500, keep), keep, false);
    }
    if let Some(cb) = partial_hijack(resp) {
        // partial hijack: send the status line and headers, then hand the socket to `cb`
        let mut head = format!("HTTP/1.1 {} {}\r\n", fix(rary(resp).v[0]), reason(fix(rary(resp).v[0]))).into_bytes();
        for (k, v) in crate::hash::pairs(rary(resp).v[1]) {
            let name = crate::fmt::to_s(k).unwrap_or_default();
            if name.eq_ignore_ascii_case(b"rack.hijack") {
                continue;
            }
            let values: Vec<V> = if is_ary(v) { rary(v).v.clone() } else { vec![v] };
            for val in values {
                for line in crate::fmt::to_s(val).unwrap_or_default().split(|c| *c == b'\n') {
                    head.extend_from_slice(&name);
                    head.extend_from_slice(b": ");
                    head.extend_from_slice(line);
                    head.extend_from_slice(b"\r\n");
                }
            }
        }
        head.extend_from_slice(b"\r\n");
        let io = hijack_io(fd);
        if io != UNDEF && send(io, sym("write"), &[str_new(head)], QNIL) != UNDEF && send(io, sym("flush"), &[], QNIL) != UNDEF {
            if send(cb, sym("call"), &[io], QNIL) == UNDEF {
                report(&format!("{} {} (rack.hijack)", req.method, req.target));
            }
        }
        pst().pending = Throw::None;
        close_body(resp);
        close_input(input_file);
        return (Vec::new(), false, true);
    }
    let out = match serialize(resp, keep, req.method == "HEAD") {
        Ok(bytes) => (bytes, keep, false),
        Err(msg) => {
            err_out(format!("{} {}: {}\n", req.method, req.target, msg).as_bytes());
            (error_response(500, keep), keep, false)
        }
    };
    close_input(input_file);
    out
}

/// Close a request's body file (its `rack.input`) unless the app did.
fn close_input(io: V) {
    if io != QNIL && io != UNDEF {
        let _ = send0(io, "close");
        pst().pending = Throw::None;
    }
}

/// `body.close` of a Rack response the server doesn't send (hijacked connections).
fn close_body(resp: V) {
    if resp != UNDEF && is_ary(resp) && rary(resp).v.len() == 3 {
        let body = rary(resp).v[2];
        if respond_to(body, sym("close")) {
            let _ = send0(body, "close");
            pst().pending = Throw::None;
        }
    }
}

/// The callable of a `rack.hijack` response header, if any.
fn partial_hijack(resp: V) -> Option<V> {
    if !is_ary(resp) || rary(resp).v.len() != 3 || !is_fix(rary(resp).v[0]) || !is_hash(rary(resp).v[1]) {
        return None;
    }
    crate::hash::pairs(rary(resp).v[1])
        .into_iter()
        .find(|(k, _)| crate::fmt::to_s(*k).is_ok_and(|b| b.eq_ignore_ascii_case(b"rack.hijack")))
        .map(|(_, v)| v)
}

fn error_response(code: i64, keep: bool) -> Vec<u8> {
    let body = format!("{} {}\n", code, reason(code));
    format!(
        "HTTP/1.1 {code} {}\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: {}\r\n\r\n{body}",
        reason(code),
        body.len(),
        if keep { "keep-alive" } else { "close" }
    )
    .into_bytes()
}

fn serialize(resp: V, keep: bool, head: bool) -> Result<Vec<u8>, String> {
    if !is_ary(resp) || rary(resp).v.len() != 3 {
        return Err("app must return [status, headers, body]".into());
    }
    let (status, headers, body) = (rary(resp).v[0], rary(resp).v[1], rary(resp).v[2]);
    let code = if is_fix(status) { fix(status) } else { return Err("status must be an Integer".into()) };
    // collect body chunks
    let mut chunks: Vec<u8> = Vec::new();
    if is_ary(body) {
        for x in crate::gc::Rooted::of(rary(body).v.clone()) {
            chunks.extend(crate::fmt::to_s(x).map_err(|_| "body#to_s raised".to_string())?);
        }
    } else {
        let parts = crate::misc::each_to_vec(body).map_err(|_| {
            report("body#each");
            "body#each raised".to_string()
        })?;
        for x in parts {
            chunks.extend(crate::fmt::to_s(x).map_err(|_| "body#to_s raised".to_string())?);
        }
    }
    if respond_to(body, sym("close")) && !is_ary(body) {
        let _ = send0(body, "close");
        pst().pending = Throw::None;
    }
    let mut out = format!("HTTP/1.1 {code} {}\r\n", reason(code)).into_bytes();
    let mut has_len = false;
    if is_hash(headers) {
        for (k, v) in crate::hash::pairs(headers) {
            let name = String::from_utf8_lossy(&crate::fmt::to_s(k).unwrap_or_default()).into_owned();
            if name.eq_ignore_ascii_case("content-length") {
                has_len = true;
            }
            let values: Vec<V> = if is_ary(v) { rary(v).v.clone() } else { vec![v] };
            for val in values {
                let bytes = crate::fmt::to_s(val).unwrap_or_default();
                for line in bytes.split(|c| *c == b'\n') {
                    out.extend_from_slice(name.as_bytes());
                    out.extend_from_slice(b": ");
                    out.extend_from_slice(line);
                    out.extend_from_slice(b"\r\n");
                }
            }
        }
    }
    let no_body = code == 204 || code == 304 || (100..200).contains(&code);
    if !has_len && !no_body {
        out.extend_from_slice(format!("content-length: {}\r\n", chunks.len()).as_bytes());
    }
    out.extend_from_slice(if keep { b"connection: keep-alive\r\n" } else { b"connection: close\r\n" });
    out.extend_from_slice(b"\r\n");
    if !head && !no_body {
        out.extend_from_slice(&chunks);
    }
    Ok(out)
}
