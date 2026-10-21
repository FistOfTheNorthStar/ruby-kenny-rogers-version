//! JSON (the `json` gem's C extension, json 2.9 behaviour): parser, generator,
//! JSON::Ext::Generator::State and `#to_json` on the core classes.
//!
//! The generator handles core values natively. Any value whose `to_json` is not one of the
//! built-in methods (a user/ActiveSupport override) gets `to_json(state)` called, with the
//! state's `depth` set, so overrides that call `super(state)` nest correctly.

use crate::core::*;
use crate::tri;
use num_bigint::BigInt;

struct JC {
    parser_error: *mut Class,
    nesting_error: *mut Class,
    generator_error: *mut Class,
    state: *mut Class,
    parser: *mut Class,
    generator: *mut Class,
}
static JCS: G<Option<JC>> = G::new(None);
fn jc() -> &'static JC {
    JCS.get().as_ref().unwrap()
}
/// The built-in `to_json` methods (anything else is a user override).
static BUILTIN_TO_JSON: G<Vec<*const Method>> = G::new(Vec::new());

// ---------- options / State ----------

#[derive(Clone)]
struct Opts {
    indent: Vec<u8>,
    space: Vec<u8>,
    space_before: Vec<u8>,
    object_nl: Vec<u8>,
    array_nl: Vec<u8>,
    allow_nan: bool,
    ascii_only: bool,
    script_safe: bool,
    strict: bool,
    max_nesting: i64,
    depth: i64,
}
impl Default for Opts {
    fn default() -> Self {
        Opts {
            indent: vec![],
            space: vec![],
            space_before: vec![],
            object_nl: vec![],
            array_nl: vec![],
            allow_nan: false,
            ascii_only: false,
            script_safe: false,
            strict: false,
            max_nesting: 100,
            depth: 0,
        }
    }
}
fn pretty() -> Opts {
    Opts { indent: b"  ".to_vec(), space: b" ".to_vec(), object_nl: b"\n".to_vec(), array_nl: b"\n".to_vec(), ..Opts::default() }
}

const STR_KEYS: [&str; 5] = ["indent", "space", "space_before", "object_nl", "array_nl"];
const BOOL_KEYS: [&str; 4] = ["allow_nan", "ascii_only", "script_safe", "strict"];

fn opt_get(h: V, k: &str) -> Option<V> {
    let m = &rhash(h).m;
    m.get(&HKey(mksym(sym(k)))).or_else(|| m.get(&HKey(str_from(k)))).copied()
}
fn bytes_of(v: V) -> Vec<u8> {
    if is_str(v) { rstr(v).b.clone() } else { crate::fmt::to_s(v).unwrap_or_default() }
}
fn nesting_of(v: V) -> i64 {
    if is_fix(v) { fix(v) } else { 0 }
}
impl Opts {
    fn merge_hash(&mut self, h: V) {
        for (i, k) in STR_KEYS.iter().enumerate() {
            if let Some(v) = opt_get(h, k) {
                let b = if v == QNIL { vec![] } else { bytes_of(v) };
                match i {
                    0 => self.indent = b,
                    1 => self.space = b,
                    2 => self.space_before = b,
                    3 => self.object_nl = b,
                    _ => self.array_nl = b,
                }
            }
        }
        if let Some(v) = opt_get(h, "allow_nan") {
            self.allow_nan = truthy(v);
        }
        if let Some(v) = opt_get(h, "ascii_only") {
            self.ascii_only = truthy(v);
        }
        if let Some(v) = opt_get(h, "script_safe").or_else(|| opt_get(h, "escape_slash")) {
            self.script_safe = truthy(v);
        }
        if let Some(v) = opt_get(h, "strict") {
            self.strict = truthy(v);
        }
        if let Some(v) = opt_get(h, "max_nesting") {
            self.max_nesting = nesting_of(v);
        }
        if let Some(v) = opt_get(h, "depth") {
            self.depth = nesting_of(v);
        }
    }
    fn from_state(st: V) -> Opts {
        let g = |k: &str| ivar_get(st, sym(&format!("@__{k}")));
        let s = |k: &str| {
            let v = g(k);
            if is_str(v) { rstr(v).b.clone() } else { vec![] }
        };
        Opts {
            indent: s("indent"),
            space: s("space"),
            space_before: s("space_before"),
            object_nl: s("object_nl"),
            array_nl: s("array_nl"),
            allow_nan: truthy(g("allow_nan")),
            ascii_only: truthy(g("ascii_only")),
            script_safe: truthy(g("script_safe")),
            strict: truthy(g("strict")),
            max_nesting: nesting_of(g("max_nesting")),
            depth: nesting_of(g("depth")),
        }
    }
    fn store(&self, st: V) {
        let set = |k: &str, v: V| {
            ivar_set(st, sym(&format!("@__{k}")), v);
        };
        set("indent", str_new(self.indent.clone()));
        set("space", str_new(self.space.clone()));
        set("space_before", str_new(self.space_before.clone()));
        set("object_nl", str_new(self.object_nl.clone()));
        set("array_nl", str_new(self.array_nl.clone()));
        set("allow_nan", bool_v(self.allow_nan));
        set("ascii_only", bool_v(self.ascii_only));
        set("script_safe", bool_v(self.script_safe));
        set("strict", bool_v(self.strict));
        set("max_nesting", mkfix(self.max_nesting));
        set("depth", mkfix(self.depth));
    }
}
fn state_new(o: &Opts) -> V {
    let st = object_new(jc().state);
    o.store(st);
    st
}
/// Options + State object from a `to_json`/`generate` argument (State, Hash or nil).
fn opts_arg(a: Option<V>, base: Opts) -> (Opts, V) {
    match a {
        Some(v) if is_heap(v) && is_a(v, jc().state) => (Opts::from_state(v), v),
        Some(v) if is_hash(v) => {
            let mut o = base;
            o.merge_hash(v);
            let st = state_new(&o);
            (o, st)
        }
        _ => {
            let st = state_new(&base);
            (base, st)
        }
    }
}

