//! Serializers: libxml2's XML (xmlsave.c), XHTML and HTML 4 (HTMLtree.c) output, and the
//! WHATWG HTML5 algorithm used by Nokogiri's HTML5 documents.

use super::dom::*;
use super::entities;

pub const FORMAT: i64 = 1;
pub const NO_DECLARATION: i64 = 2;
pub const NO_EMPTY_TAGS: i64 = 4;
pub const AS_XHTML: i64 = 16;
pub const AS_XML: i64 = 32;
pub const AS_HTML: i64 = 64;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Xml,
    Xhtml,
    Html,
}

#[derive(Clone, Copy, PartialEq)]
enum Enc {
    /// no encoding: XML writes &#xHEX; refs, HTML named entities
    None,
    /// UTF-8 (or another Unicode encoding): non-ASCII is written as is
    Utf,
    /// anything else: non-ASCII becomes &#DEC;
    Other,
}

fn enc_of(e: Option<&str>) -> Enc {
    match e {
        None => Enc::None,
        Some(s) if s.to_ascii_lowercase().starts_with("utf") => Enc::Utf,
        Some(_) => Enc::Other,
    }
}

// libxml2's html40ElementTable flags (name, empty, saveEndTag, isinline)
const INLINE: &[&str] = &[
    "a", "abbr", "acronym", "applet", "b", "basefont", "bdo", "big", "br", "button", "cite", "code", "del", "dfn", "em", "embed",
    "font", "i", "iframe", "img", "input", "ins", "kbd", "label", "map", "object", "q", "s", "samp", "script", "select", "small",
    "span", "strike", "strong", "sub", "sup", "textarea", "tt", "u", "var",
];
const EMPTY: &[&str] = &["area", "base", "basefont", "br", "col", "frame", "hr", "img", "input", "isindex", "link", "meta", "param"];
const KNOWN: &[&str] = &[
    "a", "abbr", "acronym", "address", "applet", "area", "b", "base", "basefont", "bdo", "big", "blockquote", "body", "br", "button",
    "caption", "center", "cite", "code", "col", "colgroup", "dd", "del", "dfn", "dir", "div", "dl", "dt", "em", "embed", "fieldset",
    "font", "form", "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "hr", "html", "i", "iframe", "img", "input",
    "ins", "isindex", "kbd", "label", "legend", "li", "link", "map", "menu", "meta", "noframes", "noscript", "object", "ol",
    "optgroup", "option", "p", "param", "pre", "q", "s", "samp", "script", "select", "small", "span", "strike", "strong", "style",
    "sub", "sup", "table", "tbody", "td", "textarea", "tfoot", "th", "thead", "title", "tr", "tt", "u", "ul", "var",
];
const BOOLEAN_ATTRS: &[&str] =
    &["checked", "compact", "declare", "defer", "disabled", "ismap", "multiple", "nohref", "noresize", "noshade", "nowrap", "readonly", "selected"];

struct Info {
    empty: bool,
    save_end: bool,
    inline: bool,
}
fn html_info(name: &str) -> Option<Info> {
    let n = name.to_ascii_lowercase();
    if !KNOWN.contains(&n.as_str()) {
        return None;
    }
    let empty = EMPTY.contains(&n.as_str());
    Some(Info { empty, save_end: empty || n == "li", inline: INLINE.contains(&n.as_str()) })
}

pub struct Ser<'a> {
    dom: &'a Dom,
    out: String,
    format: bool,
    no_decl: bool,
    no_empty: bool,
    indent: String,
    level: usize,
    enc: Enc,
    enc_name: Option<String>,
    xhtml: bool,
}

