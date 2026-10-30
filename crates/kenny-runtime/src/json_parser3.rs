//! The parser of json 2.10 and later (json 3.x behaviour): the C extension's iterative state
//! machine, ported so that its error messages, their positions (`line`, `column`) and the
//! error's `json_path` come out the same.

use super::{jc, ver, V2_12, V3_0};
use crate::core::*;
use num_bigint::BigInt;

/// A parser configuration (`JSON::Ext::ParserConfig`, or the options of one `JSON.parse`).
#[derive(Clone)]
pub struct Config {
    pub max_nesting: i64,
    pub allow_nan: bool,
    pub allow_trailing_comma: bool,
    pub allow_comments: bool,
    pub allow_control_characters: bool,
    pub allow_invalid_escape: bool,
    pub symbolize_names: bool,
    pub freeze: bool,
    pub allow_duplicate_key: bool,
    /// a Proc, or QNIL
    pub on_load: V,
    /// (receiver, method) that builds a decimal from its text, or QNIL
    pub decimal_class: V,
    pub decimal_method: u32,
    /// JSON.parse's object_class: / array_class: (applied before on_load), or QNIL
    pub object_class: V,
    pub array_class: V,
}

impl Config {
    pub fn new() -> Config {
        let v3 = ver() >= V3_0;
        Config {
            max_nesting: 100,
            allow_nan: false,
            allow_trailing_comma: false,
            allow_comments: !v3,
            allow_control_characters: false,
            allow_invalid_escape: false,
            symbolize_names: false,
            freeze: false,
            allow_duplicate_key: !v3,
            on_load: QNIL,
            decimal_class: QNIL,
            decimal_method: 0,
            object_class: QNIL,
            array_class: QNIL,
        }
    }