// ---------- generator ----------

fn gen_error(msg: &str) -> V {
    raise(jc().generator_error, msg)
}

fn escape_str(b: &[u8], o: &Opts, out: &mut Vec<u8>) -> Result<(), V> {
    let s = match std::str::from_utf8(b) {
        Ok(s) => s,
        Err(_) => return Err(gen_error("source sequence is illegal/malformed utf-8")),
    };
    out.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes()),
            '/' if o.script_safe => out.extend_from_slice(b"\\/"),
            '\u{2028}' | '\u{2029}' if o.script_safe => out.extend_from_slice(format!("\\u{:04x}", ch as u32).as_bytes()),
            c if o.ascii_only && (c as u32) > 0x7f => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.extend_from_slice(format!("\\u{:04x}", u).as_bytes());
                }
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
    Ok(())
}

fn indent(out: &mut Vec<u8>, o: &Opts, depth: i64) {
    if !o.indent.is_empty() {
        for _ in 0..depth {
            out.extend_from_slice(&o.indent);
        }
    }
}

fn is_builtin_to_json(v: V) -> bool {
    // as in the json gem's C generator: instances of the core classes themselves are always
    // generated natively, whatever `to_json` they have (ActiveSupport adds one to each)
    let c = class_of(v);
    let k = &rt().c;
    if v == QNIL || v == QTRUE || v == QFALSE || [k.hash, k.array, k.string, k.integer, k.float].contains(&c) {
        return true;
    }
    let m = find_method(c, sym("to_json"));
    m.is_null() || BUILTIN_TO_JSON.get().contains(&m)
}

fn gen_float(f: f64, o: &Opts, out: &mut Vec<u8>) -> Result<(), V> {
    if f.is_nan() || f.is_infinite() {
        let s = crate::fmt::float_to_s(f);
        if !o.allow_nan {
            return Err(gen_error(&format!("{s} not allowed in JSON")));
        }
        out.extend_from_slice(s.as_bytes());
        return Ok(());
    }
    out.extend_from_slice(crate::fmt::float_to_s(f).as_bytes());
    Ok(())
}

fn gen_value(v: V, o: &Opts, st: V, depth: i64, out: &mut Vec<u8>) -> Result<(), V> {
    if !is_builtin_to_json(v) {
        ivar_set(st, sym("@__depth"), mkfix(depth));
        let r = send(v, sym("to_json"), &[st], QNIL);
        if r == UNDEF {
            return Err(UNDEF);
        }
        ivar_set(st, sym("@__depth"), mkfix(depth));
        if is_str(r) {
            out.extend_from_slice(&rstr(r).b);
        } else {
            out.extend(crate::fmt::to_s(r).map_err(|_| UNDEF)?);
        }
        return Ok(());
    }
    gen_native(v, o, st, depth, out)
}

/// Generate `v` with the built-in rules, without looking for a `to_json` override
/// (used by the built-in `to_json`, which an override may have reached through `super`).
fn gen_native(v: V, o: &Opts, st: V, depth: i64, out: &mut Vec<u8>) -> Result<(), V> {
    match v {
        QNIL => out.extend_from_slice(b"null"),
        QTRUE => out.extend_from_slice(b"true"),
        QFALSE => out.extend_from_slice(b"false"),
        _ if is_fix(v) => out.extend_from_slice(fix(v).to_string().as_bytes()),
        _ if is_sym(v) => {
            if o.strict {
                return Err(gen_error(&format!("{} not allowed in JSON", crate::fmt::inspect_lossy(v))));
            }
            escape_str(sym_name(sym_id(v)), o, out)?
        }
        _ => match kind(v) {
            Some(Kind::Bignum) => out.extend_from_slice(rbig(v).n.to_string().as_bytes()),
            Some(Kind::Float) => gen_float(float_val(v), o, out)?,
            Some(Kind::String) => escape_str(&rstr(v).b, o, out)?,
            Some(Kind::Hash) => gen_hash(v, o, st, depth + 1, out)?,
            Some(Kind::Array) => gen_array(v, o, st, depth + 1, out)?,
            _ => {
                if o.strict {
                    return Err(gen_error(&format!("{} not allowed in JSON", crate::fmt::inspect_lossy(v))));
                }
                let s = crate::fmt::to_s(v).map_err(|_| UNDEF)?;
                escape_str(&s, o, out)?
            }
        },
    }
    Ok(())
}

