//! Array#pack, String#unpack and String#unpack1 (CRuby pack.c semantics).
//!
//! Supported directives: a A Z B b H h C c S s L l Q q J j I i N n V v U w m M u
//! e E f F d D g G x X @, with counts, `*`, and the `_` `!` `<` `>` modifiers.

use crate::core::*;
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const UU: &[u8; 64] = b"`!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_";
const HEX_UP: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Copy, PartialEq)]
enum Count {
    None,
    Star,
    N(usize),
}

struct Dir {
    t: u8,
    native: bool,
    endian: Option<bool>, // Some(true) = big endian
    count: Count,
}

fn parse(fmt: &[u8]) -> Result<Vec<Dir>, V> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        let t = fmt[i];
        i += 1;
        if t.is_ascii_whitespace() {
            continue;
        }
        if t == b'#' {
            while i < fmt.len() && fmt[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let mut d = Dir { t, native: false, endian: None, count: Count::None };
        while i < fmt.len() && matches!(fmt[i], b'_' | b'!' | b'<' | b'>') {
            let m = fmt[i];
            if !b"sSiIlLqQjJ".contains(&t) {
                return Err(arg_error(&format!("'{}' allowed only after types sSiIlLqQjJ", m as char)));
            }
            match m {
                b'<' => d.endian = Some(false),
                b'>' => d.endian = Some(true),
                _ => d.native = true,
            }
            i += 1;
        }
        if i < fmt.len() && fmt[i] == b'*' {
            d.count = Count::Star;
            i += 1;
        } else if i < fmt.len() && fmt[i].is_ascii_digit() {
            let mut n: usize = 0;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                n = n.saturating_mul(10).saturating_add((fmt[i] - b'0') as usize);
                i += 1;
            }
            d.count = Count::N(n);
        }
        out.push(d);
    }
    Ok(out)
}

/// (size in bytes, signed, big endian) for an integer directive.
fn int_spec(d: &Dir) -> Option<(usize, bool, bool)> {
    let native_be = cfg!(target_endian = "big");
    let (size, signed, be) = match d.t {
        b'c' => (1, true, false),
        b'C' => (1, false, false),
        b's' => (2, true, native_be),
        b'S' => (2, false, native_be),
        b'i' => (4, true, native_be),
        b'I' => (4, false, native_be),
        b'l' => (if d.native { 8 } else { 4 }, true, native_be),
        b'L' => (if d.native { 8 } else { 4 }, false, native_be),
        b'q' => (8, true, native_be),
        b'Q' => (8, false, native_be),
        b'j' => (8, true, native_be),
        b'J' => (8, false, native_be),
        b'n' => (2, false, true),
        b'N' => (4, false, true),
        b'v' => (2, false, false),
        b'V' => (4, false, false),
        _ => return None,
    };
    Some((size, signed, d.endian.unwrap_or(be)))
}

fn float_spec(t: u8) -> Option<(usize, bool)> {
    let native_be = cfg!(target_endian = "big");
    Some(match t {
        b'f' | b'F' => (4, native_be),
        b'd' | b'D' => (8, native_be),
        b'e' => (4, false),
        b'E' => (8, false),
        b'g' => (4, true),
        b'G' => (8, true),
        _ => return None,
    })
}

fn no_implicit(v: V, into: &str) -> V {
    let what = if v == QNIL {
        "nil".to_string()
    } else if v == QTRUE {
        "true".to_string()
    } else if v == QFALSE {
        "false".to_string()
    } else {
        class_name(real_class(v))
    };
    type_error(&format!("no implicit conversion of {what} into {into}"))
}

