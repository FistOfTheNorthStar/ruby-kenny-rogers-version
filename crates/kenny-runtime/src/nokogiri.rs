//! Native core of Nokogiri (`require "nokogiri/nokogiri"`), replacing the libxml2/gumbo C
//! extension: documents, nodes, node sets, namespaces and XPath contexts. Nokogiri's own
//! Ruby layer (lib/kenny/nokogiri/**) runs on top, as it does on CRuby.
//!
//! Model: each Document object owns a Rust [`Dom`] (hidden `@__dom` RData, freed with the
//! document). A node object is a plain Ruby object of the right class holding `@__d` (its
//! document) and `@__i` (node index); wrappers are cached per node in the document's `@__w`
//! Array, so identity and instance variables survive. Moving a node into another document
//! copies it there and re-points every existing wrapper of the moved subtree.

mod dom;
mod entities;
mod html;
mod ser;
mod xml;
mod xpath;

use crate::core::*;
use crate::tri;
use dom::*;

struct Cls {
    node: *mut Class,
    document: *mut Class,
    html_document: *mut Class,
    element: *mut Class,
    attr: *mut Class,
    text: *mut Class,
    cdata: *mut Class,
    comment: *mut Class,
    pi: *mut Class,
    entity_ref: *mut Class,
    dtd: *mut Class,
    fragment: *mut Class,
    namespace: *mut Class,
    node_set: *mut Class,
    xml_syntax_error: *mut Class,
}
static C: G<Option<Cls>> = G::new(None);
fn c() -> &'static Cls {
    C.get().as_ref().unwrap()
}

struct Syms {
    d: u32,
    i: u32,
    dom: u32,
    w: u32,
    nsw: u32,
    n: u32,
    a: u32,
    decorators: u32,
    decorate: u32,
    document: u32,
    errors: u32,
    initialize: u32,
    node: u32,
    nsh: u32,
    vars: u32,
}
static S: G<Option<Syms>> = G::new(None);
fn sy() -> &'static Syms {
    S.get().as_ref().unwrap()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn rs(v: V) -> String {
    if is_str(v) {
        return String::from_utf8_lossy(&rstr(v).b).into_owned();
    }
    if is_sym(v) {
        return sym_str(sym_id(v));
    }
    match crate::fmt::to_s(v) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(()) => String::new(),
    }
}
fn ostr(s: Option<&str>) -> V {
    match s {
        Some(s) => str_from(s),
        None => QNIL,
    }
}
fn class_le(mut k: *mut Class, base: *mut Class) -> bool {
    unsafe {
        while !k.is_null() {
            if k == base {
                return true;
            }
            k = (*k).superclass;
        }
    }
    false
}
fn is_node(v: V) -> bool {
    is_kind(v, Kind::Object) && class_le(real_class(v), c().node)
}
fn class_named(name: &str) -> Option<*mut Class> {
    let mut cur = rt().c.object;
    for part in name.split("::") {
        let v = const_get_under(cur, sym(part))?;
        if !is_class(v) {
            return None;
        }
        cur = v as *mut Class;
    }
    Some(cur)
}
fn runtime_error(msg: &str) -> V {
    raise(rt().c.runtime_error, msg)
}

unsafe extern "C" fn dom_free(p: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(p as *mut Dom) });
}

fn dom_ptr(doc: V) -> *mut Dom {
    if !is_kind(doc, Kind::Object) {
        return std::ptr::null_mut();
    }
    let d = ivar_get(doc, sy().dom);
    if is_kind(d, Kind::Data) {
        unsafe { (*(d as *mut RData)).data as *mut Dom }
    } else {
        std::ptr::null_mut()
    }
}
fn dom_of(doc: V) -> &'static mut Dom {
    let p = dom_ptr(doc);
    assert!(!p.is_null(), "nokogiri: not a document");
    unsafe { &mut *p }
}

/// (document object, node index) of a node object.
fn node_ref(v: V) -> Result<(V, u32), V> {
    if !is_kind(v, Kind::Object) {
        return Err(type_error(&format!("wrong argument type {} (expected Nokogiri::XML::Node)", class_name(real_class(v)))));
    }
    let d = ivar_get(v, sy().d);
    if dom_ptr(d).is_null() {
        return Err(runtime_error(&format!("uninitialized {}", class_name(real_class(v)))));
    }
    let i = ivar_get(v, sy().i);
    Ok((d, if is_fix(i) { fix(i) as u32 } else { 0 }))
}

macro_rules! this {
    ($v:expr) => {
        match node_ref($v) {
            Ok((d, i)) => (d, i, dom_of(d)),
            Err(e) => return e,
        }
    };
}

/// The Dom of a document receiver (raises for an uninitialized one, e.g. from `allocate`).
macro_rules! sdom {
    ($v:expr) => {{
        let p = dom_ptr($v);
        if p.is_null() {
            return runtime_error("uninitialized document");
        }
        unsafe { &mut *p }
    }};
}

fn new_doc_obj(cls: *mut Class, mut dom: Dom) -> V {
    if class_named("Nokogiri::HTML5::Document").is_some_and(|h5| class_le(cls, h5)) {
        dom.html = true;
    }
    let o = object_new(cls);
    let d = alloc(RData {
        h: Header { klass: rt().c.object, kind: Kind::Data, flags: 0 },
        data: Box::into_raw(Box::new(dom)) as *mut std::ffi::c_void,
        dmark: None,
        dfree: Some(dom_free),
        roots: Vec::new(),
    });
    ivar_set(o, sy().dom, d);
    ivar_set(o, sy().w, ary_new(Vec::new()));
    ivar_set(o, sy().d, o);
    ivar_set(o, sy().i, mkfix(0));
    ivar_set(o, sy().decorators, QNIL);
    o
}

fn is_html5_doc(doc: V) -> bool {
    class_named("Nokogiri::HTML5::Document").is_some_and(|h5| class_le(real_class(doc), h5))
}

fn class_for(dom: &Dom, id: u32) -> *mut Class {
    let k = c();
    match dom.t(id) {
        T::Element => k.element,
        T::Attr => k.attr,
        T::Text => k.text,
        T::CData => k.cdata,
        T::Comment => k.comment,
        T::PI => k.pi,
        T::EntityRef => k.entity_ref,
        T::DocType => k.dtd,
        T::Fragment => k.fragment,
        T::Document => k.document,
    }
}

fn cache(doc: V) -> &'static mut SVec<V> {
    let mut w = ivar_get(doc, sy().w);
    if !is_ary(w) {
        w = ary_new(Vec::new());
        ivar_set(doc, sy().w, w);
    }
    &mut rary(w).v
}
fn cache_get(doc: V, id: u32) -> V {
    cache(doc).get(id as usize).copied().unwrap_or(QNIL)
}
fn cache_set(doc: V, id: u32, w: V) {
    let c = cache(doc);
    if c.len() <= id as usize {
        c.resize(id as usize + 1, QNIL);
    }
    c[id as usize] = w;
}

fn decorate(doc: V, obj: V) -> V {
    if truthy(ivar_get(doc, sy().decorators)) {
        tri!(send(doc, sy().decorate, &[obj], QNIL));
    }
    obj
}

/// The Ruby object for a node (created and cached on first use).
fn wrap(doc: V, id: u32) -> V {
    if id == 0 {
        return doc;
    }
    if id == NONE {
        return QNIL;
    }
    let w = cache_get(doc, id);
    if w != QNIL {
        return w;
    }
    let cls = class_for(dom_of(doc), id);
    wrap_new(doc, id, cls)
}
fn wrap_new(doc: V, id: u32, cls: *mut Class) -> V {
    let o = object_new(cls);
    ivar_set(o, sy().d, doc);
    ivar_set(o, sy().i, mkfix(id as i64));
    cache_set(doc, id, o);
    tri!(decorate(doc, o));
    o
}

fn node_set(doc: V, items: Vec<V>) -> V {
    let o = object_new(c().node_set);
    ivar_set(o, sy().document, doc);
    ivar_set(o, sy().a, ary_new(items));
    tri!(decorate(doc, o));
    o
}
fn node_set_of(doc: V, ids: &[u32]) -> V {
    let arr = ary_new(Vec::with_capacity(ids.len()));
    for &id in ids {
        let w = tri!(wrap(doc, id));
        rary(arr).v.push(w);
    }
    let o = object_new(c().node_set);
    ivar_set(o, sy().document, doc);
    ivar_set(o, sy().a, arr);
    tri!(decorate(doc, o));
    o
}