/// Serialize `id` like Nokogiri's native_write_to (xmlSaveTree).
pub fn serialize(dom: &Dom, id: u32, options: i64, indent: &str, encoding: Option<&str>) -> String {
    let mode = if options & AS_XHTML != 0 {
        Mode::Xhtml
    } else if (dom.html && options & AS_XML == 0) || options & AS_HTML != 0 {
        Mode::Html
    } else {
        Mode::Xml
    };
    let mut s = Ser {
        dom,
        out: String::new(),
        format: options & FORMAT != 0,
        no_decl: options & NO_DECLARATION != 0,
        no_empty: options & NO_EMPTY_TAGS != 0,
        indent: indent.to_string(),
        level: 0,
        enc: enc_of(encoding),
        enc_name: encoding.map(|e| e.to_string()),
        xhtml: mode == Mode::Xhtml,
    };
    match mode {
        Mode::Html => {
            s.html_node(id, true);
            finish_encoding(s.out, s.enc, true)
        }
        _ => {
            if id == 0 && dom.html && mode == Mode::Xml {
                // an HTML document saved as XML keeps libxml2's HTML document dump
                s.format = false;
                s.html_node(0, true);
                return finish_encoding(s.out, s.enc, false);
            }
            s.xml_node(id, true);
            finish_encoding(s.out, s.enc, false)
        }
    }
}

/// Apply the output encoding to non-ASCII characters left in the output.
fn finish_encoding(out: String, enc: Enc, html: bool) -> String {
    if enc == Enc::Utf || out.is_ascii() {
        return out;
    }
    let mut r = String::with_capacity(out.len() + 16);
    for c in out.chars() {
        if c.is_ascii() {
            r.push(c);
        } else if enc == Enc::None && html {
            match entities::name_of(c as u32) {
                Some(n) => {
                    r.push('&');
                    r.push_str(n);
                    r.push(';');
                }
                None => r.push_str(&format!("&#{};", c as u32)),
            }
        } else if enc == Enc::None {
            // XML without encoding: text was already escaped; comments etc. stay UTF-8
            r.push(c);
        } else {
            r.push_str(&format!("&#{};", c as u32));
        }
    }
    r
}