/// Low 64 bits of an Integer-like value (two's complement).
fn int_bits(v: V) -> Result<u64, V> {
    if is_fix(v) {
        return Ok(fix(v) as u64);
    }
    if is_big(v) {
        let m = BigInt::from(1u128 << 64);
        let mut r = &rbig(v).n % &m;
        if r < BigInt::zero() {
            r += &m;
        }
        return Ok(r.to_u64().unwrap_or(0));
    }
    if is_float(v) {
        let f = float_val(v);
        if !f.is_finite() {
            return Err(raise(rt().c.float_domain_error, &crate::fmt::float_to_s(f)));
        }
        return Ok(f.trunc() as i128 as u64);
    }
    if is_heap(v) && respond_to(v, sym("to_int")) {
        let r = send0(v, "to_int");
        if r == UNDEF {
            return Err(UNDEF);
        }
        if is_int(r) {
            return int_bits(r);
        }
    }
    Err(no_implicit(v, "Integer"))
}

/// The string of an `a A Z B b H h` item: as in CRuby, nil packs as an empty string.
fn str_item_nil(v: V) -> Result<Vec<u8>, V> {
    if v == QNIL { Ok(Vec::new()) } else { str_item(v) }
}

fn str_item(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    if is_heap(v) && respond_to(v, sym("to_str")) {
        let r = send0(v, "to_str");
        if r == UNDEF {
            return Err(UNDEF);
        }
        if is_str(r) {
            return Ok(rstr(r).b.clone());
        }
    }
    Err(no_implicit(v, "String"))
}

fn encodes(out: &mut Vec<u8>, s: &[u8], uu: bool, tail_lf: bool) {
    let trans = if uu { UU } else { B64 };
    let pad = if uu { b'`' } else { b'=' };
    if uu {
        out.push(s.len() as u8 + b' ');
    }
    let mut i = 0;
    while s.len() - i >= 3 {
        let (a, b, c) = (s[i], s[i + 1], s[i + 2]);
        out.push(trans[(a >> 2) as usize]);
        out.push(trans[(((a << 4) & 0o60) | ((b >> 4) & 0o17)) as usize]);
        out.push(trans[(((b << 2) & 0o74) | ((c >> 6) & 0o3)) as usize]);
        out.push(trans[(c & 0o77) as usize]);
        i += 3;
    }
    match s.len() - i {
        2 => {
            let (a, b) = (s[i], s[i + 1]);
            out.push(trans[(a >> 2) as usize]);
            out.push(trans[(((a << 4) & 0o60) | ((b >> 4) & 0o17)) as usize]);
            out.push(trans[((b << 2) & 0o74) as usize]);
            out.push(pad);
        }
        1 => {
            let a = s[i];
            out.push(trans[(a >> 2) as usize]);
            out.push(trans[((a << 4) & 0o60) as usize]);
            out.push(pad);
            out.push(pad);
        }
        _ => {}
    }
    if tail_lf {
        out.push(b'\n');
    }
}

fn qpencode(out: &mut Vec<u8>, s: &[u8], len: usize) {
    let mut n = 0usize;
    let mut prev: i32 = -1;
    for &c in s {
        if c > 126 || (c < 32 && c != b'\n' && c != b'\t') || c == b'=' {
            out.push(b'=');
            out.push(HEX_UP[(c >> 4) as usize]);
            out.push(HEX_UP[(c & 15) as usize]);
            n += 3;
            prev = -1;
        } else if c == b'\n' {
            if prev == b' ' as i32 || prev == b'\t' as i32 {
                out.push(b'=');
                out.push(c);
            }
            out.push(c);
            n = 0;
            prev = c as i32;
        } else {
            out.push(c);
            n += 1;
            prev = c as i32;
        }
        if n > len {
            out.push(b'=');
            out.push(b'\n');
            n = 0;
            prev = b'\n' as i32;
        }
    }
    if n > 0 {
        out.push(b'=');
        out.push(b'\n');
    }
}

