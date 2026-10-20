//! The document tree: one arena of nodes per document (node 0 is the document node),
//! linked like libxml2's (parent / first / last / prev / next), with attributes as nodes
//! owned by their element and namespaces in a per-document table.
//!
//! Nodes are never freed individually: an unlinked node stays in the arena (Ruby objects
//! may still refer to it) until the whole document is collected.

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum T {
    Element,
    Attr,
    Text,
    CData,
    EntityRef,
    PI,
    Comment,
    Document,
    DocType,
    Fragment,
}

#[derive(Clone)]
pub struct Node {
    pub t: T,
    /// element / attribute local name, PI target, DTD name, entity name
    pub name: String,
    /// text, comment, CDATA, attribute value, PI data
    pub content: String,
    /// namespace (index into `Dom::nss`) or NONE
    pub ns: u32,
    pub parent: u32,
    pub prev: u32,
    pub next: u32,
    pub first: u32,
    pub last: u32,
    /// attribute nodes of an element, in order
    pub attrs: Vec<u32>,
    /// namespaces declared on this element (xmlns attributes)
    pub ns_defs: Vec<u32>,
    pub line: u32,
    /// DTD external id / system id
    pub ids: Option<Box<(Option<String>, Option<String>)>>,
}

impl Node {
    pub fn new(t: T, name: &str, content: &str) -> Node {
        Node {
            t,
            name: name.to_string(),
            content: content.to_string(),
            ns: NONE,
            parent: NONE,
            prev: NONE,
            next: NONE,
            first: NONE,
            last: NONE,
            attrs: Vec::new(),
            ns_defs: Vec::new(),
            line: 0,
            ids: None,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Ns {
    pub prefix: Option<String>,
    pub href: String,
}

pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

pub struct Dom {
    pub nodes: Vec<Node>,
    pub nss: Vec<Ns>,
    /// an HTML document (HTML4 or HTML5): node type 13, HTML serialization by default
    pub html: bool,
    pub encoding: Option<String>,
    pub version: Option<String>,
    pub url: Option<String>,
    pub serial: u64,
}

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Dom {
    pub fn new(html: bool) -> Dom {
        Dom {
            nodes: vec![Node::new(T::Document, "", "")],
            nss: Vec::new(),
            html,
            encoding: None,
            version: None,
            url: None,
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    pub fn add(&mut self, n: Node) -> u32 {
        self.nodes.push(n);
        (self.nodes.len() - 1) as u32
    }
    pub fn new_node(&mut self, t: T, name: &str, content: &str) -> u32 {
        self.add(Node::new(t, name, content))
    }
    #[inline]
    pub fn n(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }
    #[inline]
    pub fn m(&mut self, id: u32) -> &mut Node {
        &mut self.nodes[id as usize]
    }
    pub fn t(&self, id: u32) -> T {
        self.nodes[id as usize].t
    }

    /// The namespace index for (prefix, href), reusing an equal entry.
    pub fn intern_ns(&mut self, prefix: Option<&str>, href: &str) -> u32 {
        if let Some(i) = self.nss.iter().position(|n| n.prefix.as_deref() == prefix && n.href == href) {
            return i as u32;
        }
        self.nss.push(Ns { prefix: prefix.map(|s| s.to_string()), href: href.to_string() });
        (self.nss.len() - 1) as u32
    }

    // ---------- tree links ----------

    pub fn unlink(&mut self, id: u32) {
        let (t, parent, prev, next) = {
            let n = self.n(id);
            (n.t, n.parent, n.prev, n.next)
        };
        if t == T::Attr {
            if parent != NONE {
                self.m(parent).attrs.retain(|a| *a != id);
            }
            self.m(id).parent = NONE;
            return;
        }
        if prev != NONE {
            self.m(prev).next = next;
        } else if parent != NONE {
            self.m(parent).first = next;
        }
        if next != NONE {
            self.m(next).prev = prev;
        } else if parent != NONE {
            self.m(parent).last = prev;
        }
        let n = self.m(id);
        n.parent = NONE;
        n.prev = NONE;
        n.next = NONE;
    }

    /// Append an unlinked node as the last child of `parent`.
    pub fn append(&mut self, parent: u32, child: u32) {
        self.unlink(child);
        if self.t(child) == T::Attr {
            self.m(parent).attrs.push(child);
            self.m(child).parent = parent;
            return;
        }
        let last = self.n(parent).last;
        {
            let c = self.m(child);
            c.parent = parent;
            c.prev = last;
            c.next = NONE;
        }
        if last != NONE {
            self.m(last).next = child;
        } else {
            self.m(parent).first = child;
        }
        self.m(parent).last = child;
    }

    pub fn insert_before(&mut self, sib: u32, child: u32) {
        if child == sib {
            return;
        }
        self.unlink(child);
        let (parent, prev) = (self.n(sib).parent, self.n(sib).prev);
        {
            let c = self.m(child);
            c.parent = parent;
            c.prev = prev;
            c.next = sib;
        }
        self.m(sib).prev = child;
        if prev != NONE {
            self.m(prev).next = child;
        } else if parent != NONE {
            self.m(parent).first = child;
        }
    }

    pub fn insert_after(&mut self, sib: u32, child: u32) {
        if child == sib {
            return;
        }
        self.unlink(child);
        let (parent, next) = (self.n(sib).parent, self.n(sib).next);
        {
            let c = self.m(child);
            c.parent = parent;
            c.prev = sib;
            c.next = next;
        }
        self.m(sib).next = child;
        if next != NONE {
            self.m(next).prev = child;
        } else if parent != NONE {
            self.m(parent).last = child;
        }
    }

    pub fn children(&self, id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = self.n(id).first;
        while c != NONE {
            out.push(c);
            c = self.n(c).next;
        }
        out
    }

    pub fn is_ancestor(&self, anc: u32, mut id: u32) -> bool {
        while id != NONE {
            if id == anc {
                return true;
            }
            id = self.n(id).parent;
        }
        false
    }

    /// The document element.
    pub fn root(&self) -> u32 {
        let mut c = self.n(0).first;
        while c != NONE {
            if self.t(c) == T::Element {
                return c;
            }
            c = self.n(c).next;
        }
        NONE
    }

    pub fn qname(&self, id: u32) -> String {
        let n = self.n(id);
        match (n.ns, n.t) {
            (ns, T::Element | T::Attr) if ns != NONE => match &self.nss[ns as usize].prefix {
                Some(p) if !p.is_empty() => format!("{p}:{}", n.name),
                _ => n.name.clone(),
            },
            _ => n.name.clone(),
        }
    }

    // ---------- attributes ----------

    pub fn attr_node(&self, el: u32, name: &str) -> u32 {
        // a plain name matches an attribute without namespace first, then a prefixed one
        for &a in &self.n(el).attrs {
            let an = self.n(a);
            if an.name == name && an.ns == NONE {
                return a;
            }
        }
        for &a in &self.n(el).attrs {
            if self.qname(a) == name {
                return a;
            }
        }
        NONE
    }
    pub fn attr_node_ns(&self, el: u32, name: &str, href: Option<&str>) -> u32 {
        for &a in &self.n(el).attrs {
            let an = self.n(a);
            if an.name != name {
                continue;
            }
            let h = if an.ns == NONE { None } else { Some(self.nss[an.ns as usize].href.as_str()) };
            if h == href {
                return a;
            }
        }
        NONE
    }
    pub fn get_attr(&self, el: u32, name: &str) -> Option<&str> {
        let a = self.attr_node(el, name);
        if a == NONE { None } else { Some(self.n(a).content.as_str()) }
    }

    /// Set an attribute by (possibly prefixed) name, as libxml2's xmlSetProp does.
    pub fn set_attr(&mut self, el: u32, name: &str, value: &str) -> u32 {
        let a = self.attr_node(el, name);
        if a != NONE {
            self.m(a).content = value.to_string();
            return a;
        }
        let mut node = Node::new(T::Attr, name, value);
        if !self.html {
            if let Some((p, local)) = name.split_once(':') {
                if let Some(ns) = self.lookup_prefix(el, Some(p)) {
                    node.name = local.to_string();
                    node.ns = ns;
                }
            }
        }
        let id = self.add(node);
        self.m(id).parent = el;
        self.m(el).attrs.push(id);
        id
    }

    // ---------- namespaces ----------

    /// The in-scope namespace for a prefix (None = default namespace).
    pub fn lookup_prefix(&mut self, mut id: u32, prefix: Option<&str>) -> Option<u32> {
        if prefix == Some("xml") {
            return Some(self.intern_ns(Some("xml"), XML_NS));
        }
        while id != NONE {
            let n = self.n(id);
            if n.t == T::Element {
                for &ns in &n.ns_defs {
                    if self.nss[ns as usize].prefix.as_deref() == prefix {
                        return Some(ns);
                    }
                }
            }
            id = n.parent;
        }
        None
    }
    /// Namespaces in scope at an element, innermost declaration of each prefix first.
    pub fn ns_scopes(&self, mut id: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        while id != NONE {
            let n = self.n(id);
            if n.t == T::Element {
                for &ns in &n.ns_defs {
                    let p = &self.nss[ns as usize].prefix;
                    if !out.iter().any(|o| &self.nss[*o as usize].prefix == p) {
                        out.push(ns);
                    }
                }
            }
            id = n.parent;
        }
        out
    }

    // ---------- content ----------

    /// libxml2's xmlNodeGetContent.
    pub fn content(&self, id: u32) -> String {
        let n = self.n(id);
        match n.t {
            T::Element | T::Document | T::Fragment => {
                let mut s = String::new();
                self.collect_text(id, &mut s);
                s
            }
            T::DocType | T::EntityRef => String::new(),
            _ => n.content.clone(),
        }
    }
    pub fn collect_text(&self, id: u32, out: &mut String) {
        let mut c = self.n(id).first;
        while c != NONE {
            let n = self.n(c);
            match n.t {
                T::Text | T::CData => out.push_str(&n.content),
                T::Element | T::Fragment => self.collect_text(c, out),
                _ => {}
            }
            c = n.next;
        }
    }

    /// Replace a node's content (libxml2's xmlNodeSetContent: children become one text node).
    pub fn set_content(&mut self, id: u32, s: &str) {
        match self.t(id) {
            T::Element | T::Document | T::Fragment => {
                for c in self.children(id) {
                    self.unlink(c);
                }
                if !s.is_empty() {
                    let t = self.new_node(T::Text, "text", s);
                    self.append(id, t);
                }
            }
            _ => self.m(id).content = s.to_string(),
        }
    }

    // ---------- copying ----------

    /// Deep-copy (level 1), or copy without children (level 0/2), a node from `src` into
    /// `self`. `map` receives (old id, new id) for every copied node (attributes included).
    pub fn copy_from(&mut self, src: &Dom, id: u32, level: i64, map: &mut Vec<(u32, u32)>) -> u32 {
        let sn = src.n(id);
        let mut n = Node::new(sn.t, &sn.name, &sn.content);
        if n.t == T::Document {
            n.t = T::Fragment;
        }
        n.line = sn.line;
        n.ids = sn.ids.clone();
        n.ns = if sn.ns != NONE { self.import_ns(src, sn.ns) } else { NONE };
        n.ns_defs = sn.ns_defs.iter().map(|&x| self.import_ns(src, x)).collect();
        let new = self.add(n);
        map.push((id, new));
        if sn.t == T::Element && level != 0 {
            for &a in &src.n(id).attrs {
                let na = self.copy_from(src, a, 1, map);
                self.m(na).parent = new;
                self.m(new).attrs.push(na);
            }
        }
        if level == 1 {
            let mut c = src.n(id).first;
            while c != NONE {
                let nc = self.copy_from(src, c, 1, map);
                self.append(new, nc);
                c = src.n(c).next;
            }
        }
        new
    }
    fn import_ns(&mut self, src: &Dom, ns: u32) -> u32 {
        let n = &src.nss[ns as usize];
        let (p, h) = (n.prefix.clone(), n.href.clone());
        self.intern_ns(p.as_deref(), &h)
    }

    /// Copy within the same document.
    pub fn copy_local(&mut self, id: u32, level: i64, map: &mut Vec<(u32, u32)>) -> u32 {
        let sn = self.n(id).clone();
        let mut n = Node::new(sn.t, &sn.name, &sn.content);
        if n.t == T::Document {
            n.t = T::Fragment;
        }
        n.line = sn.line;
        n.ids = sn.ids;
        n.ns = sn.ns;
        n.ns_defs = sn.ns_defs;
        let new = self.add(n);
        map.push((id, new));
        if sn.t == T::Element && level != 0 {
            for a in sn.attrs {
                let na = self.copy_local(a, 1, map);
                self.m(na).parent = new;
                self.m(new).attrs.push(na);
            }
        }
        if level == 1 {
            for c in self.children(id) {
                let nc = self.copy_local(c, 1, map);
                self.append(new, nc);
            }
        }
        new
    }

    // ---------- order ----------

    /// Document-order keys for every node: the document tree first, then each detached
    /// tree (attributes come right after their element).
    pub fn order_all(&self) -> Vec<u64> {
        let mut keys = vec![u64::MAX; self.nodes.len()];
        let mut counter = 0u64;
        self.number(0, &mut keys, &mut counter);
        for id in 0..self.nodes.len() as u32 {
            if keys[id as usize] != u64::MAX {
                continue;
            }
            let mut r = id;
            while self.n(r).parent != NONE {
                r = self.n(r).parent;
            }
            if keys[r as usize] == u64::MAX {
                self.number(r, &mut keys, &mut counter);
            }
            if keys[id as usize] == u64::MAX {
                keys[id as usize] = counter;
                counter += 1;
            }
        }
        keys
    }
    fn number(&self, root: u32, keys: &mut [u64], counter: &mut u64) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            keys[id as usize] = *counter;
            *counter += 1;
            let n = self.n(id);
            for &a in &n.attrs {
                keys[a as usize] = *counter;
                *counter += 1;
            }
            // push children in reverse so the first child is numbered first
            let mut c = n.last;
            while c != NONE {
                stack.push(c);
                c = self.n(c).prev;
            }
        }
    }
}
