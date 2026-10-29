//! YAML (the Psych API subset Rails uses), implemented natively.
//!
//! * Loading: `saphyr-parser` produces events, which are built into a node tree and then
//!   converted to Ruby objects following Psych's `ToRuby` visitor and `ScalarScanner`
//!   (YAML 1.1 scalar resolution, merge keys, anchors/aliases, safe-load class checks).
//! * Dumping: a port of Psych's `YAMLTree` visitor feeding a port of the libyaml 0.2.5
//!   emitter (style selection, quoting and line folding), so output matches CRuby byte for byte.
//!
//! The Ruby-level API (`Psych.load`, `YAML.dump`, exception classes, `Psych::Coder`) lives in
//! the overlay file `lib/kenny/psych.rb`, which calls `Psych.__kn_load` / `Psych.__kn_dump`.
//! The tree API (`Psych.parse`, `Psych::Nodes`, `Psych::Visitors::YAMLTree` ...) is psych's
//! own Ruby code, loaded on first use by `lib/kenny/kenny/psych_tree.rb`, on the two native
//! pieces libyaml is to CRuby's psych: parse events (`Psych.__kn_events`) and the emitter
//! (`Psych.__kn_emit`, one document's events at a time).

use crate::core::*;
use crate::gc::RVec;
use crate::hash::{hset, pairs};
use onig::{Regex, RegexOptions, Syntax};
use saphyr_parser::{Event, Parser, ScalarStyle};
use std::collections::HashMap;

type R = Result<V, ()>;

/// A raise helper's result (always UNDEF) as an error.
fn fail<T>(_raised: V) -> Result<T, ()> {
    Err(())
}
fn ck(v: V) -> R {
    if v == UNDEF { Err(()) } else { Ok(v) }
}
fn s_of(v: V) -> String {
    String::from_utf8_lossy(&rstr(v).b).into_owned()
}
/// Ruby `to_s` of a value as a Rust string ("" if it raised).
fn rstring(v: V) -> String {
    if is_str(v) {
        return s_of(v);
    }
    if is_sym(v) {
        return sym_str(sym_id(v));
    }
    match crate::fmt::to_s(v) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(()) => String::new(),
    }
}
fn cls(name: &str) -> Option<V> {
    const_lookup(rt().c.object, sym(name))
}
/// Constant lookup by path ("A::B").
fn path2class(path: &str) -> Option<V> {
    let mut cur = rt().c.object;
    let mut v = None;
    for part in path.split("::").filter(|p| !p.is_empty()) {
        let c = const_get_under(cur, sym(part))?;
        if !is_class(c) {
            return None;
        }
        cur = c as *mut Class;
        v = Some(c);
    }
    v
}
fn psych_const(name: &str) -> *mut Class {
    let p = cls("Psych").unwrap_or(rt().c.object as V) as *mut Class;
    const_get_under(p, sym(name)).map(|c| c as *mut Class).unwrap_or(rt().c.runtime_error)
}
fn raise_new(k: *mut Class, args: &[V]) -> V {
    let e = send(k as V, sym("new"), args, QNIL);
    if e == UNDEF {
        return UNDEF;
    }
    raise_exc(e)
}

// ---------------------------------------------------------------------------
// regexes (Psych's, compiled with Ruby syntax so ^/$ are line anchors as in Ruby)
// ---------------------------------------------------------------------------

struct Rx {
    strish: Regex,
    newline: Regex,
    not_ytonf: Regex,
    null: Regex,
    yes: Regex,
    no: Regex,
    time: Regex,
    date: Regex,
    pinf: Regex,
    ninf: Regex,
    nan: Regex,
    sym: Regex,
    sym_quoted: Regex,
    sexa_int: Regex,
    sexa_float: Regex,
    float: Regex,
    dot_only: Regex,
    int_strict: Regex,
    int_legacy: Regex,
    // dumping
    literal_nl: Regex,
    nonword_start: Regex,
    octal89: Regex,
}

fn re(p: &str) -> Regex {
    Regex::with_options(p, RegexOptions::REGEX_OPTION_NONE, Syntax::ruby()).expect("psych regex")
}
fn rex(p: &str) -> Regex {
    Regex::with_options(p, RegexOptions::REGEX_OPTION_EXTEND, Syntax::ruby()).expect("psych regex")
}
fn rxi(p: &str) -> Regex {
    Regex::with_options(p, RegexOptions::REGEX_OPTION_IGNORECASE, Syntax::ruby()).expect("psych regex")
}