fn utf8_encode(out: &mut Vec<u8>, c: u64) -> Result<(), V> {
    if c > 0x7fff_ffff {
        return Err(raise(rt().c.range_error, "pack(U): value out of range"));
    }
    let c = c as u32;
    if let Some(ch) = char::from_u32(c) {
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        return Ok(());
    }
    // surrogates and > 0x10FFFF: encode the bit pattern anyway, like CRuby
    if c < 0x10000 {
        out.extend_from_slice(&[0xe0 | (c >> 12) as u8, 0x80 | ((c >> 6) & 0x3f) as u8, 0x80 | (c & 0x3f) as u8]);
    } else if c < 0x200000 {
        out.extend_from_slice(&[0xf0 | (c >> 18) as u8, 0x80 | ((c >> 12) & 0x3f) as u8, 0x80 | ((c >> 6) & 0x3f) as u8, 0x80 | (c & 0x3f) as u8]);
    } else {
        return Err(raise(rt().c.range_error, "pack(U): value out of range"));
    }
    Ok(())
}

pub fn pack(items: &[V], fmt: &[u8]) -> Result<Vec<u8>, V> {
    let dirs = parse(fmt)?;
    let mut out: Vec<u8> = Vec::new();
    let mut idx = 0usize;
    let too_few = || arg_error("too few arguments");
    macro_rules! next_item {
        () => {{
            if idx >= items.len() {
                return Err(too_few());
            }
            idx += 1;
            items[idx - 1]
        }};
    }
    for d in &dirs {
        let t = d.t;
        match t {
            b'a' | b'A' | b'Z' => {
                let s = str_item_nil(next_item!())?;
                let pad = if t == b'A' { b' ' } else { 0 };
                match d.count {
                    Count::Star => {
                        out.extend_from_slice(&s);
                        if t == b'Z' {
                            out.push(0);
                        }
                    }
                    c => {
                        let n = if let Count::N(n) = c { n } else { 1 };
                        if s.len() >= n {
                            out.extend_from_slice(&s[..n]);
                        } else {
                            out.extend_from_slice(&s);
                            out.extend(std::iter::repeat_n(pad, n - s.len()));
                        }
                    }
                }
            }
            b'B' | b'b' => {
                let s = str_item_nil(next_item!())?;
                let n = match d.count {
                    Count::Star => s.len(),
                    Count::None => 1,
                    Count::N(n) => n,
                };
                let take = n.min(s.len());
                let mut byte = 0u8;
                let mut bits = 0;
                for &ch in &s[..take] {
                    let bit = ch & 1;
                    if t == b'B' {
                        byte |= bit << (7 - bits);
                    } else {
                        byte |= bit << bits;
                    }
                    bits += 1;
                    if bits == 8 {
                        out.push(byte);
                        byte = 0;
                        bits = 0;
                    }
                }
                if bits > 0 {
                    out.push(byte);
                }
                // CRuby pads a short bit string with (count - len + 1) / 2 NUL bytes
                if n > s.len() {
                    out.extend(std::iter::repeat_n(0u8, (n - s.len()).div_ceil(2)));
                }
            }
            b'H' | b'h' => {
                let s = str_item_nil(next_item!())?;
                let n = match d.count {
                    Count::Star => s.len(),
                    Count::None => 1,
                    Count::N(n) => n,
                };
                let take = n.min(s.len());
                let mut byte = 0u8;
                let mut half = 0;
                for &ch in &s[..take] {
                    let nib = if ch.is_ascii_alphabetic() { ((ch & 7) + 9) & 15 } else { ch & 15 };
                    if t == b'H' {
                        byte |= if half == 0 { nib << 4 } else { nib };
                    } else {
                        byte |= if half == 0 { nib } else { nib << 4 };
                    }
                    half += 1;
                    if half == 2 {
                        out.push(byte);
                        byte = 0;
                        half = 0;
                    }
                }
                if half > 0 {
                    out.push(byte);
                }
                let want = n.div_ceil(2);
                let have = take.div_ceil(2);
                if want > have {
                    out.extend(std::iter::repeat_n(0u8, want - have));
                }
            }
            b'U' | b'w' => {
                let n = match d.count {
                    Count::Star => items.len().saturating_sub(idx),
                    Count::None => 1,
                    Count::N(n) => n,
                };
                for _ in 0..n {
                    let v = next_item!();
                    if t == b'U' {
                        let bits = int_bits(v)?;
                        if (bits as i64) < 0 {
                            return Err(raise(rt().c.range_error, "pack(U): value out of range"));
                        }
                        utf8_encode(&mut out, bits)?;
                    } else {
                        // BER-compressed integer
                        let big = if is_fix(v) {
                            BigInt::from(fix(v))
                        } else if is_big(v) {
                            rbig(v).n.clone()
                        } else if is_float(v) {
                            BigInt::from(float_val(v).trunc() as i128)
                        } else {
                            return Err(no_implicit(v, "Integer"));
                        };
                        if big < BigInt::zero() {
                            return Err(arg_error("can't compress negative numbers"));
                        }
                        let mut bytes = Vec::new();
                        let mut x = big;
                        let b128 = BigInt::from(128);
                        loop {
                            let low = (&x % &b128).to_u8().unwrap_or(0);
                            bytes.push(low);
                            x /= &b128;
                            if x.is_zero() {
                                break;
                            }
                        }
                        bytes.reverse();
                        let last = bytes.len() - 1;
                        for (i, b) in bytes.iter().enumerate() {
                            out.push(if i < last { b | 0x80 } else { *b });
                        }
                    }
                }
            }
            b'm' | b'u' => {
                let s = str_item(next_item!())?;
                let mut len = match d.count {
                    Count::Star | Count::None => 1,
                    Count::N(n) => n,
                };
                if len == 0 && t == b'm' {
                    encodes(&mut out, &s, false, false);
                    continue;
                }
                if len <= 2 {
                    len = 45;
                } else if len > 63 && t == b'u' {
                    len = 63;
                } else {
                    len = len / 3 * 3;
                }
                for chunk in s.chunks(len) {
                    encodes(&mut out, chunk, t == b'u', true);
                }
            }
            b'M' => {
                let v = next_item!();
                let s = match crate::fmt::to_s(v) {
                    Ok(b) => b,
                    Err(()) => return Err(UNDEF),
                };
                let mut len = match d.count {
                    Count::N(n) => n,
                    _ => 1,
                };
                if len <= 1 {
                    len = 72;
                }
                qpencode(&mut out, &s, len);
            }
            b'x' => {
                let n = match d.count {
                    Count::Star => 0,
                    Count::None => 1,
                    Count::N(n) => n,
                };
                out.extend(std::iter::repeat_n(0u8, n));
            }
            b'X' => {
                let n = match d.count {
                    Count::Star => 0,
                    Count::None => 1,
                    Count::N(n) => n,
                };
                if n > out.len() {
                    return Err(arg_error("X outside of string"));
                }
                out.truncate(out.len() - n);
            }
            b'@' => {
                let n = match d.count {
                    Count::Star => out.len(),
                    Count::None => 1,
                    Count::N(n) => n,
                };
                out.resize(n, 0);
            }
            b'p' | b'P' => return Err(raise(rt().c.not_implemented_error, "pack('p'/'P') (pointers) is not supported by Kenny")),
            _ => {
                if let Some((size, _signed, be)) = int_spec(d) {
                    let n = match d.count {
                        Count::Star => items.len().saturating_sub(idx),
                        Count::None => 1,
                        Count::N(n) => n,
                    };
                    for _ in 0..n {
                        let bits = int_bits(next_item!())?;
                        let le = bits.to_le_bytes();
                        if be {
                            out.extend(le[..size].iter().rev());
                        } else {
                            out.extend_from_slice(&le[..size]);
                        }
                    }
                } else if let Some((size, be)) = float_spec(t) {
                    let n = match d.count {
                        Count::Star => items.len().saturating_sub(idx),
                        Count::None => 1,
                        Count::N(n) => n,
                    };
                    for _ in 0..n {
                        let v = next_item!();
                        let f = match crate::fmt::num_to_f(v) {
                            Some(f) => f,
                            None => {
                                if is_str(v) {
                                    return Err(type_error("can't convert String into Float"));
                                }
                                return Err(no_implicit(v, "Float"));
                            }
                        };
                        let bytes: Vec<u8> = if size == 4 {
                            if be { (f as f32).to_be_bytes().to_vec() } else { (f as f32).to_le_bytes().to_vec() }
                        } else if be {
                            f.to_be_bytes().to_vec()
                        } else {
                            f.to_le_bytes().to_vec()
                        };
                        out.extend_from_slice(&bytes);
                    }
                } else {
                    return Err(arg_error(&format!("unknown pack directive '{}' in '{}'", t as char, String::from_utf8_lossy(fmt))));
                }
            }
        }
    }
    Ok(out)
}