fn check_nesting(o: &Opts, depth: i64) -> Result<(), V> {
    if o.max_nesting != 0 && depth > o.max_nesting {
        return Err(raise(jc().nesting_error, &format!("nesting of {depth} is too deep")));
    }
    Ok(())
}

fn gen_hash(h: V, o: &Opts, st: V, depth: i64, out: &mut Vec<u8>) -> Result<(), V> {
    check_nesting(o, depth)?;
    if rhash(h).m.is_empty() {
        out.extend_from_slice(b"{}");
        return Ok(());
    }
    out.push(b'{');
    out.extend_from_slice(&o.object_nl);
    let mut i = 0;
    loop {
        let Some((k, val)) = rhash(h).m.get_index(i).map(|(k, v)| (k.0, *v)) else { break };
        if i > 0 {
            out.push(b',');
            out.extend_from_slice(&o.object_nl);
        }
        indent(out, o, depth);
        let kb = if is_str(k) {
            rstr(k).b.clone()
        } else if is_sym(k) {
            sym_name(sym_id(k)).to_vec()
        } else {
            crate::fmt::to_s(k).map_err(|_| UNDEF)?
        };
        escape_str(&kb, o, out)?;
        out.extend_from_slice(&o.space_before);
        out.push(b':');
        out.extend_from_slice(&o.space);
        gen_value(val, o, st, depth, out)?;
        i += 1;
    }
    out.extend_from_slice(&o.object_nl);
    indent(out, o, depth - 1);
    out.push(b'}');
    Ok(())
}

fn gen_array(a: V, o: &Opts, st: V, depth: i64, out: &mut Vec<u8>) -> Result<(), V> {
    check_nesting(o, depth)?;
    if rary(a).v.is_empty() {
        out.extend_from_slice(b"[]");
        return Ok(());
    }
    out.push(b'[');
    out.extend_from_slice(&o.array_nl);
    let mut i = 0;
    while let Some(&x) = rary(a).v.get(i) {
        if i > 0 {
            out.push(b',');
            out.extend_from_slice(&o.array_nl);
        }
        indent(out, o, depth);
        gen_value(x, o, st, depth, out)?;
        i += 1;
    }
    out.extend_from_slice(&o.array_nl);
    indent(out, o, depth - 1);
    out.push(b']');
    Ok(())
}

/// Generate `v` with options; `depth` starts at the state's depth.
fn generate(v: V, o: &Opts, st: V) -> V {
    let mut out = Vec::new();
    match gen_value(v, o, st, o.depth, &mut out) {
        Ok(()) => str_new(out),
        Err(e) => e,
    }
}

/// `x.to_json(state_or_opts = nil)` for the core classes.
fn to_json_native(s: V, a: &[V], _: V) -> V {
    let (o, st) = opts_arg(a.first().copied(), Opts::default());
    let mut out = Vec::new();
    // generate `s` itself natively even if a subclass overrides to_json (we may be its super)
    let r = match kind(s) {
        Some(Kind::Hash) => gen_hash(s, &o, st, o.depth + 1, &mut out),
        Some(Kind::Array) => gen_array(s, &o, st, o.depth + 1, &mut out),
        Some(Kind::String) => escape_str(&rstr(s).b, &o, &mut out),
        Some(Kind::Float) => gen_float(float_val(s), &o, &mut out),
        Some(Kind::Bignum) => {
            out.extend_from_slice(rbig(s).n.to_string().as_bytes());
            Ok(())
        }
        _ if is_fix(s) || s == QNIL || s == QTRUE || s == QFALSE || is_sym(s) => {
            let o2 = Opts { strict: false, ..o.clone() };
            gen_native(s, &o2, st, o.depth, &mut out)
        }
        _ => match crate::fmt::to_s(s) {
            Ok(b) => escape_str(&b, &o, &mut out),
            Err(()) => Err(UNDEF),
        },
    };
    match r {
        Ok(()) => str_new(out),
        Err(e) => e,
    }
}

// ---------- parser ----------

struct PO {
    symbolize: bool,
    freeze: bool,
    allow_nan: bool,
    max_nesting: i64,
    object_class: V,
    array_class: V,
}
struct P<'a> {
    b: &'a [u8],
    i: usize,
    o: PO,
}