static RX: std::sync::OnceLock<Rx> = std::sync::OnceLock::new();
fn rx() -> &'static Rx {
    RX.get_or_init(|| Rx {
        strish: re(r"^[^\d.:-]?[[:alpha:]_\s!@#$%\^&*(){}<>|/\\~;=]+"),
        newline: re(r"\n"),
        not_ytonf: rxi(r"^[^ytonf~]"),
        null: rxi(r"^null$"),
        yes: rxi(r"^(yes|true|on)$"),
        no: rxi(r"^(no|false|off)$"),
        time: re(r"^-?\d{4}-\d{1,2}-\d{1,2}(?:[Tt]|\s+)\d{1,2}:\d\d:\d\d(?:\.\d*)?(?:\s*(?:Z|[-+]\d{1,2}:?(?:\d\d)?))?$"),
        date: re(r"^\d{4}-(?:1[012]|0\d|\d)-(?:[12]\d|3[01]|0\d|\d)$"),
        pinf: rxi(r"^\+?\.inf$"),
        ninf: rxi(r"^-\.inf$"),
        nan: rxi(r"^\.nan$"),
        sym: re(r"^:."),
        sym_quoted: re(r#"^:(["'])(.*)\1"#),
        sexa_int: re(r"^[-+]?[0-9][0-9_]*(:[0-5]?[0-9]){1,2}$"),
        sexa_float: re(r"^[-+]?[0-9][0-9_]*(:[0-5]?[0-9]){1,2}\.[0-9_]*$"),
        float: rex(r"^(?:[-+]?([0-9][0-9_,]*)?\.[0-9]*([eE][-+][0-9]+)?(?# base 10))$"),
        dot_only: re(r"\A[-+]?\.\Z"),
        int_strict: rex(
            r"^(?:[-+]?0b[0-1_]+                  (?# base 2)
                 |[-+]?0[0-7_]+                   (?# base 8)
                 |[-+]?(0|[1-9][0-9_]*)           (?# base 10)
                 |[-+]?0x[0-9a-fA-F_]+            (?# base 16))$",
        ),
        int_legacy: rex(
            r"^(?:[-+]?0b[0-1_,]+                        (?# base 2)
                 |[-+]?0[0-7_,]+                         (?# base 8)
                 |[-+]?(?:0|[1-9](?:[0-9]|,[0-9]|_[0-9])*) (?# base 10)
                 |[-+]?0x[0-9a-fA-F_,]+                  (?# base 16))$",
        ),
        literal_nl: re(r"\n(?!\Z)"),
        nonword_start: re(r#"^[^[:word:]][^"]*$"#),
        octal89: re(r"\A0[0-7]*[89]"),
    })
}
fn m(r: &Regex, s: &str) -> bool {
    r.search_with_options(s, 0, s.len(), onig::SearchOptions::SEARCH_OPTION_NONE, None).is_some()
}

// ---------------------------------------------------------------------------
// ScalarScanner#tokenize
// ---------------------------------------------------------------------------

enum Tok {
    Str,
    Nil,
    Bool(bool),
    Int(V),
    Float(f64),
    Sym(String),
    Time,
    Date(i64, i64, i64),
}

/// Ruby `Integer(str)` for the strings Psych's INTEGER regex accepts (after deleting `,_`).
fn ruby_integer(s: &str) -> Option<V> {
    let (neg, b) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = b.to_ascii_lowercase();
    let (radix, digits) = if let Some(r) = lower.strip_prefix("0x") {
        (16, r.to_string())
    } else if let Some(r) = lower.strip_prefix("0b") {
        (2, r.to_string())
    } else if let Some(r) = lower.strip_prefix("0o") {
        (8, r.to_string())
    } else if lower.len() > 1 && lower.starts_with('0') {
        (8, lower[1..].to_string())
    } else {
        (10, lower)
    };
    if digits.is_empty() {
        return None;
    }
    let n = num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(int_from_big(if neg { -n } else { n }))
}
/// Ruby `String#to_i` (leading integer, underscores allowed).
fn ruby_to_i(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, b) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut n: i64 = 0;
    for c in b.chars() {
        if let Some(d) = c.to_digit(10) {
            n = n.saturating_mul(10).saturating_add(d as i64);
        } else if c != '_' {
            break;
        }
    }
    if neg { -n } else { n }
}
fn ruby_to_f(s: &str) -> f64 {
    let t: String = s.trim_start().chars().filter(|c| *c != '_').collect();
    let mut end = 0;
    let b = t.as_bytes();
    let mut seen_e = false;
    let mut seen_dot = false;
    while end < b.len() {
        let c = b[end];
        let ok = c.is_ascii_digit()
            || ((c == b'-' || c == b'+') && (end == 0 || matches!(b[end - 1], b'e' | b'E')))
            || (c == b'.' && !seen_dot && !seen_e)
            || ((c == b'e' || c == b'E') && !seen_e && end > 0);
        if !ok {
            break;
        }
        seen_dot |= c == b'.';
        seen_e |= c == b'e' || c == b'E';
        end += 1;
    }
    let mut t = &t[..end];
    while !t.is_empty() && t.parse::<f64>().is_err() {
        t = &t[..t.len() - 1];
    }
    t.parse().unwrap_or(0.0)
}

fn tokenize(s: &str, strict_int: bool) -> Tok {
    let x = rx();
    if s.is_empty() {
        return Tok::Nil;
    }
    if m(&x.strish, s) || m(&x.newline, s) {
        if s.chars().count() > 5 {
            return Tok::Str;
        }
        return if m(&x.not_ytonf, s) {
            Tok::Str
        } else if s == "~" || m(&x.null, s) {
            Tok::Nil
        } else if m(&x.yes, s) {
            Tok::Bool(true)
        } else if m(&x.no, s) {
            Tok::Bool(false)
        } else {
            Tok::Str
        };
    }
    if m(&x.time, s) {
        return Tok::Time;
    }
    if m(&x.date, s) {
        let p: Vec<i64> = s.split('-').map(|p| p.parse().unwrap_or(0)).collect();
        return Tok::Date(p[0], p[1], p[2]);
    }
    if m(&x.pinf, s) {
        return Tok::Float(f64::INFINITY);
    }
    if m(&x.ninf, s) {
        return Tok::Float(f64::NEG_INFINITY);
    }
    if m(&x.nan, s) {
        return Tok::Float(f64::NAN);
    }
    if m(&x.sym, s) {
        if let Some(c) = x.sym_quoted.captures(s) {
            let inner = c.at(2).unwrap_or("");
            return Tok::Sym(inner.strip_prefix(':').unwrap_or(inner).to_string());
        }
        return Tok::Sym(s[1..].to_string());
    }
    if m(&x.sexa_int, s) {
        let parts: Vec<&str> = s.split(':').collect();
        let mut i: i64 = 0;
        for (e, n) in parts.iter().enumerate() {
            i += ruby_to_i(n) * 60i64.pow((e as i64 - 2).unsigned_abs() as u32);
        }
        return Tok::Int(mkfix(i));
    }
    if m(&x.sexa_float, s) {
        let parts: Vec<&str> = s.split(':').collect();
        let mut f = 0.0;
        for (e, n) in parts.iter().enumerate() {
            f += ruby_to_f(n) * 60f64.powi((e as i32 - 2).abs());
        }
        return Tok::Float(f);
    }
    if m(&x.float, s) {
        if m(&x.dot_only, s) {
            return Tok::Str;
        }
        let mut t: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();
        // gsub(/\.([Ee]|$)/, '\1')
        if let Some(i) = t.find('.') {
            let rest = &t[i + 1..];
            if rest.is_empty() || rest.starts_with('e') || rest.starts_with('E') {
                t.remove(i);
            }
        }
        return match t.parse::<f64>() {
            Ok(f) => Tok::Float(f),
            Err(_) => Tok::Str,
        };
    }
    let ir = if strict_int { &x.int_strict } else { &x.int_legacy };
    if m(ir, s) {
        let t: String = s.chars().filter(|c| *c != ',' && *c != '_').collect();
        return match ruby_integer(&t) {
            Some(v) => Tok::Int(v),
            None => Tok::Str,
        };
    }
    Tok::Str
}

// ---------------------------------------------------------------------------
// node tree
// ---------------------------------------------------------------------------

enum NK {
    Scalar(String, bool /* plain style */),
    Seq(Vec<usize>),
    Map(Vec<usize>),
    Alias(usize),
}
struct Node {
    k: NK,
    tag: Option<String>,
    anchor: usize,
}
impl Node {
    fn quoted(&self) -> bool {
        matches!(self.k, NK::Scalar(_, false)) && self.tag.is_none()
    }
}

struct PErr {
    line: usize,
    col: usize,
    offset: usize,
    problem: String,
    context: Option<String>,
    unknown_anchor: Option<String>,
}

/// Parse a stream into documents (root node indices into `nodes`).
fn parse_stream(src: &str, first_only: bool, nodes: &mut Vec<Node>) -> Result<Vec<usize>, PErr> {
    let mut p = Parser::new_from_str(src);
    let mut docs = Vec::new();
    // stack of open collections: (node index)
    let mut stack: Vec<usize> = Vec::new();
    let mut root: Option<usize> = None;
    loop {
        let ev = match p.next_event() {
            None => break,
            Some(Ok(e)) => e,
            Some(Err(e)) => {
                let mk = e.marker();
                let info = e.info().to_string();
                let unknown_anchor = if info.contains("unknown anchor") {
                    let rest = &src[mk.index().min(src.len())..];
                    let rest = rest.strip_prefix('*').unwrap_or(rest);
                    Some(rest.chars().take_while(|c| !c.is_whitespace() && !",[]{}".contains(*c)).collect())
                } else {
                    None
                };
                let (problem, context) = match info.split_once(", ") {
                    Some((ctx, prob)) if ctx.starts_with("while ") => (prob.to_string(), Some(ctx.to_string())),
                    _ => (info, None),
                };
                return Err(PErr { line: mk.line(), col: mk.col() + 1, offset: mk.index(), problem, context, unknown_anchor });
            }
        };
        let (ev, _span) = ev;
        let push = |nodes: &mut Vec<Node>, n: Node, stack: &mut Vec<usize>, root: &mut Option<usize>| -> usize {
            let i = nodes.len();
            nodes.push(n);
            match stack.last() {
                Some(&parent) => match &mut nodes[parent].k {
                    NK::Seq(c) | NK::Map(c) => c.push(i),
                    _ => {}
                },
                None => *root = Some(i),
            }
            i
        };
        let tag_s = |t: &Option<std::borrow::Cow<saphyr_parser::Tag>>| t.as_ref().map(|t| format!("{}{}", t.handle, t.suffix));
        match ev {
            Event::StreamStart | Event::StreamEnd | Event::Nothing => {}
            Event::DocumentStart(_) => {
                root = None;
            }
            Event::DocumentEnd => {
                if let Some(r) = root.take() {
                    docs.push(r);
                }
                if first_only && !docs.is_empty() {
                    break;
                }
            }
            Event::Alias(id) => {
                push(nodes, Node { k: NK::Alias(id), tag: None, anchor: 0 }, &mut stack, &mut root);
            }
            Event::Scalar(v, style, anchor, tag) => {
                let plain = matches!(style, ScalarStyle::Plain);
                let t = tag_s(&tag);
                push(nodes, Node { k: NK::Scalar(v.into_owned(), plain), tag: t, anchor }, &mut stack, &mut root);
            }
            Event::SequenceStart(anchor, tag) => {
                let t = tag_s(&tag);
                let i = push(nodes, Node { k: NK::Seq(Vec::new()), tag: t, anchor }, &mut stack, &mut root);
                stack.push(i);
            }
            Event::MappingStart(anchor, tag) => {
                let t = tag_s(&tag);
                let i = push(nodes, Node { k: NK::Map(Vec::new()), tag: t, anchor }, &mut stack, &mut root);
                stack.push(i);
            }
            Event::SequenceEnd | Event::MappingEnd => {
                stack.pop();
            }
        }
    }
    Ok(docs)
}

// ---------------------------------------------------------------------------
// ToRuby
// ---------------------------------------------------------------------------

struct Loader<'a> {
    nodes: &'a [Node],
    restricted: bool,
    classes: Vec<String>,
    symbols: Vec<String>,
    aliases: bool,
    symbolize: bool,
    freeze: bool,
    strict_int: bool,
    anchors: HashMap<usize, V>,
    pins: RVec,
}

impl Loader<'_> {
    fn pin(&mut self, v: V) -> V {
        self.pins.push(v);
        v
    }
    fn disallowed(&self, name: &str) -> V {
        raise_new(psych_const("DisallowedClass"), &[str_from("load"), str_from(name)])
    }
    /// ClassLoader#load (with the Restricted check).
    fn load_class(&mut self, name: &str) -> R {
        if self.restricted && !self.classes.iter().any(|c| c == name) {
            return fail(self.disallowed(name));
        }
        if let Some(c) = path2class(name).or_else(|| path2class(&format!("Struct::{name}"))) {
            return Ok(c);
        }
        fail(arg_error(&format!("undefined class/module {name}")))
    }
    fn check_class(&mut self, name: &str) -> Result<(), ()> {
        if self.restricted && !self.classes.iter().any(|c| c == name) {
            ck(self.disallowed(name))?;
            return Err(());
        }
        Ok(())
    }
    fn symbolize(&mut self, s: &str) -> R {
        self.check_class("Symbol")?;
        if self.restricted && !self.symbols.is_empty() && !self.symbols.iter().any(|x| x == s) {
            ck(self.disallowed("Symbol"))?;
            return Err(());
        }
        Ok(mksym(intern(s.as_bytes())))
    }
    fn register(&mut self, idx: usize, v: V) -> V {
        let a = self.nodes[idx].anchor;
        if a != 0 {
            self.anchors.insert(a, v);
        }
        self.pin(v)
    }

    fn accept(&mut self, idx: usize) -> R {
        let r = self.visit(idx)?;
        if self.freeze {
            let r = if is_str(r) && !is_frozen(r) { ck(send0(r, "-@"))? } else { r };
            return ck(send0(r, "freeze"));
        }
        Ok(r)
    }

    fn visit(&mut self, idx: usize) -> R {
        let n = &self.nodes[idx];
        match &n.k {
            NK::Alias(id) => {
                if !self.aliases {
                    return fail(raise_new(psych_const("AliasesNotEnabled"), &[]));
                }
                match self.anchors.get(id) {
                    Some(&v) => Ok(v),
                    None => fail(raise_new(psych_const("AnchorNotDefined"), &[str_from(&id.to_string())])),
                }
            }
            NK::Scalar(..) => {
                let v = self.deserialize(idx)?;
                Ok(self.register(idx, v))
            }
            NK::Seq(ch) => {
                let ch = ch.clone();
                let tag = n.tag.clone();
                match tag.as_deref() {
                    Some(t) if t.starts_with("!ruby/array:") || t.starts_with("!seq:") => {
                        let name = t.split_once(':').map(|x| x.1).unwrap_or("");
                        let k = self.load_class(name)?;
                        let list = ck(send0(k, "allocate"))?;
                        self.register(idx, list);
                        for c in ch {
                            let v = self.accept(c)?;
                            ck(send1(list, "push", v))?;
                        }
                        Ok(list)
                    }
                    _ => {
                        let list = ary_new(Vec::new());
                        self.register(idx, list);
                        for c in ch {
                            let v = self.accept(c)?;
                            rary(list).v.push(v);
                        }
                        Ok(list)
                    }
                }
            }
            NK::Map(ch) => {
                let ch = ch.clone();
                self.visit_map(idx, ch)
            }
        }
    }

    fn scalar_value(&self, idx: usize) -> &str {
        match &self.nodes[idx].k {
            NK::Scalar(s, _) => s,
            _ => "",
        }
    }

    fn tok_to_ruby(&mut self, s: &str, t: Tok) -> R {
        Ok(match t {
            Tok::Str => str_from(s),
            Tok::Nil => QNIL,
            Tok::Bool(b) => bool_v(b),
            Tok::Int(v) => v,
            Tok::Float(f) => float_new(f),
            Tok::Sym(name) => self.symbolize(&name)?,
            Tok::Time => match self.parse_time(s)? {
                Some(v) => v,
                None => str_from(s),
            },
            Tok::Date(y, mo, d) => {
                self.check_class("Date")?;
                match cls("Date") {
                    Some(dc) => {
                        let r = send(dc, sym("new"), &[mkfix(y), mkfix(mo), mkfix(d)], QNIL);
                        if r == UNDEF {
                            // ArgumentError (invalid date) -> keep the string
                            if let Throw::Raise(e) = pst().pending {
                                if is_a(e, rt().c.argument_error) {
                                    pst().pending = Throw::None;
                                    return Ok(str_from(s));
                                }
                            }
                            return Err(());
                        }
                        r
                    }
                    None => str_from(s),
                }
            }
        })
    }

    /// ScalarScanner#parse_time: build the Time through `Time.at`.
    fn parse_time(&mut self, s: &str) -> Result<Option<V>, ()> {
        self.check_class("Time")?;
        let Some(tc) = cls("Time") else { return Ok(None) };
        let (date, time) = match s.find(['T', 't', ' ']) {
            Some(i) => (&s[..i], s[i + 1..].trim_start()),
            None => return Ok(None),
        };
        let neg_year = date.starts_with('-');
        let dp: Vec<i64> = date.trim_start_matches('-').split('-').map(|x| x.parse().unwrap_or(0)).collect();
        let y = if neg_year { -dp[0] } else { dp[0] };
        let (mo, d) = (dp[1], dp[2]);
        // hh:mm:ss(.frac)? zone?
        let tb = time.as_bytes();
        let mut i = 0;
        while i < tb.len() && (tb[i].is_ascii_digit() || tb[i] == b':') {
            i += 1;
        }
        let hms: Vec<i64> = time[..i].split(':').map(|x| x.parse().unwrap_or(0)).collect();
        let mut frac = String::new();
        if i < tb.len() && tb[i] == b'.' {
            i += 1;
            while i < tb.len() && tb[i].is_ascii_digit() {
                frac.push(tb[i] as char);
                i += 1;
            }
        }
        let zone = time[i..].trim();
        let days = days_from_civil(y, mo, d);
        let mut secs = days * 86400 + hms[0] * 3600 + hms.get(1).copied().unwrap_or(0) * 60 + hms.get(2).copied().unwrap_or(0);
        let fracv: f64 = if frac.is_empty() { 0.0 } else { format!("0.{frac}").parse().unwrap_or(0.0) };
        let utc;
        if zone.is_empty() {
            // local time with the given wall clock: Psych uses Time.at(utc_wallclock.to_i)
            utc = false;
        } else if zone == "Z" {
            utc = true;
        } else {
            let (sign, rest) = (if zone.starts_with('-') { -1 } else { 1 }, zone.trim_start_matches(['+', '-']));
            let (hh, mm) = match rest.split_once(':') {
                Some((h, m)) => (h.parse::<i64>().unwrap_or(0), m.parse::<i64>().unwrap_or(0)),
                None if rest.len() > 2 => (rest[..rest.len() - 2].parse().unwrap_or(0), rest[rest.len() - 2..].parse().unwrap_or(0)),
                None => (rest.parse().unwrap_or(0), 0),
            };
            secs -= sign * (hh * 3600 + mm * 60);
            utc = false;
        }
        let t = ck(send1(tc, "at", if fracv == 0.0 { mkfix(secs) } else { float_new(secs as f64 + fracv) }))?;
        Ok(Some(if utc { ck(send0(t, "utc"))? } else { t }))
    }

    fn deserialize(&mut self, idx: usize) -> R {
        let n = &self.nodes[idx];
        let value = self.scalar_value(idx).to_string();
        if n.quoted() {
            return Ok(str_from(&value));
        }
        let Some(tag) = n.tag.clone() else {
            let t = tokenize(&value, self.strict_int);
            return self.tok_to_ruby(&value, t);
        };
        match tag.as_str() {
            "!binary" | "tag:yaml.org,2002:binary" => {
                let v = str_new(base64_decode(&value));
                hdr(v).flags |= crate::compat::FL_BINARY;
                Ok(v)
            }
            "tag:yaml.org,2002:str" | "!str" | "!ruby/string" => Ok(str_from(&value)),
            t if t.starts_with("!str:") || t.starts_with("!ruby/string:") => {
                let name = t.split_once(':').unwrap().1;
                let k = self.load_class(name)?;
                let o = ck(send0(k, "allocate"))?;
                ck(send1(o, "replace", str_from(&value)))?;
                Ok(o)
            }
            "!ruby/object:BigDecimal" => {
                self.check_class("BigDecimal")?;
                if cls("BigDecimal").is_none() {
                    crate::compat::require_feature("bigdecimal");
                }
                ck(send1(cls("BigDecimal").unwrap(), "_load", str_from(&value)))
            }
            "!ruby/class" | "!ruby/module" => self.load_class(&value),
            "tag:yaml.org,2002:float" | "!float" => {
                let t = tokenize(&value, self.strict_int);
                let v = self.tok_to_ruby(&value, t)?;
                ck(send(rt().main_obj, sym("Float"), &[v], QNIL))
            }
            "!ruby/regexp" => {
                self.check_class("Regexp")?;
                let (src, flags) = match (value.find('/'), value.rfind('/')) {
                    (Some(0), Some(e)) if e > 0 => (&value[1..e], &value[e + 1..]),
                    _ => (value.as_str(), ""),
                };
                let mut opts = 0u32;
                for c in flags.chars() {
                    match c {
                        'i' => opts |= 1,
                        'x' => opts |= 2,
                        'm' => opts |= 4,
                        _ => {}
                    }
                }
                ck(crate::regex::regexp_new(src.as_bytes(), opts))
            }
            "!ruby/range" => {
                self.check_class("Range")?;
                let (lo, excl, hi) = match value.find("...") {
                    Some(i) => (&value[..i], true, &value[i + 3..]),
                    None => match value.find("..") {
                        Some(i) => (&value[..i], false, &value[i + 2..]),
                        None => return Ok(str_from(&value)),
                    },
                };
                let a = { let t = tokenize(lo, self.strict_int); self.tok_to_ruby(lo, t)? };
                let b = { let t = tokenize(hi, self.strict_int); self.tok_to_ruby(hi, t)? };
                Ok(range_new(a, b, excl))
            }
            t if t.starts_with("!ruby/sym") => self.symbolize(&value),
            _ => {
                let t = tokenize(&value, self.strict_int);
                self.tok_to_ruby(&value, t)
            }
        }
    }

    fn init_with(&mut self, o: V, h: V, tag: &Option<String>) -> R {
        if respond_to(o, sym("init_with")) {
            let coder_k = psych_const("Coder");
            let c = ck(send1(coder_k as V, "new", tag.as_deref().map(str_from).unwrap_or(QNIL)))?;
            self.pin(c);
            ck(send1(c, "map=", h))?;
            ck(send1(o, "init_with", c))?;
        } else {
            for (k, v) in pairs(h) {
                let name = if is_sym(k) { sym_str(sym_id(k)) } else if is_str(k) { s_of(k) } else { crate::fmt::inspect_lossy(k) };
                ivar_set(o, sym(&format!("@{name}")), v);
            }
        }
        Ok(o)
    }

    fn revive(&mut self, k: V, idx: usize, ch: Vec<usize>) -> R {
        let o = ck(send0(k, "allocate"))?;
        self.register(idx, o);
        let h = self.pin(hash_new());
        self.revive_hash(h, &ch, true)?;
        let tag = self.nodes[idx].tag.clone();
        self.init_with(o, h, &tag)
    }

    fn visit_map(&mut self, idx: usize, ch: Vec<usize>) -> R {
        let tag = self.nodes[idx].tag.clone();
        let Some(t) = tag.as_deref() else {
            let h = hash_new();
            self.register(idx, h);
            return self.revive_hash(h, &ch, false);
        };
        if let Some(rest) = t.strip_prefix("!ruby/struct") {
            let name = rest.strip_prefix(':').unwrap_or("");
            if !name.is_empty() {
                let k = self.load_class(name)?;
                let s = ck(send0(k, "allocate"))?;
                self.register(idx, s);
                let members_v = ck(send0(s, "members"))?;
                let members: Vec<String> = rary(members_v).v.iter().map(|m| if is_sym(*m) { sym_str(sym_id(*m)) } else { String::new() }).collect();
                let extra = self.pin(hash_new());
                for pair in ch.chunks(2) {
                    let key = self.accept(pair[0])?;
                    let val = self.accept(pair[1])?;
                    let kname = if is_sym(key) { sym_str(sym_id(key)) } else if is_str(key) { s_of(key) } else { String::new() };
                    if members.contains(&kname) {
                        ck(send(s, sym(&format!("{kname}=")), &[val], QNIL))?;
                    } else {
                        hset(extra, str_from(kname.trim_start_matches('@')), val);
                    }
                }
                return self.init_with(s, extra, &tag);
            }
            let h = self.pin(hash_new());
            self.revive_hash(h, &ch, true)?;
            return Ok(self.register(idx, h));
        }
        if let Some(rest) = t.strip_prefix("!ruby/object") {
            let name = rest.strip_prefix(':').unwrap_or("Object");
            if name == "Hash" {
                let h = hash_new();
                self.register(idx, h);
                return self.revive_hash(h, &ch, false);
            }
            let k = self.load_class(if name.is_empty() { "Object" } else { name })?;
            return self.revive(k, idx, ch);
        }
        if let Some(rest) = t.strip_prefix("!ruby/exception") {
            let name = rest.strip_prefix(':').unwrap_or("Exception");
            let k = self.load_class(if name.is_empty() { "Exception" } else { name })?;
            let h = self.pin(hash_new());
            self.revive_hash(h, &ch, true)?;
            let msg = crate::hash::hget(h, str_from("message"));
            let e = ck(send(k, sym("new"), if msg == QNIL { &[] } else { std::slice::from_ref(&msg) }, QNIL))?;
            self.register(idx, e);
            let rest = self.pin(hash_new());
            for (kk, v) in pairs(h) {
                let ks = if is_str(kk) { s_of(kk) } else { String::new() };
                if ks == "backtrace" {
                    ck(send1(e, "set_backtrace", v))?;
                } else if ks != "message" {
                    hset(rest, kk, v);
                }
            }
            return self.init_with(e, rest, &tag);
        }
        if t == "!ruby/range" {
            self.check_class("Range")?;
            let h = self.pin(hash_new());
            self.revive_hash(h, &ch, true)?;
            let g = |k: &str| crate::hash::hget(h, str_from(k));
            let r = range_new(g("begin"), g("end"), truthy(g("excl")));
            return Ok(self.register(idx, r));
        }
        if let Some(rest) = t.strip_prefix("!ruby/hash-with-ivars") {
            let h = match rest.strip_prefix(':') {
                Some(name) => {
                    let k = self.load_class(name)?;
                    ck(send0(k, "allocate"))?
                }
                None => hash_new(),
            };
            self.register(idx, h);
            for pair in ch.chunks(2) {
                let key = self.scalar_value(pair[0]).to_string();
                let vch = match &self.nodes[pair[1]].k {
                    NK::Map(c) => c.clone(),
                    _ => Vec::new(),
                };
                if key == "elements" {
                    self.revive_hash(h, &vch, false)?;
                } else if key == "ivars" {
                    for p in vch.chunks(2) {
                        let k = self.accept(p[0])?;
                        let v = self.accept(p[1])?;
                        ck(send(h, sym("instance_variable_set"), &[k, v], QNIL))?;
                    }
                }
            }
            return Ok(h);
        }
        if let Some(name) = t.strip_prefix("!ruby/hash:").or_else(|| t.strip_prefix("!map:")) {
            let k = self.load_class(name)?;
            let h = ck(send0(k, "allocate"))?;
            self.register(idx, h);
            return self.revive_hash(h, &ch, false);
        }
        let h = hash_new();
        self.register(idx, h);
        self.revive_hash(h, &ch, false)
    }

    fn hash_store(&mut self, h: V, k: V, v: V) -> Result<(), ()> {
        if is_hash(h) && real_class(h) == rt().c.hash {
            ck(hset(h, k, v))?;
        } else {
            ck(send(h, sym("[]="), &[k, v], QNIL))?;
        }
        Ok(())
    }
    fn merge_into(&mut self, h: V, src: V) -> Result<bool, ()> {
        if !is_hash(src) {
            return Ok(false);
        }
        for (k, v) in pairs(src) {
            self.hash_store(h, k, v)?;
        }
        Ok(true)
    }

    fn revive_hash(&mut self, h: V, ch: &[usize], tagged: bool) -> R {
        for pair in ch.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let (kn, vn) = (pair[0], pair[1]);
            let mut key = self.accept(kn)?;
            let val = self.accept(vn)?;
            let is_merge = is_str(key) && rstr(key).b == b"<<" && self.nodes[kn].tag.as_deref() != Some("tag:yaml.org,2002:str");
            if is_merge {
                match &self.nodes[vn].k {
                    NK::Alias(_) | NK::Map(_) => {
                        if !self.merge_into(h, val)? {
                            self.hash_store(h, key, val)?;
                        }
                    }
                    NK::Seq(_) => {
                        let tmp = self.pin(hash_new());
                        let items: Vec<V> = if is_ary(val) { rary(val).v.clone() } else { Vec::new() };
                        if items.iter().all(|x| is_hash(*x)) && is_ary(val) {
                            for x in items.iter().rev() {
                                self.merge_into(tmp, *x)?;
                            }
                            self.merge_into(h, tmp)?;
                        } else {
                            self.hash_store(h, key, val)?;
                        }
                    }
                    _ => self.hash_store(h, key, val)?,
                }
                continue;
            }
            if !tagged && self.symbolize && is_str(key) {
                key = mksym(intern(&rstr(key).b));
            } else if !self.freeze && is_str(key) && !is_frozen(key) {
                key = ck(send0(key, "-@"))?;
            }
            self.hash_store(h, key, val)?;
        }
        Ok(h)
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn base64_encode(b: &[u8]) -> String {
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match B64.iter().position(|&x| x == c) {
            Some(v) => v as u32,
            None => continue,
        };
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

fn opt(opts: V, name: &str) -> V {
    if is_hash(opts) { crate::hash::hget(opts, mksym(sym(name))) } else { QNIL }
}
fn names_of(v: V) -> Vec<String> {
    if !is_ary(v) {
        return Vec::new();
    }
    rary(v)
        .v
        .iter()
        .map(|x| {
            if is_class(*x) {
                rclass(*x).name.clone()
            } else if is_sym(*x) {
                sym_str(sym_id(*x))
            } else if is_str(*x) {
                s_of(*x)
            } else {
                crate::fmt::inspect_lossy(*x)
            }
        })
        .collect()
}

/// Psych.__kn_load(yaml, opts) -> Array of documents.
/// opts: filename:, unsafe:, permitted_classes:, permitted_symbols:, aliases:,
///       symbolize_names:, freeze:, strict_integer:, first_only:
fn kn_load(src: V, opts: V) -> V {
    if !is_str(src) {
        return type_error("YAML source must be a String");
    }
    let mut text = s_of(src);
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    let mut nodes = Vec::new();
    let docs = match parse_stream(&text, truthy(opt(opts, "first_only")), &mut nodes) {
        Ok(d) => d,
        Err(e) => {
            if let Some(name) = e.unknown_anchor {
                if !truthy(opt(opts, "unsafe")) && !truthy(opt(opts, "aliases")) {
                    return raise_new(psych_const("AliasesNotEnabled"), &[]);
                }
                return raise_new(psych_const("AnchorNotDefined"), &[str_from(&name)]);
            }
            let file = opt(opts, "filename");
            return raise_new(
                psych_const("SyntaxError"),
                &[
                    if file == QNIL { QNIL } else { crate::tri!(send0(file, "to_s")) },
                    mkfix(e.line as i64),
                    mkfix(e.col as i64),
                    mkfix(e.offset as i64),
                    str_from(&e.problem),
                    e.context.as_deref().map(str_from).unwrap_or(QNIL),
                ],
            );
        }
    };
    let unsafe_ = truthy(opt(opts, "unsafe"));
    let mut l = Loader {
        nodes: &nodes,
        restricted: !unsafe_,
        classes: names_of(opt(opts, "permitted_classes")),
        symbols: names_of(opt(opts, "permitted_symbols")),
        aliases: unsafe_ || truthy(opt(opts, "aliases")),
        symbolize: truthy(opt(opts, "symbolize_names")),
        freeze: truthy(opt(opts, "freeze")),
        strict_int: truthy(opt(opts, "strict_integer")),
        anchors: HashMap::new(),
        pins: RVec::new(),
    };
    let out = ary_new(Vec::new());
    l.pin(out);
    for d in docs {
        l.anchors.clear();
        match l.accept(d) {
            Ok(v) => rary(out).v.push(v),
            Err(()) => return UNDEF,
        }
    }
    out
}

// ---------------------------------------------------------------------------
// YAMLTree (Ruby objects -> emitter nodes)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Sty {
    Any,
    Plain,
    Single,
    Double,
    Literal,
    Folded,
}

enum DK {
    Scalar { v: String, plain: bool, quoted: bool, style: Sty },
    Seq(Vec<usize>, bool /* flow */),
    Map(Vec<usize>, bool /* flow */),
    Alias(String),
}
struct DN {
    k: DK,
    tag: Option<String>,
    implicit: bool,
    anchor: Option<String>,
}

struct Dumper {
    nodes: Vec<DN>,
    reg: HashMap<V, usize>,
    ids: HashMap<V, usize>,
    counter: usize,
    pins: RVec,
    line_width: Option<i64>,
    restricted: Option<(Vec<V>, Vec<V>, bool)>,
    stringify_names: bool,
}

impl Dumper {
    fn add(&mut self, n: DN) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }
    fn scalar(&mut self, v: &str, tag: Option<&str>, plain: bool, quoted: bool, style: Sty) -> usize {
        self.add(DN { k: DK::Scalar { v: v.to_string(), plain, quoted, style }, tag: tag.map(String::from), implicit: false, anchor: None })
    }
    fn plain(&mut self, v: &str) -> usize {
        self.scalar(v, None, true, false, Sty::Any)
    }
    fn start(&mut self, map: bool, tag: Option<String>, implicit: bool) -> usize {
        let k = if map { DK::Map(Vec::new(), false) } else { DK::Seq(Vec::new(), false) };
        self.add(DN { k, tag, implicit, anchor: None })
    }
    fn child(&mut self, parent: usize, c: usize) {
        match &mut self.nodes[parent].k {
            DK::Seq(v, _) | DK::Map(v, _) => v.push(c),
            _ => {}
        }
    }
    fn register(&mut self, v: V, n: usize) -> usize {
        // like Psych, Floats are never aliased
        if is_heap(v) && !is_float(v) {
            self.reg.insert(v, n);
            self.pins.push(v);
        }
        n
    }

    fn accept(&mut self, v: V) -> Result<usize, ()> {
        if let Some((perm, syms, aliases)) = &self.restricted {
            if !*aliases && is_heap(v) && self.reg.contains_key(&v) {
                ck(raise(psych_const("BadAlias"), "Tried to dump an aliased object"))?;
            }
            let k = real_class(v);
            let base_ok = v == QNIL
                || v == QTRUE
                || v == QFALSE
                || is_int(v)
                || is_float(v)
                || (is_str(v) && k == rt().c.string)
                || (is_ary(v) && k == rt().c.array)
                || (is_hash(v) && k == rt().c.hash);
            if is_sym(v) {
                if !perm.contains(&(rt().c.symbol as V)) && !syms.contains(&v) {
                    let name = format!("Symbol({})", crate::fmt::inspect_lossy(v));
                    ck(raise_new(psych_const("DisallowedClass"), &[str_from("dump"), str_from(&name)]))?;
                }
            } else if !base_ok && !perm.contains(&(k as V)) {
                ck(raise_new(psych_const("DisallowedClass"), &[str_from("dump"), str_from(&class_name(k))]))?;
            }
        }
        if is_heap(v) {
            if let Some(&node) = self.reg.get(&v) {
                let id = match self.ids.get(&v) {
                    Some(&i) => i,
                    None => {
                        self.counter += 1;
                        self.ids.insert(v, self.counter);
                        self.counter
                    }
                };
                let a = id.to_string();
                self.nodes[node].anchor = Some(a.clone());
                return Ok(self.add(DN { k: DK::Alias(a), tag: None, implicit: false, anchor: None }));
            }
        }
        let plain_core = {
            let k = real_class(v);
            k == rt().c.string || k == rt().c.array || k == rt().c.hash || k == rt().c.float
        };
        if is_heap(v) && !plain_core && respond_to(v, sym("encode_with")) {
            return self.dump_coder(v);
        }
        self.dispatch(v)
    }

    fn tag_for(&self, v: V, prefix: &str) -> String {
        let k = real_class(v);
        if k == rt().c.object { prefix.to_string() } else { format!("{prefix}:{}", class_name(k)) }
    }

    fn dump_coder(&mut self, o: V) -> Result<usize, ()> {
        let tag = self.tag_for(o, "!ruby/object");
        let c = ck(send1(psych_const("Coder") as V, "new", str_from(&tag)))?;
        self.pins.push(c);
        ck(send1(o, "encode_with", c))?;
        let ty = ck(send0(c, "type"))?;
        let ctag = ck(send0(c, "tag"))?;
        let tag_s = if ctag == QNIL { None } else { Some(rstring(ctag)) };
        let ty = if is_sym(ty) { sym_str(sym_id(ty)) } else { "map".into() };
        match ty.as_str() {
            "scalar" => {
                let sc = ck(send0(c, "scalar"))?;
                let s = rstring(sc);
                let st = ck(send0(c, "style"))?;
                let style = sty_of(st);
                let plain = tag_s.is_none();
                Ok(self.scalar(&s, tag_s.as_deref(), plain, false, style))
            }
            "seq" => {
                let seq = ck(send0(c, "seq"))?;
                let n = self.start(false, tag_s.clone(), tag_s.is_none());
                let items = if is_ary(seq) { rary(seq).v.clone() } else { Vec::new() };
                for x in items {
                    let cn = self.accept(x)?;
                    self.child(n, cn);
                }
                Ok(n)
            }
            "object" => {
                let ob = ck(send0(c, "object"))?;
                self.accept(ob)
            }
            _ => {
                let map = ck(send0(c, "map"))?;
                let implicit = truthy(ck(send0(c, "implicit"))?);
                let n = self.start(true, tag_s, implicit);
                self.register(o, n);
                if is_hash(map) {
                    for (k, v) in pairs(map) {
                        let kn = self.accept(k)?;
                        self.child(n, kn);
                        let vn = self.accept(v)?;
                        self.child(n, vn);
                    }
                }
                Ok(n)
            }
        }
    }

    fn dump_ivars(&mut self, n: usize, o: V) -> Result<(), ()> {
        // Kenny keeps Struct members in ivars: those are dumped as members already
        let members: Vec<u32> = if is_a(o, rt().c.struct_) { unsafe { (*real_class(o)).struct_members.clone() } } else { Vec::new() };
        for (id, val) in ivar_list(o) {
            let name = sym_str(id);
            if members.iter().any(|m| sym_str(*m) == name.trim_start_matches('@')) {
                continue;
            }
            let kn = self.plain(name.trim_start_matches('@'));
            self.child(n, kn);
            let vn = self.accept(val)?;
            self.child(n, vn);
        }
        Ok(())
    }

    fn dispatch(&mut self, v: V) -> Result<usize, ()> {
        let c = rt().c;
        if v == QNIL {
            return Ok(self.scalar("", Some("tag:yaml.org,2002:null"), true, false, Sty::Any));
        }
        if v == QTRUE || v == QFALSE {
            return Ok(self.plain(if v == QTRUE { "true" } else { "false" }));
        }
        if is_int(v) {
            return Ok(self.plain(&crate::fmt::int_to_s(v)));
        }
        if is_float(v) {
            let f = float_val(v);
            let s = if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if f > 0.0 { ".inf".into() } else { "-.inf".into() }
            } else {
                crate::fmt::float_to_s(f)
            };
            return Ok(self.plain(&s));
        }
        if is_sym(v) {
            let name = sym_str(sym_id(v));
            if name.is_empty() {
                return Ok(self.scalar("", Some("!ruby/symbol"), false, false, Sty::Any));
            }
            return Ok(self.plain(&format!(":{name}")));
        }
        if is_str(v) {
            return self.visit_string(v);
        }
        if is_ary(v) {
            let k = real_class(v);
            let (tag, implicit) = if k == c.array { (None, true) } else { (Some(format!("!ruby/array:{}", class_name(k))), false) };
            let n = self.start(false, tag, implicit);
            self.register(v, n);
            for x in crate::gc::Rooted::of(rary(v).v.clone()) {
                let cn = self.accept(x)?;
                self.child(n, cn);
            }
            return Ok(n);
        }
        if is_hash(v) {
            let k = real_class(v);
            let ivars = ivar_list(v);
            let n;
            if k == c.hash {
                n = self.start(true, None, true);
                self.register(v, n);
            } else if !ivars.is_empty() {
                n = self.start(true, Some(format!("!ruby/hash-with-ivars:{}", class_name(k))), false);
                self.register(v, n);
                let kn = self.plain("ivars");
                self.child(n, kn);
                let iv = self.start(true, None, true);
                self.child(n, iv);
                for (id, val) in ivars {
                    let kn = self.plain(&format!(":{}", sym_str(id)));
                    self.child(iv, kn);
                    let vn = self.accept(val)?;
                    self.child(iv, vn);
                }
                let kn = self.plain("elements");
                self.child(n, kn);
                let el = self.start(true, None, true);
                self.child(n, el);
                for (k2, v2) in pairs(v) {
                    let kn = self.accept(k2)?;
                    self.child(el, kn);
                    let vn = self.accept(v2)?;
                    self.child(el, vn);
                }
                return Ok(n);
            } else {
                n = self.start(true, Some(format!("!ruby/hash:{}", class_name(k))), false);
                self.register(v, n);
            }
            for (k2, v2) in pairs(v) {
                let k2 = if self.stringify_names && is_sym(k2) { str_from(&sym_str(sym_id(k2))) } else { k2 };
                let kn = self.accept(k2)?;
                self.child(n, kn);
                let vn = self.accept(v2)?;
                self.child(n, vn);
            }
            return Ok(n);
        }
        if is_range(v) {
            let r = rrange(v);
            let n = self.start(true, Some("!ruby/range".into()), false);
            self.register(v, n);
            for (name, x) in [("begin", r.lo), ("end", r.hi), ("excl", bool_v(r.excl))] {
                let kn = self.visit_string(str_from(name))?;
                self.child(n, kn);
                let vn = self.accept(x)?;
                self.child(n, vn);
            }
            return Ok(n);
        }
        if is_class(v) {
            let k = rclass(v);
            let what = if k.is_module { "module" } else { "class" };
            if k.name.is_empty() {
                return fail(type_error(&format!("can't dump anonymous {what}: {}", crate::fmt::inspect_lossy(v))));
            }
            let n = self.scalar(&k.name.clone(), Some(&format!("!ruby/{what}")), false, false, Sty::Single);
            return Ok(self.register(v, n));
        }
        let k = real_class(v);
        let kname = class_name(k);
        let isa = |name: &str| cls(name).is_some_and(|kc| is_class(kc) && is_a(v, kc as *mut Class));
        if isa("BigDecimal") {
            let d = ck(send0(v, "_dump"))?;
            return Ok(self.scalar(&rstring(d), Some("!ruby/object:BigDecimal"), false, false, Sty::Any));
        }
        if isa("Regexp") {
            let s = ck(send0(v, "inspect"))?;
            let n = self.scalar(&rstring(s), Some("!ruby/regexp"), false, false, Sty::Any);
            return Ok(self.register(v, n));
        }
        if isa("DateTime") {
            let s = ck(send1(v, "strftime", str_from("%Y-%m-%d %H:%M:%S.%9N %:z")))?;
            let n = self.scalar(&rstring(s), Some("!ruby/object:DateTime"), false, false, Sty::Any);
            return Ok(self.register(v, n));
        }
        if isa("Date") {
            let s = ck(send0(v, "to_s"))?;
            let n = self.plain(&rstring(s));
            return Ok(self.register(v, n));
        }
        if is_a(v, c.time) {
            let s = format_time(v)?;
            let n = self.plain(&s);
            return Ok(self.register(v, n));
        }
        if is_a(v, c.exception) {
            let n = self.start(true, Some(format!("!ruby/exception:{kname}")), false);
            let msg = ck(send0(v, "message"))?;
            let msg = ck(send0(msg, "to_s"))?;
            let kn = self.plain("message");
            self.child(n, kn);
            let vn = self.accept(msg)?;
            self.child(n, vn);
            let kn = self.plain("backtrace");
            self.child(n, kn);
            let bt = ck(send0(v, "backtrace"))?;
            let vn = self.accept(bt)?;
            self.child(n, vn);
            self.dump_ivars(n, v)?;
            return Ok(n);
        }
        if is_a(v, c.struct_) {
            let n = self.start(true, Some(if kname.is_empty() { "!ruby/struct".into() } else { format!("!ruby/struct:{kname}") }), false);
            self.register(v, n);
            let members = ck(send0(v, "members"))?;
            for mbr in crate::gc::Rooted::of(rary(members).v.clone()) {
                let kn = self.plain(&rstring(mbr));
                self.child(n, kn);
                let val = ck(send1(v, "[]", mbr))?;
                let vn = self.accept(val)?;
                self.child(n, vn);
            }
            self.dump_ivars(n, v)?;
            return Ok(n);
        }
        if is_proc(v) {
            return fail(type_error(&format!("can't dump {kname}")));
        }
        let tag = self.tag_for(v, "!ruby/object");
        let n = self.start(true, Some(tag), false);
        self.register(v, n);
        self.dump_ivars(n, v)?;
        Ok(n)
    }

    fn visit_string(&mut self, v: V) -> Result<usize, ()> {
        let b = &rstr(v).b;
        let binary = hdr(v).flags & crate::compat::FL_BINARY != 0 && !b.is_ascii();
        let mut plain = true;
        let mut quote = true;
        let mut style = Sty::Plain;
        let mut tag: Option<String> = None;
        let o: String;
        if binary || std::str::from_utf8(b).is_err() {
            o = base64_encode(b);
            tag = Some("!binary".into());
            style = Sty::Literal;
            plain = false;
            quote = false;
        } else {
            o = String::from_utf8(b.clone()).unwrap();
            let x = rx();
            if m(&x.literal_nl, &o) {
                style = Sty::Literal;
            } else if o == "<<" {
                style = Sty::Single;
                tag = Some("tag:yaml.org,2002:str".into());
                plain = false;
                quote = false;
            } else if o == "y" || o == "n" {
                style = Sty::Double;
            } else if self.line_width.is_some_and(|w| o.chars().count() as i64 > w) {
                style = Sty::Folded;
            } else if m(&x.nonword_start, &o) {
                style = Sty::Double;
            } else if !matches!(tokenize(&o, false), Tok::Str) || m(&x.octal89, &o) {
                style = Sty::Single;
            }
        }
        let k = real_class(v);
        if k != rt().c.string {
            tag = Some(format!("!ruby/string:{}", class_name(k)));
            plain = false;
            quote = false;
        }
        Ok(self.scalar(&o, tag.as_deref(), plain, quote, style))
    }
}

fn sty_of(v: V) -> Sty {
    match if is_fix(v) { fix(v) } else { 0 } {
        1 => Sty::Plain,
        2 => Sty::Single,
        3 => Sty::Double,
        4 => Sty::Literal,
        5 => Sty::Folded,
        _ => Sty::Any,
    }
}

fn format_time(t: V) -> Result<String, ()> {
    let utc = truthy(ck(send0(t, "utc?"))?);
    let base = ck(send1(t, "strftime", str_from("%Y-%m-%d %H:%M:%S")))?;
    let nsec = ck(send0(t, "nsec"))?;
    let mut s = format!("{}.{:09}", rstring(base), if is_fix(nsec) { fix(nsec) } else { 0 });
    if utc {
        s.push_str(" Z");
    } else {
        let off = ck(send0(t, "utc_offset"))?;
        let off = if is_fix(off) { fix(off) } else { 0 };
        let (sign, a) = if off < 0 { ('-', -off) } else { ('+', off) };
        s.push_str(&format!(" {sign}{:02}:{:02}", a / 3600, a % 3600 / 60));
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// libyaml emitter (port of the parts Psych's output exercises)
// ---------------------------------------------------------------------------

struct ScalarData {
    multiline: bool,
    flow_plain_allowed: bool,
    block_plain_allowed: bool,
    single_quoted_allowed: bool,
    block_allowed: bool,
}

fn is_break(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}
fn is_blankz(c: Option<char>) -> bool {
    match c {
        None => true,
        Some(c) => c == ' ' || c == '\t' || is_break(c) || c == '\0',
    }
}
/// libyaml's IS_PRINTABLE (byte-level definition: 4-byte characters and U+0085 are not printable).
fn is_printable(c: char) -> bool {
    let u = c as u32;
    u == 0x0A || (0x20..=0x7E).contains(&u) || (0xA0..=0xD7FF).contains(&u) || ((0xE000..=0xFFFD).contains(&u) && u != 0xFEFF)
}

fn analyze_scalar(v: &str) -> ScalarData {
    let s: Vec<char> = v.chars().collect();
    if s.is_empty() {
        return ScalarData { multiline: false, flow_plain_allowed: false, block_plain_allowed: true, single_quoted_allowed: true, block_allowed: false };
    }
    let (mut block_ind, mut flow_ind, mut line_breaks, mut special) = (false, false, false, false);
    let (mut leading_space, mut leading_break, mut trailing_space, mut trailing_break) = (false, false, false, false);
    let (mut break_space, mut space_break) = (false, false);
    let (mut prev_space, mut prev_break) = (false, false);
    if v.starts_with("---") || v.starts_with("...") {
        block_ind = true;
        flow_ind = true;
    }
    let mut preceded_ws = true;
    let mut followed_ws = is_blankz(s.get(1).copied());
    let n = s.len();
    for i in 0..n {
        let c = s[i];
        if i == 0 {
            if "#,[]{}&*!|>'\"%@`".contains(c) {
                flow_ind = true;
                block_ind = true;
            }
            if c == '?' || c == ':' {
                flow_ind = true;
                if followed_ws {
                    block_ind = true;
                }
            }
            if c == '-' && followed_ws {
                flow_ind = true;
                block_ind = true;
            }
        } else {
            if ",?[]{}".contains(c) {
                flow_ind = true;
            }
            if c == ':' {
                flow_ind = true;
                if followed_ws {
                    block_ind = true;
                }
            }
            if c == '#' && preceded_ws {
                flow_ind = true;
                block_ind = true;
            }
        }
        if !is_printable(c) {
            special = true;
        }
        if is_break(c) {
            line_breaks = true;
        }
        if c == ' ' {
            if i == 0 {
                leading_space = true;
            }
            if i == n - 1 {
                trailing_space = true;
            }
            if prev_break {
                break_space = true;
            }
            prev_space = true;
            prev_break = false;
        } else if is_break(c) {
            if i == 0 {
                leading_break = true;
            }
            if i == n - 1 {
                trailing_break = true;
            }
            if prev_space {
                space_break = true;
            }
            prev_space = false;
            prev_break = true;
        } else {
            prev_space = false;
            prev_break = false;
        }
        preceded_ws = is_blankz(Some(c));
        if i + 1 < n {
            followed_ws = is_blankz(s.get(i + 2).copied());
        }
    }
    let mut d = ScalarData { multiline: line_breaks, flow_plain_allowed: true, block_plain_allowed: true, single_quoted_allowed: true, block_allowed: true };
    if leading_space || leading_break || trailing_space || trailing_break {
        d.flow_plain_allowed = false;
        d.block_plain_allowed = false;
    }
    if trailing_space {
        d.block_allowed = false;
    }
    if break_space {
        d.flow_plain_allowed = false;
        d.block_plain_allowed = false;
        d.single_quoted_allowed = false;
    }
    if space_break || special {
        d.flow_plain_allowed = false;
        d.block_plain_allowed = false;
        d.single_quoted_allowed = false;
        d.block_allowed = false;
    }
    if line_breaks {
        d.flow_plain_allowed = false;
        d.block_plain_allowed = false;
    }
    if flow_ind {
        d.flow_plain_allowed = false;
    }
    if block_ind {
        d.block_plain_allowed = false;
    }
    d
}

/// (handle, suffix) per yaml_emitter_analyze_tag: the document's tag directives (`dirs`),
/// then the default ones for the handles they don't redefine.
fn analyze_tag(tag: &str, dirs: &[(String, String)]) -> (Option<String>, String) {
    let defaults = [("!", "!"), ("!!", "tag:yaml.org,2002:")];
    let all = dirs
        .iter()
        .map(|(h, p)| (h.as_str(), p.as_str()))
        .chain(defaults.into_iter().filter(|(h, _)| !dirs.iter().any(|(dh, _)| dh == h)));
    for (h, prefix) in all {
        if prefix.len() < tag.len() && tag.starts_with(prefix) {
            return (Some(h.to_string()), tag[prefix.len()..].to_string());
        }
    }
    (None, tag.to_string())
}

struct Em<'a> {
    nodes: &'a [DN],
    out: String,
    column: i64,
    whitespace: bool,
    indention: bool,
    open_ended: u8,
    indent: i64,
    indents: Vec<i64>,
    flow_level: usize,
    mapping_ctx: bool,
    simple_key_ctx: bool,
    best_indent: i64,
    best_width: i64,
    /// the current document's %TAG directives (handle, prefix)
    dirs: Vec<(String, String)>,
}

impl Em<'_> {
    fn put(&mut self, c: char) {
        self.out.push(c);
        self.column += 1;
    }
    fn put_break(&mut self) {
        self.out.push('\n');
        self.column = 0;
    }
    fn write_break(&mut self, c: char) {
        if c == '\n' {
            self.put_break();
        } else {
            self.out.push(c);
            self.column = 0;
        }
    }
    fn write_indent(&mut self) {
        let indent = self.indent.max(0);
        if !self.indention || self.column > indent || (self.column == indent && !self.whitespace) {
            self.put_break();
        }
        while self.column < indent {
            self.put(' ');
        }
        self.whitespace = true;
        self.indention = true;
    }
    fn write_indicator(&mut self, ind: &str, need_ws: bool, is_ws: bool, is_indention: bool) {
        if need_ws && !self.whitespace {
            self.put(' ');
        }
        for c in ind.chars() {
            self.put(c);
        }
        self.whitespace = is_ws;
        self.indention = self.indention && is_indention;
    }
    fn write_raw(&mut self, s: &str) {
        for c in s.chars() {
            self.put(c);
        }
        self.whitespace = false;
        self.indention = false;
    }
    fn increase_indent(&mut self, flow: bool, indentless: bool) {
        self.indents.push(self.indent);
        if self.indent < 0 {
            self.indent = if flow { self.best_indent } else { 0 };
        } else if !indentless {
            self.indent += self.best_indent;
        }
    }
    fn pop_indent(&mut self) {
        self.indent = self.indents.pop().unwrap_or(-1);
    }

    fn process_anchor(&mut self, n: &DN) {
        match &n.k {
            DK::Alias(a) => {
                self.write_indicator("*", true, false, false);
                self.write_raw(a);
            }
            _ => {
                if let Some(a) = &n.anchor {
                    self.write_indicator("&", true, false, false);
                    self.write_raw(a);
                }
            }
        }
    }
    fn process_tag(&mut self, handle: Option<&str>, suffix: Option<&str>) {
        match (handle, suffix) {
            (None, None) => {}
            (Some(h), s) => {
                if !self.whitespace {
                    self.put(' ');
                }
                self.write_raw(h);
                if let Some(s) = s {
                    self.write_tag_content(s);
                }
            }
            (None, Some(s)) => {
                self.write_indicator("!<", true, false, false);
                self.write_tag_content(s);
                self.write_indicator(">", false, false, false);
            }
        }
    }
    fn write_tag_content(&mut self, s: &str) {
        for c in s.chars() {
            if c.is_ascii_alphanumeric() || "-;/?:@&=+$,_.~*'()[]".contains(c) {
                self.put(c);
            } else {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    for ch in format!("%{b:02X}").chars() {
                        self.put(ch);
                    }
                }
            }
        }
        self.whitespace = false;
        self.indention = false;
    }

    /// Tag data (handle, suffix) and whether the node is a simple key candidate.
    fn tag_data(&self, n: &DN) -> (Option<String>, Option<String>) {
        let use_tag = match &n.k {
            DK::Scalar { plain, quoted, .. } => !*plain && !*quoted,
            DK::Seq(..) | DK::Map(..) => !n.implicit,
            DK::Alias(_) => false,
        };
        match (&n.tag, use_tag) {
            (Some(t), true) => {
                let (h, s) = analyze_tag(t, &self.dirs);
                (h, Some(s))
            }
            _ => (None, None),
        }
    }
    fn check_simple_key(&self, idx: usize) -> bool {
        let n = &self.nodes[idx];
        let alen = n.anchor.as_ref().map(|a| a.len()).unwrap_or(0);
        let (h, s) = self.tag_data(n);
        let tlen = h.as_ref().map(|h| h.len()).unwrap_or(0) + s.map(|s| s.len()).unwrap_or(0);
        let len = match &n.k {
            DK::Alias(a) => a.len(),
            DK::Scalar { v, .. } => {
                if analyze_scalar(v).multiline {
                    return false;
                }
                alen + tlen + v.len()
            }
            DK::Seq(c, _) | DK::Map(c, _) => {
                if !c.is_empty() {
                    return false;
                }
                alen + tlen
            }
        };
        len <= 128
    }

    fn emit_node(&mut self, idx: usize, _root: bool, _seq: bool, map: bool, simple_key: bool) {
        self.mapping_ctx = map;
        self.simple_key_ctx = simple_key;
        let nodes = self.nodes;
        let n = &nodes[idx];
        match &n.k {
            DK::Alias(_) => {
                self.process_anchor(n);
                if self.simple_key_ctx {
                    self.put(' ');
                }
            }
            DK::Scalar { v, plain, quoted, style } => {
                let data = analyze_scalar(v);
                let (mut handle, suffix) = self.tag_data(n);
                let no_tag = handle.is_none() && suffix.is_none();
                let mut st = if *style == Sty::Any { Sty::Plain } else { *style };
                if self.simple_key_ctx && data.multiline {
                    st = Sty::Double;
                }
                if st == Sty::Plain {
                    if (self.flow_level > 0 && !data.flow_plain_allowed) || (self.flow_level == 0 && !data.block_plain_allowed) {
                        st = Sty::Single;
                    }
                    if v.is_empty() && (self.flow_level > 0 || self.simple_key_ctx) {
                        st = Sty::Single;
                    }
                    if no_tag && !*plain {
                        st = Sty::Single;
                    }
                }
                if st == Sty::Single && !data.single_quoted_allowed {
                    st = Sty::Double;
                }
                if (st == Sty::Literal || st == Sty::Folded) && (!data.block_allowed || self.flow_level > 0 || self.simple_key_ctx) {
                    st = Sty::Double;
                }
                if no_tag && !*quoted && st != Sty::Plain {
                    handle = Some("!".to_string());
                }
                self.process_anchor(n);
                self.process_tag(handle.as_deref(), suffix.as_deref());
                self.increase_indent(true, false);
                let allow_breaks = !self.simple_key_ctx;
                match st {
                    Sty::Plain | Sty::Any => self.write_plain(v, allow_breaks),
                    Sty::Single => self.write_single(v, allow_breaks),
                    Sty::Double => self.write_double(v, allow_breaks),
                    Sty::Literal => self.write_literal(v),
                    Sty::Folded => self.write_folded(v),
                }
                self.pop_indent();
            }
            DK::Seq(ch, flow) => {
                let (h, s) = self.tag_data(n);
                self.process_anchor(n);
                self.process_tag(h.as_deref(), s.as_deref());
                if self.flow_level > 0 || *flow || ch.is_empty() {
                    self.write_indicator("[", true, true, false);
                    self.increase_indent(true, false);
                    self.flow_level += 1;
                    for (i, &c) in ch.iter().enumerate() {
                        if i > 0 {
                            self.write_indicator(",", false, false, false);
                        }
                        if self.column > self.best_width {
                            self.write_indent();
                        }
                        self.emit_node(c, false, true, false, false);
                    }
                    self.flow_level -= 1;
                    self.pop_indent();
                    self.write_indicator("]", false, false, false);
                } else {
                    let indentless = self.mapping_ctx && !self.indention;
                    self.increase_indent(false, indentless);
                    for &c in ch {
                        self.write_indent();
                        self.write_indicator("-", true, false, true);
                        self.emit_node(c, false, true, false, false);
                    }
                    self.pop_indent();
                }
            }
            DK::Map(ch, flow) => {
                let (h, s) = self.tag_data(n);
                self.process_anchor(n);
                self.process_tag(h.as_deref(), s.as_deref());
                if self.flow_level > 0 || *flow || ch.is_empty() {
                    self.write_indicator("{", true, true, false);
                    self.increase_indent(true, false);
                    self.flow_level += 1;
                    for (i, pair) in ch.chunks(2).enumerate() {
                        if i > 0 {
                            self.write_indicator(",", false, false, false);
                        }
                        if self.column > self.best_width {
                            self.write_indent();
                        }
                        if self.check_simple_key(pair[0]) {
                            self.emit_node(pair[0], false, false, true, true);
                            self.write_indicator(":", false, false, false);
                        } else {
                            self.write_indicator("?", true, false, false);
                            self.emit_node(pair[0], false, false, true, false);
                            if self.column > self.best_width {
                                self.write_indent();
                            }
                            self.write_indicator(":", true, false, false);
                        }
                        if let Some(&v) = pair.get(1) {
                            self.emit_node(v, false, false, true, false);
                        }
                    }
                    self.flow_level -= 1;
                    self.pop_indent();
                    self.write_indicator("}", false, false, false);
                } else {
                    self.increase_indent(false, false);
                    for pair in ch.chunks(2) {
                        self.write_indent();
                        if self.check_simple_key(pair[0]) {
                            self.emit_node(pair[0], false, false, true, true);
                            self.write_indicator(":", false, false, false);
                        } else {
                            self.write_indicator("?", true, false, true);
                            self.emit_node(pair[0], false, false, true, false);
                            self.write_indent();
                            self.write_indicator(":", true, false, true);
                        }
                        if let Some(&v) = pair.get(1) {
                            self.emit_node(v, false, false, true, false);
                        }
                    }
                    self.pop_indent();
                }
            }
        }
    }

    fn write_plain(&mut self, v: &str, allow_breaks: bool) {
        let s: Vec<char> = v.chars().collect();
        if !self.whitespace && (!s.is_empty() || self.flow_level > 0) {
            self.put(' ');
        }
        let (mut spaces, mut breaks) = (false, false);
        let mut i = 0;
        while i < s.len() {
            let c = s[i];
            if c == ' ' {
                if allow_breaks && !spaces && self.column > self.best_width && s.get(i + 1) != Some(&' ') {
                    self.write_indent();
                } else {
                    self.put(c);
                }
                spaces = true;
            } else if is_break(c) {
                if !breaks && c == '\n' {
                    self.put_break();
                }
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                }
                self.put(c);
                self.indention = false;
                spaces = false;
                breaks = false;
            }
            i += 1;
        }
        self.whitespace = false;
        self.indention = false;
    }

    fn write_single(&mut self, v: &str, allow_breaks: bool) {
        let s: Vec<char> = v.chars().collect();
        self.write_indicator("'", true, false, false);
        let (mut spaces, mut breaks) = (false, false);
        let n = s.len();
        for i in 0..n {
            let c = s[i];
            if c == ' ' {
                if allow_breaks && !spaces && self.column > self.best_width && i != 0 && i != n - 1 && s.get(i + 1) != Some(&' ') {
                    self.write_indent();
                } else {
                    self.put(c);
                }
                spaces = true;
            } else if is_break(c) {
                if !breaks && c == '\n' {
                    self.put_break();
                }
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                }
                if c == '\'' {
                    self.put('\'');
                }
                self.put(c);
                self.indention = false;
                spaces = false;
                breaks = false;
            }
        }
        if breaks {
            self.write_indent();
        }
        self.write_indicator("'", false, false, false);
        self.whitespace = false;
        self.indention = false;
    }

    fn write_double(&mut self, v: &str, allow_breaks: bool) {
        let s: Vec<char> = v.chars().collect();
        self.write_indicator("\"", true, false, false);
        let mut spaces = false;
        let n = s.len();
        for i in 0..n {
            let c = s[i];
            if !is_printable(c) || c == '\u{feff}' || is_break(c) || c == '"' || c == '\\' {
                self.put('\\');
                let u = c as u32;
                match u {
                    0x00 => self.put('0'),
                    0x07 => self.put('a'),
                    0x08 => self.put('b'),
                    0x09 => self.put('t'),
                    0x0A => self.put('n'),
                    0x0B => self.put('v'),
                    0x0C => self.put('f'),
                    0x0D => self.put('r'),
                    0x1B => self.put('e'),
                    0x22 => self.put('"'),
                    0x5C => self.put('\\'),
                    0x85 => self.put('N'),
                    0xA0 => self.put('_'),
                    0x2028 => self.put('L'),
                    0x2029 => self.put('P'),
                    _ => {
                        let (p, w) = if u <= 0xFF {
                            ('x', 2)
                        } else if u <= 0xFFFF {
                            ('u', 4)
                        } else {
                            ('U', 8)
                        };
                        self.put(p);
                        for ch in format!("{u:0w$X}", w = w).chars() {
                            self.put(ch);
                        }
                    }
                }
                spaces = false;
            } else if c == ' ' {
                if allow_breaks && !spaces && self.column > self.best_width && i != 0 && i != n - 1 {
                    self.write_indent();
                    if s.get(i + 1) == Some(&' ') {
                        self.put('\\');
                    }
                } else {
                    self.put(c);
                }
                spaces = true;
            } else {
                self.put(c);
                spaces = false;
            }
        }
        self.write_indicator("\"", false, false, false);
        self.whitespace = false;
        self.indention = false;
    }

    fn block_hints(&mut self, s: &[char]) {
        if let Some(&c) = s.first() {
            if c == ' ' || is_break(c) {
                let h = self.best_indent.to_string();
                self.write_indicator(&h, false, false, false);
            }
        }
        self.open_ended = 0;
        let chomp = match s.len() {
            0 => Some("-"),
            n => {
                if !is_break(s[n - 1]) {
                    Some("-")
                } else if n == 1 || is_break(s[n - 2]) {
                    self.open_ended = 2;
                    Some("+")
                } else {
                    None
                }
            }
        };
        if let Some(c) = chomp {
            self.write_indicator(c, false, false, false);
        }
    }

    fn write_literal(&mut self, v: &str) {
        let s: Vec<char> = v.chars().collect();
        self.write_indicator("|", true, false, false);
        self.block_hints(&s);
        self.put_break();
        self.indention = true;
        self.whitespace = true;
        let mut breaks = true;
        for &c in &s {
            if is_break(c) {
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                }
                self.put(c);
                self.indention = false;
                breaks = false;
            }
        }
    }

    fn write_folded(&mut self, v: &str) {
        let s: Vec<char> = v.chars().collect();
        self.write_indicator(">", true, false, false);
        self.block_hints(&s);
        self.put_break();
        self.indention = true;
        self.whitespace = true;
        let (mut breaks, mut leading_spaces) = (true, true);
        let n = s.len();
        let mut i = 0;
        while i < n {
            let c = s[i];
            if is_break(c) {
                if !breaks && !leading_spaces && c == '\n' {
                    let mut k = 0;
                    while i + k < n && is_break(s[i + k]) {
                        k += 1;
                    }
                    if !is_blankz(s.get(i + k).copied()) {
                        self.put_break();
                    }
                }
                self.write_break(c);
                self.indention = true;
                breaks = true;
            } else {
                if breaks {
                    self.write_indent();
                    leading_spaces = c == ' ' || c == '\t';
                }
                if !breaks && c == ' ' && s.get(i + 1) != Some(&' ') && self.column > self.best_width {
                    self.write_indent();
                } else {
                    self.put(c);
                }
                self.indention = false;
                breaks = false;
            }
            i += 1;
        }
    }

    fn emit_document(&mut self, root: usize, version: Option<(i64, i64)>) {
        let version = version.map(|(maj, min)| format!("{maj}.{min}"));
        self.emit_document_full(root, version.as_deref(), &[], false, true, false);
    }
    /// yaml_emitter_emit_document_start / _content / _end. `implicit` (no `---`) holds only
    /// for the first document of a stream with no directives; `implicit_end` false writes `...`.
    fn emit_document_full(&mut self, root: usize, version: Option<&str>, tags: &[(String, String)], mut implicit: bool, implicit_end: bool, first: bool) {
        self.dirs = tags.to_vec();
        if !first {
            implicit = false;
        }
        if (version.is_some() || !tags.is_empty()) && self.open_ended > 0 {
            self.write_indicator("...", true, false, false);
            self.write_indent();
        }
        self.open_ended = 0;
        if let Some(v) = version {
            implicit = false;
            self.write_indicator("%YAML", true, false, false);
            self.write_indicator(v, true, false, false);
            self.write_indent();
        }
        if !tags.is_empty() {
            implicit = false;
            for (h, p) in tags {
                self.write_indicator("%TAG", true, false, false);
                if !self.whitespace {
                    self.put(' ');
                }
                self.write_raw(h);
                if !self.whitespace {
                    self.put(' ');
                }
                self.write_tag_content(p);
                self.write_indent();
            }
        }
        if !implicit {
            self.write_indent();
            self.write_indicator("---", true, false, false);
        }
        self.open_ended = 0;
        self.emit_node(root, true, false, false, false);
        self.write_indent();
        if !implicit_end {
            self.write_indicator("...", true, false, false);
            self.open_ended = 0;
            self.write_indent();
        } else if self.open_ended == 0 {
            self.open_ended = 1;
        }
        self.dirs.clear();
    }
    fn end_stream(&mut self) {
        if self.open_ended == 2 {
            self.write_indicator("...", true, false, false);
            self.open_ended = 0;
            self.write_indent();
        }
    }
}

