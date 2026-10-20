//! HTML parsing with html5ever (the WHATWG algorithm, like Nokogiri's gumbo-based HTML5
//! parser) into a [`Dom`]. In HTML4 mode the tree is massaged towards what libxml2's HTML
//! parser builds: names lowercased, no namespaces, no implied `<head>`/`<tbody>`, and a
//! default HTML 4.0 Transitional doctype.

use super::dom::*;
use html5ever::interface::tree_builder::{ElemName, ElementFlags, NodeOrText, QuirksMode, Tracer, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use html5ever::TokenizerResult;
use html5ever::tree_builder::{TreeBuilder, TreeBuilderOpts};
use html5ever::{ns, Attribute, LocalName, Namespace, QualName};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

pub const SVG_NS: &str = "http://www.w3.org/2000/svg";
pub const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";
pub const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

#[derive(Debug)]
pub struct OwnedName(QualName);
impl ElemName for OwnedName {
    fn ns(&self) -> &Namespace {
        &self.0.ns
    }
    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

struct Sink {
    dom: RefCell<Dom>,
    names: RefCell<Vec<Option<QualName>>>,
    aip: RefCell<Vec<u32>>,
    html4: bool,
    line: Cell<u64>,
    quirks: Cell<QuirksMode>,
    errors: RefCell<Vec<String>>,
    /// name of the start tag being processed, and the elements created with that name
    cur_tag: RefCell<Option<LocalName>>,
    cur_created: RefCell<Vec<u32>>,
    /// elements the tree builder made up (implied, reconstructed or cloned formatting
    /// elements, phantom <p>/<tbody>...) rather than created for a start tag
    synthetic: RefCell<Vec<u32>>,
    /// element depth per node: start tags nesting deeper than MAX_DEPTH are ignored (libxml2
    /// stops at 256 levels, gumbo at 400), which also keeps the HTML5 algorithm's stack scans
    /// from going quadratic on pathological input
    depth: RefCell<Vec<u16>>,
}

const MAX_DEPTH: u16 = 400;

impl Sink {
    fn new(html4: bool) -> Sink {
        Sink {
            dom: RefCell::new(Dom::new(true)),
            names: RefCell::new(vec![None]),
            aip: RefCell::new(Vec::new()),
            html4,
            line: Cell::new(1),
            quirks: Cell::new(QuirksMode::NoQuirks),
            errors: RefCell::new(Vec::new()),
            cur_tag: RefCell::new(None),
            cur_created: RefCell::new(Vec::new()),
            synthetic: RefCell::new(Vec::new()),
            depth: RefCell::new(vec![0]),
        }
    }
    fn add(&self, mut n: Node, name: Option<QualName>) -> u32 {
        n.line = self.line.get() as u32;
        let id = self.dom.borrow_mut().add(n);
        let mut names = self.names.borrow_mut();
        while names.len() <= id as usize {
            names.push(None);
        }
        names[id as usize] = name;
        id
    }
    fn text_node(&self, text: &str) -> u32 {
        self.add(Node::new(T::Text, "text", text), None)
    }
    fn attr_node(&self, el: u32, a: &Attribute) {
        let mut node = Node::new(T::Attr, "", &a.value);
        let local: &str = &a.name.local;
        let prefix = a.name.prefix.as_ref().map(|p| p.to_string());
        if a.name.ns == ns!() || self.html4 {
            node.name = match &prefix {
                Some(p) => format!("{p}:{local}"),
                None => local.to_string(),
            };
            if self.html4 {
                node.name = node.name.to_ascii_lowercase();
            }
        } else {
            node.name = local.to_string();
            let href: &str = &a.name.ns;
            node.ns = self.dom.borrow_mut().intern_ns(prefix.as_deref(), href);
        }
        let mut dom = self.dom.borrow_mut();
        if dom.n(el).attrs.iter().any(|&x| dom.n(x).name == node.name && dom.n(x).ns == node.ns) {
            return;
        }
        let id = dom.add(node);
        dom.m(id).parent = el;
        dom.m(el).attrs.push(id);
    }
    fn depth_of(&self, id: u32) -> u16 {
        self.depth.borrow().get(id as usize).copied().unwrap_or(0)
    }
    fn set_depth(&self, id: u32, d: u16) {
        let mut v = self.depth.borrow_mut();
        if v.len() <= id as usize {
            v.resize(id as usize + 1, 0);
        }
        v[id as usize] = d;
    }
    fn append_text(&self, parent: u32, text: &str) {
        let last = self.dom.borrow().n(parent).last;
        if last != NONE && self.dom.borrow().t(last) == T::Text {
            self.dom.borrow_mut().m(last).content.push_str(text);
            return;
        }
        let t = self.text_node(text);
        self.dom.borrow_mut().append(parent, t);
    }
}

impl TreeSink for Sink {
    type Handle = u32;
    type Output = Self;
    type ElemName<'a> = OwnedName;

    fn finish(self) -> Self {
        self
    }
    fn parse_error(&self, msg: Cow<'static, str>) {
        let mut e = self.errors.borrow_mut();
        if e.len() < 100 {
            e.push(msg.into_owned());
        }
    }
    fn get_document(&self) -> u32 {
        0
    }
    fn elem_name<'a>(&'a self, target: &'a u32) -> OwnedName {
        let names = self.names.borrow();
        match names.get(*target as usize).and_then(|n| n.clone()) {
            Some(q) => OwnedName(q),
            None => OwnedName(QualName::new(None, ns!(html), LocalName::from(""))),
        }
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> u32 {
        let local: &str = &name.local;
        let mut node = Node::new(T::Element, local, "");
        if self.html4 {
            node.name = node.name.to_ascii_lowercase();
        } else if name.ns == ns!(svg) {
            node.ns = self.dom.borrow_mut().intern_ns(Some("svg"), SVG_NS);
        } else if name.ns == ns!(mathml) {
            node.ns = self.dom.borrow_mut().intern_ns(Some("math"), MATHML_NS);
        }
        let own = self.cur_tag.borrow().as_ref().is_some_and(|n| *n == name.local);
        let id = self.add(node, Some(name));
        if own {
            self.cur_created.borrow_mut().push(id);
        } else {
            self.synthetic.borrow_mut().push(id);
        }
        if flags.mathml_annotation_xml_integration_point {
            self.aip.borrow_mut().push(id);
        }
        for a in &attrs {
            self.attr_node(id, a);
        }
        id
    }
    fn create_comment(&self, text: StrTendril) -> u32 {
        self.add(Node::new(T::Comment, "comment", &text), None)
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> u32 {
        self.add(Node::new(T::PI, &target, &data), None)
    }
    fn append(&self, parent: &u32, child: NodeOrText<u32>) {
        let parent = *parent;
        match child {
            NodeOrText::AppendText(t) => self.append_text(parent, &t),
            NodeOrText::AppendNode(n) => {
                self.dom.borrow_mut().append(parent, n);
                self.set_depth(n, self.depth_of(parent).saturating_add(1));
            }
        }
    }
    fn append_based_on_parent_node(&self, element: &u32, prev_element: &u32, child: NodeOrText<u32>) {
        if self.dom.borrow().n(*element).parent != NONE {
            self.append_before_sibling(element, child)
        } else {
            self.append(prev_element, child)
        }
    }
    fn append_doctype_to_document(&self, name: StrTendril, public_id: StrTendril, system_id: StrTendril) {
        let mut n = Node::new(T::DocType, &name, "");
        let opt = |s: &StrTendril| if s.is_empty() { None } else { Some(s.to_string()) };
        n.ids = Some(Box::new((opt(&public_id), opt(&system_id))));
        let id = self.add(n, None);
        self.dom.borrow_mut().append(0, id);
    }
    fn get_template_contents(&self, target: &u32) -> u32 {
        // like libxml2 and Nokogiri's HTML5 parser, template contents are plain children
        *target
    }
    fn same_node(&self, x: &u32, y: &u32) -> bool {
        x == y
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks.set(mode);
    }
    fn append_before_sibling(&self, sibling: &u32, child: NodeOrText<u32>) {
        match child {
            NodeOrText::AppendText(t) => {
                let prev = self.dom.borrow().n(*sibling).prev;
                if prev != NONE && self.dom.borrow().t(prev) == T::Text {
                    self.dom.borrow_mut().m(prev).content.push_str(&t);
                    return;
                }
                let n = self.text_node(&t);
                self.dom.borrow_mut().insert_before(*sibling, n);
            }
            NodeOrText::AppendNode(n) => {
                self.dom.borrow_mut().insert_before(*sibling, n);
                self.set_depth(n, self.depth_of(*sibling));
            }
        }
    }
    fn add_attrs_if_missing(&self, target: &u32, attrs: Vec<Attribute>) {
        // libxml2 ignores the attributes of a repeated <html>/<body>
        if self.html4 {
            return;
        }
        for a in &attrs {
            self.attr_node(*target, a);
        }
    }
    fn remove_from_parent(&self, target: &u32) {
        self.dom.borrow_mut().unlink(*target);
    }
    fn reparent_children(&self, node: &u32, new_parent: &u32) {
        let kids = self.dom.borrow().children(*node);
        let mut dom = self.dom.borrow_mut();
        for k in kids {
            dom.append(*new_parent, k);
        }
    }
    fn is_mathml_annotation_xml_integration_point(&self, handle: &u32) -> bool {
        self.aip.borrow().contains(handle)
    }
    fn set_current_line(&self, line: u64) {
        self.line.set(line);
    }
}

/// Forwards tokens to the tree builder, noting which start tag is being processed so the
/// sink can tell elements made for a tag from the ones the algorithm makes up.
struct Spy {
    tb: TreeBuilder<u32, Sink>,
}

/// Deepest element among the tree builder's open elements.
struct DepthTracer<'a> {
    sink: &'a Sink,
    max: Cell<u16>,
}
impl Tracer for DepthTracer<'_> {
    type Handle = u32;
    fn trace_handle(&self, h: &u32) {
        self.max.set(self.max.get().max(self.sink.depth_of(*h)));
    }
}

impl TokenSink for Spy {
    type Handle = u32;
    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<u32> {
        let start = match &token {
            Token::TagToken(t) if t.kind == TagKind::StartTag => Some(t.name.clone()),
            _ => None,
        };
        let sink = &self.tb.sink;
        if start.is_some() && sink.dom.borrow().nodes.len() > MAX_DEPTH as usize {
            let t = DepthTracer { sink, max: Cell::new(0) };
            self.tb.trace_handles(&t);
            if t.max.get() >= MAX_DEPTH {
                return TokenSinkResult::Continue;
            }
        }
        *sink.cur_tag.borrow_mut() = start;
        sink.cur_created.borrow_mut().clear();
        let r = self.tb.process_token(token, line);
        // a start tag may first reconstruct formatting elements of the same name: the
        // element made for the tag itself is the last one
        let created = std::mem::take(&mut *sink.cur_created.borrow_mut());
        if let Some((_, rest)) = created.split_last() {
            sink.synthetic.borrow_mut().extend_from_slice(rest);
        }
        *sink.cur_tag.borrow_mut() = None;
        r
    }
    fn end(&self) {
        self.tb.end()
    }
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.tb.adjusted_current_node_present_but_not_in_html_namespace()
    }
}