fn perr(msg: &str) -> V {
    raise(jc().parser_error, msg)
}
impl P<'_> {
    fn err_at(&self, at: usize) -> V {
        if at >= self.b.len() {
            return perr("unexpected end of input");
        }
        let end = (at + 32).min(self.b.len());
        perr(&format!("unexpected token at '{}'", String::from_utf8_lossy(&self.b[at..end])))
    }
    fn ws(&mut self) -> Result<(), V> {
        loop {
            match self.b.get(self.i) {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.i += 1,
                Some(b'/') => match self.b.get(self.i + 1) {
                    Some(b'*') => {
                        let start = self.i;
                        self.i += 2;
                        loop {
                            if self.i + 1 >= self.b.len() {
                                return Err(self.err_at(start));
                            }
                            if self.b[self.i] == b'*' && self.b[self.i + 1] == b'/' {
                                self.i += 2;
                                break;
                            }
                            self.i += 1;
                        }
                    }
                    Some(b'/') => {
                        while self.i < self.b.len() && self.b[self.i] != b'\n' {
                            self.i += 1;
                        }
                    }
                    _ => return Ok(()),
                },
                _ => return Ok(()),
            }
        }
    }
    fn lit(&mut self, w: &[u8]) -> bool {
        if self.b[self.i..].starts_with(w) {
            self.i += w.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: i64) -> Result<V, V> {
        self.ws()?;
        let Some(&c) = self.b.get(self.i) else { return Err(self.err_at(self.i)) };
        let start = self.i;
        let v = match c {
            b'{' => self.object(depth + 1)?,
            b'[' => self.array(depth + 1)?,
            b'"' => {
                let s = self.string()?;
                let v = str_new(s);
                if self.o.freeze {
                    hdr(v).flags |= FL_FROZEN;
                }
                v
            }
            b'-' | b'0'..=b'9' => {
                if self.lit(b"-Infinity") {
                    if !self.o.allow_nan {
                        return Err(self.err_at(start));
                    }
                    float_new(f64::NEG_INFINITY)
                } else {
                    self.number()?
                }
            }
            b't' if self.lit(b"true") => QTRUE,
            b'f' if self.lit(b"false") => QFALSE,
            b'n' if self.lit(b"null") => QNIL,
            b'N' if self.lit(b"NaN") => {
                if !self.o.allow_nan {
                    return Err(self.err_at(start));
                }
                float_new(f64::NAN)
            }
            b'I' if self.lit(b"Infinity") => {
                if !self.o.allow_nan {
                    return Err(self.err_at(start));
                }
                float_new(f64::INFINITY)
            }
            _ => return Err(self.err_at(start)),
        };
        Ok(v)
    }
    fn number(&mut self) -> Result<V, V> {
        let b = self.b;
        let start = self.i;
        let mut i = self.i;
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        match b.get(i) {
            Some(b'0') => {
                i += 1;
                if b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    return Err(self.err_at(start));
                }
            }
            Some(c) if c.is_ascii_digit() => {
                while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    i += 1;
                }
            }
            _ => return Err(self.err_at(start)),
        }
        let mut float = false;
        if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
            float = true;
            i += 1;
            while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(b.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if b.get(j).is_some_and(|c| c.is_ascii_digit()) {
                float = true;
                while b.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    j += 1;
                }
                i = j;
            }
        }
        self.i = i;
        let text = std::str::from_utf8(&b[start..i]).unwrap();
        if float {
            return Ok(float_new(text.parse::<f64>().unwrap_or(0.0)));
        }
        if i - start < 18 {
            return Ok(mkfix(text.parse::<i64>().unwrap_or(0)));
        }
        Ok(int_from_big(text.parse::<BigInt>().unwrap_or_default()))
    }
    fn hex4(&self, at: usize) -> Option<u32> {
        let h = self.b.get(at..at + 4)?;
        u32::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()
    }
    fn string(&mut self) -> Result<Vec<u8>, V> {
        let start = self.i;
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err(self.err_at(start)) };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let Some(&e) = self.b.get(self.i + 1) else { return Err(self.err_at(start)) };
                    self.i += 2;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let Some(mut cp) = self.hex4(self.i) else { return Err(self.err_at(start)) };
                            self.i += 4;
                            if (0xd800..0xdc00).contains(&cp) {
                                if self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                                    if let Some(lo) = self.hex4(self.i + 2).filter(|lo| (0xdc00..0xe000).contains(lo)) {
                                        cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                        self.i += 6;
                                    }
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c if c < 0x20 => return Err(self.err_at(start)),
                _ => {
                    // copy a run of plain bytes
                    let run_start = self.i;
                    while let Some(&d) = self.b.get(self.i) {
                        if d == b'"' || d == b'\\' || d < 0x20 {
                            break;
                        }
                        self.i += 1;
                    }
                    out.extend_from_slice(&self.b[run_start..self.i]);
                }
            }
        }
    }
    fn nest(&self, depth: i64) -> Result<(), V> {
        if self.o.max_nesting != 0 && depth > self.o.max_nesting {
            return Err(raise(jc().nesting_error, &format!("nesting of {depth} is too deep")));
        }
        Ok(())
    }
    fn array(&mut self, depth: i64) -> Result<V, V> {
        self.nest(depth)?;
        let start = self.i;
        self.i += 1;
        let custom = self.o.array_class != QNIL;
        let arr = if custom { send(self.o.array_class, sym("new"), &[], QNIL) } else { ary_new(Vec::new()) };
        if arr == UNDEF {
            return Err(UNDEF);
        }
        self.ws()?;
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
        } else {
            loop {
                let v = self.value(depth)?;
                if custom {
                    if send(arr, sym("<<"), &[v], QNIL) == UNDEF {
                        return Err(UNDEF);
                    }
                } else {
                    rary(arr).v.push(v);
                }
                self.ws()?;
                match self.b.get(self.i) {
                    Some(b',') => {
                        self.i += 1;
                        self.ws()?;
                        if self.b.get(self.i) == Some(&b']') {
                            return Err(self.err_at(start));
                        }
                    }
                    Some(b']') => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(self.err_at(start)),
                }
            }
        }
        if self.o.freeze && !custom {
            hdr(arr).flags |= FL_FROZEN;
        }
        Ok(arr)
    }
    fn object(&mut self, depth: i64) -> Result<V, V> {
        self.nest(depth)?;
        let start = self.i;
        self.i += 1;
        let custom = self.o.object_class != QNIL;
        let h = if custom { send(self.o.object_class, sym("new"), &[], QNIL) } else { hash_new() };
        if h == UNDEF {
            return Err(UNDEF);
        }
        self.ws()?;
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
        } else {
            loop {
                self.ws()?;
                if self.b.get(self.i) != Some(&b'"') {
                    return Err(self.err_at(start));
                }
                let kb = self.string()?;
                let k = if self.o.symbolize {
                    mksym(intern(&kb))
                } else {
                    let k = str_new(kb);
                    hdr(k).flags |= FL_FROZEN;
                    k
                };
                self.ws()?;
                if self.b.get(self.i) != Some(&b':') {
                    return Err(self.err_at(start));
                }
                self.i += 1;
                let v = self.value(depth)?;
                if custom {
                    if send(h, sym("[]="), &[k, v], QNIL) == UNDEF {
                        return Err(UNDEF);
                    }
                } else {
                    rhash(h).m.insert(HKey(k), v);
                }
                self.ws()?;
                match self.b.get(self.i) {
                    Some(b',') => {
                        self.i += 1;
                    }
                    Some(b'}') => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(self.err_at(start)),
                }
            }
        }
        if self.o.freeze && !custom {
            hdr(h).flags |= FL_FROZEN;
        }
        Ok(h)
    }
}

