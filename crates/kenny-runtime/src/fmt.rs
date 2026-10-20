//! to_s / inspect / Float formatting / sprintf.

use crate::core::*;

pub fn float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    // Ruby's flo_to_s: shortest digits, fixed notation unless the exponent is out of range
    let a = f.abs();
    let sci = format!("{:e}", a);
    let (m, e) = sci.split_once('e').unwrap();
    let digs = m.chars().filter(|c| c.is_ascii_digit()).count() as i32;
    let decpt = e.parse::<i32>().unwrap() + 1;
    let fixed = if decpt > 0 { decpt < digs || decpt <= 15 } else { decpt > -4 };
    if !fixed {
        // Ruby: 1.0e+20, 1.5e-05
        let s = format!("{:e}", f);
        let (mant, exp) = s.split_once('e').unwrap();
        let mant = if mant.contains('.') { mant.to_string() } else { format!("{mant}.0") };
        let e: i32 = exp.parse().unwrap();
        return format!("{}e{}{:02}", mant, if e < 0 { '-' } else { '+' }, e.abs());
    }
    let s = format!("{}", f);
    if s.contains('.') { s } else { format!("{s}.0") }
}

pub fn int_to_s(v: V) -> String {
    if is_fix(v) { fix(v).to_string() } else { rbig(v).n.to_string() }
}

/// String bytes of `v.to_s`, calling user-defined to_s. Err(()) means an exception is pending.
pub fn to_s(v: V) -> Result<Vec<u8>, ()> {
    if is_fix(v) {
        return Ok(fix(v).to_string().into_bytes());
    }
    match kind(v) {
        Some(Kind::String) => return Ok(rstr(v).b.clone()),
        _ if v == QNIL => return Ok(Vec::new()),
        _ if is_sym(v) => return Ok(sym_name(sym_id(v)).to_vec()),
        _ => {}
    }
    let r = send(v, sym("to_s"), &[], QNIL);
    if r == UNDEF {
        return Err(());
    }
    if is_str(r) { Ok(rstr(r).b.clone()) } else { Ok(any_to_s(v).into_bytes()) }
}

pub fn inspect(v: V) -> Result<Vec<u8>, ()> {
    if is_fix(v) {
        return Ok(fix(v).to_string().into_bytes());
    }
    if is_str(v) {
        return Ok(str_inspect_v(v).into_bytes());
    }
    let r = send(v, sym("inspect"), &[], QNIL);
    if r == UNDEF {
        return Err(());
    }
    if is_str(r) { Ok(rstr(r).b.clone()) } else { Ok(any_to_s(v).into_bytes()) }
}

pub fn inspect_lossy(v: V) -> String {
    let saved = pst().pending;
    let r = match inspect(v) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(()) => any_to_s(v),
    };
    pst().pending = saved;
    r
}

pub fn any_to_s(v: V) -> String {
    format!("#<{}:0x{:016x}>", class_name(real_class(v)), v)
}
thread_local! {
    /// Objects whose `inspect` is running on this thread (Ruby's recursion guard: an object
    /// that contains itself prints as `#<Foo ...>`, `[...]`, `{...}`).
    static INSPECTING: std::cell::RefCell<Vec<V>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn guarded(v: V, on_cycle: impl FnOnce() -> String, f: impl FnOnce() -> Result<String, ()>) -> Result<String, ()> {
    if INSPECTING.with(|s| s.borrow().contains(&v)) {
        return Ok(on_cycle());
    }
    INSPECTING.with(|s| s.borrow_mut().push(v));
    let r = f();
    INSPECTING.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(i) = s.iter().rposition(|x| *x == v) {
            s.remove(i);
        }
    });
    r
}

pub fn obj_inspect(v: V) -> Result<String, ()> {
    guarded(v, || format!("#<{}:0x{:016x} ...>", class_name(real_class(v)), v), || obj_inspect_inner(v))
}
fn obj_inspect_inner(v: V) -> Result<String, ()> {
    let ivs = ivar_list(v);
    let mut s = format!("#<{}:0x{:016x}", class_name(real_class(v)), v);
    for (i, (name, val)) in ivs.iter().enumerate() {
        s.push_str(if i == 0 { " " } else { ", " });
        s.push_str(&sym_str(*name));
        s.push('=');
        s.push_str(&String::from_utf8_lossy(&inspect(*val)?));
    }
    s.push('>');
    Ok(s)
}