fn tb_opts() -> TreeBuilderOpts {
    // parse <noscript> contents as markup (as libxml2 and gumbo do)
    TreeBuilderOpts { scripting_enabled: false, ..Default::default() }
}

fn run(tok: Tokenizer<Spy>, src: &str) -> Sink {
    let buf = BufferQueue::default();
    buf.push_back(StrTendril::from(src));
    while !matches!(tok.feed(&buf), TokenizerResult::Done) {}
    tok.end();
    tok.sink.tb.sink
}

pub struct Parsed {
    pub dom: Dom,
    pub quirks: i64,
    pub errors: Vec<String>,
}

fn quirks_code(q: QuirksMode) -> i64 {
    match q {
        QuirksMode::NoQuirks => 0,
        QuirksMode::Quirks => 1,
        QuirksMode::LimitedQuirks => 2,
    }
}

/// Parse a whole document.
pub fn parse_document(src: &str, html4: bool) -> Parsed {
    let tb = TreeBuilder::new(Sink::new(html4), tb_opts());
    let sink = run(Tokenizer::new(Spy { tb }, TokenizerOpts::default()), src);
    let quirks = quirks_code(sink.quirks.get());
    let errors = sink.errors.into_inner();
    let synthetic = sink.synthetic.into_inner();
    let mut dom = sink.dom.into_inner();
    if html4 {
        html4_common_fixups(&mut dom, src, &synthetic);
        html4_fixups(&mut dom);
    }
    Parsed { dom, quirks, errors }
}

