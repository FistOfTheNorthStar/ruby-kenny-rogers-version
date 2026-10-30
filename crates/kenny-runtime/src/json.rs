//! JSON (the `json` gem's C extension): parser, generator, JSON::Ext::Generator::State and
//! `#to_json` on the core classes.
//!
//! The behaviour follows the json version the program gets: the one in its Gemfile.lock, else
//! the one bundled with the Ruby it is built against (2.9.1 in Ruby 3.4). json 2.10 and later
//! ("modern") have a new parser (json_parser3.rs: other error messages, `allow_trailing_comma`,
//! `JSON::Coder`, `JSON::Fragment` ...); json 3.0 rejects comments, duplicate keys and unknown
//! options by default and drops the old API. lib/kenny/json.rb adds the Ruby-level parts.
//!
//! The generator handles core values natively. Any value whose `to_json` is not one of the
//! built-in methods (a user/ActiveSupport override) gets `to_json(state)` called, with the
//! state's `depth` set, so overrides that call `super(state)` nest correctly.

use crate::core::*;
use crate::tri;
use num_bigint::BigInt;

#[path = "json_parser3.rs"]
mod parser3;

/// The json version, as major * 10000 + minor * 100 + patch.
static VER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(20901);
pub(crate) fn ver() -> u32 {
    VER.load(std::sync::atomic::Ordering::Relaxed)
}
pub(crate) const V2_10: u32 = 21000;
pub(crate) const V2_12: u32 = 21200;
pub(crate) const V2_14: u32 = 21400;
pub(crate) const V3_0: u32 = 30000;
pub(crate) const V3_1: u32 = 30100;
fn modern() -> bool {
    ver() >= V2_10
}
fn parse_ver(s: &str) -> Option<u32> {
    let mut it = s.split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().unwrap_or(0));
    let (a, b, c) = (it.next()?, it.next().unwrap_or(0), it.next().unwrap_or(0));
    Some(a * 10000 + b.min(99) * 100 + c.min(99))
}
/// The json gem's version: `Gem.loaded_specs["json"]` (from the lock), else the Ruby's own.
fn detect_version() -> String {
    let default = if crate::misc::ruby4() { "2.18.0" } else { "2.9.1" };
    let saved = pst().pending;
    let r = (|| {
        let gem = const_lookup(rt().c.object, sym("Gem"))?;
        if !respond_to(gem, sym("loaded_specs")) {
            return None;
        }
        let specs = send0(gem, "loaded_specs");
        if !is_hash(specs) {
            return None;
        }
        let spec = crate::hash::hget(specs, str_from("json"));
        if spec == QNIL || spec == UNDEF {
            return None;
        }
        let v = send0(spec, "version");
        if v == UNDEF {
            return None;
        }
        let s = crate::fmt::to_s(v).ok()?;
        Some(String::from_utf8_lossy(&s).into_owned())
    })();
    pst().pending = saved;
    r.filter(|v| parse_ver(v).is_some()).unwrap_or_else(|| default.to_string())
}

struct JC {
    parser_error: *mut Class,
    nesting_error: *mut Class,
    generator_error: *mut Class,
    state: *mut Class,
    parser: *mut Class,
    generator: *mut Class,
    json: *mut Class,
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
    // json 2.10+
    /// strict mode's converter for other objects (a Proc), or QFALSE
    as_json: V,
    /// a Proc that orders a Hash's pairs, or QFALSE
    sort_keys: V,
    rfc8785: bool,
    allow_duplicate_key: bool,
    buffer_initial_length: i64,
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
            as_json: QFALSE,
            sort_keys: QFALSE,
            rfc8785: false,
            allow_duplicate_key: false,
            buffer_initial_length: 1024,
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
            as_json: if modern() && truthy(g("as_json")) { g("as_json") } else { QFALSE },
            sort_keys: if modern() && truthy(g("sort_keys")) { g("sort_keys") } else { QFALSE },
            rfc8785: modern() && truthy(g("rfc8785")),
            allow_duplicate_key: modern() && truthy(g("allow_duplicate_key")),
            buffer_initial_length: { let b = g("buffer_initial_length"); if is_fix(b) { fix(b) } else { 1024 } },
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
        if modern() {
            set("as_json", self.as_json);
            set("sort_keys", self.sort_keys);
            set("rfc8785", bool_v(self.rfc8785));
            set("allow_duplicate_key", bool_v(self.allow_duplicate_key));
            set("buffer_initial_length", mkfix(self.buffer_initial_length));
        }
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

// ---------- generator of json 2.10+ ----------

const FL_ASCII: u32 = crate::compat::FL_BINARY << 1;

fn gen_error_obj(msg: &[u8], invalid: V) -> V {
    let e = object_new(jc().generator_error);
    ivar_set(e, sym("@__message"), str_new(msg.to_vec()));
    ivar_set(e, sym("@invalid_object"), invalid);
    raise_exc(e)
}
fn gen_error3(msg: &str, invalid: V) -> V {
    gen_error_obj(msg.as_bytes(), invalid)
}
/// `CLASS_OF(v)` as an error message shows it (a singleton class if `v` has one).
fn class_label(v: V) -> String {
    String::from_utf8_lossy(&crate::fmt::to_s(class_of(v) as V).unwrap_or_default()).into_owned()
}
fn wrong_type(v: V, expected: &str) -> V {
    type_error(&format!("wrong argument type {} (expected {expected})", class_name(real_class(v))))
}
fn call_proc(p: V, args: &[V]) -> V {
    send(p, sym("call"), args, QNIL)
}
/// The JSON::Fragment class, once lib/kenny/json.rb has defined it.
static FRAGMENT: G<V> = G::new(QNIL);
fn fragment_class() -> V {
    let f = *FRAGMENT.get();
    if f != QNIL {
        return f;
    }
    match const_get_under(jc().json, sym("Fragment")) {
        Some(c) if is_class(c) => {
            *FRAGMENT.get() = c;
            c
        }
        _ => QNIL,
    }
}
/// Valid as it is: ASCII, or valid UTF-8 in a UTF-8 String.
fn valid_json_str(s: V) -> bool {
    let b = &rstr(s).b;
    if b.is_ascii() {
        return true;
    }
    let f = hdr(s).flags;
    if f & FL_ASCII != 0 {
        return false;
    }
    if f & crate::compat::FL_BINARY != 0 {
        // json < 3.1 still took BINARY Strings holding UTF-8 (with a deprecation warning)
        if ver() >= V3_1 || crate::transcode::enc_of(s).codec != crate::transcode::Codec::Binary {
            return false;
        }
    }
    std::str::from_utf8(b).is_ok()
}

/// fpconv_dtoa's format (json 2.12+): shortest digits, scientific notation only for very
/// large or small numbers, without Float#to_s's ".0" before an exponent.
fn fmt_float3(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let neg = f < 0.0;
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant.bytes().filter(|c| *c != b'.').collect();
    let nd = digits.len() as i32;
    let k = exp - (nd - 1);
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let ds = |a: usize, b: usize| String::from_utf8_lossy(&digits[a..b]).into_owned();
    let aexp = (k + nd - 1).abs();
    if k >= 0 && aexp < 15 {
        out += &ds(0, nd as usize);
        out += &"0".repeat(k as usize);
        out += ".0";
        return out;
    }
    if k < 0 && (k > -7 || aexp < 10) {
        let offset = nd + k;
        if offset <= 0 {
            out += "0.";
            out += &"0".repeat((-offset) as usize);
            out += &ds(0, nd as usize);
        } else {
            out += &ds(0, offset as usize);
            out.push('.');
            out += &ds(offset as usize, nd as usize);
        }
        return out;
    }
    let nd = nd.min(18 - neg as i32) as usize;
    out += &ds(0, 1);
    if nd > 1 {
        out.push('.');
        out += &ds(1, nd);
    }
    out.push('e');
    out.push(if k + nd as i32 - 1 < 0 { '-' } else { '+' });
    out += &aexp.to_string();
    out
}

struct G3<'a> {
    o: &'a Opts,
    /// the State passed to `to_json` calls (made when first needed)
    vstate: V,
    depth: i64,
    out: Vec<u8>,
}