fn b64_val(c: u8) -> i32 {
    match c {
        b'A'..=b'Z' => (c - b'A') as i32,
        b'a'..=b'z' => (c - b'a' + 26) as i32,
        b'0'..=b'9' => (c - b'0' + 52) as i32,
        b'+' => 62,
        b'/' => 63,
        _ => -1,
    }
}

fn b64_strict(s: &[u8]) -> Result<Vec<u8>, V> {
    let bad = || arg_error("invalid base64");
    if s.len() % 4 != 0 {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut i = 0;
    while i < s.len() {
        let last = i + 4 == s.len();
        let a = b64_val(s[i]);
        let b = b64_val(s[i + 1]);
        if a < 0 || b < 0 {
            return Err(bad());
        }
        if s[i + 2] == b'=' {
            if !last || s[i + 3] != b'=' || b & 0xf != 0 {
                return Err(bad());
            }
            out.push((a << 2 | b >> 4) as u8);
            break;
        }
        let c = b64_val(s[i + 2]);
        if c < 0 {
            return Err(bad());
        }
        if s[i + 3] == b'=' {
            if !last || c & 0x3 != 0 {
                return Err(bad());
            }
            out.push((a << 2 | b >> 4) as u8);
            out.push(((b << 4) | (c >> 2)) as u8);
            break;
        }
        let d = b64_val(s[i + 3]);
        if d < 0 {
            return Err(bad());
        }
        out.push((a << 2 | b >> 4) as u8);
        out.push(((b << 4) | (c >> 2)) as u8);
        out.push(((c << 6) | d) as u8);
        i += 4;
    }
    Ok(out)
}

fn b64_loose(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let n = s.len();
    let mut i = 0;
    let at = |i: usize| if i < n { s[i] } else { 0 };
    let (mut a, mut b, mut c);
    loop {
        a = -1;
        b = -1;
        c = -1;
        while i < n {
            a = b64_val(s[i]);
            if a != -1 {
                break;
            }
            i += 1;
        }
        if i >= n {
            break;
        }
        i += 1;
        while i < n {
            b = b64_val(s[i]);
            if b != -1 {
                break;
            }
            i += 1;
        }
        if i >= n {
            break;
        }
        i += 1;
        while i < n {
            c = b64_val(s[i]);
            if c != -1 || s[i] == b'=' {
                break;
            }
            i += 1;
        }
        if i >= n || at(i) == b'=' {
            break;
        }
        i += 1;
        let mut d = -1;
        while i < n {
            d = b64_val(s[i]);
            if d != -1 || s[i] == b'=' {
                break;
            }
            i += 1;
        }
        if i >= n || at(i) == b'=' {
            break;
        }
        i += 1;
        out.push((a << 2 | b >> 4) as u8);
        out.push(((b << 4) | (c >> 2)) as u8);
        out.push(((c << 6) | d) as u8);
    }
    if a != -1 && b != -1 {
        out.push((a << 2 | b >> 4) as u8);
        if c != -1 {
            out.push(((b << 4) | (c >> 2)) as u8);
        }
    }
    out
}

fn hex2num(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a' + 10) as i32,
        b'A'..=b'F' => (c - b'A' + 10) as i32,
        _ => -1,
    }
}