fn source_bytes(src: V) -> Result<Vec<u8>, V> {
    if is_str(src) {
        return Ok(rstr(src).b.clone());
    }
    if src != QNIL && respond_to(src, sym("to_str")) {
        let s = send0(src, "to_str");
        if s == UNDEF {
            return Err(UNDEF);
        }
        if is_str(s) {
            return Ok(rstr(s).b.clone());
        }
    }
    let name = match src {
        QNIL => "nil".to_string(),
        QTRUE => "true".to_string(),
        QFALSE => "false".to_string(),
        _ => class_name(real_class(src)),
    };
    Err(type_error(&format!("no implicit conversion of {name} into String")))
}

fn parse_opts(h: Option<V>, base: PO) -> PO {
    let mut o = base;
    if let Some(h) = h.filter(|h| is_hash(*h)) {
        if let Some(v) = opt_get(h, "symbolize_names") {
            o.symbolize = truthy(v);
        }
        if let Some(v) = opt_get(h, "freeze") {
            o.freeze = truthy(v);
        }
        if let Some(v) = opt_get(h, "allow_nan") {
            o.allow_nan = truthy(v);
        }
        if let Some(v) = opt_get(h, "max_nesting") {
            o.max_nesting = nesting_of(v);
        }
        if let Some(v) = opt_get(h, "object_class") {
            o.object_class = v;
        }
        if let Some(v) = opt_get(h, "array_class") {
            o.array_class = v;
        }
    }
    o
}
fn default_po() -> PO {
    PO { symbolize: false, freeze: false, allow_nan: false, max_nesting: 100, object_class: QNIL, array_class: QNIL }
}

fn parse_bytes(b: &[u8], o: PO) -> V {
    let mut p = P { b, i: 0, o };
    // skip a UTF-8 BOM
    if p.b.starts_with(&[0xef, 0xbb, 0xbf]) {
        p.i = 3;
    }
    let r = (|| {
        let v = p.value(0)?;
        p.ws()?;
        if p.i < p.b.len() {
            return Err(p.err_at(p.i));
        }
        Ok(v)
    })();
    match r {
        Ok(v) => v,
        Err(e) => e,
    }
}