/// Psych.__kn_dump(objects_array, options_hash) -> String (one document per object).
fn kn_dump(objs: V, opts: V) -> V {
    let lw = opt(opts, "line_width");
    let line_width = if is_fix(lw) { Some(fix(lw)) } else { None };
    if line_width.is_some_and(|w| w < -1) {
        return arg_error(&format!("Invalid line_width {}, must be non-negative or -1 for unlimited.", line_width.unwrap()));
    }
    let ind = opt(opts, "indentation");
    let mut best_indent = if is_fix(ind) { fix(ind) } else { 2 };
    if !(2..=9).contains(&best_indent) {
        best_indent = 2;
    }
    let mut best_width = match line_width {
        Some(w) => w,
        None => 0,
    };
    if best_width >= 0 && best_width <= best_indent * 2 {
        best_width = 80;
    }
    if best_width < 0 {
        best_width = i64::MAX;
    }
    let restricted = if truthy(opt(opts, "__restricted")) {
        let pc = opt(opts, "permitted_classes");
        let ps = opt(opts, "permitted_symbols");
        Some((
            if is_ary(pc) { rary(pc).v.clone() } else { Vec::new() },
            if is_ary(ps) { rary(ps).v.clone() } else { Vec::new() },
            truthy(opt(opts, "aliases")),
        ))
    } else {
        None
    };
    let mut version = None;
    if truthy(opt(opts, "header")) {
        version = Some((1, 1));
    }
    let ver = opt(opts, "version");
    if ver != QNIL {
        let parts: Vec<i64> = if is_ary(ver) {
            rary(ver).v.iter().map(|x| if is_fix(*x) { fix(*x) } else { 0 }).collect()
        } else if is_str(ver) {
            s_of(ver).split('.').map(|p| p.parse().unwrap_or(0)).collect()
        } else {
            vec![1, 1]
        };
        version = Some((parts.first().copied().unwrap_or(1), parts.get(1).copied().unwrap_or(1)));
    }
    let mut d = Dumper { nodes: Vec::new(), reg: HashMap::new(), ids: HashMap::new(), counter: 0, pins: RVec::new(), line_width: line_width.filter(|w| *w >= 0), restricted, stringify_names: truthy(opt(opts, "stringify_names")) };
    let list = if is_ary(objs) { rary(objs).v.clone() } else { vec![objs] };
    d.pins.extend(list.iter().copied());
    let mut roots = Vec::new();
    for o in list {
        match d.accept(o) {
            Ok(r) => roots.push(r),
            Err(()) => return UNDEF,
        }
    }
    let mut em = Em {
        nodes: &d.nodes,
        out: String::new(),
        column: 0,
        whitespace: true,
        indention: true,
        open_ended: 0,
        indent: -1,
        indents: Vec::new(),
        flow_level: 0,
        mapping_ctx: false,
        simple_key_ctx: false,
        best_indent,
        best_width,
        dirs: Vec::new(),
    };
    for r in roots {
        em.emit_document(r, version);
    }
    em.end_stream();
    str_from(&em.out)
}