fn qpdecode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let n = s.len();
    let mut i = 0;
    let mut ss = 0;
    while i < n {
        if s[i] == b'=' {
            i += 1;
            if i == n {
                break;
            }
            if i + 1 < n && s[i] == b'\r' && s[i + 1] == b'\n' {
                i += 1;
            }
            if s[i] != b'\n' {
                let c1 = hex2num(s[i]);
                if c1 == -1 {
                    break;
                }
                i += 1;
                if i == n {
                    break;
                }
                let c2 = hex2num(s[i]);
                if c2 == -1 {
                    break;
                }
                out.push((c1 << 4 | c2) as u8);
            }
        } else {
            out.push(s[i]);
        }
        i += 1;
        ss = i;
    }
    out.extend_from_slice(&s[ss.min(n)..]);
    out
}

fn uudecode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let n = s.len();
    let mut i = 0;
    while i < n && s[i] > b' ' && s[i] < b'a' {
        let mut len = ((s[i] - b' ') & 0o77) as usize;
        i += 1;
        while len > 0 {
            let mlen = len.min(3);
            let mut g = [0u32; 4];
            for x in g.iter_mut() {
                if i < n && s[i] >= b' ' && s[i] < b'a' {
                    *x = ((s[i] - b' ') & 0o77) as u32;
                    i += 1;
                }
            }
            let hunk = [(g[0] << 2 | g[1] >> 4) as u8, (g[1] << 4 | g[2] >> 2) as u8, (g[2] << 6 | g[3]) as u8];
            out.extend_from_slice(&hunk[..mlen]);
            len -= mlen;
        }
        if i < n && s[i] != b'\r' && s[i] != b'\n' {
            i += 1;
        }
        if i < n && s[i] == b'\r' {
            i += 1;
        }
        if i < n && s[i] == b'\n' {
            i += 1;
        }
    }
    out
}