fn ns_wrap(doc: V, ns: u32) -> V {
    if ns == NONE {
        return QNIL;
    }
    let mut w = ivar_get(doc, sy().nsw);
    if !is_ary(w) {
        w = ary_new(Vec::new());
        ivar_set(doc, sy().nsw, w);
    }
    let v = &mut rary(w).v;
    if let Some(&o) = v.get(ns as usize) {
        if o != QNIL {
            return o;
        }
    }
    let o = object_new(c().namespace);
    ivar_set(o, sy().d, doc);
    ivar_set(o, sy().n, mkfix(ns as i64));
    ivar_set(o, sy().document, doc);
    let v = &mut rary(w).v;
    if v.len() <= ns as usize {
        v.resize(ns as usize + 1, QNIL);
    }
    v[ns as usize] = o;
    o
}

/// Cached classes standing for "class X extended with modules M...": a hidden (singleton-
/// flagged, so `#class` still answers X) subclass including M.
static DECORATED: crate::mt::L<Option<std::collections::HashMap<(usize, Vec<usize>), usize>>> = crate::mt::L::new(None);

fn is_user_method(v: V, name: &str) -> bool {
    let m = find_method(class_of(v), sym(name));
    !m.is_null() && !matches!(unsafe { &(*m).body }, Body::Native(..))
}

/// Document#decorate's `node.extend(mod)` for every decorator module that applies.
fn kenny_decorate(node: V, decorators: V) -> V {
    if !is_hash(decorators) || !is_heap(node) {
        return QNIL;
    }
    let pairs: Vec<(V, V)> = rhash(decorators).m.iter().map(|(k, v)| (k.0, *v)).collect();
    let mut mods: Vec<V> = Vec::new();
    for (k, list) in pairs {
        if is_class(k) && is_a(node, k as *mut Class) && is_ary(list) {
            for &m in rary(list).v.iter() {
                if !mods.contains(&m) {
                    mods.push(m);
                }
            }
        }
    }
    let cur = hdr(node).klass;
    let anc = ancestors(cur);
    mods.retain(|&m| !anc.contains(&(m as *mut Class)));
    if mods.is_empty() {
        return node;
    }
    let own_singleton = unsafe { (*cur).is_singleton && (*cur).attached == node };
    let hooks = mods.iter().any(|&m| !is_class(m) || is_user_method(m, "extended") || is_user_method(m, "extend_object"));
    if own_singleton || hooks || !is_kind(node, Kind::Object) {
        for m in mods {
            tri!(send(node, sym("extend"), &[m], QNIL));
        }
        return node;
    }
    let key = (cur as usize, mods.iter().map(|&m| m as usize).collect::<Vec<_>>());
    let mut g = DECORATED.get();
    let cache = g.get_or_insert_with(std::collections::HashMap::new);
    let k = match cache.get(&key) {
        Some(&k) => k as *mut Class,
        None => {
            let k = new_class_raw("", cur, false);
            unsafe {
                (*k).is_singleton = true;
                (*k).attached = QNIL;
                (*k).h.klass = rt().c.class;
                (*k).alloc = (*cur).alloc;
            }
            for &m in &mods {
                tri!(crate::kernel::do_include(k, m));
            }
            bump_serial();
            cache.insert(key, k as usize);
            k
        }
    };
    hdr(node).klass = k;
    node
}

fn syntax_error(cls: *mut Class, line: u32, col: u32, msg: &str) -> V {
    let e = exc_new(cls, msg);
    ivar_set(e, sym("@level"), mkfix(2));
    ivar_set(e, sym("@line"), mkfix(line as i64));
    ivar_set(e, sym("@column"), mkfix(col as i64));
    ivar_set(e, sym("@domain"), mkfix(1));
    ivar_set(e, sym("@code"), mkfix(1));
    e
}

fn xpath_error(msg: &str) -> V {
    let cls = class_named("Nokogiri::XML::XPath::SyntaxError").unwrap_or(c().xml_syntax_error);
    raise_exc(syntax_error(cls, 0, 0, &format!("ERROR: {msg}")))
}

// ---------------------------------------------------------------------------
// moving nodes between documents
// ---------------------------------------------------------------------------

/// Copy a subtree of `src_doc` into `dst_doc`, unlink the original and re-point its
/// wrappers. Returns the new index.
fn adopt(src_doc: V, id: u32, dst_doc: V) -> V {
    let src = dom_of(src_doc);
    let dst = dom_of(dst_doc);
    let mut map = Vec::new();
    let new = dst.copy_from(src, id, 1, &mut map);
    src.unlink(id);
    let decorated = truthy(ivar_get(dst_doc, sy().decorators));
    let mut moved = Vec::new();
    for (old, nid) in map {
        let w = cache_get(src_doc, old);
        if w != QNIL {
            ivar_set(w, sy().d, dst_doc);
            ivar_set(w, sy().i, mkfix(nid as i64));
            cache_set(dst_doc, nid, w);
            cache(src_doc)[old as usize] = QNIL;
            moved.push(w);
        }
    }
    if decorated {
        for w in moved {
            tri!(send(dst_doc, sy().decorate, &[w], QNIL));
        }
    }
    mkfix(new as i64)
}