// ---------------------------------------------------------------------------
// The tree API's native halves (lib/kenny/psych_tree.rb): Psych::Emitter's documents run
// through the emitter above, and Psych::Parser's events come from the parser below.
// ---------------------------------------------------------------------------

/// Psych.__kn_emit(events, [indentation, line_width, first, open_ended]) -> [yaml, open_ended]
/// for one document's events, as Psych::Emitter collects them:
/// [0, version, tag_directives, implicit] start_document, [1, implicit] end_document,
/// [2, value, anchor, tag, plain, quoted, style] scalar, [3 | 5, anchor, tag, implicit, style]
/// start_sequence | start_mapping, [4] / [6] their ends, [7, anchor] alias.
fn kn_emit(evs: V, conf: V) -> V {
    let c = if is_ary(conf) { rary(conf).v.clone() } else { Vec::new() };
    let geti = |i: usize, d: i64| c.get(i).copied().filter(|v| is_fix(*v)).map(fix).unwrap_or(d);
    // yaml_emitter_emit_stream_start's defaults
    let mut best_indent = geti(0, 2);
    if !(2..=9).contains(&best_indent) {
        best_indent = 2;
    }
    let mut best_width = geti(1, 0);
    if best_width >= 0 && best_width <= best_indent * 2 {
        best_width = 80;
    }
    if best_width < 0 {
        best_width = i64::MAX;
    }
    let first = c.get(2).is_some_and(|v| truthy(*v));
    let open_ended = geti(3, 0).clamp(0, 2) as u8;
    let opt_s = |v: V| if v == QNIL { None } else { Some(rstring(v)) };
    let evs = if is_ary(evs) { rary(evs).v.clone() } else { Vec::new() };
    let mut nodes: Vec<DN> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut root = None;
    let (mut version, mut tags, mut implicit, mut implicit_end) = (None, Vec::new(), false, true);
    for e in evs {
        if !is_ary(e) {
            continue;
        }
        let a = rary(e).v.clone();
        let g = |i: usize| a.get(i).copied().unwrap_or(QNIL);
        let code = if is_fix(g(0)) { fix(g(0)) } else { -1 };
        let node = match code {
            0 => {
                let v = g(1);
                if is_ary(v) && !rary(v).v.is_empty() {
                    let parts: Vec<i64> = rary(v).v.iter().map(|x| if is_fix(*x) { fix(*x) } else { 0 }).collect();
                    let (maj, min) = (parts[0], parts.get(1).copied().unwrap_or(0));
                    if maj != 1 || (min != 1 && min != 2) {
                        return raise(rt().c.runtime_error, "incompatible %YAML directive");
                    }
                    version = Some(if min == 1 { "1.1" } else { "1.2" });
                }
                let t = g(2);
                if is_ary(t) {
                    for pair in rary(t).v.clone() {
                        if !is_ary(pair) || rary(pair).v.len() != 2 {
                            return raise(rt().c.runtime_error, "tag tuple must be of length 2");
                        }
                        let p = rary(pair).v.clone();
                        tags.push((rstring(p[0]), rstring(p[1])));
                    }
                }
                implicit = truthy(g(3));
                None
            }
            1 => {
                implicit_end = truthy(g(1));
                None
            }
            2 => Some(DN {
                k: DK::Scalar { v: rstring(g(1)), plain: truthy(g(4)), quoted: truthy(g(5)), style: sty_of(g(6)) },
                tag: opt_s(g(3)),
                implicit: false,
                anchor: opt_s(g(2)),
            }),
            3 | 5 => {
                let flow = is_fix(g(4)) && fix(g(4)) == 2;
                let k = if code == 3 { DK::Seq(Vec::new(), flow) } else { DK::Map(Vec::new(), flow) };
                Some(DN { k, tag: opt_s(g(2)), implicit: truthy(g(3)), anchor: opt_s(g(1)) })
            }
            4 | 6 => {
                stack.pop();
                None
            }
            7 => Some(DN { k: DK::Alias(rstring(g(1))), tag: None, implicit: false, anchor: None }),
            _ => None,
        };
        if let Some(n) = node {
            let open = matches!(n.k, DK::Seq(..) | DK::Map(..));
            let i = nodes.len();
            nodes.push(n);
            match stack.last() {
                Some(&p) => match &mut nodes[p].k {
                    DK::Seq(ch, _) | DK::Map(ch, _) => ch.push(i),
                    _ => {}
                },
                None if root.is_none() => root = Some(i),
                None => return raise(rt().c.runtime_error, "expected DOCUMENT-END"),
            }
            if open {
                stack.push(i);
            }
        }
    }
    let Some(root) = root else {
        return raise(rt().c.runtime_error, "expected SCALAR, SEQUENCE-START, MAPPING-START, or ALIAS");
    };
    let mut em = Em {
        nodes: &nodes,
        out: String::new(),
        column: 0,
        whitespace: true,
        indention: true,
        open_ended,
        indent: -1,
        indents: Vec::new(),
        flow_level: 0,
        mapping_ctx: false,
        simple_key_ctx: false,
        best_indent,
        best_width,
        dirs: Vec::new(),
    };
    em.emit_document_full(root, version, &tags, implicit, implicit_end, first);
    let open = em.open_ended as i64;
    ary_new(vec![str_from(&em.out), mkfix(open)])
}