impl Ser<'_> {
    fn ind(&mut self) {
        // libxml2 caps indentation at 60 columns (MAX_INDENT / indent size levels)
        let max = if self.indent.is_empty() { 0 } else { 60 / self.indent.len() };
        for _ in 0..self.level.min(max) {
            self.out.push_str(&self.indent);
        }
    }

    // ---------- XML ----------

    fn xml_text(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '&' => self.out.push_str("&amp;"),
                '\r' => self.out.push_str("&#13;"),
                c if !c.is_ascii() && self.enc == Enc::None => self.out.push_str(&format!("&#x{:X};", c as u32)),
                c => self.out.push(c),
            }
        }
    }
    fn xml_attr_value(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '&' => self.out.push_str("&amp;"),
                '"' => self.out.push_str("&quot;"),
                '\n' => self.out.push_str("&#10;"),
                '\r' => self.out.push_str("&#13;"),
                '\t' => self.out.push_str("&#9;"),
                c if !c.is_ascii() && self.enc == Enc::None && self.dom.encoding.is_none() => {
                    self.out.push_str(&format!("&#x{:X};", c as u32))
                }
                c => self.out.push(c),
            }
        }
    }
    fn ns_defs(&mut self, id: u32) {
        for &ns in &self.dom.n(id).ns_defs {
            let n = &self.dom.nss[ns as usize];
            self.out.push_str(" xmlns");
            if let Some(p) = &n.prefix {
                self.out.push(':');
                self.out.push_str(p);
            }
            self.out.push_str("=\"");
            let href = n.href.clone();
            self.xml_attr_value(&href);
            self.out.push('"');
        }
    }
    fn xml_attrs(&mut self, id: u32) {
        for &a in &self.dom.n(id).attrs {
            self.out.push(' ');
            self.out.push_str(&self.dom.qname(a));
            self.out.push_str("=\"");
            self.xml_attr_value(&self.dom.n(a).content);
            self.out.push('"');
        }
    }
    fn doctype(&mut self, id: u32) {
        let n = self.dom.n(id);
        self.out.push_str("<!DOCTYPE ");
        self.out.push_str(&n.name);
        if let Some(ids) = &n.ids {
            match (&ids.0, &ids.1) {
                (Some(p), Some(s)) => self.out.push_str(&format!(" PUBLIC \"{p}\" \"{s}\"")),
                (Some(p), None) => self.out.push_str(&format!(" PUBLIC \"{p}\"")),
                (None, Some(s)) => self.out.push_str(&format!(" SYSTEM \"{s}\"")),
                (None, None) => {}
            }
        }
        self.out.push('>');
    }

    fn xml_node(&mut self, id: u32, root: bool) {
        let dom = self.dom;
        let n = dom.n(id);
        match n.t {
            T::Document => {
                if !self.no_decl {
                    self.out.push_str("<?xml version=\"");
                    self.out.push_str(dom.version.as_deref().unwrap_or("1.0"));
                    self.out.push('"');
                    let enc = self.enc_name.clone().or_else(|| dom.encoding.clone());
                    if let Some(e) = enc {
                        self.out.push_str(" encoding=\"");
                        self.out.push_str(&e);
                        self.out.push('"');
                    }
                    self.out.push_str("?>\n");
                }
                let mut c = n.first;
                while c != NONE {
                    self.level = 0;
                    self.xml_node(c, true);
                    self.out.push('\n');
                    c = dom.n(c).next;
                }
            }
            T::Fragment => {
                let mut c = n.first;
                while c != NONE {
                    self.xml_node(c, false);
                    c = dom.n(c).next;
                }
            }
            T::Element => {
                if !root && self.format {
                    self.ind();
                }
                let q = dom.qname(id);
                self.out.push('<');
                self.out.push_str(&q);
                self.ns_defs(id);
                self.xml_attrs(id);
                if n.first == NONE {
                    if self.xhtml {
                        if EMPTY.contains(&q.as_str()) && n.ns == NONE {
                            self.out.push_str(" />");
                        } else {
                            self.out.push_str("></");
                            self.out.push_str(&q);
                            self.out.push('>');
                        }
                    } else if self.no_empty {
                        self.out.push_str("></");
                        self.out.push_str(&q);
                        self.out.push('>');
                    } else {
                        self.out.push_str("/>");
                    }
                    return;
                }
                let saved = self.format;
                if self.format {
                    let mut c = n.first;
                    while c != NONE {
                        if matches!(dom.t(c), T::Text | T::CData | T::EntityRef) {
                            self.format = false;
                            break;
                        }
                        c = dom.n(c).next;
                    }
                }
                self.out.push('>');
                if self.format {
                    self.out.push('\n');
                }
                self.level += 1;
                let mut c = n.first;
                while c != NONE {
                    self.xml_node(c, false);
                    if self.format {
                        self.out.push('\n');
                    }
                    c = dom.n(c).next;
                }
                self.level -= 1;
                if self.format {
                    self.ind();
                }
                self.out.push_str("</");
                self.out.push_str(&q);
                self.out.push('>');
                self.format = saved;
            }
            T::Text => {
                let parent = n.parent;
                if self.xhtml && parent != NONE && matches!(dom.n(parent).name.to_ascii_lowercase().as_str(), "script" | "style") {
                    self.out.push_str(&n.content);
                } else {
                    self.xml_text(&n.content);
                }
            }
            T::CData => {
                self.out.push_str("<![CDATA[");
                self.out.push_str(&n.content.replace("]]>", "]]]]><![CDATA[>"));
                self.out.push_str("]]>");
            }
            T::Comment => {
                if !root && self.format {
                    self.ind();
                }
                self.out.push_str("<!--");
                self.out.push_str(&n.content);
                self.out.push_str("-->");
            }
            T::PI => {
                if !root && self.format {
                    self.ind();
                }
                self.out.push_str("<?");
                self.out.push_str(&n.name);
                if !n.content.is_empty() {
                    self.out.push(' ');
                    self.out.push_str(&n.content);
                }
                self.out.push_str("?>");
            }
            T::EntityRef => {
                self.out.push('&');
                self.out.push_str(&n.name);
                self.out.push(';');
            }
            T::DocType => self.doctype(id),
            T::Attr => {
                self.out.push(' ');
                self.out.push_str(&dom.qname(id));
                self.out.push_str("=\"");
                self.xml_attr_value(&n.content);
                self.out.push('"');
            }
        }
    }

    // ---------- HTML 4 (libxml2) ----------

    fn html_text(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '&' => self.out.push_str("&amp;"),
                c => self.out.push(c),
            }
        }
    }
    fn html_attr(&mut self, el: u32, a: u32) {
        let dom = self.dom;
        let an = dom.n(a);
        self.out.push(' ');
        self.out.push_str(&dom.qname(a));
        let lname = an.name.to_ascii_lowercase();
        if an.ns == NONE && BOOLEAN_ATTRS.contains(&lname.as_str()) {
            return;
        }
        self.out.push('=');
        let mut v = String::with_capacity(an.content.len());
        for c in an.content.chars() {
            match c {
                '<' => v.push_str("&lt;"),
                '>' => v.push_str("&gt;"),
                '&' => v.push_str("&amp;"),
                c => v.push(c),
            }
        }
        let el_n = dom.n(el);
        let uri = an.ns == NONE
            && el_n.ns == NONE
            && (lname == "href" || lname == "action" || lname == "src" || (lname == "name" && el_n.name.eq_ignore_ascii_case("a")));
        if uri {
            let t = v.trim_start_matches([' ', '\t', '\n', '\r']);
            let mut e = String::with_capacity(t.len());
            for b in t.bytes() {
                if b <= 0x20 || b >= 0x7f {
                    e.push_str(&format!("%{b:02X}"));
                } else {
                    e.push(b as char);
                }
            }
            v = e;
        }
        if v.contains('"') {
            if v.contains('\'') {
                self.out.push('"');
                self.out.push_str(&v.replace('"', "&quot;"));
                self.out.push('"');
            } else {
                self.out.push('\'');
                self.out.push_str(&v);
                self.out.push('\'');
            }
        } else {
            self.out.push('"');
            self.out.push_str(&v);
            self.out.push('"');
        }
    }

    fn html_node(&mut self, id: u32, _root: bool) {
        let dom = self.dom;
        let n = dom.n(id);
        match n.t {
            T::Document => {
                let mut c = n.first;
                while c != NONE {
                    if dom.t(c) == T::DocType {
                        self.doctype(c);
                        self.out.push('\n');
                    }
                    c = dom.n(c).next;
                }
                let mut c = n.first;
                while c != NONE {
                    if dom.t(c) != T::DocType {
                        self.html_node(c, false);
                    }
                    c = dom.n(c).next;
                }
                self.out.push('\n');
            }
            T::Fragment => {
                let mut c = n.first;
                while c != NONE {
                    self.html_node(c, false);
                    c = dom.n(c).next;
                }
            }
            T::Element => {
                let info = if n.ns == NONE { html_info(&n.name) } else { None };
                let q = dom.qname(id);
                self.out.push('<');
                self.out.push_str(&q);
                self.ns_defs(id);
                for &a in &n.attrs {
                    self.html_attr(id, a);
                }
                let not_text = |c: u32| !matches!(dom.t(c), T::Text | T::EntityRef);
                let block = self.format && info.as_ref().is_some_and(|i| !i.inline);
                if info.as_ref().is_some_and(|i| i.empty) {
                    self.out.push('>');
                } else if n.first == NONE {
                    if info.as_ref().is_some_and(|i| i.save_end) && q != "html" && q != "body" {
                        self.out.push('>');
                    } else {
                        self.out.push_str("></");
                        self.out.push_str(&q);
                        self.out.push('>');
                    }
                } else {
                    self.out.push('>');
                    let p_like = n.name.starts_with('p');
                    if block && not_text(n.first) && n.first != n.last && !p_like {
                        self.out.push('\n');
                    }
                    let mut c = n.first;
                    while c != NONE {
                        self.html_node(c, false);
                        c = dom.n(c).next;
                    }
                    if block && not_text(n.last) && n.first != n.last && !p_like {
                        self.out.push('\n');
                    }
                    self.out.push_str("</");
                    self.out.push_str(&q);
                    self.out.push('>');
                }
                if block && n.next != NONE && not_text(n.next) && n.parent != NONE {
                    let pn = dom.n(n.parent);
                    if pn.t == T::Element && !pn.name.is_empty() && !pn.name.starts_with('p') {
                        self.out.push('\n');
                    }
                }
            }
            T::Text => {
                let raw = n.parent != NONE && {
                    let pn = &dom.n(n.parent).name;
                    pn.eq_ignore_ascii_case("script") || pn.eq_ignore_ascii_case("style")
                };
                if raw {
                    self.out.push_str(&n.content);
                } else {
                    self.html_text(&n.content);
                }
            }
            T::CData => self.out.push_str(&n.content),
            T::Comment => {
                self.out.push_str("<!--");
                self.out.push_str(&n.content);
                self.out.push_str("-->");
            }
            T::PI => {
                self.out.push_str("<?");
                self.out.push_str(&n.name);
                if !n.content.is_empty() {
                    self.out.push(' ');
                    self.out.push_str(&n.content);
                }
                self.out.push('>');
            }
            T::EntityRef => {
                self.out.push('&');
                self.out.push_str(&n.name);
                self.out.push(';');
            }
            T::DocType => self.doctype(id),
            T::Attr => {
                let p = n.parent;
                if p != NONE {
                    self.html_attr(p, id);
                }
            }
        }
    }
}