/// libxml2/Nokogiri namespace fix-up after reparenting in an XML document.
fn relink(dom: &mut Dom, id: u32, inherit: bool) {
    if dom.t(id) != T::Element {
        return;
    }
    if dom.n(id).ns == NONE {
        let name = dom.n(id).name.clone();
        if let Some((p, l)) = name.split_once(':') {
            if let Some(ns) = dom.lookup_prefix(id, Some(p)) {
                dom.m(id).ns = ns;
                dom.m(id).name = l.to_string();
            }
        } else if inherit {
            let parent = dom.n(id).parent;
            if parent != NONE && dom.t(parent) == T::Element {
                let pns = dom.n(parent).ns;
                if pns != NONE && dom.nss[pns as usize].prefix.is_none() {
                    dom.m(id).ns = pns;
                }
            }
        }
    }
    for a in dom.n(id).attrs.clone() {
        if dom.n(a).ns == NONE {
            let name = dom.n(a).name.clone();
            if let Some((p, l)) = name.split_once(':') {
                if p != "xmlns" {
                    if let Some(ns) = dom.lookup_prefix(id, Some(p)) {
                        dom.m(a).ns = ns;
                        dom.m(a).name = l.to_string();
                    }
                }
            }
        }
    }
    for k in dom.children(id) {
        relink(dom, k, inherit);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum How {
    Child,
    Next,
    Prev,
    Replace,
}

fn reparent(pivot: V, node: V, how: How) -> V {
    if !is_node(node) {
        return arg_error(&format!("node must be a Nokogiri::XML::Node, not {}", class_name(real_class(node))));
    }
    let (pdoc, pid, _) = this!(pivot);
    let (ndoc, mut nid) = match node_ref(node) {
        Ok(x) => x,
        Err(e) => return e,
    };
    if nid == 0 {
        return arg_error("cannot reparent a document node");
    }
    if how != How::Child && dom_of(pdoc).n(pid).parent == NONE && dom_of(pdoc).t(pid) != T::Attr {
        return runtime_error("could not reparent node: pivot has no parent");
    }
    if ndoc != pdoc {
        let r = tri!(adopt(ndoc, nid, pdoc));
        nid = fix(r) as u32;
    } else if pid == nid || dom_of(pdoc).is_ancestor(nid, pid) {
        return arg_error("cannot reparent a node into its own subtree");
    }
    let dom = dom_of(pdoc);
    let mut result = nid;
    let is_text = dom.t(nid) == T::Text;
    match how {
        How::Child => {
            if dom.t(nid) == T::Attr {
                if dom.t(pid) != T::Element {
                    return arg_error("cannot add an attribute to a non-element");
                }
                let (name, ns) = (dom.n(nid).name.clone(), dom.n(nid).ns);
                let existing: Vec<u32> = dom.n(pid).attrs.iter().copied().filter(|&a| a != nid && dom.n(a).name == name && dom.n(a).ns == ns).collect();
                for a in existing {
                    dom.unlink(a);
                }
                dom.append(pid, nid);
            } else if is_text && dom.t(pid) == T::Text {
                let s = dom.n(nid).content.clone();
                dom.m(pid).content.push_str(&s);
                dom.unlink(nid);
                result = pid;
            } else if is_text && dom.n(pid).last != NONE && dom.n(pid).last != nid && dom.t(dom.n(pid).last) == T::Text {
                let last = dom.n(pid).last;
                let s = dom.n(nid).content.clone();
                dom.m(last).content.push_str(&s);
                dom.unlink(nid);
                result = last;
            } else {
                dom.append(pid, nid);
            }
        }
        How::Next => {
            if is_text && dom.t(pid) == T::Text {
                let s = dom.n(nid).content.clone();
                dom.m(pid).content.push_str(&s);
                dom.unlink(nid);
                result = pid;
            } else {
                dom.insert_after(pid, nid);
            }
        }
        How::Prev => {
            if is_text && dom.t(pid) == T::Text {
                let s = dom.n(nid).content.clone() + &dom.n(pid).content;
                dom.m(pid).content = s;
                dom.unlink(nid);
                result = pid;
            } else {
                dom.insert_before(pid, nid);
            }
        }
        How::Replace => {
            dom.insert_before(pid, nid);
            dom.unlink(pid);
        }
    }
    if !dom.html {
        let inherit = truthy(ivar_get(pdoc, sym("@namespace_inheritance")));
        relink(dom, result, inherit);
    }
    if result != nid {
        // merged into an adjacent text node: like Nokogiri, the node object now stands for it
        ivar_set(node, sy().d, pdoc);
        ivar_set(node, sy().i, mkfix(result as i64));
        cache_set(pdoc, result, node);
        return node;
    }
    let w = tri!(wrap(pdoc, result));
    if truthy(ivar_get(pdoc, sy().decorators)) {
        tri!(send(pdoc, sy().decorate, &[w], QNIL));
    }
    w
}

// ---------------------------------------------------------------------------
// parsing entry points
// ---------------------------------------------------------------------------

fn input_string(v: V) -> Result<String, V> {
    if is_str(v) {
        return Ok(rs(v));
    }
    if respond_to(v, sym("read")) {
        // like libxml2's IO callbacks: read in chunks until EOF
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let r = send(v, sym("read"), &[mkfix(4096)], QNIL);
            if r == UNDEF {
                return Err(UNDEF);
            }
            if !is_str(r) || rstr(r).b.is_empty() {
                break;
            }
            buf.extend_from_slice(&rstr(r).b);
        }
        return Ok(String::from_utf8_lossy(&buf).into_owned());
    }
    if respond_to(v, sym("to_str")) {
        let r = send(v, sym("to_str"), &[], QNIL);
        if r == UNDEF {
            return Err(UNDEF);
        }
        return Ok(rs(r));
    }
    Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
}

fn errors_ary(errs: &[(u32, u32, String)]) -> V {
    let out = ary_new(Vec::new());
    for (l, col, m) in errs {
        let e = syntax_error(c().xml_syntax_error, *l, *col, m);
        rary(out).v.push(e);
    }
    out
}

fn xml_read(cls: V, a: &[V]) -> V {
    let src = match input_string(a[0]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let opts = a.get(3).copied().filter(|v| is_fix(*v)).map(fix).unwrap_or(1);
    let parsed = xml::parse_document(&src);
    let mut dom = parsed.dom;
    if let Some(e) = a.get(2).filter(|v| truthy(**v)) {
        dom.encoding = Some(rs(*e));
    }
    if let Some(u) = a.get(1).filter(|v| truthy(**v)) {
        dom.url = Some(rs(*u));
    }
    if !parsed.errors.is_empty() && opts & 1 == 0 {
        let (l, col, m) = &parsed.errors[0];
        return raise_exc(syntax_error(c().xml_syntax_error, *l, *col, m));
    }
    let doc = new_doc_obj(cls as *mut Class, dom);
    tri!(send(doc, sy().initialize, &[], QNIL));
    ivar_set(doc, sy().errors, errors_ary(&parsed.errors));
    doc
}

fn html_read(cls: V, a: &[V]) -> V {
    let src = match input_string(a[0]) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let parsed = html::parse_document(&src, true);
    let mut dom = parsed.dom;
    if let Some(e) = a.get(2).filter(|v| truthy(**v)) {
        dom.encoding = Some(rs(*e));
    }
    if let Some(u) = a.get(1).filter(|v| truthy(**v)) {
        dom.url = Some(rs(*u));
    }
    let doc = new_doc_obj(cls as *mut Class, dom);
    tri!(send(doc, sy().initialize, &[], QNIL));
    doc
}

fn gumbo_parse(a: &[V]) -> V {
    let src = rs(a[0]);
    let cls = if a.len() > 5 && is_class(a[5]) { a[5] } else { class_named("Nokogiri::HTML5::Document").map(|c| c as V).unwrap_or(c().html_document as V) };
    let parsed = html::parse_document(&src, false);
    let mut dom = parsed.dom;
    dom.html = true;
    let doc = new_doc_obj(cls as *mut Class, dom);
    tri!(send(doc, sy().initialize, &[], QNIL));
    ivar_set(doc, sym("@url"), if a.len() > 1 { a[1] } else { QNIL });
    ivar_set(doc, sym("@quirks_mode"), mkfix(parsed.quirks));
    let max_errors = a.get(3).filter(|v| is_fix(**v)).map(|v| fix(*v)).unwrap_or(0);
    let errs: Vec<(u32, u32, String)> = if max_errors != 0 {
        parsed.errors.iter().take(if max_errors < 0 { usize::MAX } else { max_errors as usize }).map(|m| (1, 1, m.clone())).collect()
    } else {
        Vec::new()
    };
    ivar_set(doc, sy().errors, errors_ary(&errs));
    doc
}

/// Copy parsed top-level nodes from a scratch dom into `doc`, as unparented nodes.
fn import_nodes(doc: V, src: &Dom, ids: &[u32]) -> Vec<u32> {
    let dst = dom_of(doc);
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let mut map = Vec::new();
        out.push(dst.copy_from(src, id, 1, &mut map));
    }
    out
}

fn gumbo_fragment(a: &[V]) -> V {
    let frag = a[0];
    let (doc, fid, _) = this!(frag);
    let src = rs(a[1]);
    let ctx = if a.len() > 2 { a[2] } else { QNIL };
    let (name, nsh) = if ctx == QNIL {
        ("body".to_string(), html::HTML_NS.to_string())
    } else if is_str(ctx) {
        let s = rs(ctx);
        match s.split_once(' ') {
            Some(("svg", n)) => (n.to_string(), html::SVG_NS.to_string()),
            Some(("math", n)) => (n.to_string(), html::MATHML_NS.to_string()),
            _ => (s, html::HTML_NS.to_string()),
        }
    } else {
        let (_, cid, cdom) = this!(ctx);
        let n = cdom.n(cid);
        if n.t == T::Element {
            let h = if n.ns == NONE { html::HTML_NS.to_string() } else { cdom.nss[n.ns as usize].href.clone() };
            (n.name.clone(), h)
        } else {
            ("body".to_string(), html::HTML_NS.to_string())
        }
    };
    let (sdom, kids, errors) = html::parse_fragment(&src, &name, &nsh, false);
    let ids = import_nodes(doc, &sdom, &kids);
    let dom = dom_of(doc);
    for id in ids {
        dom.append(fid, id);
    }
    let max_errors = a.get(4).filter(|v| is_fix(**v)).map(|v| fix(*v)).unwrap_or(0);
    let errs: Vec<(u32, u32, String)> = if max_errors != 0 { errors.iter().map(|m| (1, 1, m.clone())).collect() } else { Vec::new() };
    ivar_set(frag, sy().errors, errors_ary(&errs));
    ivar_set(frag, sym("@quirks_mode"), QNIL);
    frag
}

/// Node#in_context: parse markup in the context of this node; returns a NodeSet of new,
/// unparented nodes.
fn in_context(node: V, a: &[V]) -> V {
    let (doc, id, dom) = this!(node);
    let src = rs(a[0]);
    if dom.html {
        let n = dom.n(id);
        let html4 = !is_html5_doc(doc);
        let (name, nsh) = if n.t == T::Element {
            let table_part = matches!(n.name.as_str(), "table" | "tbody" | "thead" | "tfoot" | "tr" | "colgroup" | "select");
            if html4 && n.ns == NONE && !table_part {
                ("body".to_string(), html::HTML_NS.to_string())
            } else {
                (n.name.clone(), if n.ns == NONE { html::HTML_NS.to_string() } else { dom.nss[n.ns as usize].href.clone() })
            }
        } else {
            ("body".to_string(), html::HTML_NS.to_string())
        };
        let (sdom, kids, _errors) = html::parse_fragment(&src, &name, &nsh, html4);
        let ids = import_nodes(doc, &sdom, &kids);
        return node_set_of(doc, &ids);
    }
    // XML: parse under a scratch element carrying the namespaces in scope
    let ctx_el = if dom.t(id) == T::Element { id } else { dom.root() };
    let scratch = dom.new_node(T::Element, "__kenny_scratch__", "");
    if ctx_el != NONE {
        let scopes = dom.ns_scopes(ctx_el);
        dom.m(scratch).ns_defs = scopes;
    }
    let errors = xml::parse_into(dom, &src, scratch, true);
    let kids = dom.children(scratch);
    for &k in &kids {
        dom.unlink(k);
    }
    if !errors.is_empty() {
        let errs = ivar_get(doc, sy().errors);
        if is_ary(errs) {
            let e = errors_ary(&errors);
            let ev = rary(e).v.clone();
            rary(errs).v.extend(ev);
        }
    }
    node_set_of(doc, &kids)
}

// ---------------------------------------------------------------------------
// XPath
// ---------------------------------------------------------------------------

struct RubyHost {
    handler: V,
    doc: V,
    raised: bool,
}

impl RubyHost {
    fn to_ruby(&self, v: &xpath::Val) -> V {
        match v {
            xpath::Val::Nodes(ids) => node_set_of(self.doc, ids),
            xpath::Val::Str(s) => str_from(s),
            xpath::Val::Num(n) => float_new(*n),
            xpath::Val::Bool(b) => bool_v(*b),
        }
    }
}

fn ruby_to_val(doc: V, r: V) -> Result<xpath::Val, String> {
    let nodes_of = |items: &[V]| -> xpath::Val {
        let mut ids = Vec::new();
        for &x in items {
            if is_node(x) {
                if let Ok((d, i)) = node_ref(x) {
                    if d == doc {
                        ids.push(i);
                    }
                }
            }
        }
        xpath::Val::Nodes(ids)
    };
    if r == QNIL {
        return Ok(xpath::Val::Nodes(vec![]));
    }
    if r == QTRUE || r == QFALSE {
        return Ok(xpath::Val::Bool(r == QTRUE));
    }
    if is_fix(r) {
        return Ok(xpath::Val::Num(fix(r) as f64));
    }
    if is_float(r) {
        return Ok(xpath::Val::Num(float_val(r)));
    }
    if is_str(r) {
        return Ok(xpath::Val::Str(rs(r)));
    }
    if is_ary(r) {
        return Ok(nodes_of(&rary(r).v.clone()));
    }
    if is_kind(r, Kind::Object) && class_le(real_class(r), c().node_set) {
        let a = ivar_get(r, sy().a);
        return Ok(if is_ary(a) { nodes_of(&rary(a).v.clone()) } else { xpath::Val::Nodes(vec![]) });
    }
    if is_node(r) {
        return Ok(nodes_of(&[r]));
    }
    Err(format!("Invalid return type {}", class_name(real_class(r))))
}

impl xpath::Host for RubyHost {
    fn call(&mut self, name: &str, args: Vec<xpath::Val>) -> Result<xpath::Val, String> {
        if !respond_to(self.handler, sym(name)) && !respond_to(self.handler, sym("method_missing")) {
            return Err(format!("Unregistered function: {name}"));
        }
        let ra = ary_new(Vec::with_capacity(args.len()));
        for a in &args {
            let v = self.to_ruby(a);
            if v == UNDEF {
                self.raised = true;
                return Err("raised".into());
            }
            rary(ra).v.push(v);
        }
        let argv = rary(ra).v.clone();
        let r = send(self.handler, sym(name), &argv, QNIL);
        if r == UNDEF {
            self.raised = true;
            return Err("raised".into());
        }
        ruby_to_val(self.doc, r)
    }
}

fn xpath_evaluate(ctx: V, a: &[V]) -> V {
    let node = ivar_get(ctx, sy().node);
    let (doc, id, dom) = this!(node);
    let expr_s = rs(a[0]);
    let expr = match xpath::parse_cached(&expr_s) {
        Ok(e) => e,
        Err(m) => return xpath_error(&format!("{m}: {expr_s}")),
    };
    let handler = a.get(1).copied().unwrap_or(QNIL);
    let mut host = RubyHost { handler, doc, raised: false };
    let mut ev = xpath::Eval::new(dom);
    let nsh = ivar_get(ctx, sy().nsh);
    if is_hash(nsh) {
        for (k, v) in rhash(nsh).m.iter() {
            ev.ns.insert(rs(k.0), rs(*v));
        }
    }
    let vars = ivar_get(ctx, sy().vars);
    if is_hash(vars) {
        let pairs: Vec<(V, V)> = rhash(vars).m.iter().map(|(k, v)| (k.0, *v)).collect();
        for (k, v) in pairs {
            match ruby_to_val(doc, v) {
                Ok(x) => {
                    ev.vars.insert(rs(k), x);
                }
                Err(m) => return xpath_error(&m),
            }
        }
    }
    if handler != QNIL {
        ev.host = Some(&mut host);
    }
    let r = ev.evaluate(&expr, id);
    drop(ev);
    if host.raised {
        return UNDEF;
    }
    match r {
        Err(m) => xpath_error(&m),
        Ok(xpath::Val::Nodes(ids)) => node_set_of(doc, &ids),
        Ok(xpath::Val::Str(s)) => str_from(&s),
        Ok(xpath::Val::Num(n)) => float_new(n),
        Ok(xpath::Val::Bool(b)) => bool_v(b),
    }
}

// ---------------------------------------------------------------------------
// NodeSet
// ---------------------------------------------------------------------------

fn set_items(set: V) -> &'static mut SVec<V> {
    let mut a = ivar_get(set, sy().a);
    if !is_ary(a) {
        a = ary_new(Vec::new());
        ivar_set(set, sy().a, a);
    }
    &mut rary(a).v
}
fn set_doc(set: V) -> V {
    ivar_get(set, sy().document)
}
fn want_set(v: V) -> Result<&'static mut SVec<V>, V> {
    if is_kind(v, Kind::Object) && class_le(real_class(v), c().node_set) {
        Ok(set_items(v))
    } else {
        Err(arg_error("node_set must be a Nokogiri::XML::NodeSet"))
    }
}
fn is_ns_obj(v: V) -> bool {
    is_kind(v, Kind::Object) && class_le(real_class(v), c().namespace)
}