/// The anchor (and start) of a node whose anchor/tag properties come before `pos`, the
/// position the parser reports for the node itself: `&a !t node`, `!t &a node`, or the
/// properties on the line above a block collection (comments in between are skipped).
fn node_props(chars: &[char], mut pos: usize) -> (Option<String>, usize) {
    let mut anchor = None;
    let mut start = pos;
    for _ in 0..2 {
        // back over blanks, line breaks and the comments of the lines crossed
        loop {
            while pos > 0 && chars[pos - 1].is_whitespace() {
                pos -= 1;
            }
            let ls = chars[..pos].iter().rposition(|c| *c == '\n').map(|i| i + 1).unwrap_or(0);
            let comment = (ls..pos).find(|&i| chars[i] == '#' && (i == ls || chars[i - 1].is_whitespace()));
            match comment {
                Some(i) if i < pos && !chars[ls..i].iter().any(|c| *c == '"' || *c == '\'') => pos = i,
                _ => break,
            }
        }
        let end = pos;
        while pos > 0 && !chars[pos - 1].is_whitespace() && !"[{,".contains(chars[pos - 1]) {
            pos -= 1;
        }
        let tok: String = chars[pos..end].iter().collect();
        if let Some(name) = tok.strip_prefix('&') {
            if anchor.is_none() {
                anchor = Some(name.to_string());
                start = pos;
                continue;
            }
        } else if tok.starts_with('!') {
            start = pos;
            continue;
        }
        break;
    }
    (anchor, start)
}

