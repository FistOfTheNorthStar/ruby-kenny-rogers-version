//! XML parsing (quick-xml tokenizer) into a [`Dom`], with namespace resolution, predefined
//! and character entities, and libxml2-style recovery: errors are collected and the tree
//! built so far is kept.

use super::dom::*;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

pub struct XmlParsed {
    pub dom: Dom,
    pub errors: Vec<(u32, u32, String)>,
}

struct Lines<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}
impl Lines<'_> {
    fn at(&mut self, off: usize) -> u32 {
        let off = off.min(self.src.len());
        if off < self.pos {
            self.pos = 0;
            self.line = 1;
        }
        self.line += self.src[self.pos..off].iter().filter(|&&b| b == b'\n').count() as u32;
        self.pos = off;
        self.line
    }
}

fn predefined(name: &str) -> Option<&'static str> {
    Some(match name {
        "lt" => "<",
        "gt" => ">",
        "amp" => "&",
        "apos" => "'",
        "quot" => "\"",
        _ => return None,
    })
}

/// Parse a document. `into` (a parent node in an existing dom) is used for in-context
/// parsing: the parsed nodes become its children and namespaces in scope there resolve.
pub fn parse_into(dom: &mut Dom, src: &str, parent: u32, fragment: bool) -> Vec<(u32, u32, String)> {
    let mut errors: Vec<(u32, u32, String)> = Vec::new();
    let mut reader = Reader::from_str(src);
    {
        let c = reader.config_mut();
        c.check_end_names = false;
        c.allow_unmatched_ends = true;
        c.expand_empty_elements = false;
        c.trim_text(false);
    }
    let mut lines = Lines { src: src.as_bytes(), pos: 0, line: 1 };
    let mut stack: Vec<u32> = vec![parent];
    let mut seen_root = false;
    let mut last_pos = 0usize;
    loop {
        let ev = reader.read_event();
        let start = last_pos;
        let pos = reader.buffer_position() as usize;
        last_pos = pos;
        let line = lines.at(start);
        let cur = *stack.last().unwrap();
        match ev {
            Err(e) => {
                let l = lines.at(reader.error_position() as usize);
                errors.push((l, 0, format!("{e}")));
                break;
            }
            Ok(Event::Eof) => break,
            Ok(Event::Decl(d)) => {
                if !fragment {
                    if let Ok(v) = d.version() {
                        dom.version = Some(v.into_owned());
                    }
                    if let Some(Ok(e)) = d.encoding() {
                        dom.encoding = Some(e.into_owned());
                    }
                }
            }
            Ok(Event::DocType(t)) => {
                let raw = t.into_inner();
                let (name, ext, sys) = parse_doctype(&raw);
                let mut n = Node::new(T::DocType, &name, "");
                n.ids = Some(Box::new((ext, sys)));
                n.line = line;
                let id = dom.add(n);
                dom.append(cur, id);
            }
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if cur == 0 && !fragment && seen_root => {
                let _ = e;
                errors.push((line, 0, "Extra content at the end of the document".into()));
                break;
            }
            Ok(Event::Start(e)) => {
                if cur == 0 {
                    seen_root = true;
                }
                if stack.len() > 256 {
                    errors.push((line, 0, "Excessive depth in document: 256 use XML_PARSE_HUGE option".into()));
                    break;
                }
                let id = start_element(dom, &e, cur, line, &mut errors);
                stack.push(id);
            }
            Ok(Event::Empty(e)) => {
                if cur == 0 {
                    seen_root = true;
                }
                start_element(dom, &e, cur, line, &mut errors);
            }
            Ok(Event::End(e)) => {
                let name = e.name().0.to_string();
                if stack.len() <= 1 {
                    errors.push((line, 0, format!("Unexpected end tag : {name}")));
                    continue;
                }
                let top = *stack.last().unwrap();
                if dom.qname(top) == name {
                    stack.pop();
                } else if let Some(i) = stack.iter().rposition(|&s| s != parent && dom.qname(s) == name) {
                    errors.push((line, 0, format!("Opening and ending tag mismatch: {} line {} and {}", dom.qname(top), dom.n(top).line, name)));
                    stack.truncate(i);
                } else {
                    errors.push((line, 0, format!("Opening and ending tag mismatch: {} line {} and {}", dom.qname(top), dom.n(top).line, name)));
                }
            }
            Ok(Event::Text(t)) => {
                let s = t.xml10_content();
                if cur == 0 && !fragment {
                    if !s.trim().is_empty() {
                        errors.push((line, 0, if seen_root { "Extra content at the end of the document".into() } else { "Start tag expected, '<' not found".into() }));
                        break;
                    }
                    continue;
                }
                append_text(dom, cur, &s, line);
            }
            Ok(Event::GeneralRef(r)) => {
                if cur == 0 && !fragment {
                    errors.push((line, 0, "Start tag expected, '<' not found".into()));
                    break;
                }
                let name = r.into_inner().to_string();
                if let Some(num) = name.strip_prefix('#') {
                    let cp = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    match cp.and_then(char::from_u32) {
                        Some(c) => append_text(dom, cur, c.encode_utf8(&mut [0; 4]), line),
                        None => errors.push((line, 0, format!("xmlParseCharRef: invalid xmlChar value {num}"))),
                    }
                } else if let Some(v) = predefined(&name) {
                    append_text(dom, cur, v, line);
                } else {
                    errors.push((line, 0, format!("Entity '{name}' not defined")));
                    let mut n = Node::new(T::EntityRef, &name, "");
                    n.line = line;
                    let id = dom.add(n);
                    dom.append(cur, id);
                }
            }
            Ok(Event::CData(c)) => {
                let s = c.into_inner();
                let mut n = Node::new(T::CData, "#cdata-section", &s);
                n.line = line;
                let id = dom.add(n);
                dom.append(cur, id);
            }
            Ok(Event::Comment(c)) => {
                let s = c.into_inner();
                let mut n = Node::new(T::Comment, "comment", &s);
                n.line = line;
                let id = dom.add(n);
                dom.append(cur, id);
            }
            Ok(Event::PI(p)) => {
                let target = p.target().to_string();
                let content = p.content().trim_start().to_string();
                let mut n = Node::new(T::PI, &target, &content);
                n.line = line;
                let id = dom.add(n);
                dom.append(cur, id);
            }
        }
    }
    if stack.len() > 1 {
        let top = *stack.last().unwrap();
        let l = lines.at(src.len());
        errors.push((l, 0, format!("Premature end of data in tag {} line {}", dom.qname(top), dom.n(top).line)));
    }
    if !fragment && !seen_root && errors.is_empty() {
        errors.push((1, 1, "Start tag expected, '<' not found".into()));
    }
    errors
}