fn find_elements(dom: &Dom, id: u32, name: &str, out: &mut Vec<u32>) {
    let mut c = dom.n(id).first;
    while c != NONE {
        if dom.t(c) == T::Element {
            if dom.n(c).name == name {
                out.push(c);
            }
            find_elements(dom, c, name, out);
        }
        c = dom.n(c).next;
    }
}

const FORMATTING: &[&str] = &["a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u"];

/// Fix-ups for documents and fragments, towards what libxml2's HTML parser builds: made-up
/// elements are dropped (formatting elements reconstructed or cloned by the HTML5 algorithm,
/// implied <tbody>, the empty <p> of a stray </p>, an empty implied <head>), and script/style
/// contents are CDATA nodes.
fn html4_common_fixups(dom: &mut Dom, src: &str, synthetic: &[u32]) {
    for &id in synthetic {
        if dom.t(id) != T::Element || dom.n(id).parent == NONE {
            continue;
        }
        let name = dom.n(id).name.clone();
        let unwrap = FORMATTING.contains(&name.as_str()) || matches!(name.as_str(), "tbody" | "tr" | "colgroup");
        let drop_empty = (name == "p" || name == "head") && dom.n(id).first == NONE && dom.n(id).attrs.is_empty();
        if unwrap {
            for c in dom.children(id) {
                dom.insert_before(id, c);
            }
            dom.unlink(id);
        } else if drop_empty {
            dom.unlink(id);
        }
    }
    // an unterminated comment at the end of the input is dropped by libxml2
    if let Some(open) = src.rfind("<!--") {
        if !src[open..].contains("-->") {
            let keys = dom.order_all();
            let last = (0..dom.nodes.len() as u32)
                .filter(|&i| dom.t(i) == T::Comment && keys[i as usize] != u64::MAX && dom.n(i).parent != NONE)
                .max_by_key(|&i| keys[i as usize]);
            if let Some(c) = last {
                dom.unlink(c);
            }
        }
    }
    let mut raw = Vec::new();
    find_elements(dom, 0, "script", &mut raw);
    find_elements(dom, 0, "style", &mut raw);
    for el in raw {
        for c in dom.children(el) {
            if dom.t(c) == T::Text {
                let n = dom.m(c);
                n.t = T::CData;
                n.name = "#cdata-section".into();
            }
        }
    }
}

