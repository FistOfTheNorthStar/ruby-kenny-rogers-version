//! `require "cgi/escape"`: CRuby's C accelerator for CGI escaping, as CGI::Escape.

use crate::core::*;

fn arg_bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        Ok(rstr(v).b.clone())
    } else {
        Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
    }
}

/// Result string keeping the argument's encoding flags (binary stays binary).
fn same_enc(src: V, b: Vec<u8>) -> V {
    let out = str_new(b);
    let flags = crate::compat::FL_BINARY | (crate::compat::FL_BINARY << 1);
    hdr(out).flags |= hdr(src).flags & flags;
    out
}

pub fn escape_html(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 16);
    for &c in b {
        match c {
            b'\'' => out.extend_from_slice(b"&#39;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape_html(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = b[i + 1..].iter().take(12).position(|&c| c == b';') {
                let ent = &b[i + 1..i + 1 + semi];
                let rep: Option<Vec<u8>> = match ent {
                    b"apos" => Some(b"'".to_vec()),
                    b"amp" => Some(b"&".to_vec()),
                    b"quot" => Some(b"\"".to_vec()),
                    b"gt" => Some(b">".to_vec()),
                    b"lt" => Some(b"<".to_vec()),
                    _ if ent.first() == Some(&b'#') => {
                        let (digits, radix) = match ent.get(1) {
                            Some(b'x') | Some(b'X') => (&ent[2..], 16),
                            _ => (&ent[1..], 10),
                        };
                        std::str::from_utf8(digits)
                            .ok()
                            .filter(|d| !d.is_empty())
                            .and_then(|d| u32::from_str_radix(d, radix).ok())
                            .and_then(char::from_u32)
                            .map(|c| c.to_string().into_bytes())
                    }
                    _ => None,
                };
                if let Some(r) = rep {
                    out.extend_from_slice(&r);
                    i += semi + 2;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// URL-encode everything except a-zA-Z0-9 and `-._~` (CRuby's unreserved set);
/// space becomes `+` unless `component`.
fn escape_url(b: &[u8], component: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() * 3);
    for &c in b {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~') {
            out.push(c);
        } else if c == b' ' && !component {
            out.push(b'+');
        } else {
            out.extend_from_slice(format!("%{c:02X}").as_bytes());
        }
    }
    out
}

fn unescape_url(b: &[u8], component: bool) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' if !component => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn init_cgi_escape() {
    let o = rt().c.object;
    let cgi = match const_lookup(o, sym("CGI")) {
        Some(c) if is_class(c) => c as *mut Class,
        _ => {
            let c = new_class("CGI", o);
            set_global_const("CGI", c as V);
            c
        }
    };
    let esc = match const_get_under(cgi, sym("Escape")) {
        Some(m) if is_class(m) => m as *mut Class,
        _ => {
            let m = new_module("CGI::Escape");
            const_set(cgi, sym("Escape"), m as V);
            m
        }
    };
    macro_rules! def {
        ($name:expr, $f:expr) => {
            defn(esc, $name, 1, 1, |_, a, _| match arg_bytes(a[0]) {
                Ok(b) => same_enc(a[0], $f(&b)),
                Err(e) => e,
            });
        };
    }
    def!("escapeHTML", escape_html);
    def!("unescapeHTML", unescape_html);
    def!("escape", |b: &[u8]| escape_url(b, false));
    def!("unescape", |b: &[u8]| unescape_url(b, false));
    def!("escapeURIComponent", |b: &[u8]| escape_url(b, true));
    def!("unescapeURIComponent", |b: &[u8]| unescape_url(b, true));
    for (alias, name) in [("escape_html", "escapeHTML"), ("h", "escapeHTML"), ("unescape_html", "unescapeHTML"), ("escape_uri_component", "escapeURIComponent"), ("unescape_uri_component", "unescapeURIComponent")] {
        let m = unsafe { *(*esc).methods.get(&sym(name)).unwrap() };
        add_method(esc, sym(alias), crate::kernel::clone_body(unsafe { &(*m).body }));
    }
    // like CRuby: CGI.escapeHTML etc. and CGI::Util picks up the fast versions
    let _ = send(cgi as V, sym("extend"), &[esc as V], QNIL);
    let util = match const_get_under(cgi, sym("Util")) {
        Some(m) if is_class(m) => m as *mut Class,
        _ => {
            let m = new_module("CGI::Util");
            const_set(cgi, sym("Util"), m as V);
            m
        }
    };
    let _ = send(util as V, sym("prepend"), &[esc as V], QNIL);
}

pub fn init() {
    crate::compat::provide_feature(&["cgi/escape"], Some(init_cgi_escape));
}