pub fn str_inspect(b: &[u8]) -> String {
    str_inspect_enc(b, false)
}
/// `String#inspect` honouring the binary (ASCII-8BIT) flag.
pub fn str_inspect_v(v: V) -> String {
    str_inspect_enc(&rstr(v).b, hdr(v).flags & FL_BINARY != 0)
}
/// Ruby 3.4 `String#inspect`: UTF-8 strings escape control characters as `\uXXXX` and
/// invalid bytes as `\xNN`; binary strings escape every non-printable byte as `\xNN`.
pub fn str_inspect_enc(b: &[u8], binary: bool) -> String {
    let mut out = String::with_capacity(b.len() + 2);
    out.push('"');
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        let simple = match c {
            b'"' => Some("\\\""),
            b'\\' => Some("\\\\"),
            b'\n' => Some("\\n"),
            b'\t' => Some("\\t"),
            b'\r' => Some("\\r"),
            0x1b => Some("\\e"),
            0x07 => Some("\\a"),
            0x08 => Some("\\b"),
            0x0b => Some("\\v"),
            0x0c => Some("\\f"),
            b'#' if matches!(next, Some(b'{') | Some(b'$') | Some(b'@')) => Some("\\#"),
            _ => None,
        };
        if let Some(e) = simple {
            out.push_str(e);
            i += 1;
            continue;
        }
        if c < 0x20 || c == 0x7f {
            if binary { out.push_str(&format!("\\x{:02X}", c)) } else { out.push_str(&format!("\\u{:04X}", c)) }
            i += 1;
            continue;
        }
        if c < 0x80 {
            out.push(c as char);
            i += 1;
            continue;
        }
        if !binary {
            let len = match c {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 0,
            };
            if len > 0 && i + len <= b.len() {
                if let Ok(s) = std::str::from_utf8(&b[i..i + len]) {
                    out.push_str(s);
                    i += len;
                    continue;
                }
            }
        }
        out.push_str(&format!("\\x{:02X}", c));
        i += 1;
    }
    out.push('"');
    out
}

pub fn sym_inspect(id: u32) -> String {
    let n = sym_name(id);
    let s = String::from_utf8_lossy(n);
    let plain = !n.is_empty()
        && (is_ident(&s)
            || matches!(
                &*s,
                "+" | "-" | "*" | "/" | "%" | "**" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "<=>" | "===" | "[]"
                    | "[]=" | "<<" | ">>" | "!" | "=~" | "&" | "|" | "^" | "~" | "+@" | "-@"
            ));
    if plain { format!(":{s}") } else { format!(":{}", str_inspect(n)) }
}
fn is_ident(s: &str) -> bool {
    let s = s.strip_prefix("@@").or_else(|| s.strip_prefix('@')).or_else(|| s.strip_prefix('$')).unwrap_or(s);
    let body = s.strip_suffix(['?', '!', '=']).unwrap_or(s);
    let mut cs = body.chars();
    matches!(cs.next(), Some(c) if c.is_alphabetic() || c == '_') && cs.all(|c| c.is_alphanumeric() || c == '_')
}

/// Ruby 3.4 style hash key: `name: 1`, `"str" => 1`
pub fn hash_inspect(v: V) -> Result<String, ()> {
    guarded(v, || "{...}".into(), || hash_inspect_inner(v))
}
fn hash_inspect_inner(v: V) -> Result<String, ()> {
    let h = rhash(v);
    if h.m.is_empty() {
        return Ok("{}".into());
    }
    let mut s = String::from("{");
    for (i, (k, val)) in h.m.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        if is_sym(k.0) && is_ident(&sym_str(sym_id(k.0))) && !sym_str(sym_id(k.0)).starts_with(['@', '$']) {
            s.push_str(&sym_str(sym_id(k.0)));
            s.push_str(": ");
        } else {
            s.push_str(&String::from_utf8_lossy(&inspect(k.0)?));
            s.push_str(" => ");
        }
        s.push_str(&String::from_utf8_lossy(&inspect(*val)?));
    }
    s.push('}');
    Ok(s)
}

pub fn ary_inspect(v: V) -> Result<String, ()> {
    guarded(v, || "[...]".into(), || ary_inspect_inner(v))
}
fn ary_inspect_inner(v: V) -> Result<String, ()> {
    let mut s = String::from("[");
    let items = rary(v).v.clone();
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        if *x == v {
            s.push_str("[...]");
        } else {
            s.push_str(&String::from_utf8_lossy(&inspect(*x)?));
        }
    }
    s.push(']');
    Ok(s)
}

// ---------- sprintf ----------

pub fn num_to_f(v: V) -> Option<f64> {
    use num_traits::ToPrimitive;
    if is_fix(v) {
        Some(fix(v) as f64)
    } else if is_float(v) {
        Some(float_val(v))
    } else if is_big(v) {
        rbig(v).n.to_f64()
    } else {
        None
    }
}