/// libxml2's htmlSetMetaEncoding, run before saving an HTML document with an encoding.
pub fn set_meta_encoding(dom: &mut Dom, encoding: &str) {
    let newcontent = format!("text/html; charset={encoding}");
    let is = |dom: &Dom, c: u32, name: &str| dom.t(c) == T::Element && dom.n(c).name.eq_ignore_ascii_case(name);
    let mut head = NONE;
    let mut cur;
    let mut meta_start = NONE;
    // find html/head (or a meta directly)
    let mut c = dom.n(0).first;
    let mut html = NONE;
    while c != NONE {
        if is(dom, c, "html") {
            html = c;
            break;
        }
        if is(dom, c, "head") {
            head = c;
            break;
        }
        if is(dom, c, "meta") {
            meta_start = c;
            break;
        }
        c = dom.n(c).next;
    }
    if html != NONE {
        let mut c = dom.n(html).first;
        while c != NONE {
            if is(dom, c, "head") {
                head = c;
                break;
            }
            if is(dom, c, "meta") {
                head = html;
                meta_start = c;
                break;
            }
            c = dom.n(c).next;
        }
        if head == NONE {
            return;
        }
    } else if head == NONE && meta_start == NONE {
        return;
    }
    cur = if meta_start != NONE { meta_start } else { dom.n(head).first };
    let mut meta = NONE;
    let mut content = String::new();
    while cur != NONE {
        if is(dom, cur, "meta") {
            let mut http = false;
            let mut cont: Option<String> = None;
            for &a in &dom.n(cur).attrs {
                let an = dom.n(a);
                if an.name.eq_ignore_ascii_case("http-equiv") && an.content.eq_ignore_ascii_case("Content-Type") {
                    http = true;
                } else if an.name.eq_ignore_ascii_case("content") {
                    cont = Some(an.content.clone());
                }
                if http && cont.is_some() {
                    break;
                }
            }
            if http {
                if let Some(cn) = cont {
                    meta = cur;
                    content = cn;
                    break;
                }
            }
        }
        cur = dom.n(cur).next;
    }
    if meta == NONE {
        if head != NONE {
            let m = dom.new_node(T::Element, "meta", "");
            let first = dom.n(head).first;
            if first == NONE {
                dom.append(head, m);
            } else {
                dom.insert_before(first, m);
            }
            dom.set_attr(m, "http-equiv", "Content-Type");
            dom.set_attr(m, "content", &newcontent);
        }
    } else if !content.to_ascii_lowercase().contains(&encoding.to_ascii_lowercase()) {
        dom.set_attr(meta, "content", &newcontent);
    }
}

