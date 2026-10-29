//! Nokogiri's SAX parsers (`XML::SAX::Parser`, `PushParser`, `HTML4::SAX::Parser`) without
//! libxml2.
//!
//! XML is read by a scanner that follows libxml2 2.13's parse loop, so a handler sees the
//! same callbacks with the same arguments: text split where libxml2 splits it (at entity
//! references, at the first non-ASCII byte, then every 300 bytes), libxml2's error messages,
//! and the first fatal error ending the parse (then `end_document`). Its state survives
//! between calls, so a push parser reports each construct as soon as it is complete.
//!
//! HTML is parsed into the HTML4 tree (see `html.rs`), which is then walked.

use super::dom::*;
use super::*;
use std::collections::HashMap;

pub struct Attr {
    pub local: String,
    pub prefix: Option<String>,
    pub uri: Option<String>,
    pub value: String,
}

pub enum Ev<'a> {
    XmlDecl(&'a str, Option<&'a str>, Option<&'a str>),
    StartDocument,
    EndDocument,
    Start { local: &'a str, prefix: Option<&'a str>, uri: Option<&'a str>, attrs: &'a [Attr], ns: &'a [(Option<String>, String)] },
    End { local: &'a str, prefix: Option<&'a str>, uri: Option<&'a str> },
    Chars(&'a str),
    Reference(&'a str, &'a str),
    Comment(&'a str),
    CData(&'a str),
    Pi(&'a str, Option<&'a str>),
    Error(&'a str),
}

/// A handler returns Err when Ruby raised: the parse stops and the exception propagates.
pub struct Abort;
type R<T> = Result<T, Abort>;
pub type Handler<'a> = dyn FnMut(Ev, u32, u32) -> R<()> + 'a;

#[derive(PartialEq, Clone, Copy)]
enum Phase {
    Start,
    Prolog,
    Content,
    Epilog,
    Done,
}

enum Step {
    Again,
    NeedMore,
    Stop,
}

struct Open {
    qname: String,
    prefix: Option<String>,
    local: String,
    uri: Option<String>,
    line: u32,
    nns: usize,
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// libxml2's XML_PARSER_BIG_BUFFER_SIZE: text with non-ASCII characters is reported in
/// pieces of this many bytes
const BIG: usize = 300;
const MAX_DEPTH: usize = 256;

pub struct Sax {
    buf: Vec<u8>,
    pos: usize,
    lc_off: usize,
    lc_line: u32,
    lc_col: u32,
    phase: Phase,
    stack: Vec<Open>,
    ns: Vec<(Option<String>, String)>,
    entities: HashMap<String, Option<String>>,
    /// attribute defaults from <!ATTLIST> declarations: element name -> (attribute, value)
    defaults: HashMap<String, Vec<(String, String)>>,
    latin1: bool,
    fin: bool,
    /// a push parser: after a fatal error it reports nothing more (not even end_document)
    pub push: bool,
    pub recovery: bool,
    pub replace_entities: bool,
    /// the last error reported: message (without the newline), line, column, level
    pub last_error: Option<(String, u32, u32, i64)>,
}

fn blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}
fn name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':' || b >= 0x80
}
fn name_char(b: u8) -> bool {
    name_start(b) || b.is_ascii_digit() || b == b'.' || b == b'-'
}
fn xml_char(c: u32) -> bool {
    matches!(c, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}
fn find(hay: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..].windows(pat.len()).position(|w| w == pat).map(|i| i + from)
}
fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}
/// `\r\n` and a lone `\r` read as `\n`, as libxml2 reads them.
fn newlines(b: &[u8]) -> String {
    let t = s(b);
    if t.contains('\r') {
        t.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        t
    }
}
fn predefined(name: &[u8]) -> Option<char> {
    Some(match name {
        b"lt" => '<',
        b"gt" => '>',
        b"amp" => '&',
        b"apos" => '\'',
        b"quot" => '"',
        _ => return None,
    })
}
fn latin1_to_utf8(b: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(b.len());
    for &c in b {
        let mut t = [0; 4];
        o.extend_from_slice((c as char).encode_utf8(&mut t).as_bytes());
    }
    o
}
fn utf16_to_utf8(b: &[u8], le: bool) -> Vec<u8> {
    let units = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect::<String>().into_bytes()
}

/// Text split the way libxml2's xmlParseCharData reports it: a run of ASCII goes in one
/// piece (a `\r\n` starts a new one), and from the first other character on the text is
/// gathered into pieces of BIG bytes. Returns the pieces with the offset (in `b`) where
/// each ends, and the error that stopped the text, if any.
fn chunk_text(b: &[u8]) -> (Vec<(String, usize)>, Option<(String, usize)>) {
    let mut out = Vec::new();
    let mut i = 0;
    let end = b.len();
    let cdata_end = |i: usize| b[i] == b']' && b.get(i + 1) == Some(&b']') && b.get(i + 2) == Some(&b'>');
    loop {
        let st = i;
        while i < end && ((0x20..=0x7F).contains(&b[i]) || b[i] == b'\t' || b[i] == b'\n') {
            if cdata_end(i) {
                return (out, Some(("Sequence ']]>' not allowed in content".into(), i)));
            }
            i += 1;
        }
        if i > st {
            out.push((s(&b[st..i]), i));
        }
        if i + 1 < end && b[i] == b'\r' && b[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        break;
    }
    let mut acc = String::new();
    while i < end {
        let rest = &b[i..];
        let len = match rest[0] {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 0,
        };
        let ch = if len > 0 && rest.len() >= len { std::str::from_utf8(&rest[..len]).ok().and_then(|t| t.chars().next()) } else { None };
        let Some(mut c) = ch else {
            if !acc.is_empty() {
                out.push((std::mem::take(&mut acc), i));
            }
            let bytes: Vec<String> = rest.iter().take(4).map(|x| format!("0x{x:02X}")).collect();
            return (out, Some((format!("Input is not proper UTF-8, indicate encoding !\nBytes: {}", bytes.join(" ")), i)));
        };
        let mut n = len;
        if c == '\r' {
            if rest.get(1) == Some(&b'\n') {
                n = 2;
            }
            c = '\n';
        } else if !xml_char(c as u32) {
            if !acc.is_empty() {
                out.push((std::mem::take(&mut acc), i));
            }
            return (out, Some((format!("PCDATA invalid Char value {}", c as u32), i)));
        } else if cdata_end(i) {
            return (out, Some(("Sequence ']]>' not allowed in content".into(), i)));
        }
        acc.push(c);
        i += n;
        if acc.len() >= BIG {
            out.push((std::mem::take(&mut acc), i));
        }
    }
    if !acc.is_empty() {
        out.push((acc, i));
    }
    (out, None)
}

impl Sax {
    pub fn new() -> Sax {
        Sax {
            buf: Vec::new(),
            pos: 0,
            lc_off: 0,
            lc_line: 1,
            lc_col: 1,
            phase: Phase::Start,
            stack: Vec::new(),
            ns: Vec::new(),
            entities: HashMap::new(),
            defaults: HashMap::new(),
            latin1: false,
            fin: false,
            push: false,
            recovery: false,
            replace_entities: false,
            last_error: None,
        }
    }

    /// Line and column (1-based, in characters) at buffer offset `off`.
    fn lc(&mut self, off: usize) -> (u32, u32) {
        let off = off.min(self.buf.len());
        if off > self.lc_off {
            for &b in &self.buf[self.lc_off..off] {
                if b == b'\n' {
                    self.lc_line += 1;
                    self.lc_col = 1;
                } else if b & 0xC0 != 0x80 {
                    self.lc_col += 1;
                }
            }
            self.lc_off = off;
        }
        (self.lc_line, self.lc_col)
    }

    fn emit(&mut self, h: &mut Handler, ev: Ev, off: usize) -> R<()> {
        let (l, c) = self.lc(off);
        h(ev, l, c)
    }

    fn error(&mut self, h: &mut Handler, msg: &str, off: usize, level: i64) -> R<()> {
        let (l, c) = self.lc(off);
        self.last_error = Some((msg.to_string(), l, c, level));
        h(Ev::Error(&format!("{msg}\n")), l, c)
    }

    /// Report fatal errors and end the parse (then `end_document`).
    fn fatal(&mut self, h: &mut Handler, msgs: &[&str], off: usize) -> R<Step> {
        for m in msgs {
            self.error(h, m, off, 3)?;
        }
        self.phase = Phase::Done;
        if !self.push || self.fin {
            self.emit(h, Ev::EndDocument, off)?;
        }
        Ok(Step::Stop)
    }

    /// Feed more input (`fin`: it is the last). Callbacks run as constructs complete.
    pub fn feed(&mut self, data: &[u8], fin: bool, h: &mut Handler) -> R<()> {
        if self.phase == Phase::Done {
            return Ok(());
        }
        self.fin = fin;
        if self.latin1 {
            self.buf.extend_from_slice(&latin1_to_utf8(data));
        } else {
            self.buf.extend_from_slice(data);
        }
        loop {
            let st = match self.phase {
                Phase::Start => self.start(fin, h)?,
                Phase::Prolog | Phase::Epilog => self.misc(fin, h)?,
                Phase::Content => self.content(fin, h)?,
                Phase::Done => Step::Stop,
            };
            match st {
                Step::Again => {}
                Step::NeedMore | Step::Stop => break,
            }
        }
        // drop what has been read, so a long push parse doesn't keep its whole input
        if self.pos > 1 << 16 {
            let p = self.pos;
            self.lc(p);
            self.buf.drain(..p);
            self.lc_off = 0;
            self.pos = 0;
        }
        Ok(())
    }

    fn rest(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    fn start(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let n = self.rest().len();
        if n == 0 {
            if fin {
                self.error(h, "Document is empty", 0, 3)?;
                self.phase = Phase::Done;
                return Ok(Step::Stop);
            }
            return Ok(Step::NeedMore);
        }
        // (as libxml2: 4 bytes to tell the encoding, and whether an XML declaration starts)
        if !fin && (n < 4 || (n < 6 && b"<?xml ".starts_with(self.rest()))) {
            return Ok(Step::NeedMore);
        }
        if self.rest().starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.pos += 3;
            self.lc_off = self.pos;
        } else if self.rest().starts_with(&[0xFF, 0xFE]) || self.rest().starts_with(&[0xFE, 0xFF]) {
            let le = self.rest()[0] == 0xFF;
            let t = utf16_to_utf8(&self.buf[self.pos + 2..], le);
            self.buf.truncate(self.pos);
            self.buf.extend_from_slice(&t);
            self.lc_off = self.pos;
        }
        let r = self.rest();
        if r.starts_with(b"<?xml") && r.len() > 5 && blank(r[5]) {
            let Some(end) = find(&self.buf, self.pos, b"?>") else {
                if !fin {
                    return Ok(Step::NeedMore);
                }
                let l = self.buf.len();
                return self.fatal(h, &["parsing XML declaration: '?>' expected"], l);
            };
            let decl = self.buf[self.pos + 5..end].to_vec();
            let mut attrs: Vec<(String, String)> = Vec::new();
            let mut i = 0;
            loop {
                while i < decl.len() && blank(decl[i]) {
                    i += 1;
                }
                let ks = i;
                while i < decl.len() && decl[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if ks == i {
                    break;
                }
                let k = s(&decl[ks..i]);
                while i < decl.len() && blank(decl[i]) {
                    i += 1;
                }
                if decl.get(i) != Some(&b'=') {
                    break;
                }
                i += 1;
                while i < decl.len() && blank(decl[i]) {
                    i += 1;
                }
                let Some(&q) = decl.get(i).filter(|&&q| q == b'"' || q == b'\'') else { break };
                let vs = i + 1;
                let Some(ve) = decl[vs..].iter().position(|&b| b == q).map(|p| p + vs) else { break };
                attrs.push((k, s(&decl[vs..ve])));
                i = ve + 1;
            }
            let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            let off = end + 2;
            let Some(version) = attrs.first().filter(|(k, _)| k == "version").map(|(_, v)| v.clone()) else {
                return self.fatal(h, &["Malformed declaration expecting version", "Blank needed here"], off);
            };
            let enc = get("encoding");
            let standalone = get("standalone");
            self.pos = off;
            self.emit(h, Ev::XmlDecl(&version, enc.as_deref(), standalone.as_deref()), off)?;
            if let Some(e) = &enc {
                let e = e.to_ascii_uppercase();
                if matches!(e.as_str(), "ISO-8859-1" | "ISO_8859-1" | "LATIN1" | "ISO-LATIN-1" | "L1" | "WINDOWS-1252" | "CP1252") {
                    self.latin1 = true;
                    let t = latin1_to_utf8(&self.buf[self.pos..]);
                    self.buf.truncate(self.pos);
                    self.buf.extend_from_slice(&t);
                }
            }
        }
        let p = self.pos;
        self.emit(h, Ev::StartDocument, p)?;
        self.phase = Phase::Prolog;
        Ok(Step::Again)
    }

    /// Before and after the root element: blanks, comments, PIs (and one DOCTYPE before).
    fn misc(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        while self.pos < self.buf.len() && blank(self.buf[self.pos]) {
            self.pos += 1;
        }
        let prolog = self.phase == Phase::Prolog;
        let bad = if prolog { "Start tag expected, '<' not found" } else { "Extra content at the end of the document" };
        let r = self.rest();
        if r.is_empty() {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let p = self.pos;
            if prolog {
                return self.fatal(h, &[bad], p);
            }
            self.phase = Phase::Done;
            self.emit(h, Ev::EndDocument, p)?;
            return Ok(Step::Stop);
        }
        if r[0] != b'<' {
            let p = self.pos;
            return self.fatal(h, &[bad], p);
        }
        if r.len() < 2 && !fin {
            return Ok(Step::NeedMore);
        }
        match r.get(1) {
            Some(b'?') => self.pi(fin, h),
            Some(b'!') => {
                if r.starts_with(b"<!--") {
                    self.comment(fin, h)
                } else if prolog && r.starts_with(b"<!DOCTYPE") {
                    self.doctype(fin, h)
                } else if !fin && (b"<!--".starts_with(r) || b"<!DOCTYPE".starts_with(r)) {
                    Ok(Step::NeedMore)
                } else {
                    let p = self.pos;
                    self.fatal(h, &[bad], p)
                }
            }
            _ if prolog => self.start_tag(fin, h),
            _ => {
                let p = self.pos;
                self.fatal(h, &[bad], p)
            }
        }
    }

    fn content(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let r = self.rest();
        if r.is_empty() {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let top = self.stack.last().unwrap();
            let msg = format!("Premature end of data in tag {} line {}", top.qname, top.line);
            let p = self.buf.len();
            if self.recovery && self.last_error.is_some() {
                self.phase = Phase::Done;
                self.emit(h, Ev::EndDocument, p)?;
                return Ok(Step::Stop);
            }
            return self.fatal(h, &[&msg], p);
        }
        match r[0] {
            b'<' => {
                if r.len() < 2 && !fin {
                    return Ok(Step::NeedMore);
                }
                match r.get(1) {
                    Some(b'/') => self.end_tag(fin, h),
                    Some(b'?') => self.pi(fin, h),
                    Some(b'!') => {
                        if r.starts_with(b"<!--") {
                            self.comment(fin, h)
                        } else if r.starts_with(b"<![CDATA[") {
                            self.cdata(fin, h)
                        } else if !fin && (b"<!--".starts_with(r) || b"<![CDATA[".starts_with(r)) {
                            Ok(Step::NeedMore)
                        } else {
                            let p = self.pos + 1;
                            self.fatal(h, &["StartTag: invalid element name"], p)
                        }
                    }
                    _ => self.start_tag(fin, h),
                }
            }
            b'&' => self.reference(fin, h),
            _ => self.text(fin, h),
        }
    }

    fn text(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let end = match self.buf[st..].iter().position(|&b| b == b'<' || b == b'&') {
            Some(i) => st + i,
            None if fin => self.buf.len(),
            None => return Ok(Step::NeedMore),
        };
        let (chunks, err) = chunk_text(&self.buf[st..end]);
        for (c, e) in chunks {
            self.emit(h, Ev::Chars(&c), st + e)?;
        }
        if let Some((m, e)) = err {
            return self.fatal(h, &[&m], st + e);
        }
        self.pos = end;
        Ok(Step::Again)
    }

    fn name_at(&self, i: usize) -> Option<usize> {
        let b = &self.buf;
        if i >= b.len() || !name_start(b[i]) {
            return None;
        }
        let mut j = i + 1;
        while j < b.len() && name_char(b[j]) {
            j += 1;
        }
        Some(j)
    }

    fn reference(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let b = &self.buf;
        if b.get(st + 1) == Some(&b'#') {
            let hex = matches!(b.get(st + 2), Some(b'x'));
            let ds = st + 2 + hex as usize;
            let mut j = ds;
            while j < b.len() && (if hex { b[j].is_ascii_hexdigit() } else { b[j].is_ascii_digit() }) {
                j += 1;
            }
            if j >= b.len() && !fin {
                return Ok(Step::NeedMore);
            }
            if j == ds || b.get(j) != Some(&b';') {
                let m = if hex { "xmlParseCharRef: invalid hexadecimal value" } else { "xmlParseCharRef: invalid decimal value" };
                return self.fatal(h, &[m], j);
            }
            let v = u32::from_str_radix(&s(&b[ds..j]), if hex { 16 } else { 10 }).unwrap_or(0x110000);
            let p = j + 1;
            match char::from_u32(v).filter(|_| xml_char(v)) {
                Some(c) => {
                    self.pos = p;
                    self.emit(h, Ev::Chars(c.encode_utf8(&mut [0; 4])), p)?;
                    Ok(Step::Again)
                }
                None => self.fatal(h, &[&format!("xmlParseCharRef: invalid xmlChar value {v}")], p),
            }
        } else {
            let ne = self.name_at(st + 1);
            let j = ne.unwrap_or(st + 1);
            if j >= self.buf.len() && !fin {
                return Ok(Step::NeedMore);
            }
            let Some(ne) = ne else {
                return self.fatal(h, &["xmlParseEntityRef: no name"], st + 1);
            };
            if self.buf.get(ne) != Some(&b';') {
                return self.fatal(h, &["EntityRef: expecting ';'"], ne);
            }
            let name = self.buf[st + 1..ne].to_vec();
            let p = ne + 1;
            self.pos = p;
            if let Some(c) = predefined(&name) {
                self.emit(h, Ev::Chars(c.encode_utf8(&mut [0; 4])), p)?;
                return Ok(Step::Again);
            }
            let name = s(&name);
            match self.entities.get(&name).cloned() {
                Some(Some(value)) => {
                    for c in self.expand(&value, 0) {
                        self.emit(h, Ev::Chars(&c), p)?;
                    }
                    if !self.replace_entities {
                        self.emit(h, Ev::Reference(&name, &value), p)?;
                    }
                    Ok(Step::Again)
                }
                // an external entity isn't loaded
                Some(None) => Ok(Step::Again),
                None => self.fatal(h, &[&format!("Entity '{name}' not defined")], p),
            }
        }
    }

    /// The text of an internal entity's replacement (references in it expanded), split as
    /// libxml2 reports it.
    fn expand(&self, value: &str, depth: u32) -> Vec<String> {
        let mut out = Vec::new();
        let b = value.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'&' {
                if let Some(k) = b[i..].iter().position(|&c| c == b';') {
                    let r = &b[i + 1..i + k];
                    i += k + 1;
                    if let Some(num) = r.strip_prefix(b"#") {
                        let v = match num.strip_prefix(b"x") {
                            Some(hx) => u32::from_str_radix(&s(hx), 16).ok(),
                            None => s(num).parse().ok(),
                        };
                        if let Some(c) = v.and_then(char::from_u32) {
                            out.push(c.to_string());
                        }
                    } else if let Some(c) = predefined(r) {
                        out.push(c.to_string());
                    } else if let Some(Some(v)) = self.entities.get(&s(r)).filter(|_| depth < 8) {
                        out.extend(self.expand(v, depth + 1));
                    }
                    continue;
                }
            }
            let e = b[i..].iter().position(|&c| c == b'&').map(|k| i + k).unwrap_or(b.len());
            let e = if e == i { b.len() } else { e };
            out.extend(chunk_text(&b[i..e]).0.into_iter().map(|(c, _)| c));
            i = e;
        }
        out
    }

    /// An attribute value as libxml2 reports it: references replaced (but `&amp;` kept as
    /// `&#38;` unless replace_entities is set), whitespace characters read as spaces.
    fn att_value(&self, raw: &[u8]) -> Result<String, String> {
        let mut out = String::new();
        let mut i = 0;
        while i < raw.len() {
            let b = raw[i];
            if b == b'<' {
                return Err("Unescaped '<' not allowed in attributes values".into());
            }
            if b == b'&' {
                let Some(k) = raw[i..].iter().position(|&c| c == b';') else {
                    return Err("EntityRef: expecting ';'".into());
                };
                let r = &raw[i + 1..i + k];
                i += k + 1;
                let ch = if let Some(num) = r.strip_prefix(b"#") {
                    let (hex, d) = match num.strip_prefix(b"x") {
                        Some(hx) => (true, hx),
                        None => (false, num),
                    };
                    let v = u32::from_str_radix(&s(d), if hex { 16 } else { 10 }).unwrap_or(0x110000);
                    match char::from_u32(v).filter(|_| xml_char(v)) {
                        Some(c) => c,
                        None => return Err(format!("xmlParseCharRef: invalid xmlChar value {v}")),
                    }
                } else if let Some(c) = predefined(r) {
                    c
                } else {
                    let name = s(r);
                    match self.entities.get(&name) {
                        Some(Some(v)) if self.replace_entities => {
                            out.push_str(&self.expand(v, 0).concat());
                            continue;
                        }
                        Some(_) => {
                            out.push('&');
                            out.push_str(&name);
                            out.push(';');
                            continue;
                        }
                        None => return Err(format!("Entity '{name}' not defined")),
                    }
                };
                if ch == '&' && !self.replace_entities {
                    out.push_str("&#38;");
                } else {
                    out.push(ch);
                }
                continue;
            }
            if b == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                i += 1;
                continue;
            }
            let e = raw[i..].iter().position(|&c| c == b'<' || c == b'&' || c == b'\r').map(|k| i + k).unwrap_or(raw.len());
            let e = if e == i { i + 1 } else { e };
            for c in String::from_utf8_lossy(&raw[i..e]).chars() {
                out.push(if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c });
            }
            i = e;
        }
        Ok(out)
    }

    fn lookup(&self, prefix: Option<&str>) -> Option<String> {
        if prefix == Some("xml") {
            return Some(XML_NS.into());
        }
        self.ns.iter().rev().find(|(p, _)| p.as_deref() == prefix).map(|(_, u)| u.clone()).filter(|u| !u.is_empty())
    }

    fn start_tag(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        // the whole tag must be here: up to a '>' outside quotes
        let mut q = 0u8;
        let mut gt = None;
        for (k, &b) in self.buf[st + 1..].iter().enumerate() {
            if q != 0 {
                if b == q {
                    q = 0;
                }
            } else if b == b'"' || b == b'\'' {
                q = b;
            } else if b == b'>' {
                gt = Some(st + 1 + k);
                break;
            }
        }
        if gt.is_none() && !fin {
            return Ok(Step::NeedMore);
        }
        if self.stack.len() > MAX_DEPTH {
            let m = format!("Excessive depth in document: {} use XML_PARSE_HUGE option", MAX_DEPTH);
            return self.fatal(h, &[&m], st);
        }
        let Some(ne) = self.name_at(st + 1) else {
            return self.fatal(h, &["StartTag: invalid element name"], st + 1);
        };
        let qname = s(&self.buf[st + 1..ne]);
        let (line, _) = self.lc(st);
        let unfinished = format!("Couldn't find end of Start Tag {qname} line {line}");
        let mut raw_attrs: Vec<(String, String)> = Vec::new();
        let mut decls: Vec<(Option<String>, String)> = Vec::new();
        let mut i = ne;
        let len = self.buf.len();
        let empty;
        loop {
            let b0 = i;
            while i < len && blank(self.buf[i]) {
                i += 1;
            }
            if i >= len {
                return self.fatal(h, &[&unfinished], i);
            }
            match self.buf[i] {
                b'>' => {
                    empty = false;
                    break;
                }
                b'/' if self.buf.get(i + 1) == Some(&b'>') => {
                    empty = true;
                    break;
                }
                _ => {}
            }
            if b0 == i {
                return self.fatal(h, &["attributes construct error", &unfinished], i);
            }
            let Some(ae) = self.name_at(i) else {
                return self.fatal(h, &["error parsing attribute name", "attributes construct error", &unfinished], i);
            };
            let aname = s(&self.buf[i..ae]);
            i = ae;
            while i < len && blank(self.buf[i]) {
                i += 1;
            }
            if self.buf.get(i) != Some(&b'=') {
                return self.fatal(h, &[&format!("Specification mandates value for attribute {aname}")], i);
            }
            i += 1;
            while i < len && blank(self.buf[i]) {
                i += 1;
            }
            let qc = match self.buf.get(i) {
                Some(&c @ (b'"' | b'\'')) => c,
                _ => return self.fatal(h, &["AttValue: \" or ' expected", "attributes construct error", &unfinished], i),
            };
            let Some(ve) = self.buf[i + 1..].iter().position(|&c| c == qc).map(|k| i + 1 + k) else {
                return self.fatal(h, &["AttValue: ' expected", "attributes construct error", &unfinished], len);
            };
            let value = match self.att_value(&self.buf[i + 1..ve]) {
                Ok(v) => v,
                Err(m) => return self.fatal(h, &[&m], i + 1),
            };
            i = ve + 1;
            if aname == "xmlns" {
                decls.push((None, value));
            } else if let Some(p) = aname.strip_prefix("xmlns:") {
                decls.push((Some(p.to_string()), value));
            } else {
                if raw_attrs.iter().any(|(n, _)| *n == aname) {
                    return self.fatal(h, &[&format!("Attribute {aname} redefined")], i);
                }
                raw_attrs.push((aname, value));
            }
        }
        if let Some(defs) = self.defaults.get(&qname) {
            for (k, v) in defs {
                if !raw_attrs.iter().any(|(n, _)| n == k) && k != "xmlns" && !k.starts_with("xmlns:") {
                    raw_attrs.push((k.clone(), v.clone()));
                }
            }
        }
        let nns = decls.len();
        self.ns.extend(decls.iter().cloned());
        let split = |q: &str| -> (Option<String>, String) {
            match q.split_once(':') {
                Some((p, l)) if !p.is_empty() && !l.is_empty() => (Some(p.to_string()), l.to_string()),
                _ => (None, q.to_string()),
            }
        };
        let mut attrs = Vec::with_capacity(raw_attrs.len());
        for (an, v) in raw_attrs {
            let (p, l) = split(&an);
            let uri = match &p {
                Some(p) => {
                    let u = self.lookup(Some(p));
                    if u.is_none() {
                        self.error(h, &format!("Namespace prefix {p} for {l} on {qname} is not defined"), i, 2)?;
                    }
                    u
                }
                None => None,
            };
            attrs.push(Attr { local: l, prefix: p, uri, value: v });
        }
        let (prefix, local) = split(&qname);
        let uri = self.lookup(prefix.as_deref());
        if let (Some(p), None) = (&prefix, &uri) {
            self.error(h, &format!("Namespace prefix {p} on {local} is not defined"), i, 2)?;
        }
        self.emit(h, Ev::Start { local: &local, prefix: prefix.as_deref(), uri: uri.as_deref(), attrs: &attrs, ns: &decls }, i)?;
        if empty {
            let p = i + 2;
            self.pos = p;
            self.emit(h, Ev::End { local: &local, prefix: prefix.as_deref(), uri: uri.as_deref() }, p)?;
            self.ns.truncate(self.ns.len() - nns);
            if self.stack.is_empty() {
                self.phase = Phase::Epilog;
            }
        } else {
            self.pos = i + 1;
            self.stack.push(Open { qname, prefix, local, uri, line, nns });
            self.phase = Phase::Content;
        }
        Ok(Step::Again)
    }

    fn end_tag(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let Some(gt) = find(&self.buf, st, b">") else {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let l = self.buf.len();
            return self.fatal(h, &["expected '>'"], l);
        };
        let ne = self.name_at(st + 2);
        let name = match ne {
            Some(e) => s(&self.buf[st + 2..e]),
            None => "unparsable".to_string(),
        };
        let mut i = ne.unwrap_or(st + 2);
        while i < gt && blank(self.buf[i]) {
            i += 1;
        }
        let top = self.stack.last().unwrap();
        let mut errs = Vec::new();
        if i != gt {
            errs.push("expected '>'".to_string());
        }
        if name != top.qname {
            errs.push(format!("Opening and ending tag mismatch: {} line {} and {}", top.qname, top.line, name));
        }
        let p = gt + 1;
        if !errs.is_empty() {
            if !self.recovery {
                let refs: Vec<&str> = errs.iter().map(|e| e.as_str()).collect();
                return self.fatal(h, &refs, p);
            }
            for e in &errs {
                self.error(h, e, p, 3)?;
            }
        }
        self.pos = p;
        let top = self.stack.pop().unwrap();
        self.emit(h, Ev::End { local: &top.local, prefix: top.prefix.as_deref(), uri: top.uri.as_deref() }, p)?;
        self.ns.truncate(self.ns.len() - top.nns);
        if self.stack.is_empty() {
            self.phase = Phase::Epilog;
        }
        Ok(Step::Again)
    }

    fn comment(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let cs = st + 4;
        let Some(dd) = find(&self.buf, cs, b"--") else {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let l = self.buf.len();
            return self.fatal(h, &["Comment not terminated"], l);
        };
        match self.buf.get(dd + 2) {
            None if !fin => Ok(Step::NeedMore),
            Some(b'>') => {
                let text = newlines(&self.buf[cs..dd]);
                let p = dd + 3;
                self.pos = p;
                self.emit(h, Ev::Comment(&text), p)?;
                Ok(Step::Again)
            }
            _ => {
                let text: String = newlines(&self.buf[cs..dd]).chars().take(50).collect();
                self.fatal(h, &[&format!("Double hyphen within comment: <!--{text}")], dd)
            }
        }
    }

    fn cdata(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let cs = self.pos + 9;
        let Some(e) = find(&self.buf, cs, b"]]>") else {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let t = newlines(&self.buf[cs..]);
            let n = t.chars().count().saturating_sub(2);
            let t: String = t.chars().take(n.min(50)).collect();
            let l = self.buf.len();
            return self.fatal(h, &[&format!("CData section not finished\n{t}")], l);
        };
        let text = newlines(&self.buf[cs..e]);
        let p = e + 3;
        self.pos = p;
        self.emit(h, Ev::CData(&text), p)?;
        Ok(Step::Again)
    }

    fn pi(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let end = find(&self.buf, st + 2, b"?>");
        if end.is_none() && !fin {
            return Ok(Step::NeedMore);
        }
        let Some(ne) = self.name_at(st + 2) else {
            return self.fatal(h, &["xmlParsePI : no target name"], st + 2);
        };
        let target = s(&self.buf[st + 2..ne]);
        if target.eq_ignore_ascii_case("xml") {
            return self.fatal(h, &["XML declaration allowed only at the start of the document"], ne);
        }
        let Some(end) = end else {
            let l = self.buf.len();
            return self.fatal(h, &[&format!("ParsePI: PI {target} never end ...")], l);
        };
        let p = end + 2;
        if ne == end {
            self.pos = p;
            self.emit(h, Ev::Pi(&target, None), p)?;
            return Ok(Step::Again);
        }
        if !blank(self.buf[ne]) {
            return self.fatal(h, &[&format!("ParsePI: PI {target} space expected")], ne);
        }
        let mut ds = ne;
        while ds < end && blank(self.buf[ds]) {
            ds += 1;
        }
        let data = newlines(&self.buf[ds..end]);
        self.pos = p;
        self.emit(h, Ev::Pi(&target, Some(&data)), p)?;
        Ok(Step::Again)
    }

    /// Default values from `<!ATTLIST element attribute type default ...>` declarations.
    fn attlists(&mut self, sub: &[u8]) {
        let mut k = 0;
        while let Some(e) = find(sub, k, b"<!ATTLIST") {
            let end = sub[e..].iter().position(|&c| c == b'>').map(|p| e + p).unwrap_or(sub.len());
            k = end;
            // tokens: names, (enumerations), #KEYWORDS and quoted strings
            let mut toks: Vec<(bool, String)> = Vec::new();
            let mut i = e + 9;
            while i < end {
                let c = sub[i];
                if blank(c) {
                    i += 1;
                } else if c == b'"' || c == b'\'' {
                    let ve = sub[i + 1..end].iter().position(|&x| x == c).map(|p| i + 1 + p).unwrap_or(end);
                    toks.push((true, s(&sub[i + 1..ve])));
                    i = ve + 1;
                } else if c == b'(' {
                    let ce = sub[i..end].iter().position(|&x| x == b')').map(|p| i + p).unwrap_or(end);
                    toks.push((false, s(&sub[i..=ce.min(end - 1)])));
                    i = ce + 1;
                } else {
                    let st = i;
                    while i < end && !blank(sub[i]) && sub[i] != b'(' {
                        i += 1;
                    }
                    toks.push((false, s(&sub[st..i])));
                }
            }
            let Some((_, elem)) = toks.first().cloned() else { continue };
            let mut j = 1;
            // attribute, type (NOTATION takes an enumeration after it), default
            while j + 2 <= toks.len() {
                let attr = toks[j].1.clone();
                j += 1;
                if toks[j].1 == "NOTATION" {
                    j += 1;
                }
                j += 1;
                let Some((quoted, d)) = toks.get(j).cloned() else { break };
                j += 1;
                let value = if quoted {
                    Some(d)
                } else if d == "#FIXED" {
                    let v = toks.get(j).filter(|t| t.0).map(|t| t.1.clone());
                    j += 1;
                    v
                } else {
                    None
                };
                if let Some(v) = value.and_then(|v| self.att_value(v.as_bytes()).ok()) {
                    let list = self.defaults.entry(elem.clone()).or_default();
                    if !list.iter().any(|(n, _)| *n == attr) {
                        list.push((attr, v));
                    }
                }
            }
        }
    }

    /// A DOCTYPE is read past; general entities declared in its internal subset are kept.
    fn doctype(&mut self, fin: bool, h: &mut Handler) -> R<Step> {
        let st = self.pos;
        let b = &self.buf;
        let mut i = st + 9;
        let mut q = 0u8;
        let mut subset: Option<(usize, usize)> = None;
        let mut sub_start = None;
        let mut end = None;
        while i < b.len() {
            let c = b[i];
            if q != 0 {
                if c == q {
                    q = 0;
                }
            } else if c == b'"' || c == b'\'' {
                q = c;
            } else if sub_start.is_some() && b[i..].starts_with(b"<!--") {
                match find(b, i + 4, b"-->") {
                    Some(e) => i = e + 2,
                    None => break,
                }
            } else if c == b'[' && sub_start.is_none() {
                sub_start = Some(i + 1);
            } else if c == b']' && subset.is_none() && sub_start.is_some() {
                subset = Some((sub_start.unwrap(), i));
            } else if c == b'>' && (sub_start.is_none() || subset.is_some()) {
                end = Some(i);
                break;
            }
            i += 1;
        }
        let Some(end) = end else {
            if !fin {
                return Ok(Step::NeedMore);
            }
            let l = self.buf.len();
            return self.fatal(h, &["DOCTYPE improperly terminated"], l);
        };
        if let Some((a, z)) = subset {
            let sub = self.buf[a..z].to_vec();
            self.attlists(&sub);
            let mut k = 0;
            while let Some(e) = find(&sub, k, b"<!ENTITY") {
                let mut j = e + 8;
                let skip = |j: &mut usize| {
                    while *j < sub.len() && blank(sub[*j]) {
                        *j += 1;
                    }
                };
                skip(&mut j);
                k = j;
                if sub.get(j) == Some(&b'%') {
                    continue;
                }
                let ns = j;
                while j < sub.len() && name_char(sub[j]) {
                    j += 1;
                }
                let name = s(&sub[ns..j]);
                skip(&mut j);
                match sub.get(j) {
                    Some(&qc @ (b'"' | b'\'')) => {
                        if let Some(ve) = sub[j + 1..].iter().position(|&c| c == qc) {
                            let v = s(&sub[j + 1..j + 1 + ve]);
                            self.entities.entry(name).or_insert(Some(v));
                            k = j + 1 + ve;
                        }
                    }
                    _ => {
                        self.entities.entry(name).or_insert(None);
                    }
                }
            }
        }
        self.pos = end + 1;
        Ok(Step::Again)
    }
}

// ---------------------------------------------------------------------------
// Ruby glue: Nokogiri::XML::SAX::ParserContext#__kenny_feed / #__kenny_html
// ---------------------------------------------------------------------------

unsafe extern "C" fn sax_free(p: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(p as *mut Sax) });
}

fn sax_of(ctx: V, create: bool) -> Option<*mut Sax> {
    let k = sym("@__sax");
    let d = ivar_get(ctx, k);
    if is_kind(d, Kind::Data) {
        return Some(unsafe { (*(d as *mut RData)).data as *mut Sax });
    }
    if !create {
        return None;
    }
    let p = Box::into_raw(Box::new(Sax::new()));
    let d = alloc(RData {
        h: Header { klass: rt().c.object, kind: Kind::Data, flags: 0 },
        data: p as *mut std::ffi::c_void,
        dmark: None,
        dfree: Some(sax_free),
        roots: Vec::new(),
    });
    ivar_set(ctx, k, d);
    Some(p)
}

fn opt_str(v: Option<&str>) -> V {
    v.map(str_from).unwrap_or(QNIL)
}

/// `ctx.__kenny_feed(handler, data, last)`: parse more XML, calling the handler's methods.
fn feed(ctx: V, a: &[V]) -> V {
    let handler = a[0];
    let data: Vec<u8> = if is_str(a[1]) { rstr(a[1]).b.clone() } else { Vec::new() };
    let fin = truthy(a[2]);
    let sax = sax_of(ctx, true).unwrap();
    let busy = sym("@__busy");
    if truthy(ivar_get(ctx, busy)) {
        return runtime_error("parser is already running");
    }
    let sax = unsafe { &mut *sax };
    sax.recovery = truthy(ivar_get(ctx, sym("@recovery")));
    sax.push = truthy(ivar_get(ctx, sym("@__push")));
    sax.replace_entities = truthy(ivar_get(ctx, sym("@replace_entities")));
    let attr_cls = class_named("Nokogiri::XML::SAX::Parser::Attribute").map(|c| c as V).unwrap_or(QNIL);
    let (line, col) = (sym("@__line"), sym("@__column"));
    let call = |m: &str, args: &[V]| -> R<()> {
        if send(handler, sym(m), args, QNIL) == UNDEF {
            Err(Abort)
        } else {
            Ok(())
        }
    };
    let mut h = |ev: Ev, l: u32, c: u32| -> R<()> {
        ivar_set(ctx, line, mkfix(l as i64));
        ivar_set(ctx, col, mkfix(c as i64));
        match ev {
            Ev::XmlDecl(v, e, sa) => call("xmldecl", &[str_from(v), opt_str(e), opt_str(sa)]),
            Ev::StartDocument => call("start_document", &[]),
            Ev::EndDocument => call("end_document", &[]),
            Ev::Start { local, prefix, uri, attrs, ns } => {
                // (rooted: Attribute.new runs Ruby, where a collection can happen)
                let mut av = crate::gc::RVec::with_capacity(attrs.len());
                for at in attrs {
                    let x = send(attr_cls, sym("new"), &[str_from(&at.local), opt_str(at.prefix.as_deref()), opt_str(at.uri.as_deref()), str_from(&at.value)], QNIL);
                    if x == UNDEF {
                        return Err(Abort);
                    }
                    av.push(x);
                }
                let nv = ns.iter().map(|(p, u)| ary_new(vec![opt_str(p.as_deref()), str_from(u)])).collect();
                call("start_element_namespace", &[str_from(local), ary_new(av.take()), opt_str(prefix), opt_str(uri), ary_new(nv)])
            }
            Ev::End { local, prefix, uri } => call("end_element_namespace", &[str_from(local), opt_str(prefix), opt_str(uri)]),
            Ev::Chars(t) => call("characters", &[str_from(t)]),
            Ev::Reference(n, c) => call("reference", &[str_from(n), str_from(c)]),
            Ev::Comment(t) => call("comment", &[str_from(t)]),
            Ev::CData(t) => call("cdata_block", &[str_from(t)]),
            Ev::Pi(t, d) => call("processing_instruction", &[str_from(t), opt_str(d)]),
            Ev::Error(m) => call("error", &[str_from(m)]),
        }
    };
    ivar_set(ctx, busy, QTRUE);
    let r = sax.feed(&data, fin, &mut h);
    ivar_set(ctx, busy, QNIL);
    match r {
        Ok(()) => QNIL,
        Err(Abort) => UNDEF,
    }
}

/// `ctx.__kenny_error`: [message, line, column, level] of the last error, or nil.
fn last_error(ctx: V) -> V {
    match sax_of(ctx, false).and_then(|p| unsafe { (*p).last_error.clone() }) {
        Some((m, l, c, lv)) => ary_new(vec![str_from(&m), mkfix(l as i64), mkfix(c as i64), mkfix(lv)]),
        None => QNIL,
    }
}

/// `ctx.__kenny_html(handler, source)`: parse HTML, then report the tree to the handler.
fn html(ctx: V, a: &[V]) -> V {
    let handler = a[0];
    let src = rs(a[1]);
    let dom = super::html::parse_document(&src, true).dom;
    ivar_set(ctx, sym("@__line"), mkfix(1));
    ivar_set(ctx, sym("@__column"), mkfix(1));
    fn walk(dom: &Dom, id: u32, h: V) -> bool {
        let call = |m: &str, args: &[V]| send(h, sym(m), args, QNIL) != UNDEF;
        let mut c = dom.n(id).first;
        while c != NONE {
            let n = dom.n(c);
            let ok = match n.t {
                T::Element => {
                    let name = dom.qname(c);
                    let attrs = n.attrs.iter().map(|&x| ary_new(vec![str_from(&dom.qname(x)), str_from(&dom.n(x).content)])).collect();
                    call("start_element", &[str_from(&name), ary_new(attrs)]) && walk(dom, c, h) && call("end_element", &[str_from(&name)])
                }
                T::Text => call("characters", &[str_from(&n.content)]),
                T::CData => call("cdata_block", &[str_from(&n.content)]),
                T::Comment => call("comment", &[str_from(&n.content)]),
                T::PI => call("processing_instruction", &[str_from(&n.name), str_from(&n.content)]),
                _ => true,
            };
            if !ok {
                return false;
            }
            c = n.next;
        }
        true
    }
    if send(handler, sym("start_document"), &[], QNIL) == UNDEF || !walk(&dom, 0, handler) || send(handler, sym("end_document"), &[], QNIL) == UNDEF {
        return UNDEF;
    }
    QNIL
}

pub(super) fn init(xmlm: *mut Class) {
    let saxm = match const_get_under(xmlm, sym("SAX")) {
        Some(v) if is_class(v) => v as *mut Class,
        _ => {
            let m = new_module("Nokogiri::XML::SAX");
            const_set(xmlm, sym("SAX"), m as V);
            unsafe { (*m).lexical_parent = xmlm };
            m
        }
    };
    let pc = new_class("Nokogiri::XML::SAX::ParserContext", rt().c.object);
    const_set(saxm, sym("ParserContext"), pc as V);
    unsafe { (*pc).lexical_parent = saxm };
    defn(pc, "__kenny_feed", 3, 3, |s, a, _| feed(s, a));
    defn(pc, "__kenny_error", 0, 0, |s, _, _| last_error(s));
    defn(pc, "__kenny_html", 2, 2, |s, a, _| html(s, a));
}

#[cfg(test)]
mod tests {
    use super::chunk_text;

    fn pieces(s: &str) -> Vec<String> {
        chunk_text(s.as_bytes()).0.into_iter().map(|(c, _)| c).collect()
    }

    #[test]
    fn text_is_split_like_libxml2() {
        assert_eq!(pieces("héllo"), ["h", "éllo"]);
        assert_eq!(pieces("a\r\nb"), ["a", "\nb"]);
        assert_eq!(pieces("é\r\nb"), ["é\nb"]);
        let long = "é".repeat(400);
        let p = pieces(&long);
        assert_eq!(p.iter().map(|c| c.chars().count()).collect::<Vec<_>>(), [150, 150, 100]);
        assert!(chunk_text(b"x]]>y").1.is_some());
    }
}