fn parse_impl(src: V, opts: Option<V>, base: PO) -> V {
    let b = match source_bytes(src) {
        Ok(b) => b,
        Err(e) => return e,
    };
    parse_bytes(&b, parse_opts(opts, base))
}

fn recurse_proc(v: V, pr: V) -> V {
    if is_ary(v) {
        let mut i = 0;
        while let Some(&x) = rary(v).v.get(i) {
            tri!(recurse_proc(x, pr));
            i += 1;
        }
    } else if is_hash(v) {
        let mut i = 0;
        while let Some((k, x)) = rhash(v).m.get_index(i).map(|(k, x)| (k.0, *x)) {
            tri!(recurse_proc(k, pr));
            tri!(recurse_proc(x, pr));
            i += 1;
        }
    }
    send1(pr, "call", v)
}

// ---------- module setup ----------

fn last_hash(a: &[V]) -> Option<V> {
    a.last().copied().filter(|h| is_hash(*h))
}

fn init_state(state: *mut Class) {
    defn(state, "initialize", 0, 1, |s, a, _| {
        let mut o = Opts::default();
        if let Some(h) = a.first().copied().filter(|h| is_hash(*h)) {
            o.merge_hash(h);
        }
        o.store(s);
        QNIL
    });
    defs(state, "from_state", 1, 1, |c, a, _| {
        let v = a[0];
        if is_heap(v) && is_a(v, jc().state) {
            return v;
        }
        let args: Vec<V> = if is_hash(v) { vec![v] } else { vec![] };
        send(c, sym("new"), &args, QNIL)
    });
    defs(state, "generate", 1, 3, |c, a, _| {
        let st = tri!(send(c, sym("new"), &a[1..a.len().min(2)], QNIL));
        generate(a[0], &Opts::from_state(st), st)
    });
    fn configure(s: V, a: &[V]) -> V {
        let mut o = Opts::from_state(s);
        if is_hash(a[0]) {
            o.merge_hash(a[0]);
        } else if respond_to(a[0], sym("to_h")) {
            let h = tri!(send0(a[0], "to_h"));
            if is_hash(h) {
                o.merge_hash(h);
            }
        }
        o.store(s);
        s
    }
    defn(state, "configure", 1, 1, |s, a, _| configure(s, a));
    defn(state, "merge", 1, 1, |s, a, _| configure(s, a));
    defn(state, "generate", 1, 2, |s, a, _| {
        let r = generate(a[0], &Opts::from_state(s), s);
        if r != UNDEF && a.len() > 1 && a[1] != QNIL {
            tri!(send1(a[1], "write", r));
            return a[1];
        }
        r
    });
    fn to_h(s: V) -> V {
        let h = hash_new();
        for k in STR_KEYS.iter().chain(BOOL_KEYS.iter()).chain(["max_nesting", "depth"].iter()) {
            crate::hash::hset(h, mksym(sym(k)), ivar_get(s, sym(&format!("@__{k}"))));
        }
        crate::hash::hset(h, mksym(sym("escape_slash")), ivar_get(s, sym("@__script_safe")));
        crate::hash::hset(h, mksym(sym("buffer_initial_length")), mkfix(1024));
        h
    }
    defn(state, "to_h", 0, 0, |s, _, _| to_h(s));
    defn(state, "to_hash", 0, 0, |s, _, _| to_h(s));
    defn(state, "[]", 1, 1, |s, a, _| {
        let name = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).into_owned();
        let iv = sym(&format!("@__{name}"));
        if ivar_defined(s, iv) {
            ivar_get(s, iv)
        } else {
            ivar_get(s, sym(&format!("@{name}")))
        }
    });
    defn(state, "[]=", 2, 2, |s, a, _| {
        let name = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).into_owned();
        let h = hash_new();
        crate::hash::hset(h, mksym(sym(&name)), a[1]);
        let mut o = Opts::from_state(s);
        o.merge_hash(h);
        o.store(s);
        if !STR_KEYS.contains(&name.as_str()) && !BOOL_KEYS.contains(&name.as_str()) && name != "max_nesting" && name != "depth" {
            ivar_set(s, sym(&format!("@{name}")), a[1]);
        }
        a[1]
    });
    for k in STR_KEYS.iter().chain(BOOL_KEYS.iter()).chain(["max_nesting", "depth"].iter()) {
        // readers / writers built from the key name
        let iv = sym(&format!("@__{k}"));
        let rd = proc_new_native(Box::new(move |s, _, _| ivar_get(s, iv)), 0, 0);
        add_method(state, sym(k), Body::Proc(rd));
        let is_bool = BOOL_KEYS.contains(k);
        let is_str_key = STR_KEYS.contains(k);
        let wr = proc_new_native(
            Box::new(move |s, a, _| {
                let v = if is_bool {
                    bool_v(truthy(a[0]))
                } else if is_str_key {
                    if a[0] == QNIL { str_new(vec![]) } else { str_new(bytes_of(a[0])) }
                } else {
                    mkfix(nesting_of(a[0]))
                };
                ivar_set(s, iv, v);
                a[0]
            }),
            1,
            1,
        );
        add_method(state, sym(&format!("{k}=")), Body::Proc(wr));
        if is_bool {
            let q = proc_new_native(Box::new(move |s, _, _| bool_v(truthy(ivar_get(s, iv)))), 0, 0);
            add_method(state, sym(&format!("{k}?")), Body::Proc(q));
        }
    }
    defn(state, "escape_slash?", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__script_safe")))));
    defn(state, "check_circular?", 0, 0, |s, _, _| bool_v(nesting_of(ivar_get(s, sym("@__max_nesting"))) != 0));
    defn(state, "buffer_initial_length", 0, 0, |_, _, _| mkfix(1024));
    defn(state, "buffer_initial_length=", 1, 1, |_, a, _| a[0]);
}

