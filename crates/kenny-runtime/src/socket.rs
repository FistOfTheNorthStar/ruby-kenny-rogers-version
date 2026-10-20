//! Native primitives behind Kenny's `socket` library (lib/kenny/socket.rb defines the
//! classes): name resolution, TCP connect/listen/accept and socket addresses, on plain
//! file descriptors that the Ruby layer wraps in IO objects. Calls block the process
//! (like Kenny's other file IO, see docs/LANGUAGE.md rule 4).

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

fn socket_error(msg: &str) -> V {
    let c = const_lookup(rt().c.object, sym("SocketError")).map(|c| c as *mut Class).unwrap_or(rt().c.standard_error);
    raise(c, msg)
}

/// getaddrinfo(host, port, family, socktype, flags) -> [[family, ip, port], ...]
fn resolve(host: Option<&str>, port: Option<&str>, family: i32, socktype: i32, flags: i32) -> Result<Vec<(i32, String, u16, libc::sockaddr_storage, libc::socklen_t)>, V> {
    let h = host.map(|h| CString::new(h).unwrap_or_default());
    let p = port.map(|p| CString::new(p).unwrap_or_default());
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = family;
    hints.ai_socktype = socktype;
    hints.ai_flags = flags;
    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let rc = unsafe {
        libc::getaddrinfo(h.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), p.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), &hints, &mut res)
    };
    if rc != 0 {
        let msg = unsafe { CStr::from_ptr(libc::gai_strerror(rc)) }.to_string_lossy().into_owned();
        return Err(socket_error(&format!("getaddrinfo: {msg}")));
    }
    let mut out = Vec::new();
    let mut ai = res;
    while !ai.is_null() {
        let a = unsafe { &*ai };
        let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        unsafe { std::ptr::copy_nonoverlapping(a.ai_addr as *const u8, &mut ss as *mut _ as *mut u8, a.ai_addrlen as usize) };
        let (ip, port) = sockaddr_parts(&ss);
        out.push((a.ai_family, ip, port, ss, a.ai_addrlen));
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

/// [family, port, host, ip] like IPSocket#addr (numeric host)
fn addr_ary(ss: &libc::sockaddr_storage) -> V {
    let (ip, port) = sockaddr_parts(ss);
    ary_new(vec![str_from(family_name(ss.ss_family as i32)), mkfix(port as i64), str_from(&ip), str_from(&ip)])
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
    let consts: &[(&str, i64)] = &[
        ("AF_UNSPEC", libc::AF_UNSPEC as i64),
        ("AF_INET", libc::AF_INET as i64),
        ("AF_INET6", libc::AF_INET6 as i64),
        ("AF_UNIX", libc::AF_UNIX as i64),
        ("SOCK_STREAM", libc::SOCK_STREAM as i64),
        ("SOCK_DGRAM", libc::SOCK_DGRAM as i64),
        ("IPPROTO_TCP", libc::IPPROTO_TCP as i64),
        ("IPPROTO_UDP", libc::IPPROTO_UDP as i64),
        ("IPPROTO_IP", libc::IPPROTO_IP as i64),
        ("IPPROTO_IPV6", libc::IPPROTO_IPV6 as i64),
        ("SOL_SOCKET", libc::SOL_SOCKET as i64),
        ("SO_REUSEADDR", libc::SO_REUSEADDR as i64),
        ("SO_REUSEPORT", libc::SO_REUSEPORT as i64),
        ("SO_KEEPALIVE", libc::SO_KEEPALIVE as i64),
        ("SO_LINGER", libc::SO_LINGER as i64),
        ("SO_RCVBUF", libc::SO_RCVBUF as i64),
        ("SO_SNDBUF", libc::SO_SNDBUF as i64),
        ("TCP_NODELAY", libc::TCP_NODELAY as i64),
        ("AI_PASSIVE", libc::AI_PASSIVE as i64),
        ("AI_CANONNAME", libc::AI_CANONNAME as i64),
        ("AI_NUMERICHOST", libc::AI_NUMERICHOST as i64),
        ("NI_NUMERICHOST", libc::NI_NUMERICHOST as i64),
        ("SOMAXCONN", libc::SOMAXCONN as i64),
        ("SHUT_RD", libc::SHUT_RD as i64),
        ("SHUT_WR", libc::SHUT_WR as i64),
        ("SHUT_RDWR", libc::SHUT_RDWR as i64),
        ("MSG_PEEK", libc::MSG_PEEK as i64),
    ];
    let table = hash_new();
    for (n, v) in consts {
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
    // getaddrinfo(host, port, family, socktype, flags) -> [[family_name, port, ip, ip, family, socktype, proto]]
    defs(k, "getaddrinfo", 2, 5, |_, a, _| {
        let host = s_of(a[0]);
        let port = s_of(a[1]);
        let num = |i: usize| a.get(i).filter(|v| is_fix(**v)).map(|v| fix(*v) as i32).unwrap_or(0);
        match resolve(host.as_deref(), port.as_deref(), num(2), num(3), num(4)) {
            Ok(v) => ary_new(
                v.into_iter()
                    .map(|(f, ip, p, _, _)| {
                        ary_new(vec![str_from(family_name(f)), mkfix(p as i64), str_from(&ip), str_from(&ip), mkfix(f as i64), mkfix(num(3).max(libc::SOCK_STREAM) as i64), mkfix(libc::IPPROTO_TCP as i64)])
                    })
                    .collect(),
            ),
            Err(e) => e,
        }
    });
    // tcp_connect(host, port) -> fd
    defs(k, "tcp_connect", 2, 2, |_, a, _| {
        let host = s_of(a[0]).unwrap_or_else(|| "localhost".into());
        let port = s_of(a[1]);
        let addrs = match resolve(Some(&host), port.as_deref(), libc::AF_UNSPEC, libc::SOCK_STREAM, 0) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mut last = libc::ECONNREFUSED;
        for (f, _, _, ss, len) in addrs {
            let fd = unsafe { libc::socket(f, libc::SOCK_STREAM, 0) };
            if fd < 0 {
                last = errno();
                continue;
            }
            if unsafe { libc::connect(fd, &ss as *const _ as *const libc::sockaddr, len) } == 0 {
                return mkfix(fd as i64);
            }
            last = errno();
            unsafe { libc::close(fd) };
        }
        crate::io::raise_errno(last, Some("connect(2)"), format!("for \"{host}\" port {}", port.unwrap_or_default()).as_bytes())
    });
    // tcp_listen(host, port, backlog) -> fd
    defs(k, "tcp_listen", 3, 3, |_, a, _| {
        let host = s_of(a[0]);
        let host = host.filter(|h| !h.is_empty()).unwrap_or_else(|| "0.0.0.0".into());
        let port = s_of(a[1]).unwrap_or_else(|| "0".into());
        let backlog = if is_fix(a[2]) { fix(a[2]) as i32 } else { libc::SOMAXCONN };
        let addrs = match resolve(Some(&host), Some(&port), libc::AF_UNSPEC, libc::SOCK_STREAM, libc::AI_PASSIVE) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let mut last = libc::EADDRNOTAVAIL;
        for (f, _, _, ss, len) in addrs {
            let fd = unsafe { libc::socket(f, libc::SOCK_STREAM, 0) };
            if fd < 0 {
                last = errno();
                continue;
            }
            let one: libc::c_int = 1;
            unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &one as *const _ as *const libc::c_void, 4) };
            if unsafe { libc::bind(fd, &ss as *const _ as *const libc::sockaddr, len) } == 0 && unsafe { libc::listen(fd, backlog) } == 0 {
                return mkfix(fd as i64);
            }
            last = errno();
            unsafe { libc::close(fd) };
        }
        crate::io::raise_errno(last, Some("bind(2)"), format!("for \"{host}\" port {port}").as_bytes())
    });
    defs(k, "accept", 1, 1, |_, a, _| {
        let fd = fix(a[0]) as i32;
        loop {
            let c = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
            if c >= 0 {
                return mkfix(c as i64);
            }
            let e = errno();
            if e != libc::EINTR {
                return crate::io::raise_errno(e, Some("accept(2)"), b"");
            }
        }
    });
    fn name_of(fd: i32, peer: bool) -> V {
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
            return crate::io::raise_errno(errno(), Some(if peer { "getpeername(2)" } else { "getsockname(2)" }), b"");
        }
        addr_ary(&ss)
    }
    defs(k, "sockname", 1, 1, |_, a, _| name_of(fix(a[0]) as i32, false));
    defs(k, "peername", 1, 1, |_, a, _| name_of(fix(a[0]) as i32, true));
    defs(k, "setsockopt", 4, 4, |_, a, _| {
        let v: libc::c_int = if a[3] == QTRUE {
            1
        } else if is_fix(a[3]) {
            fix(a[3]) as i32
        } else {
            0
        };
        let rc = unsafe { libc::setsockopt(fix(a[0]) as i32, fix(a[1]) as i32, fix(a[2]) as i32, &v as *const _ as *const libc::c_void, 4) };
        if rc != 0 {
            return crate::io::raise_errno(errno(), Some("setsockopt(2)"), b"");
        }
        mkfix(0)
    });
    defs(k, "shutdown", 2, 2, |_, a, _| {
        unsafe { libc::shutdown(fix(a[0]) as i32, fix(a[1]) as i32) };
        mkfix(0)
    });
}

pub fn init() {
    crate::compat::provide_feature(&["kenny/socket_native"], Some(init_socket));
}
