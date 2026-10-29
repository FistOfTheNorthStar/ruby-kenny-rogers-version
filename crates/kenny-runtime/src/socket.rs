//! Native primitives behind Kenny's `socket` library (lib/kenny/socket.rb defines the
//! classes): name resolution, socket(2)/connect(2)/bind(2)/listen(2)/accept(2) and friends
//! on plain file descriptors that the Ruby layer wraps in IO objects, and socket addresses
//! as CRuby passes them around: the bytes of a `struct sockaddr_*` in a binary String.
//! A thread waiting to connect or accept parks until the fd is ready (docs/LANGUAGE.md
//! rule 4).

use crate::core::*;
use std::ffi::{CStr, CString};

fn s_of(v: V) -> Option<String> {
    if v == QNIL {
        return None;
    }
    if is_str(v) {
        return Some(String::from_utf8_lossy(&rstr(v).b).into_owned());
    }
    if is_fix(v) {
        return Some(fix(v).to_string());
    }
    Some(String::from_utf8_lossy(&rstr(send0(v, "to_s")).b).into_owned())
}

fn int_of(v: V) -> Result<i32, V> {
    if is_fix(v) {
        return Ok(fix(v) as i32);
    }
    if v == QTRUE {
        return Ok(1);
    }
    if v == QFALSE || v == QNIL {
        return Ok(0);
    }
    Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v)))))
}

fn bytes_of(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.to_vec());
    }
    Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
}

fn bin(b: &[u8]) -> V {
    let v = str_new(b.to_vec());
    hdr(v).flags |= crate::compat::FL_BINARY;
    v
}

/// Socket::ResolutionError (a SocketError) for a getaddrinfo(3) failure.
fn resolution_error(rc: i32) -> V {
    let msg = unsafe { CStr::from_ptr(libc::gai_strerror(rc)) }.to_string_lossy().into_owned();
    let socket = const_lookup(rt().c.object, sym("Socket")).map(|c| c as *mut Class);
    let cls = socket
        .and_then(|s| const_get_under(s, sym("ResolutionError")))
        .map(|c| c as *mut Class)
        .or_else(|| const_lookup(rt().c.object, sym("SocketError")).map(|c| c as *mut Class))
        .unwrap_or(rt().c.standard_error);
    let e = exc_new(cls, &format!("getaddrinfo: {msg}"));
    ivar_set(e, sym("@error_code"), mkfix(rc as i64));
    raise_exc(e)
}

struct Ai {
    family: i32,
    socktype: i32,
    protocol: i32,
    ss: libc::sockaddr_storage,
    len: libc::socklen_t,
    canonname: Option<String>,
}

/// getaddrinfo(3)
fn resolve(host: Option<&str>, port: Option<&str>, family: i32, socktype: i32, protocol: i32, flags: i32) -> Result<Vec<Ai>, V> {
    let h = host.map(|h| CString::new(h).unwrap_or_default());
    let p = port.map(|p| CString::new(p).unwrap_or_default());
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = family;
    hints.ai_socktype = socktype;
    hints.ai_protocol = protocol;
    hints.ai_flags = flags;
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let rc = unsafe {
        libc::getaddrinfo(h.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), p.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), &hints, &mut res)
    };
    if rc != 0 {
        return Err(resolution_error(rc));
    }
    let mut out = Vec::new();
    let mut ai = res;
    while !ai.is_null() {
        let a = unsafe { &*ai };
        let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let len = (a.ai_addrlen as usize).min(std::mem::size_of::<libc::sockaddr_storage>());
        unsafe { std::ptr::copy_nonoverlapping(a.ai_addr as *const u8, &mut ss as *mut _ as *mut u8, len) };
        let canonname = (!a.ai_canonname.is_null()).then(|| unsafe { CStr::from_ptr(a.ai_canonname) }.to_string_lossy().into_owned());
        out.push(Ai { family: a.ai_family, socktype: a.ai_socktype, protocol: a.ai_protocol, ss, len: len as libc::socklen_t, canonname });
        ai = a.ai_next;
    }
    unsafe { libc::freeaddrinfo(res) };
    Ok(out)
}