impl G3<'_> {
    fn new(o: &Opts) -> G3<'_> {
        G3 { o, vstate: QNIL, depth: o.depth, out: Vec::new() }
    }
    fn indent(&mut self, n: i64) {
        if !self.o.indent.is_empty() {
            for _ in 0..n {
                self.out.extend_from_slice(&self.o.indent);
            }
        }
    }
    fn depth_up(&mut self) -> Result<i64, V> {
        if self.o.max_nesting != 0 && self.depth >= self.o.max_nesting {
            return Err(raise(
                jc().nesting_error,
                &format!(
                    "nesting of {} is too deep. Did you try to serialize objects with circular references?",
                    if ver() >= V3_0 { self.depth + 1 } else { self.depth }
                ),
            ));
        }
        self.depth += 1;
        Ok(self.depth)
    }
    fn as_json(&self, v: V, is_key: bool) -> Result<V, V> {
        let r = call_proc(self.o.as_json, &[v, bool_v(is_key)]);
        if r == UNDEF { Err(UNDEF) } else { Ok(r) }
    }
    /// A String already checked to be ASCII or valid UTF-8.
    fn raw_str(&mut self, s: V) -> Result<(), V> {
        escape_str(&rstr(s).b, self.o, &mut self.out)
    }
    /// A String to generate in place of `s`, whose encoding isn't valid for JSON.
    fn convert_encoding(&mut self, s: V, as_json_called: bool, is_key: bool) -> Result<V, V> {
        if !as_json_called && self.o.strict && truthy(self.o.as_json) {
            let c = self.as_json(s, false)?;
            if c != s {
                if is_str(c) {
                    if !valid_json_str(c) {
                        return Err(gen_error3("source sequence is illegal/malformed utf-8", s));
                    }
                } else if is_key {
                    return Err(gen_error3(&format!("{} not allowed as object key in JSON", class_label(c)), c));
                }
                return Ok(c);
            }
        }
        let r = send(s, sym("encode"), &[str_from("UTF-8")], QNIL);
        if r == UNDEF {
            if let Throw::Raise(e) = pst().pending {
                if is_a(e, rt().c.standard_error) {
                    pst().pending = Throw::None;
                    let m = send0(e, "message");
                    let m = if is_str(m) { rstr(m).b.to_vec() } else { Vec::new() };
                    return Err(gen_error_obj(&m, s));
                }
            }
            return Err(UNDEF);
        }
        if !is_str(r) {
            return Err(wrong_type(r, "String"));
        }
        if !valid_json_str(r) {
            return Err(gen_error3("source sequence is illegal/malformed utf-8", r));
        }
        Ok(r)
    }
    fn ensure_valid(&mut self, s: V, as_json_called: bool, is_key: bool) -> Result<V, V> {
        if valid_json_str(s) { Ok(s) } else { self.convert_encoding(s, as_json_called, is_key) }
    }
    fn rfc_number(&mut self, v: V) -> Result<(), V> {
        let p = ivar_get(jc().state as V, sym("@__rfc8785_number_formatter_proc"));
        let r = call_proc(p, &[v]);
        if r == UNDEF {
            return Err(UNDEF);
        }
        if !is_str(r) {
            return Err(wrong_type(r, "String"));
        }
        self.out.extend_from_slice(&rstr(r).b);
        Ok(())
    }
    fn float(&mut self, v: V) -> Result<(), V> {
        let f = float_val(v);
        if f.is_nan() || f.is_infinite() {
            if !self.o.allow_nan {
                if self.o.strict && truthy(self.o.as_json) {
                    let c = self.as_json(v, false)?;
                    if c != v {
                        self.depth_up()?;
                        self.value(c, true)?;
                        self.depth -= 1;
                        return Ok(());
                    }
                }
                return Err(gen_error3(&format!("{} not allowed in JSON", crate::fmt::float_to_s(f)), v));
            }
            self.out.extend_from_slice(crate::fmt::float_to_s(f).as_bytes());
            return Ok(());
        }
        if self.o.rfc8785 {
            return self.rfc_number(v);
        }
        if ver() >= V2_12 {
            self.out.extend_from_slice(fmt_float3(f).as_bytes());
        } else {
            self.out.extend_from_slice(crate::fmt::float_to_s(f).as_bytes());
        }
        Ok(())
    }
    /// `obj.to_json(state)` (the C generator's fallback for other objects).
    fn fallback(&mut self, v: V) -> Result<(), V> {
        let m = find_method(class_of(v), sym("to_json"));
        if !m.is_null() && BUILTIN_TO_JSON.get().contains(&m) {
            // JSON::GeneratorMethods#to_json, without the call
            return self.value(to_json_target(v)?, false);
        }
        if !m.is_null() {
            if self.vstate == QNIL {
                self.vstate = state_new(self.o);
            }
            ivar_set(self.vstate, sym("@__depth"), mkfix(self.depth));
            let r = send(v, sym("to_json"), &[self.vstate], QNIL);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if !is_str(r) {
                return Err(wrong_type(r, "String"));
            }
            self.out.extend_from_slice(&rstr(r).b);
            return Ok(());
        }
        let s = str_new(crate::fmt::to_s(v).map_err(|_| UNDEF)?);
        let s = self.ensure_valid(s, false, false)?;
        self.raw_str(s)
    }
    /// generate_json_general: `fallback` calls `to_json` on subclasses of the core classes.
    fn value(&mut self, v: V, fallback: bool) -> Result<(), V> {
        let core = &rt().c;
        let mut obj = v;
        let mut as_json_called = false;
        loop {
            let mut string = false;
            if obj == QNIL {
                self.out.extend_from_slice(b"null");
                return Ok(());
            } else if obj == QFALSE {
                self.out.extend_from_slice(b"false");
                return Ok(());
            } else if obj == QTRUE {
                self.out.extend_from_slice(b"true");
                return Ok(());
            } else if is_fix(obj) {
                if self.o.rfc8785 {
                    return self.rfc_number(obj);
                }
                self.out.extend_from_slice(fix(obj).to_string().as_bytes());
                return Ok(());
            } else if is_sym(obj) {
                if !self.o.strict {
                    return self.fallback(obj);
                }
                obj = str_new(sym_name(sym_id(obj)).to_vec());
                string = true;
            } else if is_float(obj) {
                return self.float(obj);
            } else {
                let general = match kind(obj) {
                    Some(Kind::Bignum) => {
                        if self.o.rfc8785 {
                            return self.rfc_number(obj);
                        }
                        self.out.extend_from_slice(rbig(obj).n.to_string().as_bytes());
                        return Ok(());
                    }
                    Some(Kind::Hash) => {
                        if fallback && real_class(obj) != core.hash {
                            true
                        } else {
                            return self.object(obj);
                        }
                    }
                    Some(Kind::Array) => {
                        if fallback && real_class(obj) != core.array {
                            true
                        } else {
                            return self.array(obj);
                        }
                    }
                    Some(Kind::String) => {
                        if fallback && real_class(obj) != core.string {
                            true
                        } else {
                            string = true;
                            false
                        }
                    }
                    _ => {
                        let fc = fragment_class();
                        if fc != QNIL && real_class(obj) as V == fc {
                            let j = send0(obj, "json");
                            if j == UNDEF {
                                return Err(UNDEF);
                            }
                            if !is_str(j) {
                                return Err(wrong_type(j, "String"));
                            }
                            self.out.extend_from_slice(&rstr(j).b);
                            return Ok(());
                        }
                        true
                    }
                };
                if general {
                    if self.o.strict {
                        if truthy(self.o.as_json) && !as_json_called {
                            obj = self.as_json(obj, false)?;
                            as_json_called = true;
                            continue;
                        }
                        return Err(gen_error3(&format!("{} not allowed in JSON", class_label(obj)), obj));
                    }
                    return self.fallback(obj);
                }
            }
            if string {
                if valid_json_str(obj) {
                    return self.raw_str(obj);
                }
                if as_json_called {
                    return Err(gen_error3("source sequence is illegal/malformed utf-8", obj));
                }
                obj = self.ensure_valid(obj, false, false)?;
                as_json_called = true;
            }
        }
    }
    /// JSON.on_mixed_keys_hash: a Hash with both String and Symbol keys may repeat one.
    fn mixed_keys(&mut self, h: V) -> Result<(), V> {
        if self.o.allow_duplicate_key || ver() < V3_0 {
            return Ok(());
        }
        let seen = hash_new();
        let mut i = 0;
        while let Some(k) = rhash(h).m.get_index(i).map(|(k, _)| k.0) {
            i += 1;
            let ks = str_new(crate::fmt::to_s(k).map_err(|_| UNDEF)?);
            if crate::hash::hget(seen, ks) != QNIL {
                let mut msg = b"detected duplicate key ".to_vec();
                msg.extend(crate::fmt::inspect(ks).map_err(|_| UNDEF)?);
                msg.extend_from_slice(b" in ");
                msg.extend(crate::fmt::inspect(h).map_err(|_| UNDEF)?);
                return Err(gen_error_obj(&msg, QNIL));
            }
            crate::hash::hset(seen, ks, QTRUE);
        }
        Ok(())
    }
    fn object(&mut self, h0: V) -> Result<(), V> {
        let mut h = h0;
        if self.o.rfc8785 {
            let p = ivar_get(jc().state as V, sym("@__rfc8785_sort_keys_proc"));
            h = call_proc(p, &[h, if self.o.strict { self.o.as_json } else { QNIL }]);
        } else if truthy(self.o.sort_keys) {
            h = call_proc(self.o.sort_keys, &[h]);
        }
        if h == UNDEF {
            return Err(UNDEF);
        }
        if !is_hash(h) {
            return Err(wrong_type(h, "Hash"));
        }
        let depth = self.depth_up()?;
        if rhash(h).m.is_empty() {
            self.out.extend_from_slice(b"{}");
            self.depth -= 1;
            return Ok(());
        }
        self.out.push(b'{');
        // 0: String, 1: Symbol, 2: other
        let tag = |k: V| if kind(k) == Some(Kind::String) { 0 } else if is_sym(k) { 1 } else { 2 };
        let mut first_tag = 0;
        let mut mixed = false;
        let mut i = 0;
        while let Some((k, val)) = rhash(h).m.get_index(i).map(|(k, v)| (k.0, *v)) {
            if i == 0 {
                first_tag = tag(k);
            } else {
                self.out.push(b',');
            }
            i += 1;
            self.out.extend_from_slice(&self.o.object_nl);
            self.indent(depth);
            let mut key = k;
            let mut as_json_called = false;
            let ks = loop {
                let t = tag(key);
                if t != 2 && t != first_tag && !mixed {
                    mixed = true;
                    self.mixed_keys(h)?;
                }
                if t == 0 {
                    if real_class(key) == rt().c.string {
                        break key;
                    }
                    let r = send0(key, "to_s");
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    if !is_str(r) {
                        let c = class_name(real_class(key));
                        return Err(type_error(&format!("can't convert {c} to String ({c}#to_s gives {})", class_name(real_class(r)))));
                    }
                    break r;
                } else if t == 1 {
                    break str_new(sym_name(sym_id(key)).to_vec());
                }
                if self.o.strict && ver() >= V2_14 {
                    if truthy(self.o.as_json) && !as_json_called {
                        key = self.as_json(key, true)?;
                        as_json_called = true;
                        continue;
                    }
                    return Err(gen_error3(&format!("{} not allowed as object key in JSON", class_label(key)), key));
                }
                let r = send0(key, "to_s");
                if r == UNDEF {
                    return Err(UNDEF);
                }
                if !is_str(r) {
                    let c = class_name(real_class(key));
                    return Err(type_error(&format!("can't convert {c} to String ({c}#to_s gives {})", class_name(real_class(r)))));
                }
                break r;
            };
            let ks = self.ensure_valid(ks, as_json_called, true)?;
            if is_str(ks) && real_class(ks) == rt().c.string {
                self.raw_str(ks)?;
            } else {
                self.value(ks, true)?;
            }
            self.out.extend_from_slice(&self.o.space_before);
            self.out.push(b':');
            self.out.extend_from_slice(&self.o.space);
            self.value(val, true)?;
        }
        self.depth -= 1;
        if !self.o.object_nl.is_empty() {
            self.out.extend_from_slice(&self.o.object_nl);
            self.indent(self.depth);
        }
        self.out.push(b'}');
        Ok(())
    }
    fn array(&mut self, a: V) -> Result<(), V> {
        let depth = self.depth_up()?;
        if rary(a).v.is_empty() {
            self.out.extend_from_slice(b"[]");
            self.depth -= 1;
            return Ok(());
        }
        self.out.push(b'[');
        self.out.extend_from_slice(&self.o.array_nl);
        let mut i = 0;
        while let Some(&x) = rary(a).v.get(i) {
            if i > 0 {
                self.out.push(b',');
                self.out.extend_from_slice(&self.o.array_nl);
            }
            self.indent(depth);
            self.value(x, true)?;
            i += 1;
        }
        self.depth -= 1;
        if !self.o.array_nl.is_empty() {
            self.out.extend_from_slice(&self.o.array_nl);
            self.indent(self.depth);
        }
        self.out.push(b']');
        Ok(())
    }
}