pub fn sprintf(fmt: &[u8], args: &[V]) -> Result<Vec<u8>, V> {
    let mut out = Vec::new();
    let mut ai = 0;
    let mut i = 0;
    let next = |ai: &mut usize| -> Result<V, V> {
        let v = args.get(*ai).copied();
        *ai += 1;
        v.ok_or_else(|| arg_error("too few arguments"))
    };
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= fmt.len() {
            out.push(b'%');
            break;
        }
        if fmt[i] == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        let (mut left, mut zero, mut plus, mut space) = (false, false, false, false);
        while i < fmt.len() && matches!(fmt[i], b'-' | b'0' | b'+' | b' ') {
            match fmt[i] {
                b'-' => left = true,
                b'0' => zero = true,
                b'+' => plus = true,
                _ => space = true,
            }
            i += 1;
        }
        let mut width = 0usize;
        if i < fmt.len() && fmt[i] == b'*' {
            width = fix(next(&mut ai)?) as usize;
            i += 1;
        }
        while i < fmt.len() && fmt[i].is_ascii_digit() {
            width = width * 10 + (fmt[i] - b'0') as usize;
            i += 1;
        }
        let mut prec: Option<usize> = None;
        if i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            let mut p = 0;
            while i < fmt.len() && fmt[i].is_ascii_digit() {
                p = p * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
            prec = Some(p);
        }
        if i >= fmt.len() {
            break;
        }
        let conv = fmt[i];
        i += 1;
        let body: String = match conv {
            b'd' | b'i' | b'u' => {
                let v = next(&mut ai)?;
                let v = if is_float(v) { int_from_i128(float_val(v).floor() as i128) } else { v };
                if !is_int(v) {
                    return Err(type_error("can't convert to Integer"));
                }
                let s = int_to_s(v);
                if plus && !s.starts_with('-') {
                    format!("+{s}")
                } else if space && !s.starts_with('-') {
                    format!(" {s}")
                } else {
                    s
                }
            }
            b'f' | b'e' | b'g' => {
                let v = next(&mut ai)?;
                // other Numerics (BigDecimal, ...) go through #to_f like Kernel#Float
                let v = if num_to_f(v).is_none() && is_heap(v) && !is_str(v) && respond_to(v, sym("to_f")) {
                    let r = send(v, sym("to_f"), &[], QNIL);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    r
                } else {
                    v
                };
                let f = num_to_f(v).ok_or_else(|| type_error("can't convert to Float"))?;
                let p = prec.unwrap_or(6);
                let s = match conv {
                    b'f' => format!("{:.*}", p, f),
                    b'e' => {
                        let s = format!("{:.*e}", p, f);
                        let (m, e) = s.split_once('e').unwrap();
                        let e: i32 = e.parse().unwrap();
                        format!("{}e{}{:02}", m, if e < 0 { '-' } else { '+' }, e.abs())
                    }
                    _ => {
                        let s = format!("{}", f);
                        s
                    }
                };
                if plus && f >= 0.0 { format!("+{s}") } else { s }
            }
            b's' => {
                let v = next(&mut ai)?;
                let b = to_s(v).map_err(|_| UNDEF)?;
                let s = String::from_utf8_lossy(&b).into_owned();
                match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                }
            }
            b'p' => {
                let v = next(&mut ai)?;
                String::from_utf8_lossy(&inspect(v).map_err(|_| UNDEF)?).into_owned()
            }
            b'x' | b'X' | b'o' | b'b' | b'B' => {
                let v = next(&mut ai)?;
                if !is_fix(v) {
                    return Err(type_error("can't convert to Integer"));
                }
                let n = fix(v);
                let (neg, a) = (n < 0, n.unsigned_abs());
                let s = match conv {
                    b'x' => format!("{:x}", a),
                    b'X' => format!("{:X}", a),
                    b'o' => format!("{:o}", a),
                    _ => format!("{:b}", a),
                };
                if neg { format!("-{s}") } else { s }
            }
            b'c' => {
                let v = next(&mut ai)?;
                if is_fix(v) {
                    char::from_u32(fix(v) as u32).map(|c| c.to_string()).unwrap_or_default()
                } else {
                    String::from_utf8_lossy(&to_s(v).map_err(|_| UNDEF)?).chars().take(1).collect()
                }
            }
            _ => return Err(arg_error(&format!("malformed format string - %{}", conv as char))),
        };
        let len = body.chars().count();
        if len >= width {
            out.extend_from_slice(body.as_bytes());
        } else if left {
            out.extend_from_slice(body.as_bytes());
            out.extend(std::iter::repeat_n(b' ', width - len));
        } else if zero && matches!(conv, b'd' | b'i' | b'f' | b'x' | b'X' | b'o' | b'b' | b'e') {
            let (sign, digits) = match body.strip_prefix(['-', '+']) {
                Some(d) => (&body[..1], d),
                None => ("", body.as_str()),
            };
            out.extend_from_slice(sign.as_bytes());
            out.extend(std::iter::repeat_n(b'0', width - len));
            out.extend_from_slice(digits.as_bytes());
        } else {
            out.extend(std::iter::repeat_n(b' ', width - len));
            out.extend_from_slice(body.as_bytes());
        }
    }
    Ok(out)
}