fn sockaddr_parts(ss: &libc::sockaddr_storage) -> (String, u16) {
    match ss.ss_family as i32 {
        libc::AF_INET => {
            let a = unsafe { &*(ss as *const _ as *const libc::sockaddr_in) };
            (std::net::Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)).to_string(), u16::from_be(a.sin_port))
        }
        libc::AF_INET6 => {
            let a = unsafe { &*(ss as *const _ as *const libc::sockaddr_in6) };
            (std::net::Ipv6Addr::from(a.sin6_addr.s6_addr).to_string(), u16::from_be(a.sin6_port))
        }
        _ => (String::new(), 0),
    }
}

/// The path in a sockaddr_un of `len` bytes ("" for an unnamed socket).
fn unix_path(ss: &libc::sockaddr_storage, len: usize) -> Vec<u8> {
    let off = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    if len <= off {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(ss as *const _ as *const u8, std::mem::size_of::<libc::sockaddr_storage>()) };
    let p = &raw[off..len.min(raw.len())];
    let n = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    p[..n].to_vec()
}

/// A sockaddr String's bytes as a sockaddr_storage.
fn sa_of(v: V) -> Result<(libc::sockaddr_storage, libc::socklen_t), V> {
    let b = bytes_of(v)?;
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let n = b.len().min(std::mem::size_of::<libc::sockaddr_storage>());
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), &mut ss as *mut _ as *mut u8, n) };
    Ok((ss, n as libc::socklen_t))
}

fn sa_bytes(ss: &libc::sockaddr_storage, len: libc::socklen_t) -> V {
    let len = (len as usize).min(std::mem::size_of::<libc::sockaddr_storage>());
    bin(unsafe { std::slice::from_raw_parts(ss as *const _ as *const u8, len) })
}

/// How CRuby shows a socket address (Addrinfo#inspect_sockaddr, error messages):
/// "127.0.0.1:80", "[::1]:80", "/tmp/sock".
fn inspect_sa(ss: &libc::sockaddr_storage, len: usize) -> String {
    match ss.ss_family as i32 {
        libc::AF_INET => {
            let (ip, port) = sockaddr_parts(ss);
            if port == 0 { ip } else { format!("{ip}:{port}") }
        }
        libc::AF_INET6 => {
            let (ip, port) = sockaddr_parts(ss);
            if port == 0 { ip } else { format!("[{ip}]:{port}") }
        }
        libc::AF_UNIX => {
            let p = unix_path(ss, len);
            if p.is_empty() {
                "empty-path-AF_UNIX-sockaddr".into()
            } else if p.iter().all(|c| c.is_ascii_graphic()) {
                let s = String::from_utf8_lossy(&p).into_owned();
                if p[0] == b'/' { s } else { format!("UNIX {s}") }
            } else {
                let mut s = String::from("UNIX");
                for c in p {
                    s.push_str(&format!(":{c:02x}"));
                }
                s
            }
        }
        f => format!("unknown address family {f}"),
    }
}

fn family_name(f: i32) -> &'static str {
    match f {
        libc::AF_INET => "AF_INET",
        libc::AF_INET6 => "AF_INET6",
        libc::AF_UNIX => "AF_UNIX",
        _ => "AF_UNSPEC",
    }
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn sys_fail(e: i32, what: &str) -> V {
    crate::io::raise_errno(e, None, what.as_bytes())
}

/// [family, port, host, ip] like IPSocket#addr (numeric host)
fn addr_ary(ss: &libc::sockaddr_storage) -> V {
    let (ip, port) = sockaddr_parts(ss);
    ary_new(vec![str_from(family_name(ss.ss_family as i32)), mkfix(port as i64), str_from(&ip), str_from(&ip)])
}

fn set_cloexec(fd: i32) {
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
}

/// Runs `f` with O_NONBLOCK set on `fd`, then puts the flags back (Kenny's sockets are
/// blocking descriptors; waits park the thread instead).
fn nonblocking<T>(fd: i32, f: impl FnOnce() -> T) -> T {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl >= 0 && fl & libc::O_NONBLOCK == 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) };
    }
    let r = f();
    if fl >= 0 && fl & libc::O_NONBLOCK == 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, fl) };
    }
    r
}

/// connect(2) on `fd`, parking the thread until the connection is made; Err(errno).
fn connect_parked(fd: i32, ss: &libc::sockaddr_storage, len: libc::socklen_t) -> Result<(), Result<i32, V>> {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    unsafe { libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) };
    let restore = || unsafe { libc::fcntl(fd, libc::F_SETFL, fl) };
    let rc = unsafe { libc::connect(fd, ss as *const _ as *const libc::sockaddr, len) };
    if rc == 0 {
        restore();
        return Ok(());
    }
    let e = errno();
    if !matches!(e, libc::EINPROGRESS | libc::EINTR) {
        restore();
        return Err(Ok(e));
    }
    if let Err(x) = crate::io::wait_fd(fd, libc::POLLOUT) {
        restore();
        return Err(Err(x));
    }
    let mut err: libc::c_int = 0;
    let mut elen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut err as *mut _ as *mut libc::c_void, &mut elen) };
    restore();
    if err == 0 { Ok(()) } else { Err(Ok(err)) }
}