    /// Apply an options Hash. `top` accepts JSON.parse's own object_class: / array_class:.
    /// Unknown keys raise ArgumentError (json 3.0), as keyword arguments do.
    pub fn apply(&mut self, h: V, top: bool) -> Result<(), V> {
        if h == QNIL {
            return Ok(());
        }
        if !is_hash(h) {
            return Err(type_error(&format!("wrong argument type {} (expected Hash)", class_name(real_class(h)))));
        }
        let mut unknown: Vec<V> = Vec::new();
        let mut i = 0;
        while let Some((k, v)) = rhash(h).m.get_index(i).map(|(k, v)| (k.0, *v)) {
            i += 1;
            let name = if is_sym(k) { String::from_utf8_lossy(sym_name(sym_id(k))).into_owned() } else { String::new() };
            // json < 3.0 took string keys too
            let name = if name.is_empty() && is_str(k) && ver() < V3_0 { String::from_utf8_lossy(&rstr(k).b).into_owned() } else { name };
            match name.as_str() {
                "max_nesting" => self.max_nesting = if truthy(v) { if is_fix(v) { fix(v) } else { 0 } } else { 0 },
                "allow_nan" => self.allow_nan = truthy(v),
                "allow_trailing_comma" => self.allow_trailing_comma = truthy(v),
                "allow_comments" => self.allow_comments = truthy(v),
                "allow_control_characters" => self.allow_control_characters = truthy(v),
                "allow_invalid_escape" => self.allow_invalid_escape = truthy(v),
                "symbolize_names" => self.symbolize_names = truthy(v),
                "freeze" => self.freeze = truthy(v),
                "allow_duplicate_key" => self.allow_duplicate_key = truthy(v),
                "on_load" => {
                    let mut p = v;
                    if truthy(p) && !is_proc(p) {
                        p = if respond_to(p, sym("to_proc")) { tri_r(send0(p, "to_proc"))? } else { UNDEF };
                        if p == UNDEF || !is_proc(p) {
                            return Err(type_error("on_load must be a Proc"));
                        }
                    }
                    self.on_load = if truthy(p) { p } else { QNIL };
                }
                "decimal_class" => {
                    if truthy(v) {
                        if respond_to(v, sym("try_convert")) {
                            self.decimal_class = v;
                            self.decimal_method = sym("try_convert");
                        } else if respond_to(v, sym("new")) && ![rt().c.float, rt().c.integer].contains(&(v as *mut Class)) {
                            // (Float and Integer have no `new` in CRuby)
                            self.decimal_class = v;
                            self.decimal_method = sym("new");
                        } else if is_class(v) {
                            // BigDecimal: the Kernel method named after the class
                            let name = class_name(v as *mut Class);
                            match name.rfind("::") {
                                Some(p) => {
                                    let modp = &name[..p];
                                    let mut c = rt().c.object;
                                    for part in modp.split("::") {
                                        c = match const_get_under(c, sym(part)) {
                                            Some(x) if is_class(x) => x as *mut Class,
                                            _ => return Err(arg_error(&format!("undefined class/module {modp}"))),
                                        };
                                    }
                                    self.decimal_class = c as V;
                                    self.decimal_method = sym(&name[p + 2..]);
                                }
                                None => {
                                    self.decimal_class = rt().c.kernel as V;
                                    self.decimal_method = sym(&name);
                                }
                            }
                        }
                    }
                }
                "object_class" if top => self.object_class = v,
                "array_class" if top => self.array_class = v,
                // options of json < 3.0 that this parser leaves to the caller (or ignores)
                "create_additions" | "create_id" | "match_string" | "quirks_mode" | "allow_blank" | "object_class" | "array_class" | "source"
                    if ver() < V3_0 => {}
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
        Ok(())
    }

    /// Stored in a ParserConfig object (and read back by `parse`).
    pub fn store(&self, s: V) {
        let set = |k: &str, v: V| {
            ivar_set(s, sym(&format!("@__{k}")), v);
        };
        set("max_nesting", mkfix(self.max_nesting));
        let mut bits = 0i64;
        for (i, b) in [
            self.allow_nan,
            self.allow_trailing_comma,
            self.allow_comments,
            self.allow_control_characters,
            self.allow_invalid_escape,
            self.symbolize_names,
            self.freeze,
            self.allow_duplicate_key,
        ]
        .iter()
        .enumerate()
        {
            if *b {
                bits |= 1 << i;
            }
        }
        set("flags", mkfix(bits));
        set("on_load", self.on_load);
        set("decimal_class", self.decimal_class);
        set("decimal_method", if self.decimal_method != 0 { mksym(self.decimal_method) } else { QNIL });
    }
    pub fn load(s: V) -> Config {
        let g = |k: &str| ivar_get(s, sym(&format!("@__{k}")));
        let bits = { let b = g("flags"); if is_fix(b) { fix(b) } else { 0 } };
        let f = |i: i64| bits & (1 << i) != 0;
        let dm = g("decimal_method");
        Config {
            max_nesting: { let m = g("max_nesting"); if is_fix(m) { fix(m) } else { 100 } },
            allow_nan: f(0),
            allow_trailing_comma: f(1),
            allow_comments: f(2),
            allow_control_characters: f(3),
            allow_invalid_escape: f(4),
            symbolize_names: f(5),
            freeze: f(6),
            allow_duplicate_key: f(7),
            on_load: g("on_load"),
            decimal_class: g("decimal_class"),
            decimal_method: if is_sym(dm) { sym_id(dm) } else { 0 },
            object_class: QNIL,
            array_class: QNIL,
        }
    }
}

fn tri_r(v: V) -> Result<V, V> {
    if v == UNDEF { Err(UNDEF) } else { Ok(v) }
}

#[derive(Clone, Copy, PartialEq)]
enum Ft {
    Root,
    Array,
    Object,
}
#[derive(Clone, Copy, PartialEq)]
enum Ph {
    Done,
    ArrayComma,
    ObjectComma,
    Value,
    ObjectKey,
    ObjectColon,
}
struct Frame {
    t: Ft,
    ph: Ph,
    /// the value stack's length when the container opened
    head: usize,
    /// offset of the container's opening bracket
    start: usize,
}

struct St<'a> {
    b: &'a [u8],
    i: usize,
    c: &'a Config,
    /// the value stack (a Ruby Array, so the GC sees the values)
    vals: V,
    frames: Vec<Frame>,
    nesting: i64,
}

fn completed(f: &mut Frame) -> Ph {
    f.ph = match f.t {
        Ft::Root => Ph::Done,
        Ft::Array => Ph::ArrayComma,
        Ft::Object => Ph::ObjectComma,
    };
    f.ph
}

impl St<'_> {
    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }
    fn eos(&self) -> bool {
        self.i >= self.b.len()
    }
    fn nvals(&self) -> usize {
        rary(self.vals).v.len()
    }

    // ---------- errors ----------

    /// The document fragment an error message shows ('...' or EOF), as the C parser builds it.
    fn snippet(&self) -> Vec<u8> {
        if self.i >= self.b.len() {
            return b"EOF".to_vec();
        }
        let p = &self.b[self.i..];
        let mut len = 0;
        while len < 32 {
            let ch = p.get(len).copied().unwrap_or(0);
            if ch == 0 || ch == b'\n' || ch == b' ' || ch == b'\t' || ch == b'\r' {
                break;
            }
            len += 1;
        }
        if len == 0 {
            // (the C code then prints the rest of the document, up to a NUL)
            let end = p.iter().position(|&c| c == 0).unwrap_or(p.len());
            return p[..end].to_vec();
        }
        if len == 32 {
            while len > 0 && p.get(len).is_some_and(|&c| (0x80..0xc0).contains(&c)) {
                len -= 1;
            }
        }
        let mut out = vec![b'\''];
        out.extend_from_slice(&p[..len]);
        out.push(b'\'');
        out
    }
    fn position(&self) -> (i64, i64) {
        let at = self.i.min(self.b.len());
        let ls = self.b[..at].iter().rposition(|&c| c == b'\n').map_or(0, |p| p + 1);
        let col = self.b[ls..at].iter().filter(|&&c| !(0x80..0xc0).contains(&c)).count() as i64 + 1;
        let line = self.b[..ls].iter().filter(|&&c| c == b'\n').count() as i64 + 1;
        (line, col)
    }
    fn json_path(&self, dup_key: Option<V>) -> V {
        let path = ary_new(Vec::new());
        let vals = &rary(self.vals).v;
        let n = self.frames.len();
        for depth in 1..n {
            let f = &self.frames[depth];
            let innermost = depth == n - 1;
            let child_head = if innermost { vals.len() } else { self.frames[depth + 1].head };
            let count = child_head - f.head;
            if f.t == Ft::Array {
                let idx = if f.ph == Ph::ArrayComma { count as i64 - 1 } else { count as i64 };
                rary(path).v.push(mkfix(idx));
            } else if innermost && dup_key.is_some() {
                rary(path).v.push(dup_key.unwrap());
            } else if count & 1 == 1 {
                rary(path).v.push(vals[child_head - 1]);
            } else if f.ph == Ph::ObjectComma && count >= 2 {
                rary(path).v.push(vals[child_head - 2]);
            } else {
                break;
            }
        }
        path
    }
    fn error(&self, mut msg: Vec<u8>, dup_key: Option<V>) -> V {
        let mut lc = None;
        if ver() >= V2_12 {
            let (line, col) = self.position();
            msg.extend_from_slice(format!(" at line {line} column {col}").as_bytes());
            lc = Some((line, col));
        }
        let e = object_new(jc().parser_error);
        ivar_set(e, sym("@__message"), str_new(msg));
        if let Some((line, col)) = lc {
            ivar_set(e, sym("@line"), mkfix(line));
            ivar_set(e, sym("@column"), mkfix(col));
        }
        if ver() >= V3_0 {
            ivar_set(e, sym("@json_path"), self.json_path(dup_key));
        }
        raise_exc(e)
    }
    /// Raise a ParserError at the cursor; a "%s" in `fmt` is the document fragment there.
    fn raise(&self, fmt: &str) -> V {
        let mut msg = Vec::new();
        match fmt.find("%s") {
            Some(p) => {
                msg.extend_from_slice(fmt[..p].as_bytes());
                msg.extend(self.snippet());
                msg.extend_from_slice(fmt[p + 2..].as_bytes());
            }
            None => msg.extend_from_slice(fmt.as_bytes()),
        }
        self.error(msg, None)
    }
    fn raise_at(&mut self, fmt: &str, at: usize) -> V {
        self.i = at;
        self.raise(fmt)
    }
    fn nesting_error(&self) -> V {
        raise(jc().nesting_error, &format!("nesting of {} is too deep", self.nesting))
    }

    // ---------- lexing ----------

    fn comment(&mut self) -> Result<(), V> {
        if !self.c.allow_comments {
            return Err(self.raise("unexpected token %s"));
        }
        let start = self.i;
        self.i += 1;
        match self.peek() {
            b'/' => match self.b[self.i..].iter().position(|&c| c == b'\n') {
                Some(p) => self.i += p + 1,
                None => self.i = self.b.len(),
            },
            b'*' => {
                self.i += 1;
                loop {
                    let Some(p) = self.b[self.i.min(self.b.len())..].iter().position(|&c| c == b'*') else {
                        return Err(self.raise_at("unterminated comment, expected closing '*/'", start));
                    };
                    self.i += p + 1;
                    if self.peek() == b'/' {
                        self.i += 1;
                        break;
                    }
                }
            }
            _ => return Err(self.raise_at("unexpected token %s", start)),
        }
        Ok(())
    }
    fn ws(&mut self) -> Result<(), V> {
        loop {
            match self.peek() {
                b' ' | b'\n' | b'\t' | b'\r' => self.i += 1,
                b'/' => self.comment()?,
                _ => return Ok(()),
            }
        }
    }
    fn keyword(&mut self, w: &[u8]) -> Result<(), V> {
        if self.b[self.i..].starts_with(w) {
            self.i += w.len();
            Ok(())
        } else {
            Err(self.raise("unexpected token %s"))
        }
    }
    fn digits(&mut self) -> usize {
        let s = self.i;
        while self.peek().is_ascii_digit() {
            self.i += 1;
        }
        self.i - s
    }
    /// A number from the cursor (after a '-' if `neg`); None if malformed.
    fn number(&mut self, neg: bool, start: usize) -> Result<Option<V>, V> {
        let first = self.peek();
        let nd = self.digits();
        if (first == b'0' && nd > 1) || (neg && nd == 0) {
            return Ok(None);
        }
        let mut integer = true;
        if self.peek() == b'.' {
            integer = false;
            self.i += 1;
            if self.digits() == 0 {
                return Ok(None);
            }
        }
        if self.peek() | 0x20 == b'e' {
            integer = false;
            self.i += 1;
            if matches!(self.peek(), b'-' | b'+') {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Ok(None);
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).unwrap();
        if integer {
            if nd <= 18 {
                return Ok(Some(mkfix(text.parse::<i64>().unwrap_or(0))));
            }
            return Ok(Some(int_from_big(text.parse::<BigInt>().unwrap_or_default())));
        }
        if self.c.decimal_class != QNIL {
            let r = send(self.c.decimal_class, self.c.decimal_method, &[str_from(text)], QNIL);
            return if r == UNDEF { Err(UNDEF) } else { Ok(Some(r)) };
        }
        Ok(Some(float_new(text.parse::<f64>().unwrap_or(0.0))))
    }

    fn hex4(&mut self, sp: usize, end: usize) -> Result<u32, V> {
        let h = if sp + 4 <= end { std::str::from_utf8(&self.b[sp..sp + 4]).ok().and_then(|h| u32::from_str_radix(h, 16).ok()) } else { None };
        // (from_str_radix takes a sign; the C code doesn't)
        match h {
            Some(v) if self.b[sp] != b'+' && self.b[sp] != b'-' => Ok(v),
            _ => Err(self.raise_at("incomplete unicode character escape sequence at %s", sp - 2)),
        }
    }
    /// The bytes of a string with escapes, between `start` and the closing quote at `end`.
    fn unescape(&mut self, start: usize, end: usize) -> Result<Vec<u8>, V> {
        let mut out = Vec::with_capacity(end - start);
        let (mut p, mut pe) = (start, start);
        while pe < end {
            let Some(bs) = self.b[pe..end].iter().position(|&c| c == b'\\') else { break };
            pe += bs;
            if pe > p {
                out.extend_from_slice(&self.b[p..pe]);
            }
            pe += 1;
            let c = self.b[pe];
            match c {
                b'"' | b'/' => p = pe,
                b'\\' | b'n' | b'r' | b't' | b'b' | b'f' => {
                    out.push(match c {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        _ => b'\\',
                    });
                    pe += 1;
                    p = pe;
                }
                b'u' => {
                    pe += 1;
                    let mut ch = self.hex4(pe, end)?;
                    pe += 3;
                    if ch & 0xfc00 == 0xd800 {
                        pe += 1;
                        if pe + 6 <= end && &self.b[pe..pe + 2] == b"\\u" {
                            let sur = self.hex4(pe + 2, end)?;
                            if sur & 0xfc00 != 0xdc00 {
                                return Err(self.raise_at("invalid surrogate pair at %s", p));
                            }
                            ch = 0x10000 + ((ch - 0xd800) << 10) + (sur - 0xdc00);
                            pe += 5;
                        } else {
                            return Err(self.raise_at("incomplete surrogate pair at %s", p));
                        }
                    } else if ch & 0xfc00 == 0xdc00 {
                        if ver() >= V3_0 {
                            return Err(self.raise_at("unpaired trailing surrogate at %s", p));
                        }
                    }
                    match char::from_u32(ch) {
                        Some(c) => {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        // (json < 3.0: a lone trailing surrogate, as its 3 bytes)
                        None => out.extend_from_slice(&[0xe0 | (ch >> 12) as u8, 0x80 | ((ch >> 6) & 0x3f) as u8, 0x80 | (ch & 0x3f) as u8]),
                    }
                    pe += 1;
                    p = pe;
                }
                _ => {
                    if c < 0x20 && !self.c.allow_control_characters {
                        return Err(match c {
                            b'\n' => self.raise_at("Invalid unescaped newline character (\\n) in string: %s", pe - 1),
                            0 => self.raise_at("Invalid unescaped NULL character (\\0) in string: %s", pe - 1),
                            _ => self.raise_at("invalid ASCII control character in string: %s", pe - 1),
                        });
                    }
                    if self.c.allow_invalid_escape {
                        out.push(c);
                        pe += 1;
                        p = pe;
                    } else {
                        return Err(self.raise_at("invalid escape character in string: %s", pe - 1));
                    }
                }
            }
        }
        if end > p {
            out.extend_from_slice(&self.b[p..end]);
        }
        Ok(out)
    }
    /// A string from the opening quote at the cursor; None if it isn't closed.
    fn string(&mut self, is_name: bool) -> Result<Option<V>, V> {
        self.i += 1;
        let start = self.i;
        let mut escaped = false;
        loop {
            while self.i < self.b.len() && !matches!(self.b[self.i], b'"' | b'\\' | 0..=0x1f) {
                self.i += 1;
            }
            if self.i >= self.b.len() {
                self.i = self.b.len();
                return Ok(None);
            }
            match self.b[self.i] {
                b'"' => break,
                b'\\' => {
                    escaped = true;
                    self.i += 2;
                }
                _ => {
                    if !self.c.allow_control_characters {
                        return Err(self.raise("invalid ASCII control character in string: %s"));
                    }
                    self.i += 1;
                }
            }
        }
        let end = self.i;
        let bytes = if escaped { self.unescape(start, end)? } else { self.b[start..end].to_vec() };
        self.i = end + 1;
        if is_name && self.c.symbolize_names {
            return Ok(Some(mksym(intern(&bytes))));
        }
        let s = str_new(bytes);
        if is_name || self.c.freeze {
            hdr(s).flags |= FL_FROZEN;
        }
        Ok(Some(s))
    }

    // ---------- values ----------

    fn push(&mut self, mut v: V) -> Result<(), V> {
        let c = self.c;
        if c.array_class != QNIL && kind(v) == Some(Kind::Array) {
            let a = tri_r(send(c.array_class, sym("new"), &[], QNIL))?;
            let mut i = 0;
            while let Some(&x) = rary(v).v.get(i) {
                tri_r(send(a, sym("<<"), &[x], QNIL))?;
                i += 1;
            }
            v = a;
        }
        if c.object_class != QNIL && kind(v) == Some(Kind::Hash) {
            let o = tri_r(send(c.object_class, sym("new"), &[], QNIL))?;
            let mut i = 0;
            while let Some((k, x)) = rhash(v).m.get_index(i).map(|(k, x)| (k.0, *x)) {
                tri_r(send(o, sym("[]="), &[k, x], QNIL))?;
                i += 1;
            }
            v = o;
        }
        if c.on_load != QNIL {
            v = tri_r(send(c.on_load, sym("call"), &[v], QNIL))?;
        }
        rary(self.vals).v.push(v);
        Ok(())
    }
    fn decode_array(&mut self, head: usize) -> V {
        let items: Vec<V> = rary(self.vals).v.drain(head..).collect();
        let a = ary_new(items);
        if self.c.freeze {
            hdr(a).flags |= FL_FROZEN;
        }
        a
    }
    fn decode_object(&mut self, head: usize) -> Result<V, V> {
        let h = hash_new();
        let n = self.nvals() - head;
        let mut k = head;
        while k < head + n {
            let (key, v) = (rary(self.vals).v[k], rary(self.vals).v[k + 1]);
            rhash(h).m.insert(HKey(key), v);
            k += 2;
        }
        if rhash(h).m.len() < n / 2 && !self.c.allow_duplicate_key {
            // the first key seen twice
            let seen = hash_new();
            let mut dup = QNIL;
            let mut k = head;
            while k < head + n {
                let key = rary(self.vals).v[k];
                let before = rhash(seen).m.len();
                rhash(seen).m.insert(HKey(key), QTRUE);
                if rhash(seen).m.len() == before {
                    dup = if is_sym(key) { str_new(sym_name(sym_id(key)).to_vec()) } else { key };
                    break;
                }
                k += 2;
            }
            let mut msg = b"duplicate key ".to_vec();
            msg.extend(crate::fmt::inspect(dup).map_err(|_| UNDEF)?);
            return Err(self.error(msg, Some(dup)));
        }
        rary(self.vals).v.truncate(head);
        if self.c.freeze {
            hdr(h).flags |= FL_FROZEN;
        }
        Ok(h)
    }

    /// The state machine of the C parser's json_parse_any. Ok(false): end of input where a
    /// value should be.
    fn parse_any(&mut self) -> Result<bool, V> {
        let mut ph = Ph::Value;
        loop {
            match ph {
                Ph::Done => return Ok(true),
                Ph::Value => {
                    self.ws()?;
                    let ft = self.frames.last().unwrap().t;
                    if self.c.allow_trailing_comma && ft == Ft::Array && self.peek() == b']' {
                        ph = Ph::ArrayComma;
                        continue;
                    }
                    let vs = self.i;
                    let value = match self.peek() {
                        b'n' => {
                            self.keyword(b"null")?;
                            QNIL
                        }
                        b't' => {
                            self.keyword(b"true")?;
                            QTRUE
                        }
                        b'f' => {
                            self.keyword(b"false")?;
                            QFALSE
                        }
                        b'N' => {
                            if !self.c.allow_nan {
                                return Err(self.raise("unexpected token %s"));
                            }
                            self.keyword(b"NaN")?;
                            float_new(f64::NAN)
                        }
                        b'I' => {
                            if !self.c.allow_nan {
                                return Err(self.raise("unexpected token %s"));
                            }
                            self.keyword(b"Infinity")?;
                            float_new(f64::INFINITY)
                        }
                        b'-' => {
                            self.i += 1;
                            match self.number(true, vs)? {
                                Some(v) => v,
                                None if self.c.allow_nan && self.peek() == b'I' => {
                                    self.i = vs;
                                    self.keyword(b"-Infinity")?;
                                    float_new(f64::NEG_INFINITY)
                                }
                                None => return Err(self.raise_at("invalid number: %s", vs)),
                            }
                        }
                        b'0'..=b'9' => match self.number(false, vs)? {
                            Some(v) => v,
                            None => return Err(self.raise_at("invalid number: %s", vs)),
                        },
                        b'"' => match self.string(false)? {
                            Some(v) => v,
                            None => return Err(self.raise("unexpected end of input, expected closing \"")),
                        },
                        b'[' => {
                            self.i += 1;
                            self.ws()?;
                            if self.peek() == b']' {
                                self.i += 1;
                                let n = self.nvals();
                                self.decode_array(n)
                            } else {
                                self.nesting += 1;
                                if self.c.max_nesting != 0 && self.c.max_nesting < self.nesting {
                                    return Err(self.nesting_error());
                                }
                                self.frames.push(Frame { t: Ft::Array, ph: Ph::Value, head: self.nvals(), start: vs });
                                continue;
                            }
                        }
                        b'{' => {
                            self.i += 1;
                            self.ws()?;
                            if self.peek() == b'}' {
                                self.i += 1;
                                let n = self.nvals();
                                self.decode_object(n)?
                            } else {
                                self.nesting += 1;
                                if self.c.max_nesting != 0 && self.c.max_nesting < self.nesting {
                                    return Err(self.nesting_error());
                                }
                                self.frames.push(Frame { t: Ft::Object, ph: Ph::ObjectKey, head: self.nvals(), start: vs });
                                ph = Ph::ObjectKey;
                                continue;
                            }
                        }
                        0 => {
                            if self.eos() {
                                return Ok(false);
                            }
                            return Err(self.raise("unexpected NULL byte: %s"));
                        }
                        _ => return Err(self.raise("unexpected character: %s")),
                    };
                    self.push(value)?;
                    ph = completed(self.frames.last_mut().unwrap());
                }
                Ph::ObjectKey => {
                    self.ws()?;
                    if self.c.allow_trailing_comma && self.peek() == b'}' {
                        ph = Ph::ObjectComma;
                        continue;
                    }
                    if self.peek() == b'"' {
                        let Some(k) = self.string(true)? else {
                            return Err(self.raise("unexpected end of input, expected closing \""));
                        };
                        self.push(k)?;
                        self.frames.last_mut().unwrap().ph = Ph::ObjectColon;
                        ph = Ph::ObjectColon;
                    } else {
                        let f = self.frames.last().unwrap();
                        return Err(if self.nvals() - f.head == 0 {
                            self.raise("expected object key, got %s")
                        } else {
                            self.raise("expected object key, got: %s")
                        });
                    }
                }
                Ph::ObjectColon => {
                    self.ws()?;
                    if self.peek() == b':' {
                        self.i += 1;
                        self.frames.last_mut().unwrap().ph = Ph::Value;
                        ph = Ph::Value;
                    } else {
                        let f = self.frames.last().unwrap();
                        return Err(if self.nvals() - f.head == 1 {
                            self.raise("expected ':' after object key")
                        } else {
                            self.raise("expected ':' after object key, got: %s")
                        });
                    }
                }
                Ph::ArrayComma => {
                    self.ws()?;
                    match self.peek() {
                        b',' => {
                            self.i += 1;
                            self.frames.last_mut().unwrap().ph = Ph::Value;
                            ph = Ph::Value;
                        }
                        b']' => {
                            self.i += 1;
                            self.nesting -= 1;
                            let head = self.frames.last().unwrap().head;
                            let a = self.decode_array(head);
                            self.push(a)?;
                            self.frames.pop();
                            ph = completed(self.frames.last_mut().unwrap());
                        }
                        _ => return Err(self.raise("expected ',' or ']' after array value")),
                    }
                }
                Ph::ObjectComma => {
                    self.ws()?;
                    match self.peek() {
                        b',' => {
                            self.i += 1;
                            self.frames.last_mut().unwrap().ph = Ph::ObjectKey;
                            ph = Ph::ObjectKey;
                        }
                        b'}' => {
                            self.i += 1;
                            self.nesting -= 1;
                            let (head, start) = {
                                let f = self.frames.last().unwrap();
                                (f.head, f.start)
                            };
                            // (an error points at the object's opening brace)
                            let end = self.i;
                            self.i = start;
                            let h = self.decode_object(head)?;
                            self.i = end;
                            self.push(h)?;
                            self.frames.pop();
                            ph = completed(self.frames.last_mut().unwrap());
                        }
                        _ => return Err(self.raise("expected ',' or '}' after object value, got: %s")),
                    }
                }
            }
        }
    }
}

/// The bytes of a document: `to_str`, converted to UTF-8 if in another encoding.
pub fn source(src: V) -> Result<V, V> {
    let mut s = src;
    if !is_str(s) {
        s = super::source_bytes(src).map(str_new)?;
    }
    let f = hdr(s).flags;
    if f & crate::compat::FL_BINARY != 0 {
        let e = crate::transcode::enc_of(s);
        if e.codec != crate::transcode::Codec::Binary && !rstr(s).b.is_ascii() {
            let r = send(s, sym("encode"), &[str_from("UTF-8")], QNIL);
            if r == UNDEF {
                return Err(UNDEF);
            }
            return Ok(r);
        }
    }
    Ok(s)
}

pub fn parse(src: V, c: &Config) -> V {
    let s = match source(src) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // (a copy: the document can't change under the parser)
    let bytes = rstr(s).b.to_vec();
    let mut st = St {
        b: &bytes,
        i: 0,
        c,
        vals: ary_new(Vec::new()),
        frames: vec![Frame { t: Ft::Root, ph: Ph::Value, head: 0, start: 0 }],
        nesting: 0,
    };
    let r = (|| {
        if !st.parse_any()? {
            return Err(st.raise("unexpected end of input"));
        }
        let v = rary(st.vals).v[0];
        st.ws()?;
        if !st.eos() {
            return Err(st.raise("unexpected token at end of stream %s"));
        }
        Ok(v)
    })();
    match r {
        Ok(v) => v,
        Err(e) => e,
    }
}

/// `JSON.parse(source, opts)` and friends: `base` adjusted by the options Hash `opts`.
pub fn parse_with(src: V, opts: Option<V>, base: Config) -> V {
    let mut c = base;
    if let Some(h) = opts {
        if let Err(e) = c.apply(h, true) {
            return e;
        }
    }
    parse(src, &c)
}