fn start_element(dom: &mut Dom, e: &quick_xml::events::BytesStart, cur: u32, line: u32, errors: &mut Vec<(u32, u32, String)>) -> u32 {
    let qname = e.name().0.to_string();
    let mut node = Node::new(T::Element, &qname, "");
    node.line = line;
    let id = dom.add(node);
    dom.append(cur, id);
    let mut plain: Vec<(String, String)> = Vec::new();
    for a in e.attributes().with_checks(false) {
        let a = match a {
            Ok(a) => a,
            Err(err) => {
                errors.push((line, 0, format!("{err}")));
                continue;
            }
        };
        let key = a.key.0.to_string();
        let val = match a.normalized_value(XmlVersion::Implicit1_0) {
            Ok(v) => v.into_owned(),
            Err(err) => {
                errors.push((line, 0, format!("{err}")));
                a.value.to_string()
            }
        };
        if key == "xmlns" {
            let ns = dom.intern_ns(None, &val);
            dom.m(id).ns_defs.push(ns);
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            let ns = dom.intern_ns(Some(p), &val);
            dom.m(id).ns_defs.push(ns);
        } else {
            plain.push((key, val));
        }
    }
    // element namespace
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(p.to_string()), l.to_string()),
        None => (None, qname.clone()),
    };
    match dom.lookup_prefix(id, prefix.as_deref()) {
        Some(ns) => {
            dom.m(id).ns = ns;
            dom.m(id).name = local;
        }
        None => {
            if let Some(p) = &prefix {
                errors.push((line, 0, format!("Namespace prefix {p} on {local} is not defined")));
            }
        }
    }
    for (k, v) in plain {
        let mut an = Node::new(T::Attr, &k, &v);
        if let Some((p, l)) = k.split_once(':') {
            match dom.lookup_prefix(id, Some(p)) {
                Some(ns) => {
                    an.ns = ns;
                    an.name = l.to_string();
                }
                None => errors.push((line, 0, format!("Namespace prefix {p} for {l} on {qname} is not defined"))),
            }
        }
        if dom.n(id).attrs.iter().any(|&x| dom.n(x).name == an.name && dom.n(x).ns == an.ns) {
            errors.push((line, 0, format!("Attribute {k} redefined")));
            continue;
        }
        let aid = dom.add(an);
        dom.m(aid).parent = id;
        dom.m(id).attrs.push(aid);
    }
    id
}

fn append_text(dom: &mut Dom, parent: u32, s: &str, line: u32) {
    let last = dom.n(parent).last;
    if last != NONE && dom.t(last) == T::Text {
        dom.m(last).content.push_str(s);
        return;
    }
    let mut n = Node::new(T::Text, "text", s);
    n.line = line;
    let id = dom.add(n);
    dom.append(parent, id);
}

pub fn parse_document(src: &str) -> XmlParsed {
    let mut dom = Dom::new(false);
    dom.version = Some("1.0".into());
    let errors = parse_into(&mut dom, src, 0, false);
    XmlParsed { dom, errors }
}

/// `<!DOCTYPE name PUBLIC "pub" "sys" [...]>` contents (after `<!DOCTYPE`).
fn parse_doctype(raw: &str) -> (String, Option<String>, Option<String>) {
    let s = raw.trim();
    let s = match s.find('[') {
        Some(i) => &s[..i],
        None => s,
    };
    let mut toks: Vec<String> = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
        } else if c == '"' || c == '\'' {
            it.next();
            let mut t = String::new();
            for d in it.by_ref() {
                if d == c {
                    break;
                }
                t.push(d);
            }
            toks.push(t);
        } else {
            let mut t = String::new();
            while let Some(&d) = it.peek() {
                if d.is_whitespace() {
                    break;
                }
                t.push(d);
                it.next();
            }
            toks.push(t);
        }
    }
    let name = toks.first().cloned().unwrap_or_default();
    match toks.get(1).map(|s| s.to_ascii_uppercase()) {
        Some(k) if k == "PUBLIC" => (name, toks.get(2).cloned(), toks.get(3).cloned()),
        Some(k) if k == "SYSTEM" => (name, None, toks.get(2).cloned()),
        _ => (name, None, None),
    }
}
