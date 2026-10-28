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
    let mut ivs = ivar_list(v);
    // Ruby 4.0: the instance variables an object's #instance_variables_to_inspect names (nil: all)
    if crate::misc::ruby4() {
        let m = sym("instance_variables_to_inspect");
        if !find_method(class_of(v), m).is_null() {
            let names = send(v, m, &[], QNIL);
            if names == UNDEF {
                return Err(());
            }
            if is_ary(names) {
                let want: Vec<u32> = rary(names).v.iter().filter(|x| is_sym(**x)).map(|x| sym_id(*x)).collect();
                ivs = want.iter().filter_map(|n| ivs.iter().find(|(k, _)| k == n).copied()).collect();
            } else if names != QNIL {
                type_error(&format!("Expected #instance_variables_to_inspect to return an Array or nil, but it returned {}", class_name(real_class(names))));
                return Err(());
            }
        }
    }
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
    if hdr(v).flags & FL_BINARY != 0 {
        // a String in another encoding shows its characters (as CRuby's inspect does)
        // (non-ASCII ones as \uXXXX, not being in the default external encoding)
        if let Some(u) = crate::transcode::foreign_as_utf8(v) {
            let shown = str_inspect_enc(&u, false);
            let mut out = String::with_capacity(shown.len());
            for c in shown.chars() {
                match c as u32 {
                    0..=0x7F => out.push(c),
                    cp @ 0x80..=0xFFFF => out.push_str(&format!("\\u{cp:04X}")),
                    cp => out.push_str(&format!("\\u{{{cp:X}}}")),
                }
            }
            return out;
        }
    }
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
                    let ch = s.chars().next().unwrap() as u32;
                    if crate::unprint::unprintable(ch) {
                        if ch > 0xFFFF { out.push_str(&format!("\\u{{{:X}}}", ch)) } else { out.push_str(&format!("\\u{:04X}", ch)) }
                    } else {
                        out.push_str(s);
                    }
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

/// The digits of a negative Integer in two's complement, as CRuby's sprintf shows them
/// without a sign flag: `"%x" % -255` is "..f01" (`radix` 16, 8 or 2).
fn twos_complement(v: V, radix: u32) -> String {
    use num_bigint::BigInt;
    let n = if is_fix(v) { BigInt::from(fix(v)) } else { rbig(v).n.clone() };
    let digits = (-&n).to_str_radix(radix).len() as u32 + 1;
    let m: BigInt = BigInt::from(radix).pow(digits) + &n;
    let mut s = m.to_str_radix(radix);
    while (s.len() as u32) < digits {
        s.insert(0, '0');
    }
    let top = std::char::from_digit(radix - 1, radix).unwrap();
    let keep = s.find(|c| c != top).map(|i| i.saturating_sub(1)).unwrap_or(s.len() - 1);
    format!("..{}", &s[keep..])
}

/// C's `%g`: `p` significant digits, the shorter of fixed and exponent forms, trailing
/// zeros dropped unless `alt` (`#`).
fn fmt_g(f: f64, p: usize, alt: bool, upper: bool) -> String {
    let p = p.max(1);
    if f == 0.0 {
        let mut s = if f.is_sign_negative() { "-0".to_string() } else { "0".to_string() };
        if alt {
            s.push('.');
            s.extend(std::iter::repeat_n('0', p - 1));
        }
        return s;
    }
    let e = format!("{:.*e}", p - 1, f);
    let (m, ex) = e.split_once('e').unwrap();
    let x: i32 = ex.parse().unwrap();
    let strip = |s: String| -> String {
        if alt || !s.contains('.') {
            return s;
        }
        let t = s.trim_end_matches('0');
        t.trim_end_matches('.').to_string()
    };
    if x < -4 || x >= p as i32 {
        let m = strip(m.to_string());
        format!("{m}{}{}{:02}", if upper { 'E' } else { 'e' }, if x < 0 { '-' } else { '+' }, x.abs())
    } else {
        let prec = (p as i32 - 1 - x).max(0) as usize;
        let s = format!("{:.*}", prec, f);
        if alt && !s.contains('.') { format!("{s}.") } else { strip(s) }
    }
}