fn init_node_set(k: *mut Class) {
    defn(k, "length", 0, 0, |s, _, _| mkfix(set_items(s).len() as i64));
    defn(k, "push", 1, 1, |s, a, _| {
        if !is_node(a[0]) && !is_ns_obj(a[0]) {
            return arg_error("node must be a Nokogiri::XML::Node or Nokogiri::XML::Namespace");
        }
        let v = set_items(s);
        if !v.contains(&a[0]) {
            v.push(a[0]);
        }
        s
    });
    fn aref(s: V, a: &[V]) -> V {
        if a.len() == 1 && is_fix(a[0]) {
            // the hot path (NodeSet#each indexes one by one)
            let v = set_items(s);
            let i = fix(a[0]);
            let i = if i < 0 { i + v.len() as i64 } else { i };
            return if i >= 0 && (i as usize) < v.len() { v[i as usize] } else { QNIL };
        }
        let arr = ary_new(set_items(s).clone());
        let r = tri!(send(arr, sym("[]"), a, QNIL));
        if is_ary(r) {
            return node_set(set_doc(s), rary(r).v.clone());
        }
        r
    }
    defn(k, "[]", 1, 2, |s, a, _| aref(s, a));
    defn(k, "slice", 1, 2, |s, a, _| aref(s, a));
    defn(k, "to_a", 0, 0, |s, _, _| ary_new(set_items(s).clone()));
    defn(k, "delete", 1, 1, |s, a, _| {
        if !is_node(a[0]) && !is_ns_obj(a[0]) {
            return arg_error("node must be a Nokogiri::XML::Node or Nokogiri::XML::Namespace");
        }
        let v = set_items(s);
        match v.iter().position(|x| *x == a[0]) {
            Some(i) => {
                v.remove(i);
                a[0]
            }
            None => QNIL,
        }
    });
    defn(k, "&", 1, 1, |s, a, _| {
        let o = match want_set(a[0]) {
            Ok(o) => o.clone(),
            Err(e) => return e,
        };
        let r: Vec<V> = set_items(s).iter().copied().filter(|x| o.contains(x)).collect();
        node_set(set_doc(s), r)
    });
    defn(k, "|", 1, 1, |s, a, _| {
        let o = match want_set(a[0]) {
            Ok(o) => o.clone(),
            Err(e) => return e,
        };
        let mut r = set_items(s).clone();
        for x in o {
            if !r.contains(&x) {
                r.push(x);
            }
        }
        node_set(set_doc(s), r)
    });
    defn(k, "-", 1, 1, |s, a, _| {
        let o = match want_set(a[0]) {
            Ok(o) => o.clone(),
            Err(e) => return e,
        };
        let r: Vec<V> = set_items(s).iter().copied().filter(|x| !o.contains(x)).collect();
        node_set(set_doc(s), r)
    });
    defn(k, "include?", 1, 1, |s, a, _| bool_v(set_items(s).contains(&a[0])));
    defn(k, "unlink", 0, 0, |s, _, _| {
        for x in set_items(s).clone() {
            if is_node(x) {
                if let Ok((d, i)) = node_ref(x) {
                    if i != 0 {
                        dom_of(d).unlink(i);
                    }
                }
            }
        }
        s
    });
    defn(k, "dup", 0, 1, |s, a, _| {
        let doc = a.first().copied().unwrap_or_else(|| set_doc(s));
        node_set(doc, set_items(s).clone())
    });
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

fn type_code(dom: &Dom, id: u32) -> i64 {
    match dom.t(id) {
        T::Element => 1,
        T::Attr => 2,
        T::Text => 3,
        T::CData => 4,
        T::EntityRef => 5,
        T::PI => 7,
        T::Comment => 8,
        T::Document => {
            if dom.html { 13 } else { 9 }
        }
        T::DocType => 14,
        T::Fragment => 11,
    }
}

fn node_name(dom: &Dom, id: u32) -> String {
    let n = dom.n(id);
    match n.t {
        T::Text => "text".into(),
        T::Comment => "comment".into(),
        T::CData => "#cdata-section".into(),
        T::Document => "document".into(),
        T::Fragment => "#document-fragment".into(),
        _ => n.name.clone(),
    }
}

/// libxml2's xmlGetNodePath.
fn node_path(dom: &Dom, id: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = id;
    loop {
        let n = dom.n(cur);
        match n.t {
            T::Document => break,
            T::Attr => parts.push(format!("@{}", dom.qname(cur))),
            _ => {
                let parent = n.parent;
                let (label, same): (String, Box<dyn Fn(u32) -> bool>) = match n.t {
                    T::Element => {
                        let q = dom.qname(cur);
                        let qq = q.clone();
                        (q, Box::new(move |x| dom.t(x) == T::Element && dom.qname(x) == qq))
                    }
                    T::Text | T::CData => ("text()".into(), Box::new(|x| matches!(dom.t(x), T::Text | T::CData))),
                    T::Comment => ("comment()".into(), Box::new(|x| dom.t(x) == T::Comment)),
                    T::PI => (format!("processing-instruction('{}')", n.name), Box::new(|x| dom.t(x) == T::PI)),
                    _ => (node_name(dom, cur), Box::new(|_| false)),
                };
                if parent == NONE || dom.t(parent) == T::Fragment {
                    parts.push(label);
                    if parent == NONE {
                        break;
                    }
                    cur = parent;
                    continue;
                }
                let sibs = dom.children(parent);
                let matching: Vec<u32> = sibs.into_iter().filter(|&x| same(x)).collect();
                if matching.len() > 1 {
                    let idx = matching.iter().position(|&x| x == cur).unwrap_or(0) + 1;
                    parts.push(format!("{label}[{idx}]"));
                } else {
                    parts.push(label);
                }
            }
        }
        let p = dom.n(cur).parent;
        if p == NONE {
            break;
        }
        cur = p;
    }
    parts.reverse();
    let mut root = id;
    while dom.n(root).parent != NONE {
        root = dom.n(root).parent;
    }
    if root == 0 { format!("/{}", parts.join("/")) } else { parts.join("/") }
}

fn encode_special(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

/// Entity and character references in a string (as libxml2's xmlStringGetNodeList).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(';').filter(|&e| e > 1 && e < 40);
        let decoded = end.and_then(|e| {
            let name = &tail[1..e];
            let ch = if let Some(num) = name.strip_prefix('#') {
                let cp = if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    num.parse::<u32>().ok()
                };
                cp.and_then(char::from_u32).map(|c| c.to_string())
            } else {
                match name {
                    "lt" => Some("<".to_string()),
                    "gt" => Some(">".to_string()),
                    "amp" => Some("&".to_string()),
                    "quot" => Some("\"".to_string()),
                    "apos" => Some("'".to_string()),
                    _ => None,
                }
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push_str(&c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn kids_where(doc: V, dom: &Dom, id: u32, f: impl Fn(&Dom, u32) -> bool) -> V {
    let ids: Vec<u32> = if dom.t(id) == T::Attr { Vec::new() } else { dom.children(id).into_iter().filter(|&x| f(dom, x)).collect() };
    node_set_of(doc, &ids)
}

fn sibling(dom: &Dom, id: u32, next: bool, element: bool) -> u32 {
    if dom.t(id) == T::Attr {
        return NONE;
    }
    let mut x = if next { dom.n(id).next } else { dom.n(id).prev };
    while x != NONE && element && dom.t(x) != T::Element {
        x = if next { dom.n(x).next } else { dom.n(x).prev };
    }
    x
}

fn opt_doc_arg(v: V) -> Result<V, V> {
    if dom_ptr(v).is_null() {
        if is_node(v) {
            // a node stands for its document (deprecated in Nokogiri, still accepted)
            return node_ref(v).map(|(d, _)| d);
        }
        return Err(arg_error(&format!("document must be a Nokogiri::XML::Document, not {}", class_name(real_class(v)))));
    }
    Ok(v)
}

/// Shared tail of the node constructors: wrap with the receiver class, call initialize,
/// yield to the block.
fn construct(cls: V, doc: V, id: u32, args: &[V], blk: V) -> V {
    let o = tri!(wrap_new(doc, id, cls as *mut Class));
    tri!(send(o, sy().initialize, args, QNIL));
    if is_proc(blk) {
        tri!(call_block(blk, &[o]));
    }
    o
}

fn init_node(k: *mut Class) {
    defs(k, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[1]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let dom = dom_of(doc);
        let name = rs(a[0]);
        let id = dom.new_node(T::Element, &name, "");
        construct(cls, doc, id, a, blk)
    });
    defn(k, "node_name", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        str_from(&node_name(dom, id))
    });
    defn(k, "node_name=", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        dom.m(id).name = rs(a[0]);
        a[0]
    });
    defn(k, "node_type", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        mkfix(type_code(dom, id))
    });
    defn(k, "content", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        str_from(&dom.content(id))
    });
    defn(k, "native_content=", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        let v = rs(a[0]);
        if matches!(dom.t(id), T::Element | T::Document | T::Fragment) {
            dom.set_content(id, &decode_entities(&v));
        } else {
            dom.set_content(id, &v);
        }
        a[0]
    });
    defn(k, "encode_special_chars", 1, 1, |_, a, _| str_from(&encode_special(&rs(a[0]))));
    defn(k, "path", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        if id == 0 {
            return str_from("/");
        }
        str_from(&node_path(dom, id))
    });
    defn(k, "pointer_id", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        mkfix(((dom.serial & 0x3fff_ffff) << 32 | id as u64) as i64)
    });
    defn(k, "line", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        mkfix(dom.n(id).line as i64)
    });
    defn(k, "line=", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        if is_fix(a[0]) {
            dom.m(id).line = fix(a[0]).max(0) as u32;
        }
        a[0]
    });
    defn(k, "document", 0, 0, |s, _, _| {
        let (doc, _, _) = this!(s);
        doc
    });
    defn(k, "parent", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        wrap(doc, dom.n(id).parent)
    });
    defn(k, "child", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        if dom.t(id) == T::Attr {
            return QNIL;
        }
        wrap(doc, dom.n(id).first)
    });
    defn(k, "children", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        kids_where(doc, dom, id, |_, _| true)
    });
    defn(k, "element_children", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        kids_where(doc, dom, id, |d, x| d.t(x) == T::Element)
    });
    defn(k, "first_element_child", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        let f = dom.children(id).into_iter().find(|&x| dom.t(x) == T::Element).unwrap_or(NONE);
        wrap(doc, f)
    });
    defn(k, "last_element_child", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        let f = dom.children(id).into_iter().rev().find(|&x| dom.t(x) == T::Element).unwrap_or(NONE);
        wrap(doc, f)
    });
    defn(k, "next_sibling", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        wrap(doc, sibling(dom, id, true, false))
    });
    defn(k, "previous_sibling", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        wrap(doc, sibling(dom, id, false, false))
    });
    defn(k, "next_element", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        wrap(doc, sibling(dom, id, true, true))
    });
    defn(k, "previous_element", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        wrap(doc, sibling(dom, id, false, true))
    });
    defn(k, "attribute_nodes", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        let ids = if dom.t(id) == T::Element { dom.n(id).attrs.clone() } else { Vec::new() };
        let out = ary_new(Vec::with_capacity(ids.len()));
        for a in ids {
            let w = tri!(wrap(doc, a));
            rary(out).v.push(w);
        }
        out
    });
    defn(k, "attribute", 1, 1, |s, a, _| {
        let (doc, id, dom) = this!(s);
        if dom.t(id) != T::Element {
            return QNIL;
        }
        let name = rs(a[0]);
        let mut at = NONE;
        for &x in &dom.n(id).attrs {
            if dom.n(x).name == name {
                at = x;
                break;
            }
        }
        if at == NONE {
            at = dom.attr_node(id, &name);
        }
        wrap(doc, at)
    });
    defn(k, "attribute_with_ns", 2, 2, |s, a, _| {
        let (doc, id, dom) = this!(s);
        if dom.t(id) != T::Element {
            return QNIL;
        }
        let href = if a[1] == QNIL { None } else { Some(rs(a[1])) };
        wrap(doc, dom.attr_node_ns(id, &rs(a[0]), href.as_deref()))
    });
    defn(k, "key?", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        bool_v(dom.t(id) == T::Element && dom.attr_node(id, &rs(a[0])) != NONE)
    });
    defn(k, "namespaced_key?", 2, 2, |s, a, _| {
        let (_, id, dom) = this!(s);
        let href = if a[1] == QNIL { None } else { Some(rs(a[1])) };
        bool_v(dom.t(id) == T::Element && dom.attr_node_ns(id, &rs(a[0]), href.as_deref()) != NONE)
    });
    defn(k, "get", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        if dom.t(id) != T::Element || a[0] == QNIL {
            return QNIL;
        }
        let name = rs(a[0]);
        let mut at = dom.attr_node(id, &name);
        if at == NONE {
            if let Some((p, l)) = name.split_once(':') {
                if let Some(ns) = dom.lookup_prefix(id, Some(p)) {
                    let href = dom.nss[ns as usize].href.clone();
                    at = dom.attr_node_ns(id, l, Some(&href));
                }
            }
        }
        if at == NONE { QNIL } else { str_from(&dom.n(at).content) }
    });
    defn(k, "set", 2, 2, |s, a, _| {
        let (_, id, dom) = this!(s);
        if dom.t(id) != T::Element {
            return QNIL;
        }
        dom.set_attr(id, &rs(a[0]), &rs(a[1]));
        a[1]
    });
    defn(k, "blank?", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        let n = dom.n(id);
        bool_v(matches!(n.t, T::Text | T::CData) && n.content.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')))
    });
    defn(k, "dup", 0, 2, |s, a, _| {
        let (doc, id, dom) = this!(s);
        let level = a.first().filter(|v| is_fix(**v)).map(|v| fix(*v)).unwrap_or(1);
        let target = match a.get(1) {
            Some(&t) if t != QNIL => match opt_doc_arg(t) {
                Ok(d) => d,
                Err(e) => return e,
            },
            _ => doc,
        };
        let new = if target == doc {
            let mut map = Vec::new();
            dom.copy_local(id, level, &mut map)
        } else {
            let mut map = Vec::new();
            dom_of(target).copy_from(dom, id, level, &mut map)
        };
        wrap_new(target, new, real_class(s))
    });
    defn(k, "unlink", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        if id != 0 {
            dom.unlink(id);
        }
        s
    });
    defn(k, "add_child_node", 1, 1, |s, a, _| reparent(s, a[0], How::Child));
    defn(k, "add_next_sibling_node", 1, 1, |s, a, _| reparent(s, a[0], How::Next));
    defn(k, "add_previous_sibling_node", 1, 1, |s, a, _| reparent(s, a[0], How::Prev));
    defn(k, "replace_node", 1, 1, |s, a, _| reparent(s, a[0], How::Replace));
    defn(k, "in_context", 2, 2, |s, a, _| in_context(s, a));
    defn(k, "compare", 1, 1, |s, a, _| {
        let (doc, id, dom) = this!(s);
        let (odoc, oid) = match node_ref(a[0]) {
            Ok(x) => x,
            Err(e) => return e,
        };
        if odoc != doc {
            return mkfix(-2);
        }
        if id == oid {
            return mkfix(0);
        }
        let keys = dom.order_all();
        mkfix(if keys[id as usize] < keys[oid as usize] { -1 } else { 1 })
    });
    defn(k, "native_write_to", 4, 4, |s, a, _| {
        let (_, id, dom) = this!(s);
        let enc = if a[1] == QNIL { None } else { Some(rs(a[1])) };
        let indent = rs(a[2]);
        let opts = if is_fix(a[3]) { fix(a[3]) } else { 0 };
        let html_mode = opts & ser::AS_XHTML == 0 && ((dom.html && opts & ser::AS_XML == 0) || opts & ser::AS_HTML != 0);
        if html_mode && dom.html {
            if let Some(e) = &enc {
                ser::set_meta_encoding(dom, e);
            }
        }
        let out = ser::serialize(dom, id, opts, &indent, enc.as_deref());
        tri!(send(a[0], sym("write"), &[str_from(&out)], QNIL));
        a[0]
    });
    defn(k, "dump_html", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        str_from(&ser::serialize(dom, id, ser::FORMAT | ser::AS_HTML, "", None))
    });
    defn(k, "prepend_newline?", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        let n = dom.n(id);
        let f = n.first;
        bool_v(
            n.t == T::Element
                && n.ns == NONE
                && matches!(n.name.as_str(), "pre" | "textarea" | "listing")
                && f != NONE
                && dom.t(f) == T::Text
                && dom.n(f).content.starts_with('\n'),
        )
    });
    defn(k, "html_standard_serialize", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        str_from(&ser::html5(dom, id, truthy(a[0])))
    });
    defn(k, "namespace", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        if !matches!(dom.t(id), T::Element | T::Attr) {
            return QNIL;
        }
        ns_wrap(doc, dom.n(id).ns)
    });
    defn(k, "namespace_definitions", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        let v: Vec<V> = if dom.t(id) == T::Element { dom.n(id).ns_defs.iter().map(|&n| ns_wrap(doc, n)).collect() } else { Vec::new() };
        ary_new(v)
    });
    defn(k, "namespace_scopes", 0, 0, |s, _, _| {
        let (doc, id, dom) = this!(s);
        let el = if dom.t(id) == T::Element { id } else if dom.t(id) == T::Attr { dom.n(id).parent } else { NONE };
        let mut v: Vec<V> = Vec::new();
        if el != NONE {
            for n in dom.ns_scopes(el) {
                v.push(ns_wrap(doc, n));
            }
            // the implicit xml namespace
        }
        ary_new(v)
    });
    defn(k, "add_namespace_definition", 2, 2, |s, a, _| {
        let (doc, id, dom) = this!(s);
        let prefix = if a[0] == QNIL { None } else { Some(rs(a[0])) };
        let href = rs(a[1]);
        let el = if dom.t(id) == T::Element { id } else { return QNIL };
        let existing = dom.n(el).ns_defs.iter().copied().find(|&n| dom.nss[n as usize].prefix == prefix);
        let ns = match existing {
            Some(n) if dom.nss[n as usize].href == href => n,
            Some(n) => {
                let new = dom.intern_ns(prefix.as_deref(), &href);
                let defs = &mut dom.m(el).ns_defs;
                if let Some(slot) = defs.iter_mut().find(|x| **x == n) {
                    *slot = new;
                }
                new
            }
            None => {
                let new = dom.intern_ns(prefix.as_deref(), &href);
                dom.m(el).ns_defs.push(new);
                new
            }
        };
        if prefix.is_none() {
            dom.m(el).ns = ns;
        } else if dom.n(el).ns == NONE {
            // an element named "prefix:local" picks up its namespace
            let name = dom.n(el).name.clone();
            if let Some((p, l)) = name.split_once(':') {
                if Some(p) == prefix.as_deref() {
                    dom.m(el).ns = ns;
                    dom.m(el).name = l.to_string();
                }
            }
        }
        ns_wrap(doc, ns)
    });
    defn(k, "set_namespace", 1, 1, |s, a, _| {
        let (doc, id, dom) = this!(s);
        if a[0] == QNIL {
            dom.m(id).ns = NONE;
            return s;
        }
        if !is_ns_obj(a[0]) {
            return type_error("expected a Nokogiri::XML::Namespace");
        }
        let nd = ivar_get(a[0], sy().d);
        let n = ivar_get(a[0], sy().n);
        if nd != doc || !is_fix(n) {
            return arg_error("namespace must be declared on the same document");
        }
        dom.m(id).ns = fix(n) as u32;
        s
    });
    defn(k, "lang", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        let mut x = id;
        while x != NONE {
            if dom.t(x) == T::Element {
                let a = dom.attr_node_ns(x, "lang", Some(XML_NS));
                let a = if a == NONE { dom.attr_node(x, "xml:lang") } else { a };
                if a != NONE {
                    return str_from(&dom.n(a).content);
                }
            }
            x = dom.n(x).parent;
        }
        QNIL
    });
    defn(k, "lang=", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        if dom.t(id) == T::Element {
            if dom.html {
                dom.set_attr(id, "lang", &rs(a[0]));
            } else {
                let ns = dom.intern_ns(Some("xml"), XML_NS);
                let at = dom.attr_node_ns(id, "lang", Some(XML_NS));
                if at != NONE {
                    dom.m(at).content = rs(a[0]);
                } else {
                    let mut n = Node::new(T::Attr, "lang", &rs(a[0]));
                    n.ns = ns;
                    let aid = dom.add(n);
                    dom.append(id, aid);
                }
            }
        }
        a[0]
    });
    defn(k, "internal_subset", 0, 0, |s, _, _| {
        let (doc, _, dom) = this!(s);
        let d = dom.children(0).into_iter().find(|&x| dom.t(x) == T::DocType).unwrap_or(NONE);
        wrap(doc, d)
    });
    defn(k, "external_subset", 0, 0, |_, _, _| QNIL);
    defn(k, "create_internal_subset", 3, 3, |s, a, _| {
        let (doc, _, dom) = this!(s);
        if dom.children(0).into_iter().any(|x| dom.t(x) == T::DocType) {
            return runtime_error("Document already has an internal subset");
        }
        let mut n = Node::new(T::DocType, &rs(a[0]), "");
        let o = |v: V| if v == QNIL { None } else { Some(rs(v)) };
        n.ids = Some(Box::new((o(a[1]), o(a[2]))));
        let id = dom.add(n);
        let first = dom.n(0).first;
        if first == NONE {
            dom.append(0, id);
        } else {
            dom.insert_before(first, id);
        }
        wrap(doc, id)
    });
    defn(k, "create_external_subset", 3, 3, |_, _, _| raise(rt().c.not_implemented_error, "external subsets are not supported"));
    defn(k, "process_xincludes", 1, 1, |s, _, _| s);
}