/// Decode one UTF-8 character: (codepoint, length).
fn utf8_decode(s: &[u8]) -> Result<(u32, usize), V> {
    let bad = || arg_error("malformed UTF-8 character");
    let c = s[0];
    let (len, init) = if c < 0x80 {
        return Ok((c as u32, 1));
    } else if c & 0xe0 == 0xc0 {
        (2, (c & 0x1f) as u32)
    } else if c & 0xf0 == 0xe0 {
        (3, (c & 0x0f) as u32)
    } else if c & 0xf8 == 0xf0 {
        (4, (c & 0x07) as u32)
    } else {
        return Err(bad());
    };
    if s.len() < len {
        return Err(arg_error(&format!("malformed UTF-8 character (expected {} bytes, given {} bytes)", len, s.len())));
    }
    let mut cp = init;
    for &b in &s[1..len] {
        if b & 0xc0 != 0x80 {
            return Err(bad());
        }
        cp = (cp << 6) | (b & 0x3f) as u32;
    }
    Ok((cp, len))
}

pub fn unpack(s: &[u8], fmt: &[u8], offset: usize, single: bool) -> Result<Vec<V>, V> {
    let dirs = parse(fmt)?;
    let mut out: Vec<V> = Vec::new();
    let n = s.len();
    if offset > n {
        return Err(arg_error("offset outside of string"));
    }
    let mut pos = offset;
    for d in &dirs {
        if single && !out.is_empty() {
            break;
        }
        let t = d.t;
        let rest = n - pos;
        match t {
            b'a' | b'A' | b'Z' => {
                let len = match d.count {
                    Count::Star => rest,
                    Count::None => 1.min(rest),
                    Count::N(k) => k.min(rest),
                };
                let mut chunk = &s[pos..pos + len];
                match t {
                    b'A' => {
                        let end = chunk.iter().rposition(|&c| c != b' ' && c != 0).map(|i| i + 1).unwrap_or(0);
                        chunk = &chunk[..end];
                        pos += len;
                    }
                    b'Z' => {
                        if let Some(z) = chunk.iter().position(|&c| c == 0) {
                            chunk = &chunk[..z];
                            if d.count == Count::Star {
                                pos += z + 1;
                            } else {
                                pos += len;
                            }
                        } else {
                            pos += len;
                        }
                    }
                    _ => pos += len,
                }
                out.push(bin_str(chunk.to_vec()));
            }
            b'B' | b'b' => {
                let bits = match d.count {
                    Count::Star => rest * 8,
                    Count::None => 1.min(rest * 8),
                    Count::N(k) => k.min(rest * 8),
                };
                let mut st = Vec::with_capacity(bits);
                for i in 0..bits {
                    let byte = s[pos + i / 8];
                    let bit = if t == b'B' { (byte >> (7 - i % 8)) & 1 } else { (byte >> (i % 8)) & 1 };
                    st.push(b'0' + bit);
                }
                pos += bits.div_ceil(8);
                out.push(str_new(st));
            }
            b'H' | b'h' => {
                let nibs = match d.count {
                    Count::Star => rest * 2,
                    Count::None => 1.min(rest * 2),
                    Count::N(k) => k.min(rest * 2),
                };
                let mut st = Vec::with_capacity(nibs);
                const HEX: &[u8; 16] = b"0123456789abcdef";
                for i in 0..nibs {
                    let byte = s[pos + i / 2];
                    let nib = if (t == b'H') == (i % 2 == 0) { byte >> 4 } else { byte & 15 };
                    st.push(HEX[nib as usize]);
                }
                pos += nibs.div_ceil(2);
                out.push(str_new(st));
            }
            b'U' => {
                let mut k = match d.count {
                    Count::Star => usize::MAX,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                while k > 0 && pos < n {
                    let (cp, l) = utf8_decode(&s[pos..])?;
                    out.push(mkfix(cp as i64));
                    pos += l;
                    k -= 1;
                }
            }
            b'w' => {
                let mut k = match d.count {
                    Count::Star => usize::MAX,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                while k > 0 && pos < n {
                    let mut acc = BigInt::zero();
                    let mut done = false;
                    while pos < n {
                        let b = s[pos];
                        pos += 1;
                        acc = acc * 128 + BigInt::from(b & 0x7f);
                        if b & 0x80 == 0 {
                            done = true;
                            break;
                        }
                    }
                    if !done {
                        break;
                    }
                    out.push(int_from_big(acc));
                    k -= 1;
                }
            }
            b'm' => {
                let data = &s[pos..];
                let r = if d.count == Count::N(0) { b64_strict(data)? } else { b64_loose(data) };
                pos = n;
                out.push(bin_str(r));
            }
            b'M' => {
                out.push(bin_str(qpdecode(&s[pos..])));
                pos = n;
            }
            b'u' => {
                out.push(bin_str(uudecode(&s[pos..])));
                pos = n;
            }
            b'x' => {
                let k = match d.count {
                    Count::Star => 0,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                if k > rest {
                    return Err(arg_error("x outside of string"));
                }
                pos += k;
            }
            b'X' => {
                let k = match d.count {
                    Count::Star => 0,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                if k > pos {
                    return Err(arg_error("X outside of string"));
                }
                pos -= k;
            }
            b'@' => {
                let k = match d.count {
                    Count::Star => 0,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                if offset + k > n {
                    return Err(arg_error("@ outside of string"));
                }
                pos = offset + k;
            }
            b'p' | b'P' => return Err(raise(rt().c.not_implemented_error, "unpack('p'/'P') (pointers) is not supported by Kenny")),
            _ => {
                let (size, kind) = if let Some((size, signed, be)) = int_spec(d) {
                    (size, Some((signed, be)))
                } else if let Some((size, _)) = float_spec(t) {
                    (size, None)
                } else {
                    return Err(arg_error(&format!("unknown unpack directive '{}' in '{}'", t as char, String::from_utf8_lossy(fmt))));
                };
                let star = d.count == Count::Star;
                let mut k = match d.count {
                    Count::Star => rest / size,
                    Count::None => 1,
                    Count::N(k) => k,
                };
                while k > 0 {
                    if single && !out.is_empty() {
                        break;
                    }
                    if pos + size > n {
                        if !star {
                            out.push(QNIL);
                        }
                        k -= 1;
                        continue;
                    }
                    let chunk = &s[pos..pos + size];
                    pos += size;
                    k -= 1;
                    match kind {
                        Some((signed, be)) => {
                            let mut buf = [0u8; 8];
                            if be {
                                for (i, b) in chunk.iter().rev().enumerate() {
                                    buf[i] = *b;
                                }
                            } else {
                                buf[..size].copy_from_slice(chunk);
                            }
                            let u = u64::from_le_bytes(buf);
                            let v: i128 = if signed {
                                let shift = 64 - size * 8;
                                (((u << shift) as i64) >> shift) as i128
                            } else {
                                u as i128
                            };
                            out.push(int_from_i128(v));
                        }
                        None => {
                            let (_, be) = float_spec(t).unwrap();
                            let f = if size == 4 {
                                let a: [u8; 4] = chunk.try_into().unwrap();
                                (if be { f32::from_be_bytes(a) } else { f32::from_le_bytes(a) }) as f64
                            } else {
                                let a: [u8; 8] = chunk.try_into().unwrap();
                                if be { f64::from_be_bytes(a) } else { f64::from_le_bytes(a) }
                            };
                            out.push(float_new(f));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// A new ASCII-8BIT string.
pub fn bin_str(b: Vec<u8>) -> V {
    let v = str_new(b);
    hdr(v).flags |= FL_BINARY;
    v
}

fn kw_get(a: &[V], key: &str) -> Option<V> {
    let h = *a.last()?;
    if !is_hash(h) {
        return None;
    }
    rhash(h).m.get(&HKey(mksym(sym(key)))).copied()
}

fn fmt_arg(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        Ok(rstr(v).b.clone())
    } else {
        Err(no_implicit(v, "String"))
    }
}

fn unpack_impl(s: V, a: &[V], single: bool) -> V {
    let fmt = match fmt_arg(a[0]) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let offset = match kw_get(&a[1..], "offset") {
        Some(o) if is_fix(o) => {
            if fix(o) < 0 {
                return arg_error("offset can't be negative");
            }
            fix(o) as usize
        }
        Some(o) => return no_implicit(o, "Integer"),
        None => 0,
    };
    let bytes = rstr(s).b.clone();
    match unpack(&bytes, &fmt, offset, single) {
        Ok(v) => {
            if single {
                v.first().copied().unwrap_or(QNIL)
            } else {
                ary_new(v)
            }
        }
        Err(e) => e,
    }
}

pub fn init() {
    let c = rt().c;
    defn(c.array, "pack", 1, 2, |s, a, _| {
        let fmt = match fmt_arg(a[0]) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let items = rary(s).v.clone();
        let items = crate::gc::RVec::from(items);
        let bytes = match pack(&items, &fmt) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match kw_get(&a[1..], "buffer") {
            Some(buf) if is_str(buf) => {
                crate::tri!(check_frozen(buf));
                rstr(buf).b.extend_from_slice(&bytes);
                buf
            }
            Some(buf) if buf != QNIL => type_error(&format!("buffer must be String, not {}", class_name(real_class(buf)))),
            _ => {
                // CRuby's choice: UTF-8 when the template starts with U, US-ASCII when it has
                // only m/M/u (base64, quoted-printable, uuencode), else binary
                let dirs: Vec<u8> = fmt.iter().copied().filter(|c| c.is_ascii_alphabetic()).collect();
                if dirs.first() == Some(&b'U') {
                    str_new(bytes)
                } else if !dirs.is_empty() && dirs.iter().all(|c| matches!(c, b'm' | b'M' | b'u')) {
                    let v = str_new(bytes);
                    hdr(v).flags |= crate::compat::FL_BINARY << 1;
                    v
                } else {
                    bin_str(bytes)
                }
            }
        }
    });
    defn(c.string, "unpack", 1, 2, |s, a, b| {
        let r = unpack_impl(s, a, false);
        if r != UNDEF && is_proc(b) {
            let items = crate::gc::RVec::from(rary(r).v.clone());
            for i in 0..items.len() {
                if call_block(b, &[items[i]]) == UNDEF {
                    return UNDEF;
                }
            }
            return QNIL;
        }
        r
    });
    defn(c.string, "unpack1", 1, 2, |s, a, _| unpack_impl(s, a, true));
}