/// What JSON::GeneratorMethods#to_json generates for `v`: itself for the JSON types,
/// else `"#{v}"`.
fn to_json_target(v: V) -> Result<V, V> {
    if v == QNIL || v == QTRUE || v == QFALSE || is_fix(v) || is_float(v) || matches!(kind(v), Some(Kind::Bignum | Kind::Array | Kind::Hash)) {
        return Ok(v);
    }
    Ok(str_new(crate::fmt::to_s(v).map_err(|_| UNDEF)?))
}

fn validate_rfc8785(o: &Opts) -> Result<(), V> {
    if !o.rfc8785 {
        return Ok(());
    }
    let opt = if !o.indent.is_empty() {
        "indent"
    } else if !o.space.is_empty() {
        "space"
    } else if !o.space_before.is_empty() {
        "space_before"
    } else if !o.object_nl.is_empty() {
        "object_nl"
    } else if !o.array_nl.is_empty() {
        "array_nl"
    } else if o.ascii_only {
        "ascii_only"
    } else if o.script_safe {
        "script_safe"
    } else if o.allow_nan {
        "allow_nan"
    } else {
        return Ok(());
    };
    Err(arg_error(&format!("{opt} cannot be used with rfc8785")))
}

/// Generate `v` (json 2.10+), into `io` if one is given.
fn generate3(v: V, o: &Opts, io: V, fallback: bool) -> V {
    if let Err(e) = validate_rfc8785(o) {
        return e;
    }
    let mut g = G3::new(o);
    if let Err(e) = g.value(v, fallback) {
        return e;
    }
    let s = str_new(std::mem::take(&mut g.out));
    if truthy(io) {
        tri!(send1(io, "write", s));
        if respond_to(io, sym("flush")) {
            tri!(send0(io, "flush"));
        }
        return io;
    }
    s
}