fn init_document(k: *mut Class, html_doc: *mut Class) {
    defs(k, "new", 0, -1, |cls, a, blk| {
        let mut dom = Dom::new(false);
        dom.version = Some(if let Some(&v) = a.first().filter(|v| **v != QNIL) { rs(v) } else { "1.0".into() });
        let doc = new_doc_obj(cls as *mut Class, dom);
        tri!(send(doc, sy().initialize, a, blk));
        doc
    });
    defs(k, "read_memory", 4, 4, |cls, a, _| xml_read(cls, a));
    defs(k, "read_io", 4, 4, |cls, a, _| xml_read(cls, a));
    defs(html_doc, "new", 0, -1, |cls, a, blk| {
        let mut dom = Dom::new(true);
        html::add_default_dtd(&mut dom);
        if let Some(&u) = a.first().filter(|v| **v != QNIL) {
            dom.url = Some(rs(u));
        }
        let doc = new_doc_obj(cls as *mut Class, dom);
        tri!(send(doc, sy().initialize, a, blk));
        doc
    });
    defs(html_doc, "read_memory", 4, 4, |cls, a, _| html_read(cls, a));
    defs(html_doc, "read_io", 4, 4, |cls, a, _| html_read(cls, a));

    defn(k, "root", 0, 0, |s, _, _| {
        let dom = sdom!(s);
        wrap(s, dom.root())
    });
    defn(k, "root=", 1, 1, |s, a, _| {
        let dom = sdom!(s);
        let old = dom.root();
        if a[0] == QNIL {
            if old != NONE {
                dom.unlink(old);
            }
            return QNIL;
        }
        if !is_node(a[0]) {
            return arg_error("expected a Nokogiri::XML::Node");
        }
        let (nd, mut nid) = match node_ref(a[0]) {
            Ok(x) => x,
            Err(e) => return e,
        };
        if nd != s {
            let r = tri!(adopt(nd, nid, s));
            nid = fix(r) as u32;
        }
        let dom = sdom!(s);
        if old != NONE && old != nid {
            dom.insert_before(old, nid);
            dom.unlink(old);
        } else if old == NONE {
            dom.append(0, nid);
        }
        a[0]
    });
    defn(k, "encoding", 0, 0, |s, _, _| ostr(sdom!(s).encoding.as_deref()));
    defn(k, "encoding=", 1, 1, |s, a, _| {
        if a[0] == QNIL {
            return type_error("no implicit conversion of nil into String");
        }
        sdom!(s).encoding = Some(rs(a[0]));
        a[0]
    });
    defn(k, "version", 0, 0, |s, _, _| ostr(sdom!(s).version.as_deref()));
    defn(k, "url", 0, 0, |s, _, _| ostr(sdom!(s).url.as_deref()));
    defn(k, "remove_namespaces!", 0, 0, |s, _, _| {
        let dom = sdom!(s);
        for n in dom.nodes.iter_mut() {
            n.ns = NONE;
            n.ns_defs.clear();
        }
        s
    });
    defn(k, "dup", 0, 1, |s, _, _| {
        let dom = sdom!(s);
        let mut copy = Dom::new(dom.html);
        copy.nodes = dom.nodes.clone();
        copy.nss = dom.nss.clone();
        copy.encoding = dom.encoding.clone();
        copy.version = dom.version.clone();
        copy.url = dom.url.clone();
        let doc = new_doc_obj(real_class(s), copy);
        tri!(send(doc, s_init(), &[], QNIL));
        ivar_set(doc, sym("@errors"), ivar_get(s, sym("@errors")));
        doc
    });
    defn(k, "__kenny_decorate", 2, 2, |_, a, _| kenny_decorate(a[0], a[1]));
    defn(k, "create_entity", 1, 5, |_, _, _| raise(rt().c.not_implemented_error, "entity declarations are not supported"));
}
fn s_init() -> u32 {
    sy().initialize
}