/// Kernel#format / String#%, as CRuby's sprintf: flags `-+ 0#`, width and precision (also
/// `*`), `%<name>` / `%{name}` (from a Hash argument) and `%1$s` argument references.
pub fn sprintf(fmt: &[u8], args: &[V]) -> Result<Vec<u8>, V> {
    let mut out = Vec::new();
    let mut ai = 0;
    let mut i = 0;
    let hash_arg = || -> Result<V, V> {
        match args {
            [h] if is_hash(*h) => Ok(*h),
            _ => Err(arg_error("one hash required")),
        }
    };
    let lookup = |h: V, name: &[u8]| -> Result<V, V> {
        let key = mksym(sym(&String::from_utf8_lossy(name)));
        let hh = rhash(h);
        if let Some(v) = hh.m.get(&HKey(key)) {
            return Ok(*v);
        }
        if hh.default_proc != QNIL || hh.default != QNIL {
            let v = crate::hash::hget(h, key);
            return if v == UNDEF { Err(UNDEF) } else { Ok(v) };
        }
        let e = exc_new(rt().c.key_error, &format!("key<{}> not found", String::from_utf8_lossy(name)));
        Err(raise_exc(e))
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
            return Err(arg_error("incomplete format specifier; use %% (double %) instead"));
        }
        if fmt[i] == b'%' {
            out.push(b'%');
            i += 1;
            continue;
        }
        if fmt[i] == b'\n' || fmt[i] == 0 {
            return Err(arg_error("malformed format string"));
        }
        let (mut left, mut zero, mut plus, mut space, mut alt) = (false, false, false, false, false);
        let mut width: Option<usize> = None;
        let mut prec: Option<usize> = None;
        let mut value: Option<V> = None;
        let mut braced: Option<V> = None;
        // flags, width, `<name>`, `{name}` and `n$` in any order before the precision
        loop {
            if i >= fmt.len() {
                return Err(arg_error("incomplete format specifier; use %% (double %) instead"));
            }
            match fmt[i] {
                b'-' => left = true,
                b'0' => zero = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'#' => alt = true,
                b'<' | b'{' => {
                    let close = if fmt[i] == b'<' { b'>' } else { b'}' };
                    let Some(len) = fmt[i + 1..].iter().position(|&c| c == close) else {
                        return Err(arg_error("malformed name - unmatched parenthesis"));
                    };
                    let name = &fmt[i + 1..i + 1 + len];
                    let v = lookup(hash_arg()?, name)?;
                    if close == b'>' {
                        value = Some(v);
                    } else {
                        braced = Some(v);
                    }
                    i += len + 1;
                }
                b'*' => {
                    let w = args.get(ai).copied().ok_or_else(|| arg_error("too few arguments"))?;
                    ai += 1;
                    if !is_fix(w) {
                        return Err(type_error("no implicit conversion into Integer"));
                    }
                    let n = fix(w);
                    if n < 0 {
                        left = true;
                    }
                    width = Some(n.unsigned_abs() as usize);
                }
                b'1'..=b'9' => {
                    let mut n = 0usize;
                    while i < fmt.len() && fmt[i].is_ascii_digit() {
                        n = n * 10 + (fmt[i] - b'0') as usize;
                        i += 1;
                    }
                    if i < fmt.len() && fmt[i] == b'$' {
                        value = Some(args.get(n - 1).copied().ok_or_else(|| arg_error("too few arguments"))?);
                    } else {
                        width = Some(n);
                        continue;
                    }
                }
                _ => break,
            }
            i += 1;
            if braced.is_some() {
                break;
            }
        }
        if braced.is_none() && i < fmt.len() && fmt[i] == b'.' {
            i += 1;
            if i < fmt.len() && fmt[i] == b'*' {
                let p = args.get(ai).copied().ok_or_else(|| arg_error("too few arguments"))?;
                ai += 1;
                prec = Some(if is_fix(p) { fix(p).max(0) as usize } else { 0 });
                i += 1;
            } else {
                let mut p = 0;
                while i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + (fmt[i] - b'0') as usize;
                    i += 1;
                }
                prec = Some(p);
            }
        }
        let conv = if braced.is_some() {
            b'S'
        } else {
            if i >= fmt.len() {
                return Err(arg_error("incomplete format specifier; use %% (double %) instead"));
            }
            i += 1;
            fmt[i - 1]
        };
        let mut next = || -> Result<V, V> {
            if let Some(v) = value.take() {
                return Ok(v);
            }
            let v = args.get(ai).copied();
            ai += 1;
            v.ok_or_else(|| arg_error("too few arguments"))
        };
        // (sign, prefix like "0x", digits/body): padding goes between prefix and digits for `0`
        let (sign, prefix, body): (&str, String, String) = match conv {
            b'S' => {
                let b = to_s(braced.unwrap()).map_err(|_| UNDEF)?;
                ("", String::new(), String::from_utf8_lossy(&b).into_owned())
            }
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'b' | b'B' => {
                let v = next()?;
                let v = if is_float(v) {
                    let f = float_val(v);
                    if !f.is_finite() {
                        return Err(raise(rt().c.float_domain_error, &crate::fmt::float_to_s(f)));
                    }
                    send(v, sym("to_i"), &[], QNIL)
                } else if is_int(v) {
                    v
                } else if v == QNIL {
                    return Err(type_error("can't convert nil into Integer"));
                } else {
                    let r = send(rt().main_obj, sym("Integer"), &[v], QNIL);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    r
                };
                let neg = if is_fix(v) { fix(v) < 0 } else { rbig(v).n.sign() == num_bigint::Sign::Minus };
                let radix = match conv {
                    b'x' | b'X' => 16,
                    b'o' => 8,
                    b'b' | b'B' => 2,
                    _ => 10,
                };
                let mut sign = if neg { "-" } else if plus { "+" } else if space { " " } else { "" };
                let mut digits = if neg && radix != 10 && !plus && !space {
                    sign = "";
                    twos_complement(v, radix)
                } else {
                    let s = crate::numeric::int_to_s_radix(v, radix);
                    s.trim_start_matches('-').to_string()
                };
                if let Some(p) = prec {
                    if digits.starts_with("..") {
                        let fill = digits.chars().nth(2).unwrap_or('f');
                        while digits.len() - 2 < p {
                            digits.insert(2, fill);
                        }
                    } else {
                        while digits.len() < p {
                            digits.insert(0, '0');
                        }
                    }
                }
                let is_zero = v == mkfix(0);
                let prefix = match conv {
                    b'x' if alt && !is_zero => "0x",
                    b'X' if alt && !is_zero => "0X",
                    b'b' if alt && !is_zero => "0b",
                    b'B' if alt && !is_zero => "0B",
                    b'o' if alt && !digits.starts_with('0') => "0",
                    _ => "",
                };
                if conv == b'X' {
                    digits = digits.to_uppercase();
                }
                (sign, prefix.to_string(), digits)
            }
            b'f' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let v = next()?;
                // other Numerics (BigDecimal, Rational, ...) go through #to_f like Kernel#Float
                let v = if num_to_f(v).is_none() && is_heap(v) && !is_str(v) && respond_to(v, sym("to_f")) {
                    let r = send(v, sym("to_f"), &[], QNIL);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    r
                } else if is_str(v) {
                    let r = send(rt().main_obj, sym("Float"), &[v], QNIL);
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    r
                } else {
                    v
                };
                let f = num_to_f(v).ok_or_else(|| type_error(&format!("can't convert {} into Float", if v == QNIL { "nil".to_string() } else { class_name(class_of(v)) })))?;
                let sign = if f.is_sign_negative() && !(f == 0.0 && conv != b'f') || f < 0.0 { "-" } else if plus { "+" } else if space { " " } else { "" };
                let sign = if f.is_nan() { if plus { "+" } else if space { " " } else { "" } } else { sign };
                let a = f.abs();
                let body = if f.is_nan() {
                    "NaN".to_string()
                } else if f.is_infinite() {
                    "Inf".to_string()
                } else {
                    let p = prec.unwrap_or(6);
                    match conv {
                        b'f' => {
                            let s = format!("{:.*}", p, a);
                            if alt && p == 0 { format!("{s}.") } else { s }
                        }
                        b'e' | b'E' => {
                            let s = format!("{:.*e}", p, a);
                            let (m, e) = s.split_once('e').unwrap();
                            let e: i32 = e.parse().unwrap();
                            let m = if alt && p == 0 { format!("{m}.") } else { m.to_string() };
                            format!("{}{}{}{:02}", m, if conv == b'E' { 'E' } else { 'e' }, if e < 0 { '-' } else { '+' }, e.abs())
                        }
                        b'g' | b'G' => fmt_g(a, p, alt, conv == b'G'),
                        _ => return Err(arg_error(&format!("malformed format string - %{}", conv as char))),
                    }
                };
                // (zero padding doesn't apply to Inf/NaN)
                if !f.is_finite() {
                    zero = false;
                }
                (sign, String::new(), body)
            }
            b's' | b'p' => {
                let v = next()?;
                let b = if conv == b's' { to_s(v).map_err(|_| UNDEF)? } else { inspect(v).map_err(|_| UNDEF)? };
                let s = String::from_utf8_lossy(&b).into_owned();
                ("", String::new(), match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                })
            }
            b'c' => {
                let v = next()?;
                let s = if is_fix(v) {
                    char::from_u32(fix(v) as u32).map(|c| c.to_string()).unwrap_or_default()
                } else {
                    String::from_utf8_lossy(&to_s(v).map_err(|_| UNDEF)?).chars().take(1).collect()
                };
                ("", String::new(), s)
            }
            _ => return Err(arg_error(&format!("malformed format string - %{}", conv as char))),
        };
        let width = width.unwrap_or(0);
        let len = sign.chars().count() + prefix.chars().count() + body.chars().count();
        let numeric = !matches!(conv, b's' | b'p' | b'c' | b'S');
        // (an integer precision turns `0` padding off, as in C)
        let int_conv = matches!(conv, b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'b' | b'B');
        if len >= width {
            out.extend_from_slice(sign.as_bytes());
            out.extend_from_slice(prefix.as_bytes());
            out.extend_from_slice(body.as_bytes());
        } else if left {
            out.extend_from_slice(sign.as_bytes());
            out.extend_from_slice(prefix.as_bytes());
            out.extend_from_slice(body.as_bytes());
            out.extend(std::iter::repeat_n(b' ', width - len));
        } else if zero && numeric && !(int_conv && prec.is_some()) {
            out.extend_from_slice(sign.as_bytes());
            out.extend_from_slice(prefix.as_bytes());
            // a two's-complement number pads with its fill digit ("..f01" -> "..fff01")
            if let Some(rest) = body.strip_prefix("..") {
                let fill = rest.chars().next().unwrap_or('0') as u8;
                out.extend(std::iter::repeat_n(fill, width - len));
                out.extend_from_slice(rest.as_bytes());
                out.extend_from_slice(b"..");
                let n = out.len();
                // ("..": in front, as CRuby writes it)
                let start = n - rest.len() - 2 - (width - len);
                out[start..].rotate_right(2);
            } else {
                out.extend(std::iter::repeat_n(b'0', width - len));
                out.extend_from_slice(body.as_bytes());
            }
        } else {
            out.extend(std::iter::repeat_n(b' ', width - len));
            out.extend_from_slice(sign.as_bytes());
            out.extend_from_slice(prefix.as_bytes());
            out.extend_from_slice(body.as_bytes());
        }
    }
    Ok(out)
}