// ---------- HTML5 (WHATWG serialization, as Nokogiri's gumbo serializer) ----------

const VOID5: &[&str] = &[
    "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input", "keygen", "link", "meta", "param",
    "source", "track", "wbr",
];
const RAW5: &[&str] = &["style", "script", "xmp", "iframe", "noembed", "noframes", "plaintext"];

pub fn html5(dom: &Dom, id: u32, preserve_newline: bool) -> String {
    let mut out = String::new();
    match dom.t(id) {
        T::Document | T::Fragment => {
            for c in dom.children(id) {
                h5_node(dom, c, preserve_newline, &mut out);
            }
        }
        _ => h5_node(dom, id, preserve_newline, &mut out),
    }
    out
}

fn h5_escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn h5_node(dom: &Dom, id: u32, pn: bool, out: &mut String) {
    let n = dom.n(id);
    match n.t {
        T::Element => {
            let html_el = n.ns == NONE;
            let tag = if html_el || matches!(dom.nss[n.ns as usize].href.as_str(), super::html::SVG_NS | super::html::MATHML_NS) {
                n.name.clone()
            } else {
                dom.qname(id)
            };
            out.push('<');
            out.push_str(&tag);
            for &a in &n.attrs {
                out.push(' ');
                out.push_str(&dom.qname(a));
                out.push_str("=\"");
                h5_escape(&dom.n(a).content, true, out);
                out.push('"');
            }
            out.push('>');
            if html_el && VOID5.contains(&tag.as_str()) {
                return;
            }
            if pn && html_el && matches!(tag.as_str(), "pre" | "textarea" | "listing") {
                let f = n.first;
                if f != NONE && dom.t(f) == T::Text && dom.n(f).content.starts_with('\n') {
                    out.push('\n');
                }
            }
            let mut c = n.first;
            while c != NONE {
                h5_node(dom, c, pn, out);
                c = dom.n(c).next;
            }
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
        T::Text => {
            let raw = n.parent != NONE && {
                let p = dom.n(n.parent);
                p.t == T::Element && p.ns == NONE && RAW5.contains(&p.name.as_str())
            };
            if raw {
                out.push_str(&n.content);
            } else {
                h5_escape(&n.content, false, out);
            }
        }
        T::CData => {
            out.push_str("<![CDATA[");
            out.push_str(&n.content);
            out.push_str("]]>");
        }
        T::Comment => {
            out.push_str("<!--");
            out.push_str(&n.content);
            out.push_str("-->");
        }
        T::PI => {
            out.push_str("<?");
            out.push_str(&n.name);
            out.push(' ');
            out.push_str(&n.content);
            out.push('>');
        }
        T::DocType => {
            out.push_str("<!DOCTYPE ");
            out.push_str(&n.name);
            out.push('>');
        }
        T::EntityRef => {
            out.push('&');
            out.push_str(&n.name);
            out.push(';');
        }
        T::Document | T::Fragment => {
            for c in dom.children(id) {
                h5_node(dom, c, pn, out);
            }
        }
        T::Attr => {}
    }
}