/// Psych.__kn_events(yaml, path) -> [events, error]: the parse events Psych::Parser hands
/// its handler, each [name, args, start_line, start_column, end_line, end_column], and the
/// Psych::SyntaxError.new arguments of a parse error that stopped them (else nil).
fn kn_events(src: V, path: V) -> V {
    if !is_str(src) {
        return type_error("YAML source must be a String");
    }
    let mut text = s_of(src);
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut line_starts = vec![0usize];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let lines: Vec<String> = text.split('\n').map(String::from).collect();
    // (marks: 1-based lines, 0-based columns in characters)
    let pos = |m: &saphyr_parser::Marker| -> usize {
        line_starts.get(m.line().saturating_sub(1)).map(|s| (s + m.col()).min(chars.len())).unwrap_or(chars.len())
    };
    let loc = |p: usize| -> (i64, i64) {
        let l = line_starts.partition_point(|s| *s <= p).saturating_sub(1);
        (l as i64, (p - line_starts[l]) as i64)
    };
    let out = ary_new(Vec::new());
    let push = |name: &str, args: Vec<V>, s: usize, e: usize| {
        let ((sl, sc), (el, ec)) = (loc(s), loc(e.max(s)));
        let ev = ary_new(vec![mksym(sym(name)), ary_new(args), mkfix(sl), mkfix(sc), mkfix(el), mkfix(ec)]);
        rary(out).v.push(ev);
    };
    let opt_str = |s: &Option<String>| s.as_deref().map(str_from).unwrap_or(QNIL);
    let mut p = Parser::new_from_str(&text);
    let mut anchors: HashMap<usize, String> = HashMap::new();
    let mut flow: Vec<bool> = Vec::new();
    let mut dir_from = 0usize;
    let mut err = QNIL;
    loop {
        let (ev, span) = match p.next_event() {
            None => break,
            Some(Ok(e)) => e,
            Some(Err(e)) => {
                let mk = e.marker();
                let info = e.info().to_string();
                let (problem, context) = match info.split_once(", ") {
                    Some((ctx, prob)) if ctx.starts_with("while ") => (prob.to_string(), Some(ctx.to_string())),
                    _ => (info, None),
                };
                err = ary_new(vec![
                    path,
                    mkfix(mk.line() as i64),
                    mkfix(mk.col() as i64 + 1),
                    mkfix(mk.index() as i64),
                    str_from(&problem),
                    opt_str(&context),
                ]);
                break;
            }
        };
        let (s, e) = (pos(&span.start), pos(&span.end));
        let map = matches!(ev, Event::MappingStart(..) | Event::MappingEnd);
        let tag_s = |t: &Option<std::borrow::Cow<saphyr_parser::Tag>>| t.as_ref().map(|t| format!("{}{}", t.handle, t.suffix));
        let mut props = |id: usize, tagged: bool| -> (Option<String>, usize) {
            if id == 0 && !tagged {
                return (None, s);
            }
            let (a, start) = node_props(&chars, s);
            if id == 0 {
                return (None, start);
            }
            let name = a.unwrap_or_else(|| id.to_string());
            anchors.insert(id, name.clone());
            (Some(name), start)
        };
        match ev {
            Event::Nothing => {}
            Event::StreamStart => push("start_stream", vec![mkfix(1)], s, e),
            Event::StreamEnd => push("end_stream", vec![], s, e),
            Event::DocumentStart(explicit) => {
                let mut version = Vec::new();
                let mut tags = Vec::new();
                let upto = span.start.line().saturating_sub(1);
                if explicit {
                    for l in lines.iter().take(upto).skip(dir_from) {
                        let mut w = l.split_whitespace();
                        match w.next() {
                            Some("%YAML") => {
                                if let Some((a, b)) = w.next().and_then(|v| v.split_once('.')) {
                                    version = vec![mkfix(a.parse().unwrap_or(1)), mkfix(b.parse().unwrap_or(1))];
                                }
                            }
                            Some("%TAG") => {
                                if let (Some(h), Some(pf)) = (w.next(), w.next()) {
                                    tags.push(ary_new(vec![str_from(h), str_from(pf)]));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let implicit = !explicit;
                push("start_document", vec![ary_new(version), ary_new(tags), bool_v(implicit)], s, if explicit { e } else { s });
            }
            Event::DocumentEnd => {
                let explicit = chars[s..].starts_with(&['.', '.', '.']);
                dir_from = if explicit { span.start.line() } else { span.start.line().saturating_sub(1) };
                push("end_document", vec![bool_v(!explicit)], s, if explicit { e } else { s });
            }
            Event::Alias(id) => {
                let name = anchors.get(&id).cloned().unwrap_or_else(|| {
                    chars[s..].iter().skip(1).take_while(|c| !c.is_whitespace() && !",[]{}".contains(**c)).collect()
                });
                push("alias", vec![str_from(&name)], s, e);
            }
            Event::Scalar(v, style, id, tag) => {
                let t = tag_s(&tag);
                let (anchor, start) = props(id, t.is_some());
                let plain = matches!(style, ScalarStyle::Plain);
                let plain_implicit = (plain && t.is_none()) || t.as_deref() == Some("!");
                let quoted_implicit = !plain_implicit && t.is_none();
                let st = match style {
                    ScalarStyle::Plain => 1,
                    ScalarStyle::SingleQuoted => 2,
                    ScalarStyle::DoubleQuoted => 3,
                    ScalarStyle::Literal => 4,
                    ScalarStyle::Folded => 5,
                };
                push(
                    "scalar",
                    vec![str_from(&v), opt_str(&anchor), opt_str(&t), bool_v(plain_implicit), bool_v(quoted_implicit), mkfix(st)],
                    start,
                    e,
                );
            }
            Event::SequenceStart(id, tag) | Event::MappingStart(id, tag) => {
                let t = tag_s(&tag);
                let (anchor, start) = props(id, t.is_some());
                let implicit = t.is_none() || t.as_deref() == Some("!");
                let is_flow = flow.last().copied().unwrap_or(false) || matches!(chars.get(s), Some('[') | Some('{'));
                flow.push(is_flow);
                push(
                    if map { "start_mapping" } else { "start_sequence" },
                    vec![opt_str(&anchor), opt_str(&t), bool_v(implicit), mkfix(if is_flow { 2 } else { 1 })],
                    start,
                    e,
                );
            }
            Event::SequenceEnd | Event::MappingEnd => {
                flow.pop();
                push(if map { "end_mapping" } else { "end_sequence" }, vec![], s, e);
            }
        }
    }
    ary_new(vec![out, err])
}

/// Registers the native half of Psych, loaded by `require "kenny/psych.so"` in lib/kenny/psych.rb.
pub fn init() {
    crate::compat::provide_feature(&["kenny/psych"], Some(init_psych));
}

fn init_psych() {
    let p = match cls("Psych") {
        Some(p) if is_class(p) => p as *mut Class,
        _ => {
            let m = new_module("Psych");
            set_global_const("Psych", m as V);
            m
        }
    };
    defs(p, "__kn_load", 2, 2, |_, a, _| kn_load(a[0], a[1]));
    defs(p, "__kn_dump", 2, 2, |_, a, _| kn_dump(a[0], a[1]));
    defs(p, "__kn_emit", 2, 2, |_, a, _| kn_emit(a[0], a[1]));
    defs(p, "__kn_events", 2, 2, |_, a, _| kn_events(a[0], a[1]));
    // psych's build_exception: an exception with its message set, initialize not called
    defs(p, "__kn_build_exception", 2, 2, |_, a, _| {
        let e = send0(a[0], "allocate");
        if e == UNDEF {
            return UNDEF;
        }
        ivar_set(e, sym("@__message"), a[1]);
        e
    });
}