fn fd_arg(v: V) -> Result<i32, V> {
    int_of(v)
}

macro_rules! okv {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

fn init_socket() {
    let k = new_module("Kenny::SocketNative");
    let kenny = match const_lookup(rt().c.object, sym("Kenny")) {
        Some(m) => m as *mut Class,
        None => {
            let m = new_module("Kenny");
            set_global_const("Kenny", m as V);
            m
        }
    };
    const_set(kenny, sym("SocketNative"), k as V);
    let mut consts: Vec<(&str, i64)> = vec![
        ("AF_UNSPEC", libc::AF_UNSPEC as i64),
        ("AF_INET", libc::AF_INET as i64),
        ("AF_INET6", libc::AF_INET6 as i64),
        ("AF_UNIX", libc::AF_UNIX as i64),
        ("AF_LOCAL", libc::AF_LOCAL as i64),
        ("SOCK_STREAM", libc::SOCK_STREAM as i64),
        ("SOCK_DGRAM", libc::SOCK_DGRAM as i64),
        ("SOCK_RAW", libc::SOCK_RAW as i64),
        ("SOCK_SEQPACKET", libc::SOCK_SEQPACKET as i64),
        ("IPPROTO_IP", libc::IPPROTO_IP as i64),
        ("IPPROTO_ICMP", libc::IPPROTO_ICMP as i64),
        ("IPPROTO_TCP", libc::IPPROTO_TCP as i64),
        ("IPPROTO_UDP", libc::IPPROTO_UDP as i64),
        ("IPPROTO_IPV6", libc::IPPROTO_IPV6 as i64),
        ("IPPROTO_RAW", libc::IPPROTO_RAW as i64),
        ("SOL_SOCKET", libc::SOL_SOCKET as i64),
        ("SO_DEBUG", libc::SO_DEBUG as i64),
        ("SO_REUSEADDR", libc::SO_REUSEADDR as i64),
        ("SO_REUSEPORT", libc::SO_REUSEPORT as i64),
        ("SO_TYPE", libc::SO_TYPE as i64),
        ("SO_ERROR", libc::SO_ERROR as i64),
        ("SO_DONTROUTE", libc::SO_DONTROUTE as i64),
        ("SO_BROADCAST", libc::SO_BROADCAST as i64),
        ("SO_SNDBUF", libc::SO_SNDBUF as i64),
        ("SO_RCVBUF", libc::SO_RCVBUF as i64),
        ("SO_KEEPALIVE", libc::SO_KEEPALIVE as i64),
        ("SO_OOBINLINE", libc::SO_OOBINLINE as i64),
        ("SO_LINGER", libc::SO_LINGER as i64),
        ("SO_RCVLOWAT", libc::SO_RCVLOWAT as i64),
        ("SO_SNDLOWAT", libc::SO_SNDLOWAT as i64),
        ("SO_RCVTIMEO", libc::SO_RCVTIMEO as i64),
        ("SO_SNDTIMEO", libc::SO_SNDTIMEO as i64),
        ("SO_ACCEPTCONN", libc::SO_ACCEPTCONN as i64),
        ("TCP_NODELAY", libc::TCP_NODELAY as i64),
        ("TCP_MAXSEG", libc::TCP_MAXSEG as i64),
        ("TCP_KEEPINTVL", libc::TCP_KEEPINTVL as i64),
        ("TCP_KEEPCNT", libc::TCP_KEEPCNT as i64),
        ("IP_TOS", libc::IP_TOS as i64),
        ("IP_TTL", libc::IP_TTL as i64),
        ("IP_MULTICAST_IF", libc::IP_MULTICAST_IF as i64),
        ("IP_MULTICAST_TTL", libc::IP_MULTICAST_TTL as i64),
        ("IP_MULTICAST_LOOP", libc::IP_MULTICAST_LOOP as i64),
        ("IP_ADD_MEMBERSHIP", libc::IP_ADD_MEMBERSHIP as i64),
        ("IP_DROP_MEMBERSHIP", libc::IP_DROP_MEMBERSHIP as i64),
        ("IPV6_V6ONLY", libc::IPV6_V6ONLY as i64),
        ("IPV6_UNICAST_HOPS", libc::IPV6_UNICAST_HOPS as i64),
        ("IPV6_MULTICAST_IF", libc::IPV6_MULTICAST_IF as i64),
        ("IPV6_MULTICAST_HOPS", libc::IPV6_MULTICAST_HOPS as i64),
        ("IPV6_MULTICAST_LOOP", libc::IPV6_MULTICAST_LOOP as i64),
        ("AI_PASSIVE", libc::AI_PASSIVE as i64),
        ("AI_CANONNAME", libc::AI_CANONNAME as i64),
        ("AI_NUMERICHOST", libc::AI_NUMERICHOST as i64),
        ("AI_NUMERICSERV", libc::AI_NUMERICSERV as i64),
        ("AI_ADDRCONFIG", libc::AI_ADDRCONFIG as i64),
        ("AI_V4MAPPED", libc::AI_V4MAPPED as i64),
        ("AI_ALL", libc::AI_ALL as i64),
        ("NI_NUMERICHOST", libc::NI_NUMERICHOST as i64),
        ("NI_NUMERICSERV", libc::NI_NUMERICSERV as i64),
        ("NI_NOFQDN", libc::NI_NOFQDN as i64),
        ("NI_NAMEREQD", libc::NI_NAMEREQD as i64),
        ("NI_DGRAM", libc::NI_DGRAM as i64),
        ("NI_MAXHOST", 1025),
        ("NI_MAXSERV", 32),
        ("EAI_AGAIN", libc::EAI_AGAIN as i64),
        ("EAI_BADFLAGS", libc::EAI_BADFLAGS as i64),
        ("EAI_FAIL", libc::EAI_FAIL as i64),
        ("EAI_FAMILY", libc::EAI_FAMILY as i64),
        ("EAI_MEMORY", libc::EAI_MEMORY as i64),
        ("EAI_NONAME", libc::EAI_NONAME as i64),
        ("EAI_SERVICE", libc::EAI_SERVICE as i64),
        ("EAI_SOCKTYPE", libc::EAI_SOCKTYPE as i64),
        ("EAI_SYSTEM", libc::EAI_SYSTEM as i64),
        ("SOMAXCONN", libc::SOMAXCONN as i64),
        ("SHUT_RD", libc::SHUT_RD as i64),
        ("SHUT_WR", libc::SHUT_WR as i64),
        ("SHUT_RDWR", libc::SHUT_RDWR as i64),
        ("MSG_OOB", libc::MSG_OOB as i64),
        ("MSG_PEEK", libc::MSG_PEEK as i64),
        ("MSG_DONTROUTE", libc::MSG_DONTROUTE as i64),
        ("MSG_WAITALL", libc::MSG_WAITALL as i64),
        ("MSG_DONTWAIT", libc::MSG_DONTWAIT as i64),
        ("MSG_TRUNC", libc::MSG_TRUNC as i64),
        ("MSG_CTRUNC", libc::MSG_CTRUNC as i64),
        ("MSG_EOR", libc::MSG_EOR as i64),
        ("INADDR_ANY", 0),
        ("INADDR_LOOPBACK", 0x7f00_0001),
        ("INADDR_BROADCAST", 0xffff_ffff),
        ("INADDR_NONE", 0xffff_ffff),
        ("SCM_RIGHTS", libc::SCM_RIGHTS as i64),
    ];
    #[cfg(target_os = "linux")]
    consts.extend_from_slice(&[
        ("SOL_TCP", libc::SOL_TCP as i64),
        ("SOL_IP", libc::SOL_IP as i64),
        ("SOL_UDP", libc::SOL_UDP as i64),
        ("TCP_KEEPIDLE", libc::TCP_KEEPIDLE as i64),
        ("TCP_CORK", libc::TCP_CORK as i64),
        ("TCP_QUICKACK", libc::TCP_QUICKACK as i64),
        ("TCP_USER_TIMEOUT", libc::TCP_USER_TIMEOUT as i64),
        ("TCP_DEFER_ACCEPT", libc::TCP_DEFER_ACCEPT as i64),
        ("TCP_FASTOPEN", libc::TCP_FASTOPEN as i64),
        ("TCP_INFO", libc::TCP_INFO as i64),
        ("MSG_NOSIGNAL", libc::MSG_NOSIGNAL as i64),
        ("SO_PEERCRED", libc::SO_PEERCRED as i64),
        ("SO_PRIORITY", libc::SO_PRIORITY as i64),
        ("SO_BINDTODEVICE", libc::SO_BINDTODEVICE as i64),
        ("AF_PACKET", libc::AF_PACKET as i64),
    ]);
    #[cfg(target_vendor = "apple")]
    consts.extend_from_slice(&[
        ("TCP_KEEPALIVE", libc::TCP_KEEPALIVE as i64),
        ("TCP_NOPUSH", libc::TCP_NOPUSH as i64),
        ("SO_NOSIGPIPE", libc::SO_NOSIGPIPE as i64),
        ("SO_USELOOPBACK", libc::SO_USELOOPBACK as i64),
        ("SO_TIMESTAMP", libc::SO_TIMESTAMP as i64),
        ("LOCAL_PEERCRED", libc::LOCAL_PEERCRED as i64),
    ]);
    let table = hash_new();
    for (n, v) in &consts {
        crate::hash::hset(table, str_from(n), mkfix(*v));
    }
    const_set(k, sym("CONSTANTS"), table);

    defs(k, "gethostname", 0, 0, |_, _, _| {
        let mut buf = [0u8; 256];
        if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } != 0 {
            return crate::io::raise_errno(errno(), Some("gethostname"), b"");
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        str_new(buf[..n].to_vec())
    });
    // getaddrinfo(host, port, family, socktype, protocol, flags)
    //   -> [[family_name, port, ip, ip, family, socktype, protocol, sockaddr, canonname], ...]
    defs(k, "getaddrinfo", 2, 6, |_, a, _| {
        let host = s_of(a[0]);
        let port = s_of(a[1]);
        let num = |i: usize| a.get(i).filter(|v| is_fix(**v)).map(|v| fix(*v) as i32).unwrap_or(0);
        let v = okv!(resolve(host.as_deref(), port.as_deref(), num(2), num(3), num(4), num(5)));
        ary_new(
            v.into_iter()
                .map(|ai| {
                    let (ip, p) = sockaddr_parts(&ai.ss);
                    ary_new(vec![
                        str_from(family_name(ai.family)),
                        mkfix(p as i64),
                        str_from(&ip),
                        str_from(&ip),
                        mkfix(ai.family as i64),
                        mkfix(ai.socktype as i64),
                        mkfix(ai.protocol as i64),
                        sa_bytes(&ai.ss, ai.len),
                        ai.canonname.as_deref().map_or(QNIL, str_from),
                    ])
                })
                .collect(),
        )
    });
    // tcp_connect(host, port) -> fd
    defs(k, "tcp_connect", 2, 2, |_, a, _| {
        let host = s_of(a[0]).unwrap_or_else(|| "localhost".into());
        let port = s_of(a[1]);
        let addrs = okv!(resolve(Some(&host), port.as_deref(), libc::AF_UNSPEC, libc::SOCK_STREAM, 0, 0));
        let mut last = libc::ECONNREFUSED;
        for ai in addrs {
            let fd = unsafe { libc::socket(ai.family, libc::SOCK_STREAM, 0) };
            if fd < 0 {
                last = errno();
                continue;
            }
            set_cloexec(fd);
            // (non-blocking, so other threads run while the connection is made)
            match connect_parked(fd, &ai.ss, ai.len) {
                Ok(()) => return mkfix(fd as i64),
                Err(Ok(e)) => last = e,
                Err(Err(x)) => {
                    unsafe { libc::close(fd) };
                    return x;
                }
            }
            unsafe { libc::close(fd) };
        }
        sys_fail(last, &format!("connect(2) for \"{host}\" port {}", port.unwrap_or_default()))
    });
    // tcp_listen(host, port, backlog) -> fd
    defs(k, "tcp_listen", 3, 3, |_, a, _| {
        let host = s_of(a[0]);
        let host = host.filter(|h| !h.is_empty()).unwrap_or_else(|| "0.0.0.0".into());
        let port = s_of(a[1]).unwrap_or_else(|| "0".into());
        let backlog = if is_fix(a[2]) { fix(a[2]) as i32 } else { libc::SOMAXCONN };
        let addrs = okv!(resolve(Some(&host), Some(&port), libc::AF_UNSPEC, libc::SOCK_STREAM, 0, libc::AI_PASSIVE));
        let mut last = libc::EADDRNOTAVAIL;
        for ai in addrs {
            let fd = unsafe { libc::socket(ai.family, libc::SOCK_STREAM, 0) };
            if fd < 0 {
                last = errno();
                continue;
            }
            set_cloexec(fd);
            let one: libc::c_int = 1;
            unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &one as *const _ as *const libc::c_void, 4) };
            if unsafe { libc::bind(fd, &ai.ss as *const _ as *const libc::sockaddr, ai.len) } == 0 && unsafe { libc::listen(fd, backlog) } == 0 {
                return mkfix(fd as i64);
            }
            last = errno();
            unsafe { libc::close(fd) };
        }
        sys_fail(last, &format!("bind(2) for \"{host}\" port {port}"))
    });
    // socket(domain, type, protocol) -> fd
    defs(k, "socket", 3, 3, |_, a, _| {
        let fd = unsafe { libc::socket(okv!(int_of(a[0])), okv!(int_of(a[1])), okv!(int_of(a[2]))) };
        if fd < 0 {
            return sys_fail(errno(), "socket(2)");
        }
        set_cloexec(fd);
        mkfix(fd as i64)
    });
    // socketpair(domain, type, protocol) -> [fd, fd]
    defs(k, "socketpair", 3, 3, |_, a, _| {
        let mut sv = [0i32; 2];
        if unsafe { libc::socketpair(okv!(int_of(a[0])), okv!(int_of(a[1])), okv!(int_of(a[2])), sv.as_mut_ptr()) } != 0 {
            return sys_fail(errno(), "socketpair(2)");
        }
        set_cloexec(sv[0]);
        set_cloexec(sv[1]);
        ary_new(vec![mkfix(sv[0] as i64), mkfix(sv[1] as i64)])
    });
    // connect(fd, sockaddr, nonblock, exception) -> 0 (or :wait_writable)
    // (an optional 5th argument is the error message's label: UNIXSocket names the path)
    defs(k, "connect", 4, 5, |_, a, _| {
        let fd = okv!(fd_arg(a[0]));
        let (ss, len) = okv!(sa_of(a[1]));
        let label = a.get(4).copied().filter(|v| is_str(*v)).map(|v| String::from_utf8_lossy(&rstr(v).b).into_owned());
        let what = || label.clone().unwrap_or_else(|| format!("connect(2) for {}", inspect_sa(&ss, len as usize)));
        if truthy(a[2]) {
            let (rc, e) = nonblocking(fd, || {
                let rc = unsafe { libc::connect(fd, &ss as *const _ as *const libc::sockaddr, len) };
                (rc, errno())
            });
            if rc == 0 {
                return mkfix(0);
            }
            let exc = truthy(a[3]);
            return match e {
                libc::EINPROGRESS if !exc => mksym(sym("wait_writable")),
                libc::EINPROGRESS => crate::io::raise_would_block(e, true, "connect(2)"),
                libc::EISCONN if !exc => mkfix(0),
                _ => sys_fail(e, &what()),
            };
        }
        match connect_parked(fd, &ss, len) {
            Ok(()) => mkfix(0),
            Err(Ok(e)) => sys_fail(e, &what()),
            Err(Err(x)) => x,
        }
    });
    defs(k, "bind", 2, 3, |_, a, _| {
        let fd = okv!(fd_arg(a[0]));
        let (ss, len) = okv!(sa_of(a[1]));
        if unsafe { libc::bind(fd, &ss as *const _ as *const libc::sockaddr, len) } != 0 {
            let e = errno();
            if let Some(l) = a.get(2).copied().filter(|v| is_str(*v)) {
                return sys_fail(e, &String::from_utf8_lossy(&rstr(l).b));
            }
            return sys_fail(e, &format!("bind(2) for {}", inspect_sa(&ss, len as usize)));
        }
        mkfix(0)
    });
    defs(k, "listen", 2, 2, |_, a, _| {
        if unsafe { libc::listen(okv!(fd_arg(a[0])), okv!(int_of(a[1]))) } != 0 {
            return sys_fail(errno(), "listen(2)");
        }
        mkfix(0)
    });
    // accept(fd) -> fd (TCPServer#accept)
    defs(k, "accept", 1, 1, |_, a, _| {
        let fd = okv!(fd_arg(a[0]));
        match accept_parked(fd) {
            Ok((c, _, _)) => mkfix(c as i64),
            Err(e) => e,
        }
    });
    // accept2(fd, nonblock, exception) -> [fd, sockaddr] (or :wait_readable)
    defs(k, "accept2", 3, 3, |_, a, _| {
        let fd = okv!(fd_arg(a[0]));
        if truthy(a[1]) {
            let (c, e, ss, len) = nonblocking(fd, || {
                let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
                let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
                let c = unsafe { libc::accept(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len) };
                (c, errno(), ss, len)
            });
            if c < 0 {
                if matches!(e, libc::EAGAIN | libc::ECONNABORTED | libc::EPROTO | libc::EINTR) {
                    if !truthy(a[2]) {
                        return mksym(sym("wait_readable"));
                    }
                    return crate::io::raise_would_block(libc::EAGAIN, false, "accept(2)");
                }
                return sys_fail(e, "accept(2)");
            }
            accepted(c);
            return ary_new(vec![mkfix(c as i64), sa_bytes(&ss, len)]);
        }
        match accept_parked(fd) {
            Ok((c, ss, len)) => ary_new(vec![mkfix(c as i64), sa_bytes(&ss, len)]),
            Err(e) => e,
        }
    });
    fn accepted(c: i32) {
        set_cloexec(c);
        // (BSDs hand the listener's O_NONBLOCK on to the new socket)
        let fl = unsafe { libc::fcntl(c, libc::F_GETFL) };
        if fl >= 0 && fl & libc::O_NONBLOCK != 0 {
            unsafe { libc::fcntl(c, libc::F_SETFL, fl & !libc::O_NONBLOCK) };
        }
    }
    fn accept_parked(fd: i32) -> Result<(i32, libc::sockaddr_storage, libc::socklen_t), V> {
        loop {
            // (wait for a connection parked, not in accept(2): other threads keep running)
            crate::io::wait_fd(fd, libc::POLLIN)?;
            let (c, e, ss, len) = nonblocking(fd, || {
                let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
                let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
                let c = unsafe { libc::accept(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len) };
                (c, errno(), ss, len)
            });
            if c >= 0 {
                accepted(c);
                return Ok((c, ss, len));
            }
            if !matches!(e, libc::EINTR | libc::EAGAIN | libc::ECONNABORTED) {
                return Err(sys_fail(e, "accept(2)"));
            }
        }
    }
    fn name_of(fd: i32, peer: bool) -> Result<(libc::sockaddr_storage, libc::socklen_t), V> {
        let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let rc = unsafe {
            if peer {
                libc::getpeername(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len)
            } else {
                libc::getsockname(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len)
            }
        };
        if rc != 0 {
            return Err(sys_fail(errno(), if peer { "getpeername(2)" } else { "getsockname(2)" }));
        }
        Ok((ss, len))
    }
    defs(k, "sockname", 1, 1, |_, a, _| addr_ary(&okv!(name_of(okv!(fd_arg(a[0])), false)).0));
    defs(k, "peername", 1, 1, |_, a, _| addr_ary(&okv!(name_of(okv!(fd_arg(a[0])), true)).0));
    defs(k, "getsockname", 1, 1, |_, a, _| {
        let (ss, len) = okv!(name_of(okv!(fd_arg(a[0])), false));
        sa_bytes(&ss, len)
    });
    defs(k, "getpeername", 1, 1, |_, a, _| {
        let (ss, len) = okv!(name_of(okv!(fd_arg(a[0])), true));
        sa_bytes(&ss, len)
    });
    // setsockopt(fd, level, optname, value): value an Integer, true/false, or the option's bytes
    defs(k, "setsockopt", 4, 4, |_, a, _| {
        let (fd, level, opt) = (okv!(fd_arg(a[0])), okv!(int_of(a[1])), okv!(int_of(a[2])));
        let data: Vec<u8> = if is_str(a[3]) { rstr(a[3]).b.to_vec() } else { okv!(int_of(a[3])).to_ne_bytes().to_vec() };
        let rc = unsafe { libc::setsockopt(fd, level, opt, data.as_ptr() as *const libc::c_void, data.len() as libc::socklen_t) };
        if rc != 0 {
            return sys_fail(errno(), "setsockopt(2)");
        }
        mkfix(0)
    });
    // getsockopt(fd, level, optname) -> the option's bytes
    defs(k, "getsockopt", 3, 3, |_, a, _| {
        let (fd, level, opt) = (okv!(fd_arg(a[0])), okv!(int_of(a[1])), okv!(int_of(a[2])));
        let mut buf = [0u8; 256];
        let mut len = buf.len() as libc::socklen_t;
        if unsafe { libc::getsockopt(fd, level, opt, buf.as_mut_ptr() as *mut libc::c_void, &mut len) } != 0 {
            return sys_fail(errno(), "getsockopt(2)");
        }
        bin(&buf[..(len as usize).min(buf.len())])
    });
    defs(k, "shutdown", 2, 2, |_, a, _| {
        if unsafe { libc::shutdown(okv!(fd_arg(a[0])), okv!(int_of(a[1]))) } != 0 {
            return sys_fail(errno(), "shutdown(2)");
        }
        mkfix(0)
    });
    // pack_sockaddr_in(port, host) -> sockaddr_in / sockaddr_in6 bytes
    defs(k, "pack_sockaddr_in", 2, 2, |_, a, _| {
        let port = s_of(a[0]);
        if is_fix(a[1]) {
            // (an Integer host is an IPv4 address in host byte order)
            let mut sin: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
            {
                sin.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
            }
            sin.sin_addr.s_addr = (fix(a[1]) as u32).to_be();
            let p = okv!(resolve(Some("0.0.0.0"), port.as_deref(), libc::AF_INET, 0, 0, libc::AI_NUMERICHOST));
            sin.sin_port = p.first().map(|ai| sockaddr_parts(&ai.ss).1.to_be()).unwrap_or(0);
            return bin(unsafe { std::slice::from_raw_parts(&sin as *const _ as *const u8, std::mem::size_of::<libc::sockaddr_in>()) });
        }
        let host = s_of(a[1]).map(|h| match h.as_str() {
            "" | "<any>" => "0.0.0.0".to_string(),
            "<broadcast>" => "255.255.255.255".to_string(),
            _ => h,
        });
        let v = okv!(resolve(host.as_deref(), port.as_deref(), libc::AF_UNSPEC, 0, 0, 0));
        match v.first() {
            Some(ai) => sa_bytes(&ai.ss, ai.len),
            None => resolution_error(libc::EAI_NONAME),
        }
    });
    // unpack_sockaddr_in(sockaddr) -> [port, ip]
    defs(k, "unpack_sockaddr_in", 1, 1, |_, a, _| {
        let (ss, _) = okv!(sa_of(a[0]));
        if !matches!(ss.ss_family as i32, libc::AF_INET | libc::AF_INET6) {
            return arg_error("not an AF_INET/AF_INET6 sockaddr");
        }
        let (ip, port) = sockaddr_parts(&ss);
        ary_new(vec![mkfix(port as i64), str_from(&ip)])
    });
    defs(k, "pack_sockaddr_un", 1, 1, |_, a, _| {
        let path = okv!(bytes_of(a[0]));
        let mut sun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        if path.len() > sun.sun_path.len() {
            return arg_error(&format!("too long unix socket path ({} bytes given but {} bytes max)", path.len(), sun.sun_path.len()));
        }
        sun.sun_family = libc::AF_UNIX as libc::sa_family_t;
        #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
        {
            sun.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
        }
        for (d, s) in sun.sun_path.iter_mut().zip(path.iter()) {
            *d = *s as libc::c_char;
        }
        bin(unsafe { std::slice::from_raw_parts(&sun as *const _ as *const u8, std::mem::size_of::<libc::sockaddr_un>()) })
    });
    defs(k, "unpack_sockaddr_un", 1, 1, |_, a, _| {
        let (ss, len) = okv!(sa_of(a[0]));
        if ss.ss_family as i32 != libc::AF_UNIX {
            return arg_error("not an AF_UNIX sockaddr");
        }
        str_new(unix_path(&ss, len as usize))
    });
    defs(k, "sockaddr_family", 1, 1, |_, a, _| {
        let (ss, len) = okv!(sa_of(a[0]));
        if (len as usize) < std::mem::offset_of!(libc::sockaddr, sa_data) {
            return mkfix(libc::AF_UNSPEC as i64);
        }
        mkfix(ss.ss_family as i64)
    });
    defs(k, "inspect_sockaddr", 1, 1, |_, a, _| {
        let (ss, len) = okv!(sa_of(a[0]));
        str_from(&inspect_sa(&ss, len as usize))
    });
}

pub fn init() {
    crate::compat::provide_feature(&["kenny/socket_native"], Some(init_socket));
}
