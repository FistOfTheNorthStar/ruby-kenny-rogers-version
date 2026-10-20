//! The built-in HTTP server (like Go's net/http, but for Rack apps).
//!
//! A `config.ru` compiled with Kenny calls `run app` (and optionally `use Middleware`).
//! When the program's top level finishes and an app was registered, the binary starts
//! serving HTTP/1.1. No Puma/Unicorn needed.
//!
//! v0 concurrency: one event loop per worker process, one worker per CPU core sharing the
//! listening socket (like Puma's cluster mode). Lightweight threads come with the M:N scheduler.

use crate::core::*;
use crate::kernel::{err_out, flush_out};
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};

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
        if a.first().is_some_and(|n| is_fix(*n)) && pos >= b.len() {
            return QNIL;
        }
        sio_set_pos(s, end);
        str_new(b[pos..end].to_vec())
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
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
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
    MULTIPROCESS.store(procs > 1, std::sync::atomic::Ordering::Relaxed);
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

static MULTIPROCESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run an event loop on every P (this one included); returns when this P's loop ends.
fn serve_on_all_ps(listener: std::net::TcpListener, app: V, loops: usize) -> i32 {
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

fn println_flush(s: &str) {
    crate::kernel::out(s.as_bytes());
    crate::kernel::out(b"\n");
    flush_out();
}

struct Conn {
    stream: TcpStream,
    input: Vec<u8>,
    output: Vec<u8>,
    close_after: bool,
    peer: String,
}

const LISTENER: Token = Token(0);

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
    let mut conns: HashMap<Token, Conn> = HashMap::new();
    let mut next = 1usize;
    let mut events = Events::with_capacity(1024);
    let port = std_listener.local_addr().map(|a| a.port()).unwrap_or(0);
    // the loop on P 0 tidies the heap once the server has been idle for a second
    let tidier = crate::core::pst().thr.idx == 0;
    loop {
        let mut timeout = crate::thread::idle_timeout();
        if tidier && crate::gc::idle_compaction_wanted() {
            timeout = Some(timeout.map_or(IDLE_TIDY, |t| t.min(IDLE_TIDY)));
        }
        // waiting for the network doesn't hold up a collection on another thread
        if let Err(e) = crate::gc::blocking(|| poll.poll(&mut events, timeout)) {
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            err_out(format!("kenny: poll: {e}\n").as_bytes());
            return 1;
        }
        if events.is_empty() {
            if tidier && LAST_ACTIVITY.load(std::sync::atomic::Ordering::Relaxed).elapsed_ms() >= IDLE_TIDY.as_millis() as u64 {
                crate::gc::idle_compact();
            }
        } else {
            LAST_ACTIVITY.store(Stamp::now(), std::sync::atomic::Ordering::Relaxed);
        }
        for ev in events.iter() {
            if ev.token() == LISTENER {
                loop {
                    match listener.accept() {
                        Ok((mut stream, addr)) => {
                            let t = Token(next);
                            next += 1;
                            let _ = stream.set_nodelay(true);
                            if poll.registry().register(&mut stream, t, Interest::READABLE).is_ok() {
                                conns.insert(t, Conn { stream, input: Vec::new(), output: Vec::new(), close_after: false, peer: addr.ip().to_string() });
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
                continue;
            }
            let t = ev.token();
            let mut done = false;
            if let Some(c) = conns.get_mut(&t) {
                if ev.is_readable() {
                    let mut buf = [0u8; 16384];
                    loop {
                        match c.stream.read(&mut buf) {
                            Ok(0) => {
                                done = true;
                                break;
                            }
                            Ok(n) => c.input.extend_from_slice(&buf[..n]),
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                            Err(_) => {
                                done = true;
                                break;
                            }
                        }
                    }
                    while !c.close_after {
                        match parse_request(&c.input) {
                            Parsed::Partial => break,
                            Parsed::Bad => {
                                c.output.extend_from_slice(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                                c.close_after = true;
                                break;
                            }
                            Parsed::Done(req, used) => {
                                c.input.drain(..used);
                                let (resp, keep) = handle(app, &req, port, &c.peer);
                                c.output.extend_from_slice(&resp);
                                if !keep {
                                    c.close_after = true;
                                }
                            }
                        }
                    }
                }
                if !c.output.is_empty() {
                    loop {
                        match c.stream.write(&c.output) {
                            Ok(0) => {
                                done = true;
                                break;
                            }
                            Ok(n) => {
                                c.output.drain(..n);
                                if c.output.is_empty() {
                                    break;
                                }
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                            Err(_) => {
                                done = true;
                                break;
                            }
                        }
                    }
                }
                if c.output.is_empty() && c.close_after {
                    done = true;
                }
                if !done {
                    let interest = if c.output.is_empty() { Interest::READABLE } else { Interest::READABLE | Interest::WRITABLE };
                    let _ = poll.registry().reregister(&mut c.stream, t, interest);
                }
            }
            if done {
                if let Some(mut c) = conns.remove(&t) {
                    let _ = poll.registry().deregister(&mut c.stream);
                }
            }
        }
        flush_out();
        if timeout.is_some() {
            crate::thread::pass();
            pst().pending = Throw::None;
        }
    }
}

struct Request {
    method: String,
    target: String,
    version: u8,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}
enum Parsed {
    Partial,
    Bad,
    Done(Request, usize),
}

fn parse_request(buf: &[u8]) -> Parsed {
    let mut headers = [httparse::EMPTY_HEADER; 96];
    let mut req = httparse::Request::new(&mut headers);
    let hlen = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Parsed::Partial,
        Err(_) => return Parsed::Bad,
    };
    let mut clen = 0usize;
    let mut hs = Vec::new();
    for h in req.headers.iter() {
        if h.name.eq_ignore_ascii_case("content-length") {
            clen = match std::str::from_utf8(h.value).ok().and_then(|v| v.trim().parse().ok()) {
                Some(n) => n,
                None => return Parsed::Bad,
            };
        }
        if h.name.eq_ignore_ascii_case("transfer-encoding") {
            return Parsed::Bad; // chunked request bodies: not yet
        }
        hs.push((h.name.to_string(), h.value.to_vec()));
    }
    if buf.len() < hlen + clen {
        return Parsed::Partial;
    }
    Parsed::Done(
        Request {
            method: req.method.unwrap_or("GET").to_string(),
            target: req.path.unwrap_or("/").to_string(),
            version: req.version.unwrap_or(1),
            headers: hs,
            body: buf[hlen..hlen + clen].to_vec(),
        },
        hlen + clen,
    )
}

fn hset(h: V, k: &str, v: V) {
    rhash(h).m.insert(HKey(str_from(k)), v);
}

fn handle(app: V, req: &Request, port: u16, peer: &str) -> (Vec<u8>, bool) {
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
    hset(env, "rack.input", stringio_new(req.body.clone()));
    hset(env, "rack.errors", const_lookup(rt().c.object, sym("STDERR")).unwrap_or(QNIL));
    hset(env, "rack.multithread", bool_v(crate::thread::max_ps() > 1));
    hset(env, "rack.multiprocess", bool_v(MULTIPROCESS.load(std::sync::atomic::Ordering::Relaxed)));
    hset(env, "rack.run_once", QFALSE);
    hset(env, "rack.hijack?", QFALSE);
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
    if resp == UNDEF {
        report(&format!("{} {}", req.method, req.target));
        return (error_response(500, keep), keep);
    }
    match serialize(resp, keep, req.method == "HEAD") {
        Ok(bytes) => (bytes, keep),
        Err(msg) => {
            err_out(format!("{} {}: {}\n", req.method, req.target, msg).as_bytes());
            (error_response(500, keep), keep)
        }
    }
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