/// The generator options of a Hash (State#configure); unknown keys raise ArgumentError.
fn configure3(o: &mut Opts, h: V) -> Result<(), V> {
    if !truthy(h) {
        return Ok(());
    }
    if !is_hash(h) {
        return Err(wrong_type(h, "Hash"));
    }
    let mut unknown: Vec<V> = Vec::new();
    let mut i = 0;
    while let Some((k, v)) = rhash(h).m.get_index(i).map(|(k, v)| (k.0, *v)) {
        i += 1;
        let name = if is_sym(k) {
            String::from_utf8_lossy(sym_name(sym_id(k))).into_owned()
        } else if is_str(k) && ver() < V3_0 {
            String::from_utf8_lossy(&rstr(k).b).into_owned()
        } else {
            String::new()
        };
        let string_config = |v: V| -> Result<Vec<u8>, V> {
            if !truthy(v) {
                return Ok(vec![]);
            }
            if !is_str(v) {
                return Err(wrong_type(v, "String"));
            }
            Ok(rstr(v).b.to_vec())
        };
        let long_config = |v: V| -> Result<i64, V> {
            if !truthy(v) {
                return Ok(0);
            }
            if is_fix(v) {
                Ok(fix(v))
            } else if is_float(v) {
                Ok(float_val(v) as i64)
            } else {
                Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v)))))
            }
        };
        match name.as_str() {
            "indent" => o.indent = string_config(v)?,
            "space" => o.space = string_config(v)?,
            "space_before" => o.space_before = string_config(v)?,
            "object_nl" => o.object_nl = string_config(v)?,
            "array_nl" => o.array_nl = string_config(v)?,
            "max_nesting" => o.max_nesting = long_config(v)?,
            "allow_nan" => o.allow_nan = truthy(v),
            "ascii_only" => o.ascii_only = truthy(v),
            "depth" => o.depth = depth_config(v)?,
            "buffer_initial_length" => {
                if !is_fix(v) {
                    return Err(wrong_type(v, "Integer"));
                }
                if fix(v) > 0 {
                    o.buffer_initial_length = fix(v);
                }
            }
            "script_safe" => o.script_safe = truthy(v),
            "escape_slash" if ver() < V3_0 => o.script_safe = truthy(v),
            "strict" => o.strict = truthy(v),
            "allow_duplicate_key" => o.allow_duplicate_key = truthy(v),
            "rfc8785" if ver() >= V3_1 => o.rfc8785 = truthy(v),
            "as_json" => o.as_json = as_json_config(v)?,
            "sort_keys" if ver() >= 22100 => o.sort_keys = sort_keys_config(v)?,
            _ => {
                if ver() >= V3_0 {
                    unknown.push(k)
                }
            }
        }
    }
    if !unknown.is_empty() {
        let names: Vec<String> = unknown.iter().map(|k| String::from_utf8_lossy(&crate::fmt::to_s(*k).unwrap_or_default()).into_owned()).collect();
        return Err(if names.len() == 1 {
            arg_error(&format!("unknown keyword: {}", names[0]))
        } else {
            arg_error(&format!("unknown keywords: {}", names.join(", ")))
        });
    }
    validate_rfc8785(o)
}
fn depth_config(v: V) -> Result<i64, V> {
    if !truthy(v) {
        return Ok(0);
    }
    let d = if is_fix(v) {
        fix(v)
    } else if is_float(v) {
        float_val(v) as i64
    } else {
        return Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v)))));
    };
    if d < 0 {
        return Err(arg_error(&format!("depth must be >= 0 (got {d})")));
    }
    if d > i32::MAX as i64 {
        return Err(arg_error(&format!("depth is too large (got {d})")));
    }
    Ok(d)
}
fn as_json_config(v: V) -> Result<V, V> {
    if !truthy(v) {
        return Ok(QFALSE);
    }
    if is_proc(v) {
        return Ok(v);
    }
    if respond_to(v, sym("to_proc")) {
        let p = send0(v, "to_proc");
        if p == UNDEF {
            return Err(UNDEF);
        }
        if is_proc(p) {
            return Ok(p);
        }
    }
    Err(type_error("as_json must be a Proc"))
}
fn sort_keys_config(v: V) -> Result<V, V> {
    if is_proc(v) {
        Ok(v)
    } else if v == QTRUE {
        Ok(ivar_get(jc().state as V, sym("@__default_sort_keys_proc")))
    } else if truthy(v) {
        Err(type_error("The `sort_keys` argument must be a boolean or a Proc"))
    } else {
        Ok(QFALSE)
    }
}
/// A Hash from `opts` (State#configure's to_hash / to_h conversion).
fn opts_hash(v: V) -> Result<V, V> {
    if is_hash(v) {
        return Ok(v);
    }
    for m in ["to_hash", "to_h"] {
        if respond_to(v, sym(m)) {
            let h = send0(v, m);
            if h == UNDEF {
                return Err(UNDEF);
            }
            return Ok(h);
        }
    }
    Err(type_error(&format!("can't convert {} into Hash", class_name(real_class(v)))))
}
/// JSON::State.from_state: a State as it is, a Hash as `State.new(hash)`, else a new State.
fn from_state3(a: Option<V>) -> Result<Opts, V> {
    match a {
        Some(v) if is_heap(v) && is_a(v, jc().state) => Ok(Opts::from_state(v)),
        Some(v) if kind(v) == Some(Kind::Hash) => {
            let mut o = Opts::default();
            configure3(&mut o, v)?;
            Ok(o)
        }
        _ => Ok(Opts::default()),
    }
}
/// JSON::GeneratorMethods#to_json (json 2.10+).
fn to_json3(s: V, a: &[V], _: V) -> V {
    let t = tri!(to_json_target(s).unwrap_or_else(|e| e));
    let o = match from_state3(a.first().copied()) {
        Ok(o) => o,
        Err(e) => return e,
    };
    generate3(t, &o, QNIL, false)
}