fn init_json() {
    let r = rt();
    let json = new_module("JSON");
    set_global_const("JSON", json as V);
    let json_error = new_class("JSON::JSONError", r.c.standard_error);
    const_set(json, sym("JSONError"), json_error as V);
    let parser_error = new_class("JSON::ParserError", json_error);
    const_set(json, sym("ParserError"), parser_error as V);
    let nesting_error = new_class("JSON::NestingError", parser_error);
    const_set(json, sym("NestingError"), nesting_error as V);
    let generator_error = new_class("JSON::GeneratorError", json_error);
    const_set(json, sym("GeneratorError"), generator_error as V);
    const_set(json, sym("UnparserError"), generator_error as V);
    const_set(json, sym("PARSE_ERROR"), parser_error as V);
    const_set(json, sym("VERSION"), str_from("2.9.1"));
    for (n, v) in [("VERSION_MAJOR", 2), ("VERSION_MINOR", 9), ("VERSION_BUILD", 1)] {
        const_set(json, sym(n), mkfix(v));
    }
    const_set(json, sym("NaN"), float_new(f64::NAN));
    const_set(json, sym("Infinity"), float_new(f64::INFINITY));
    const_set(json, sym("MinusInfinity"), float_new(f64::NEG_INFINITY));

    let ext = new_module("JSON::Ext");
    const_set(json, sym("Ext"), ext as V);
    let genm = new_module("JSON::Ext::Generator");
    const_set(ext, sym("Generator"), genm as V);
    let state = new_class("JSON::Ext::Generator::State", r.c.object);
    const_set(genm, sym("State"), state as V);
    const_set(json, sym("State"), state as V);
    let parser = new_class("JSON::Ext::Parser", r.c.object);
    const_set(ext, sym("Parser"), parser as V);
    const_set(json, sym("Parser"), parser as V);
    *JCS.get() = Some(JC { parser_error, nesting_error, generator_error, state, parser, generator: genm });
    init_state(state);

    defn(parser, "initialize", 1, 2, |s, a, _| {
        let b = match source_bytes(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        ivar_set(s, sym("@__src"), str_new(b));
        ivar_set(s, sym("@__opts"), a.get(1).copied().unwrap_or(QNIL));
        QNIL
    });
    defn(parser, "source", 0, 0, |s, _, _| ivar_get(s, sym("@__src")));
    defn(parser, "parse", 0, 0, |s, _, _| {
        let src = ivar_get(s, sym("@__src"));
        parse_impl(src, Some(ivar_get(s, sym("@__opts"))), default_po())
    });
    defs(parser, "parse", 1, 2, |_, a, _| parse_impl(a[0], a.get(1).copied(), default_po()));

    defs(json, "parse", 1, 2, |_, a, _| parse_impl(a[0], a.get(1).copied(), default_po()));
    defs(json, "parse!", 1, 2, |_, a, _| {
        parse_impl(a[0], a.get(1).copied(), PO { allow_nan: true, max_nesting: 0, ..default_po() })
    });
    fn load(a: &[V]) -> V {
        let mut src = a[0];
        if src == QNIL {
            return QNIL;
        }
        if !is_str(src) && !respond_to(src, sym("to_str")) {
            if respond_to(src, sym("to_io")) {
                src = tri!(send0(src, "to_io"));
            }
            if respond_to(src, sym("read")) {
                src = tri!(send0(src, "read"));
            }
        }
        let b = match source_bytes(src) {
            Ok(b) => b,
            Err(e) => return e,
        };
        if b.iter().all(|c| c.is_ascii_whitespace()) {
            return QNIL;
        }
        let pr = a.get(1).copied().filter(|p| *p != QNIL && !is_hash(*p));
        let v = tri!(parse_bytes(&b, parse_opts(last_hash(&a[1..]), PO { allow_nan: true, max_nesting: 0, ..default_po() })));
        if let Some(pr) = pr {
            tri!(recurse_proc(v, pr));
        }
        v
    }
    defs(json, "load", 1, 3, |_, a, _| load(a));
    defs(json, "unsafe_load", 1, 3, |_, a, _| load(a));
    defs(json, "restore", 1, 3, |_, a, _| load(a));
    defs(json, "load_file", 1, 2, |_, a, _| {
        let path = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).into_owned();
        match crate::vfs::read_file(path.as_bytes()) {
            Ok(b) => parse_bytes(&b, parse_opts(a.get(1).copied(), default_po())),
            Err(_) => {
                let errno = const_lookup(rt().c.object, sym("Errno")).unwrap() as *mut Class;
                let enoent = const_get_under(errno, sym("ENOENT")).unwrap() as *mut Class;
                raise(enoent, &format!("No such file or directory @ rb_sysopen - {path}"))
            }
        }
    });
    fn gen_with(a: &[V], base: Opts) -> V {
        let (o, st) = opts_arg(a.get(1).copied().filter(|x| *x != QNIL), base);
        generate(a[0], &o, st)
    }
    defs(json, "generate", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "unparse", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "fast_generate", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "pretty_generate", 1, 2, |_, a, _| gen_with(a, pretty()));
    defs(json, "pretty_unparse", 1, 2, |_, a, _| gen_with(a, pretty()));
    defs(json, "dump", 1, 4, |_, a, _| {
        // JSON.dump(obj, io = nil, limit = nil, kwargs = nil)
        let mut pos: Vec<V> = vec![a[0]];
        pos.extend(a[1..].iter().copied().filter(|x| !is_hash(*x)));
        let mut o = Opts { allow_nan: true, max_nesting: 0, ..Opts::default() };
        if let Some(h) = last_hash(&a[1..]) {
            o.merge_hash(h);
        }
        if let Some(&lim) = pos.get(2).or(pos.get(1).filter(|x| is_fix(**x))) {
            if is_fix(lim) {
                o.max_nesting = fix(lim);
            }
        }
        let st = state_new(&o);
        let s = tri!(generate(pos[0], &o, st));
        match pos.get(1) {
            Some(&io) if !is_fix(io) && io != QNIL && respond_to(io, sym("write")) => {
                tri!(send1(io, "write", s));
                io
            }
            _ => s,
        }
    });
    defs(json, "[]", 1, 2, |_, a, _| {
        if is_str(a[0]) {
            parse_impl(a[0], a.get(1).copied(), default_po())
        } else {
            gen_with(a, Opts::default())
        }
    });
    defs(json, "state", 0, 0, |_, _, _| jc().state as V);
    defs(json, "generator", 0, 0, |_, _, _| jc().generator as V);
    defs(json, "parser", 0, 0, |_, _, _| jc().parser as V);
    defs(json, "create_id", 0, 0, |_, _, _| str_from("json_class"));
    defs(json, "dump_default_options", 0, 0, |_, _, _| {
        let h = hash_new();
        crate::hash::hset(h, mksym(sym("max_nesting")), QFALSE);
        crate::hash::hset(h, mksym(sym("allow_nan")), QTRUE);
        h
    });
    defs(json, "load_default_options", 0, 0, |_, _, _| {
        let h = hash_new();
        crate::hash::hset(h, mksym(sym("max_nesting")), QFALSE);
        crate::hash::hset(h, mksym(sym("allow_nan")), QTRUE);
        h
    });

    // Kernel#JSON
    defn(r.c.kernel, "JSON", 1, 2, |_, a, _| {
        if is_str(a[0]) {
            parse_impl(a[0], a.get(1).copied(), default_po())
        } else {
            let (o, st) = opts_arg(a.get(1).copied(), Opts::default());
            generate(a[0], &o, st)
        }
    });

    // as in the json gem, the core classes' to_json comes from included modules, so a module
    // included later (ActiveSupport's ToJsonWithActiveSupportEncoder in Rails 8) overrides it
    // and reaches it through `super`
    let to_json = sym("to_json");
    let mut builtin = Vec::new();
    let gm = new_module("JSON::Ext::Generator::GeneratorMethods");
    const_set(genm, sym("GeneratorMethods"), gm as V);
    for (name, c) in [
        ("Object", r.c.object),
        ("Hash", r.c.hash),
        ("Array", r.c.array),
        ("String", r.c.string),
        ("Integer", r.c.integer),
        ("Float", r.c.float),
        ("NilClass", r.c.nil),
        ("TrueClass", r.c.true_),
        ("FalseClass", r.c.false_),
    ] {
        let m = new_module(&format!("JSON::Ext::Generator::GeneratorMethods::{name}"));
        const_set(gm, sym(name), m as V);
        builtin.push(add_method(m, to_json, Body::Native(to_json_native, 0, -1)));
        crate::kernel::do_include(c, m as V);
    }
    *BUILTIN_TO_JSON.get() = builtin;
}

pub fn init() {
    crate::compat::provide_feature(
        &["json", "json/ext", "json/common", "json/ext/parser", "json/ext/generator", "json/pure", "json/generic_object"],
        Some(init_json),
    );
}