fn init_leaf_nodes() {
    let k = c();
    // Text.new(string, document)
    defs(k.text, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[1]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::Text, "text", &rs(a[0]));
        construct(cls, doc, id, a, blk)
    });
    // Comment.new(document_or_node, string)
    defs(k.comment, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::Comment, "comment", &rs(a[1]));
        construct(cls, doc, id, a, blk)
    });
    // CDATA.new(document, string)
    defs(k.cdata, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let content = if a[1] == QNIL { String::new() } else { rs(a[1]) };
        let id = dom_of(doc).new_node(T::CData, "#cdata-section", &content);
        construct(cls, doc, id, a, blk)
    });
    // ProcessingInstruction.new(document, name, content)
    defs(k.pi, "new", 3, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::PI, &rs(a[1]), &rs(a[2]));
        construct(cls, doc, id, a, blk)
    });
    // Attr.new(document, name)
    defs(k.attr, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::Attr, &rs(a[1]), "");
        construct(cls, doc, id, a, blk)
    });
    defn(k.attr, "value=", 1, 1, |s, a, _| {
        let (_, id, dom) = this!(s);
        dom.m(id).content = if a[0] == QNIL { String::new() } else { rs(a[0]) };
        a[0]
    });
    // EntityReference.new(document, name)
    defs(k.entity_ref, "new", 2, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::EntityRef, &rs(a[1]), "");
        construct(cls, doc, id, a, blk)
    });
    // DocumentFragment.new(document, *args)
    defs(k.fragment, "new", 1, -1, |cls, a, blk| {
        let doc = match opt_doc_arg(a[0]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let id = dom_of(doc).new_node(T::Fragment, "#document-fragment", "");
        let o = tri!(wrap_new(doc, id, cls as *mut Class));
        tri!(send(o, sy().initialize, a, blk));
        o
    });
    // DTD
    defn(k.dtd, "external_id", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        ostr(dom.n(id).ids.as_ref().and_then(|i| i.0.as_deref()))
    });
    defn(k.dtd, "system_id", 0, 0, |s, _, _| {
        let (_, id, dom) = this!(s);
        ostr(dom.n(id).ids.as_ref().and_then(|i| i.1.as_deref()))
    });
    for m in ["entities", "notations", "elements", "attributes"] {
        defn(k.dtd, m, 0, 0, |_, _, _| hash_new());
    }
    defn(k.dtd, "validate", 1, 1, |_, _, _| ary_new(Vec::new()));
    // Namespace
    defn(k.namespace, "prefix", 0, 0, |s, _, _| {
        let d = ivar_get(s, s_d());
        let n = ivar_get(s, s_n());
        if dom_ptr(d).is_null() || !is_fix(n) {
            return QNIL;
        }
        ostr(dom_of(d).nss[fix(n) as usize].prefix.as_deref())
    });
    defn(k.namespace, "href", 0, 0, |s, _, _| {
        let d = ivar_get(s, s_d());
        let n = ivar_get(s, s_n());
        if dom_ptr(d).is_null() || !is_fix(n) {
            return QNIL;
        }
        str_from(&dom_of(d).nss[fix(n) as usize].href)
    });
}
fn s_d() -> u32 {
    sy().d
}
fn s_n() -> u32 {
    sy().n
}