fn init_state3(state: *mut Class) {
    defn(state, "initialize", 0, 1, |s, a, _| {
        let mut o = Opts::default();
        if let Some(&h) = a.first().filter(|h| truthy(**h)) {
            let h = tri!(opts_hash(h).unwrap_or_else(|e| e));
            if !rhash(h).m.is_empty() {
                if let Err(e) = configure3(&mut o, h) {
                    return e;
                }
            }
        }
        o.store(s);
        s
    });
    defn(state, "initialize_copy", 1, 1, |s, a, _| {
        Opts::from_state(a[0]).store(s);
        s
    });
    defs(state, "from_state", 1, 1, |c, a, _| {
        let v = a[0];
        if is_heap(v) && is_a(v, c as *mut Class) {
            return v;
        }
        if kind(v) == Some(Kind::Hash) {
            return send(c, sym("new"), &[v], QNIL);
        }
        send(jc().state as V, sym("new"), &[], QNIL)
    });
    for name in ["default_sort_keys_proc", "rfc8785_number_formatter_proc", "rfc8785_sort_keys_proc"] {
        let iv = sym(&format!("@__{name}"));
        let label: &'static str = if name == "default_sort_keys_proc" { "sort_key_proc" } else { name };
        let f = proc_new_native(
            Box::new(move |c, a, _| {
                if !is_proc(a[0]) {
                    return type_error(&format!("{label} must be a Proc"));
                }
                ivar_set(c, iv, a[0]);
                a[0]
            }),
            1,
            1,
        );
        add_method(singleton_class(state as V), sym(&format!("{name}=")), Body::Proc(f));
    }
    fn gen_class(a: &[V], fallback: bool) -> V {
        let mut o = Opts::default();
        if let Err(e) = configure3(&mut o, a[1]) {
            return e;
        }
        generate3(a[0], &o, a[2], fallback)
    }
    defs(state, "generate", 3, 3, |_, a, _| gen_class(a, true));
    defs(state, "_generate_no_fallback", 3, 3, |_, a, _| gen_class(a, false));
    defn(state, "generate", 1, 2, |s, a, _| generate3(a[0], &Opts::from_state(s), a.get(1).copied().unwrap_or(QNIL), true));
    defn(state, "_generate_no_fallback", 1, 2, |s, a, _| generate3(a[0], &Opts::from_state(s), a.get(1).copied().unwrap_or(QNIL), false));
    fn configure(s: V, a: &[V]) -> V {
        tri!(check_frozen(s));
        let h = tri!(opts_hash(a[0]).unwrap_or_else(|e| e));
        let mut o = Opts::from_state(s);
        if let Err(e) = configure3(&mut o, h) {
            return e;
        }
        o.store(s);
        s
    }
    defn(state, "configure", 1, 1, |s, a, _| configure(s, a));
    defn(state, "merge", 1, 1, |s, a, _| configure(s, a));
    defn(state, "_configure", 1, 1, |s, a, _| configure(s, a));
    fn to_h(s: V) -> V {
        let o = Opts::from_state(s);
        let h = hash_new();
        let set = |k: &str, v: V| {
            crate::hash::hset(h, mksym(sym(k)), v);
        };
        let st = |b: &Vec<u8>| str_new(b.clone());
        set("indent", st(&o.indent));
        set("space", st(&o.space));
        set("space_before", st(&o.space_before));
        set("object_nl", st(&o.object_nl));
        set("array_nl", st(&o.array_nl));
        set("as_json", o.as_json);
        set("allow_nan", bool_v(o.allow_nan));
        set("ascii_only", bool_v(o.ascii_only));
        set("max_nesting", mkfix(o.max_nesting));
        set("script_safe", bool_v(o.script_safe));
        set("strict", bool_v(o.strict));
        set("depth", mkfix(o.depth));
        set("buffer_initial_length", mkfix(o.buffer_initial_length));
        set("sort_keys", o.sort_keys);
        if ver() >= V3_1 {
            set("rfc8785", bool_v(o.rfc8785));
        }
        if ver() >= V3_0 {
            set("allow_duplicate_key", bool_v(o.allow_duplicate_key));
        }
        h
    }
    defn(state, "to_h", 0, 0, |s, _, _| to_h(s));
    defn(state, "to_hash", 0, 0, |s, _, _| to_h(s));
    // the string options: "" when unset
    for k in STR_KEYS {
        let iv = sym(&format!("@__{k}"));
        let rd = proc_new_native(
            Box::new(move |s, _, _| {
                let v = ivar_get(s, iv);
                let r = if is_str(v) { str_new(rstr(v).b.to_vec()) } else { str_new(vec![]) };
                hdr(r).flags |= FL_FROZEN;
                r
            }),
            0,
            0,
        );
        add_method(state, sym(k), Body::Proc(rd));
        let wr = proc_new_native(
            Box::new(move |s, a, _| {
                tri!(check_frozen(s));
                let v = a[0];
                let b = if !truthy(v) {
                    vec![]
                } else if is_str(v) {
                    rstr(v).b.to_vec()
                } else {
                    return wrong_type(v, "String");
                };
                ivar_set(s, iv, str_new(b));
                a[0]
            }),
            1,
            1,
        );
        add_method(state, sym(&format!("{k}=")), Body::Proc(wr));
    }
    // boolean options: `name?` (strict and script_safe also without the ?) and `name=`
    for (k, plain) in [("allow_nan", false), ("ascii_only", false), ("script_safe", true), ("strict", true), ("rfc8785", false)] {
        let iv = sym(&format!("@__{k}"));
        let q = proc_new_native(Box::new(move |s, _, _| bool_v(truthy(ivar_get(s, iv)))), 0, 0);
        add_method(state, sym(&format!("{k}?")), Body::Proc(q));
        if plain {
            let q = proc_new_native(Box::new(move |s, _, _| bool_v(truthy(ivar_get(s, iv)))), 0, 0);
            add_method(state, sym(k), Body::Proc(q));
        }
        let wr = proc_new_native(
            Box::new(move |s, a, _| {
                tri!(check_frozen(s));
                ivar_set(s, iv, bool_v(truthy(a[0])));
                a[0]
            }),
            1,
            1,
        );
        add_method(state, sym(&format!("{k}=")), Body::Proc(wr));
    }
    let m = add_method(
        state,
        sym("allow_duplicate_key?"),
        Body::Proc(proc_new_native(Box::new(|s, _, _| bool_v(truthy(ivar_get(s, sym("@__allow_duplicate_key"))))), 0, 0)),
    );
    unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
    defn(state, "max_nesting", 0, 0, |s, _, _| mkfix(nesting_of(ivar_get(s, sym("@__max_nesting")))));
    defn(state, "max_nesting=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        let n = if !truthy(a[0]) {
            0
        } else if is_fix(a[0]) {
            fix(a[0])
        } else {
            return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(a[0]))));
        };
        ivar_set(s, sym("@__max_nesting"), mkfix(n));
        a[0]
    });
    defn(state, "depth", 0, 0, |s, _, _| mkfix(nesting_of(ivar_get(s, sym("@__depth")))));
    defn(state, "depth=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        let d = match depth_config(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        ivar_set(s, sym("@__depth"), mkfix(d));
        a[0]
    });
    defn(state, "check_circular?", 0, 0, |s, _, _| bool_v(nesting_of(ivar_get(s, sym("@__max_nesting"))) != 0));
    defn(state, "as_json", 0, 0, |s, _, _| {
        let v = ivar_get(s, sym("@__as_json"));
        if truthy(v) { v } else { QFALSE }
    });
    defn(state, "as_json=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        let p = match as_json_config(a[0]) {
            Ok(p) => p,
            Err(e) => return e,
        };
        ivar_set(s, sym("@__as_json"), p);
        QNIL
    });
    defn(state, "sort_keys", 0, 0, |s, _, _| {
        let v = ivar_get(s, sym("@__sort_keys"));
        if truthy(v) { v } else { QFALSE }
    });
    defn(state, "sort_keys=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        let p = match sort_keys_config(a[0]) {
            Ok(p) => p,
            Err(e) => return e,
        };
        ivar_set(s, sym("@__sort_keys"), p);
        p
    });
    defn(state, "buffer_initial_length", 0, 0, |s, _, _| {
        let b = ivar_get(s, sym("@__buffer_initial_length"));
        if is_fix(b) { b } else { mkfix(1024) }
    });
    defn(state, "buffer_initial_length=", 1, 1, |s, a, _| {
        tri!(check_frozen(s));
        if !is_fix(a[0]) {
            return wrong_type(a[0], "Integer");
        }
        if fix(a[0]) > 0 {
            ivar_set(s, sym("@__buffer_initial_length"), a[0]);
        }
        QNIL
    });
    if ver() < V3_0 {
        // (deprecated in 2.16, removed in 3.0)
        defn(state, "escape_slash?", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__script_safe")))));
        defn(state, "escape_slash", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__script_safe")))));
        defn(state, "escape_slash=", 1, 1, |s, a, _| {
            ivar_set(s, sym("@__script_safe"), bool_v(truthy(a[0])));
            a[0]
        });
        defn(state, "[]", 1, 1, |s, a, _| {
            let name = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).into_owned();
            if respond_to(s, sym(&name)) {
                return send(s, sym(&name), &[], QNIL);
            }
            ivar_get(s, sym(&format!("@{name}")))
        });
        defn(state, "[]=", 2, 2, |s, a, _| {
            let name = String::from_utf8_lossy(&crate::fmt::to_s(a[0]).unwrap_or_default()).into_owned();
            if respond_to(s, sym(&format!("{name}="))) {
                return send(s, sym(&format!("{name}=")), &[a[1]], QNIL);
            }
            ivar_set(s, sym(&format!("@{name}")), a[1]);
            a[1]
        });
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
    let version = detect_version();
    VER.store(parse_ver(&version).unwrap_or(20901), std::sync::atomic::Ordering::Relaxed);
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
    let v = str_from(&version);
    hdr(v).flags |= FL_FROZEN;
    const_set(json, sym("VERSION"), v);
    if !modern() {
        const_set(json, sym("UnparserError"), generator_error as V);
        const_set(json, sym("PARSE_ERROR"), parser_error as V);
        let n = ver();
        for (name, v) in [("VERSION_MAJOR", n / 10000), ("VERSION_MINOR", n / 100 % 100), ("VERSION_BUILD", n % 100)] {
            const_set(json, sym(name), mkfix(v as i64));
        }
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
    *JCS.get() = Some(JC { parser_error, nesting_error, generator_error, state, parser, generator: genm, json });
    if modern() {
        init_modern(json, ext, state, parser);
    } else {
        init_state(state);
        init_classic(json, parser);
    }
    defs(json, "state", 0, 0, |_, _, _| jc().state as V);
    defs(json, "generator", 0, 0, |_, _, _| jc().generator as V);
    defs(json, "parser", 0, 0, |_, _, _| jc().parser as V);

    // Kernel#JSON (private)
    fn kernel_json(_: V, a: &[V], _: V) -> V {
        if modern() {
            return json_brackets3(a);
        }
        if is_str(a[0]) {
            parse_impl(a[0], a.get(1).copied(), default_po())
        } else {
            let (o, st) = opts_arg(a.get(1).copied(), Opts::default());
            generate(a[0], &o, st)
        }
    }
    let m = add_method(r.c.kernel, sym("JSON"), Body::Native(kernel_json, 1, 2));
    unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };

    let to_json = sym("to_json");
    let mut builtin = Vec::new();
    if modern() {
        // json 2.10+: one JSON::GeneratorMethods#to_json, in Object
        let gm = new_module("JSON::GeneratorMethods");
        const_set(json, sym("GeneratorMethods"), gm as V);
        builtin.push(add_method(gm, to_json, Body::Native(to_json3, 0, -1)));
        crate::kernel::do_include(r.c.object, gm as V);
    } else {
        // as in the json gem, the core classes' to_json comes from included modules, so a module
        // included later (ActiveSupport's ToJsonWithActiveSupportEncoder in Rails 8) overrides it
        // and reaches it through `super`
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
    }
    *BUILTIN_TO_JSON.get() = builtin;
}

/// JSON[object, opts] (json 2.10+): parse a String, generate anything else.
fn json_brackets3(a: &[V]) -> V {
    let opts = a.get(1).copied().filter(|o| *o != QNIL);
    let mut src = a[0];
    if !is_str(src) && respond_to(src, sym("to_str")) {
        let s = tri!(send0(src, "to_str"));
        if is_str(s) {
            src = s;
        }
    }
    if is_str(src) {
        return parser3::parse_with(src, opts, parser3::Config::new());
    }
    generate_with3(a[0], opts, Opts::default())
}
/// JSON.generate(obj, opts) (json 2.10+): a State's options, or a Hash of options.
fn generate_with3(v: V, opts: Option<V>, base: Opts) -> V {
    match opts {
        Some(st) if is_heap(st) && is_a(st, jc().state) => generate3(v, &Opts::from_state(st), QNIL, true),
        _ => {
            let mut o = base;
            if let Some(h) = opts {
                if let Err(e) = configure3(&mut o, h) {
                    return e;
                }
            }
            generate3(v, &o, QNIL, true)
        }
    }
}

/// The parser, generator and module functions of json 2.10 and later.
fn init_modern(json: *mut Class, ext: *mut Class, state: *mut Class, parser: *mut Class) {
    init_state3(state);
    let pc = new_class("JSON::Ext::ParserConfig", rt().c.object);
    const_set(ext, sym("ParserConfig"), pc as V);
    const_set(parser, sym("Config"), pc as V);
    let m = add_method(
        pc,
        sym("initialize"),
        Body::Native(
            |s, a, _| {
                tri!(check_frozen(s));
                let mut c = parser3::Config::new();
                if let Err(e) = c.apply(a[0], false) {
                    return e;
                }
                c.store(s);
                s
            },
            1,
            1,
        ),
    );
    unsafe { (*(m as *mut Method)).vis = VIS_PRIVATE };
    defn(pc, "parse", 1, 1, |s, a, _| parser3::parse(a[0], &parser3::Config::load(s)));

    defn(parser, "initialize", 1, 2, |s, a, _| {
        let mut c = parser3::Config::new();
        if let Err(e) = c.apply(a.get(1).copied().unwrap_or(QNIL), false) {
            return e;
        }
        c.store(s);
        ivar_set(s, sym("@source"), a[0]);
        s
    });
    defn(parser, "source", 0, 0, |s, _, _| {
        let src = ivar_get(s, sym("@source"));
        if is_str(src) { str_new(rstr(src).b.to_vec()) } else { send0(src, "dup") }
    });
    defn(parser, "parse", 0, 0, |s, _, _| parser3::parse(ivar_get(s, sym("@source")), &parser3::Config::load(s)));
    defs(parser, "parse", 1, 2, |_, a, _| parser3::parse_with(a[0], a.get(1).copied(), parser3::Config::new()));

    defs(json, "parse", 1, 2, |_, a, _| parser3::parse_with(a[0], a.get(1).copied(), parser3::Config::new()));
    defs(json, "parse!", 1, 2, |_, a, _| {
        let base = parser3::Config { max_nesting: 0, allow_nan: true, ..parser3::Config::new() };
        parser3::parse_with(a[0], a.get(1).copied(), base)
    });
    defs(json, "generate", 1, 2, |_, a, _| generate_with3(a[0], a.get(1).copied().filter(|o| *o != QNIL), Opts::default()));
    defs(json, "pretty_generate", 1, 2, |_, a, _| {
        let opts = a.get(1).copied().filter(|o| truthy(*o));
        if let Some(st) = opts.filter(|st| is_heap(*st) && is_a(*st, jc().state)) {
            return generate3(a[0], &Opts::from_state(st), QNIL, true);
        }
        let mut o = pretty();
        if let Some(h) = opts {
            let h = tri!(opts_hash(h).unwrap_or_else(|e| e));
            if let Err(e) = configure3(&mut o, h) {
                return e;
            }
        }
        generate3(a[0], &o, QNIL, true)
    });
    defs(json, "[]", 1, 2, |_, a, _| json_brackets3(a));
    if ver() < V3_0 {
        // removed in json 3.0 (lib/kenny/json.rb defines 3.0's load and dump)
        init_classic_extras(json);
        defs(json, "unparse", 1, 2, |_, a, _| generate_with3(a[0], a.get(1).copied().filter(|o| *o != QNIL), Opts::default()));
        defs(json, "fast_generate", 1, 2, |_, a, _| generate_with3(a[0], a.get(1).copied().filter(|o| *o != QNIL), Opts::default()));
        defs(json, "pretty_unparse", 1, 2, |_, a, _| {
            let opts = a.get(1).copied().filter(|o| truthy(*o));
            let mut o = pretty();
            if let Some(h) = opts {
                if let Err(e) = configure3(&mut o, h) {
                    return e;
                }
            }
            generate3(a[0], &o, QNIL, true)
        });
    }
}

/// JSON.load / dump / load_file of json < 3.0 (and the parser and generator of json < 2.10).
fn init_classic(json: *mut Class, parser: *mut Class) {
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
    fn gen_with(a: &[V], base: Opts) -> V {
        let (o, st) = opts_arg(a.get(1).copied().filter(|x| *x != QNIL), base);
        generate(a[0], &o, st)
    }
    defs(json, "generate", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "unparse", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "fast_generate", 1, 2, |_, a, _| gen_with(a, Opts::default()));
    defs(json, "pretty_generate", 1, 2, |_, a, _| gen_with(a, pretty()));
    defs(json, "pretty_unparse", 1, 2, |_, a, _| gen_with(a, pretty()));
    defs(json, "[]", 1, 2, |_, a, _| {
        if is_str(a[0]) {
            parse_impl(a[0], a.get(1).copied(), default_po())
        } else {
            gen_with(a, Opts::default())
        }
    });
    init_classic_extras(json);
}

/// The parts of the json < 3.0 API that json 2.10 - 2.21 still have.
fn init_classic_extras(json: *mut Class) {
    /// The document and options of JSON.parse for `load`.
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
        let pr = a.get(1).copied().filter(|p| *p != QNIL && !is_hash(*p));
        if modern() {
            // json 2.10 - 2.21: allow_blank (default true) loads "" as nil; the proc is
            // on_load, whose results replace the values
            let opts = last_hash(&a[1..]);
            let blank = opts.and_then(|h| opt_get(h, "allow_blank")).map_or(true, truthy);
            if src == QNIL || (is_str(src) && rstr(src).b.is_empty()) {
                if blank {
                    return QNIL;
                }
            }
            let mut c = parser3::Config { max_nesting: 0, allow_nan: true, ..parser3::Config::new() };
            if let Some(h) = opts {
                if let Err(e) = c.apply(h, true) {
                    return e;
                }
            }
            if let Some(pr) = pr {
                c.on_load = if is_proc(pr) { pr } else { tri!(send0(pr, "to_proc")) };
            }
            return parser3::parse(src, &c);
        }
        let b = match source_bytes(src) {
            Ok(b) => b,
            Err(e) => return e,
        };
        if b.iter().all(|c| c.is_ascii_whitespace()) {
            return QNIL;
        }
        let v = {
            tri!(parse_bytes(&b, parse_opts(last_hash(&a[1..]), PO { allow_nan: true, max_nesting: 0, ..default_po() })))
        };
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
            Ok(b) => {
                if modern() {
                    parser3::parse_with(str_new(b.to_vec()), a.get(1).copied(), parser3::Config::new())
                } else {
                    parse_bytes(&b, parse_opts(a.get(1).copied(), default_po()))
                }
            }
            Err(_) => {
                let errno = const_lookup(rt().c.object, sym("Errno")).unwrap() as *mut Class;
                let enoent = const_get_under(errno, sym("ENOENT")).unwrap() as *mut Class;
                raise(enoent, &format!("No such file or directory @ rb_sysopen - {path}"))
            }
        }
    });
    defs(json, "dump", 1, 4, |_, a, _| {
        // JSON.dump(obj, io = nil, limit = nil, kwargs = nil)
        let mut pos: Vec<V> = vec![a[0]];
        pos.extend(a[1..].iter().copied().filter(|x| !is_hash(*x)));
        let mut o = Opts { allow_nan: true, max_nesting: 0, ..Opts::default() };
        if let Some(h) = last_hash(&a[1..]) {
            if modern() {
                if let Err(e) = configure3(&mut o, h) {
                    return e;
                }
            } else {
                o.merge_hash(h);
            }
        }
        if let Some(&lim) = pos.get(2).or(pos.get(1).filter(|x| is_fix(**x))) {
            if is_fix(lim) {
                o.max_nesting = fix(lim);
            }
        }
        let s = if modern() {
            let r = generate3(pos[0], &o, QNIL, true);
            if r == UNDEF {
                // json < 3.0: a NestingError is reported as an ArgumentError
                if let Throw::Raise(e) = pst().pending {
                    if is_a(e, jc().nesting_error) {
                        pst().pending = Throw::None;
                        return arg_error("exceed depth limit");
                    }
                }
                return UNDEF;
            }
            r
        } else {
            let st = state_new(&o);
            tri!(generate(pos[0], &o, st))
        };
        match pos.get(1) {
            Some(&io) if !is_fix(io) && io != QNIL && respond_to(io, sym("write")) => {
                tri!(send1(io, "write", s));
                io
            }
            _ => s,
        }
    });
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
}

pub fn init() {
    crate::compat::provide_feature(
        &["json", "json/ext", "json/common", "json/ext/parser", "json/ext/generator", "json/pure", "json/generic_object"],
        Some(init_json),
    );
}