fn html4_fixups(dom: &mut Dom) {
    let has_doctype = dom.children(0).iter().any(|&c| dom.t(c) == T::DocType);
    if !has_doctype {
        add_default_dtd(dom);
    }
}

/// libxml2's htmlNewDoc default: HTML 4.0 Transitional.
pub fn add_default_dtd(dom: &mut Dom) {
    let mut n = Node::new(T::DocType, "html", "");
    n.ids = Some(Box::new((
        Some("-//W3C//DTD HTML 4.0 Transitional//EN".to_string()),
        Some("http://www.w3.org/TR/REC-html40/loose.dtd".to_string()),
    )));
    let id = dom.add(n);
    let first = dom.n(0).first;
    if first == NONE {
        dom.append(0, id);
    } else {
        dom.insert_before(first, id);
    }
}

/// Parse `src` as the children of a `ctx_name` element (in namespace `ctx_ns`, default HTML).
/// Returns the parsed dom and its top-level nodes (children of the synthetic root).
pub fn parse_fragment(src: &str, ctx_name: &str, ctx_ns: &str, html4: bool) -> (Dom, Vec<u32>, Vec<String>) {
    let ns = match ctx_ns {
        SVG_NS => ns!(svg),
        MATHML_NS => ns!(mathml),
        _ => ns!(html),
    };
    let ctx = QualName::new(None, ns, LocalName::from(ctx_name));
    let sink = Sink::new(html4);
    let ctx_el = html5ever::interface::create_element(&sink, ctx, Vec::new());
    let tb = TreeBuilder::new_for_fragment(sink, ctx_el, None, tb_opts());
    let initial = tb.tokenizer_state_for_context_elem(false);
    let tok = Tokenizer::new(Spy { tb }, TokenizerOpts { initial_state: Some(initial), ..Default::default() });
    let sink = run(tok, src);
    let errors = sink.errors.into_inner();
    let synthetic = sink.synthetic.into_inner();
    let mut dom = sink.dom.into_inner();
    if html4 {
        html4_common_fixups(&mut dom, src, &synthetic);
    }
    // the fragment algorithm puts everything under a synthetic <html> root
    let root = dom.n(0).first;
    let kids = if root == NONE { Vec::new() } else { dom.children(root) };
    (dom, kids, errors)
}