fn init_nokogiri() {
    *S.get() = Some(Syms {
        d: sym("@__d"),
        i: sym("@__i"),
        dom: sym("@__dom"),
        w: sym("@__w"),
        nsw: sym("@__nsw"),
        n: sym("@__n"),
        a: sym("@__a"),
        decorators: sym("@decorators"),
        decorate: sym("decorate"),
        document: sym("@document"),
        errors: sym("@errors"),
        initialize: sym("initialize"),
        node: sym("@__node"),
        nsh: sym("@__ns"),
        vars: sym("@__vars"),
    });
    let r = rt();
    let get_mod = |parent: *mut Class, name: &str, full: &str| -> *mut Class {
        match const_get_under(parent, sym(name)) {
            Some(v) if is_class(v) => v as *mut Class,
            _ => {
                let m = new_module(full);
                const_set(parent, sym(name), m as V);
                if parent != rt().c.object {
                    unsafe { (*m).lexical_parent = parent };
                }
                m
            }
        }
    };
    let nk = get_mod(r.c.object, "Nokogiri", "Nokogiri");
    let xmlm = get_mod(nk, "XML", "Nokogiri::XML");
    let html4 = get_mod(nk, "HTML4", "Nokogiri::HTML4");
    let gumbo = get_mod(nk, "Gumbo", "Nokogiri::Gumbo");
    let mk = |parent: *mut Class, name: &str, sup: *mut Class| -> *mut Class {
        let full = format!("{}::{name}", unsafe { &(*parent).name });
        let k = new_class(&full, sup);
        const_set(parent, sym(name), k as V);
        unsafe { (*k).lexical_parent = parent };
        k
    };
    let ob = r.c.object;
    let syntax = mk(nk, "SyntaxError", r.c.standard_error);
    let xml_syntax = mk(xmlm, "SyntaxError", syntax);
    let node = mk(xmlm, "Node", ob);
    let document = mk(xmlm, "Document", node);
    let html_document = mk(html4, "Document", document);
    let fragment = mk(xmlm, "DocumentFragment", node);
    let element = mk(xmlm, "Element", node);
    let attr = mk(xmlm, "Attr", node);
    let chardata = mk(xmlm, "CharacterData", node);
    let text = mk(xmlm, "Text", chardata);
    let cdata = mk(xmlm, "CDATA", text);
    let comment = mk(xmlm, "Comment", chardata);
    let pi = mk(xmlm, "ProcessingInstruction", node);
    let entity_ref = mk(xmlm, "EntityReference", node);
    let dtd = mk(xmlm, "DTD", node);
    let namespace = mk(xmlm, "Namespace", ob);
    let node_set_k = mk(xmlm, "NodeSet", ob);
    let xpath_ctx = mk(xmlm, "XPathContext", ob);
    let entity_lookup = mk(html4, "EntityLookup", ob);
    *C.get() = Some(Cls {
        node,
        document,
        html_document,
        element,
        attr,
        text,
        cdata,
        comment,
        pi,
        entity_ref,
        dtd,
        fragment,
        namespace,
        node_set: node_set_k,
        xml_syntax_error: xml_syntax,
    });

    // version constants (what Nokogiri 1.15.7's packaged libxml2 reports)
    let set = |name: &str, v: V| const_set(nk, sym(name), v);
    set("LIBXML_COMPILED_VERSION", str_from("2.11.7"));
    set("LIBXML_LOADED_VERSION", str_from("21107"));
    set("LIBXSLT_COMPILED_VERSION", str_from("1.1.39"));
    set("LIBXSLT_LOADED_VERSION", str_from("10139"));
    set("LIBXML_ICONV_ENABLED", QTRUE);
    set("LIBXSLT_DATETIME_ENABLED", QTRUE);
    set("PACKAGED_LIBRARIES", QTRUE);
    set("PRECOMPILED_LIBRARIES", QTRUE);
    set("LIBXML_MEMORY_MANAGEMENT", str_from("ruby"));
    set("OTHER_LIBRARY_VERSIONS", str_from("libgumbo:1.0.0-nokogiri"));
    let patches = ary_new(
        [
            "0001-Remove-script-macro-support.patch",
            "0002-Update-entities-to-remove-handling-of-ssi.patch",
            "0003-libxml2.la-is-in-top_builddir.patch",
            "0009-allow-wildcard-namespaces.patch",
            "0010-update-config.guess-and-config.sub-for-libxml2.patch",
            "0011-rip-out-libxml2-s-libc_single_threaded-support.patch",
        ]
        .iter()
        .map(|p| str_from(p))
        .collect(),
    );
    set("LIBXML2_PATCHES", patches);
    set("LIBXSLT_PATCHES", ary_new(vec![str_from("0001-update-config.guess-and-config.sub-for-libxslt.patch")]));

    init_node(node);
    init_document(document, html_document);
    init_leaf_nodes();
    init_node_set(node_set_k);

    defs(xpath_ctx, "new", 1, 1, |cls, a, _| {
        if !is_node(a[0]) {
            return arg_error("XPathContext.new requires a Nokogiri::XML::Node");
        }
        let o = object_new(cls as *mut Class);
        ivar_set(o, sy().node, a[0]);
        let h = hash_new();
        crate::hash::hset(h, str_from("nokogiri"), str_from("http://www.nokogiri.org/default_ns/ruby/extensions_functions"));
        crate::hash::hset(h, str_from("nokogiri-builtin"), str_from("https://www.nokogiri.org/default_ns/ruby/builtins"));
        ivar_set(o, sy().nsh, h);
        ivar_set(o, sy().vars, hash_new());
        o
    });
    defn(xpath_ctx, "register_ns", 2, 2, |s, a, _| {
        let h = ivar_get(s, s_nsh());
        crate::hash::hset(h, str_from(&rs(a[0])), str_from(&rs(a[1])));
        s
    });
    defn(xpath_ctx, "register_variable", 2, 2, |s, a, _| {
        let h = ivar_get(s, sym("@__vars"));
        crate::hash::hset(h, str_from(&rs(a[0])), a[1]);
        s
    });
    defn(xpath_ctx, "evaluate", 1, 2, |s, a, _| xpath_evaluate(s, a));
    defn(xpath_ctx, "node", 0, 0, |s, _, _| ivar_get(s, sym("@__node")));
    defn(xpath_ctx, "node=", 1, 1, |s, a, _| ivar_set(s, sym("@__node"), a[0]));

    defs(gumbo, "parse", 1, 6, |_, a, _| gumbo_parse(a));
    defs(gumbo, "fragment", 2, 6, |_, a, _| gumbo_fragment(a));

    defn(entity_lookup, "get", 1, 1, |_, a, _| {
        let name = rs(a[0]);
        match entities::lookup(&name) {
            Some((cp, desc)) => {
                let k = match class_named("Nokogiri::HTML4::EntityDescription") {
                    Some(k) => k,
                    None => return QNIL,
                };
                send(k as V, sym("new"), &[mkfix(cp as i64), str_from(&name), str_from(desc)], QNIL)
            }
            None => QNIL,
        }
    });
}
fn s_nsh() -> u32 {
    sy().nsh
}

pub fn init() {
    crate::compat::provide_feature(&["nokogiri/nokogiri"], Some(init_nokogiri));
}
