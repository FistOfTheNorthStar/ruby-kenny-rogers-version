//! Static calls: `name(args)` with no receiver, to a method this program defines with
//! `def name` and only required parameters, calls that method's fast entry directly
//! instead of through the inline cache and direct-call stub.
//!
//! Before a file is compiled, `scan_defs` finds its `def`s at the top level and directly in
//! class/module bodies, and declares each one's fast entry ahead of time. A call site picks
//! the definition with the right arity in the same class body (else the only one in the
//! program) and guards the call with a two-word slot, [receiver class, method serial]: the
//! runtime (`kn_static_fill`) fills it once the receiver's class is seen to have exactly
//! that method; any method definition anywhere (a new serial) makes it stale again.

use crate::*;
use crate::fastpath::may_be_ruby_method;
use std::collections::HashSet;
use ruby_prism::*;

/// A `def` whose fast entry was declared ahead of time.
#[derive(Clone)]
pub(crate) struct StaticDef {
    pub file: PathBuf,
    pub offset: usize,
    pub arity: usize,
    pub fid: FuncId,
    /// Enclosing class/module names, outermost first (as written).
    pub lex: Vec<Vec<u8>>,
}

/// A `def` taking only required positional and required keyword parameters (its calls with
/// those literal keywords may be inlined; see `kw_target`).
#[derive(Clone)]
pub(crate) struct KwDef {
    pub file: PathBuf,
    pub offset: usize,
    pub npos: usize,
    pub kws: Vec<Vec<u8>>,
    pub lex: Vec<Vec<u8>>,
}

/// (positional count, keyword names) of a `def name(a, b, k:, l:)`.
pub(crate) fn kw_shape(params: &Option<ParametersNode>) -> Option<(usize, Vec<Vec<u8>>)> {
    let p = params.as_ref()?;
    if p.optionals().iter().next().is_some()
        || p.rest().is_some()
        || p.posts().iter().next().is_some()
        || p.keyword_rest().is_some()
        || p.block().is_some()
        || !p.requireds().iter().all(|r| r.as_required_parameter_node().is_some())
    {
        return None;
    }
    let mut kws = Vec::new();
    for k in p.keywords().iter() {
        let n = k.as_required_keyword_parameter_node()?.name();
        let n = n.as_slice();
        kws.push(n.strip_suffix(b":").unwrap_or(n).to_vec());
    }
    let npos = p.requireds().iter().count();
    (!kws.is_empty() && npos + kws.len() <= 6).then_some((npos, kws))
}

struct Scan {
    lex: Vec<Vec<u8>>,
    /// keyword defs: (name, offset, positional count, keywords, enclosing names)
    kw: Vec<(Vec<u8>, usize, usize, Vec<Vec<u8>>, Vec<Vec<u8>>)>,
    /// inside a method, block, singleton class...: definitions there are skipped
    dynamic: usize,
    found: Vec<(Vec<u8>, usize, usize, Vec<u8>, Vec<Vec<u8>>)>,
    /// every def: (offset, enclosing names)
    all: Vec<(usize, Vec<Vec<u8>>)>,
    /// getters in class bodies: (class name, method, instance variable, `def` offset)
    getters: Vec<(Vec<u8>, Vec<u8>, Vec<u8>, Option<usize>)>,
}

/// The last segment of a class path as written (`A::B` -> `B`).
fn last_seg(p: &[u8]) -> Vec<u8> {
    match p.windows(2).rposition(|w| w == b"::") {
        Some(i) => p[i + 2..].to_vec(),
        None => p.to_vec(),
    }
}

/// Required-parameter count of a `def` that can have a fast entry (see scopes.rs `def_node`).
pub(crate) fn fast_arity(params: &Option<ParametersNode>) -> Option<usize> {
    match params {
        None => Some(0),
        Some(p) => (p.optionals().iter().next().is_none()
            && p.rest().is_none()
            && p.posts().iter().next().is_none()
            && p.keywords().iter().next().is_none()
            && p.keyword_rest().is_none())
        .then(|| p.requireds().iter().count())
        .filter(|&n| n <= 6),
    }
}

/// `define_method(:name) { |a, b| body }` that runs exactly like `def name(a, b) body end`
/// (the block uses nothing of the scope around it, and nothing that differs between a
/// block and a method): (name, the block's parameters, the block). Compiled as a `def`
/// (scopes.rs `def_from_block`) while `define_method` is the builtin one.
pub(crate) fn block_def<'pr>(c: &CallNode<'pr>) -> Option<(Vec<u8>, Option<ParametersNode<'pr>>, BlockNode<'pr>)> {
    if c.name().as_slice() != b"define_method" || c.receiver().is_some() || c.is_safe_navigation() {
        return None;
    }
    let args: Vec<Node> = c.arguments()?.arguments().iter().collect();
    let [arg] = args.as_slice() else { return None };
    let name = arg.as_symbol_node()?.unescaped().to_vec();
    if name.is_empty() {
        return None;
    }
    let blk = c.block()?.as_block_node()?;
    let params = match blk.parameters() {
        None => None,
        Some(p) => {
            let bp = p.as_block_parameters_node()?;
            match bp.parameters() {
                None => None,
                Some(ps) => {
                    let ps = Some(ps);
                    let p = ps.as_ref().unwrap();
                    if fast_arity(&ps).is_none() || p.block().is_some() || !p.requireds().iter().all(|r| r.as_required_parameter_node().is_some()) {
                        return None;
                    }
                    ps
                }
            }
        }
    };
    let mut v = DefLike { ok: true, nest: 0 };
    if let Some(b) = blk.body() {
        v.visit(&b);
    }
    v.ok.then_some((name, params, blk))
}

/// See `block_def`: no outer locals, `yield`, `super`, `next`/`break`/`redo`/`retry`,
/// nested definitions, or calls that look at the caller's frame.
struct DefLike {
    ok: bool,
    /// blocks entered inside the body
    nest: u32,
}
impl DefLike {
    fn local(&mut self, depth: u32) {
        if depth > self.nest {
            self.ok = false;
        }
    }
}
impl<'pr> Visit<'pr> for DefLike {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        self.nest += 1;
        visit_block_node(self, node);
        self.nest -= 1;
    }
    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        self.nest += 1;
        visit_lambda_node(self, node);
        self.nest -= 1;
    }
}
impl DefLike {
    fn check(&mut self, node: &Node) {
        match node {
            Node::YieldNode { .. }
            | Node::SuperNode { .. }
            | Node::ForwardingSuperNode { .. }
            | Node::NextNode { .. }
            | Node::BreakNode { .. }
            | Node::RedoNode { .. }
            | Node::RetryNode { .. }
            | Node::DefNode { .. }
            | Node::ClassNode { .. }
            | Node::ModuleNode { .. }
            | Node::SingletonClassNode { .. }
            | Node::ForwardingArgumentsNode { .. } => self.ok = false,
            Node::LocalVariableReadNode { .. } => self.local(node.as_local_variable_read_node().unwrap().depth()),
            Node::LocalVariableWriteNode { .. } => self.local(node.as_local_variable_write_node().unwrap().depth()),
            Node::LocalVariableTargetNode { .. } => self.local(node.as_local_variable_target_node().unwrap().depth()),
            Node::LocalVariableOperatorWriteNode { .. } => self.local(node.as_local_variable_operator_write_node().unwrap().depth()),
            Node::LocalVariableAndWriteNode { .. } => self.local(node.as_local_variable_and_write_node().unwrap().depth()),
            Node::LocalVariableOrWriteNode { .. } => self.local(node.as_local_variable_or_write_node().unwrap().depth()),
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                if matches!(
                    c.name().as_slice(),
                    b"block_given?" | b"iterator?" | b"binding" | b"local_variables" | b"eval" | b"proc" | b"lambda" | b"caller" | b"caller_locations" | b"__callee__"
                ) && c.receiver().is_none()
                {
                    self.ok = false;
                }
            }
            _ => {}
        }
    }
}

fn const_name(n: &Node) -> Vec<u8> {
    let l = n.location();
    l.as_slice().to_vec()
}

impl<'pr> Visit<'pr> for Scan {
    fn visit_class_node(&mut self, node: &ClassNode<'pr>) {
        if let Some(s) = node.superclass() {
            self.visit(&s);
        }
        self.lex.push(const_name(&node.constant_path()));
        if let Some(b) = node.body() {
            self.visit(&b);
        }
        self.lex.pop();
    }
    fn visit_module_node(&mut self, node: &ModuleNode<'pr>) {
        self.lex.push(const_name(&node.constant_path()));
        if let Some(b) = node.body() {
            self.visit(&b);
        }
        self.lex.pop();
    }
    fn visit_singleton_class_node(&mut self, node: &SingletonClassNode<'pr>) {
        self.dynamic += 1;
        visit_singleton_class_node(self, node);
        self.dynamic -= 1;
    }
    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        self.dynamic += 1;
        visit_block_node(self, node);
        self.dynamic -= 1;
    }
    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        self.dynamic += 1;
        visit_lambda_node(self, node);
        self.dynamic -= 1;
    }
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // `attr_reader :a, :b` / `attr_accessor :a` directly in a class body
        if self.dynamic == 0 && node.receiver().is_none() && matches!(node.name().as_slice(), b"attr_reader" | b"attr_accessor") {
            if let (Some(cls), Some(args)) = (self.lex.last(), node.arguments()) {
                for a in args.arguments().iter() {
                    if let Some(sym) = a.as_symbol_node() {
                        let m = sym.unescaped().to_vec();
                        let iv = [&b"@"[..], &m].concat();
                        self.getters.push((last_seg(cls), m, iv, None));
                    }
                }
            }
        }
        // `define_method(:name) { |a| ... }` compiled as a def (see `block_def`)
        if self.dynamic == 0 {
            if let Some((name, params, _)) = block_def(node) {
                let off = node.location().start_offset();
                self.all.push((off, self.lex.clone()));
                if let Some(n) = fast_arity(&params) {
                    let label = String::from_utf8_lossy(&name).into_owned();
                    self.found.push((name, off, n, label.into_bytes(), self.lex.clone()));
                }
            }
        }
        visit_call_node(self, node);
    }
    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        self.all.push((node.location().start_offset(), self.lex.clone()));
        // `def a; @a; end` (any instance variable) directly in a class body
        if self.dynamic == 0 && node.receiver().is_none() && node.parameters().is_none() {
            if let Some(cls) = self.lex.last() {
                let body = node.body();
                let single = body.as_ref().and_then(|b| match b.as_statements_node() {
                    Some(s) => {
                        let v: Vec<Node> = s.body().iter().collect();
                        if v.len() == 1 { v[0].as_instance_variable_read_node().map(|r| r.name().as_slice().to_vec()) } else { None }
                    }
                    None => b.as_instance_variable_read_node().map(|r| r.name().as_slice().to_vec()),
                });
                if let Some(iv) = single {
                    self.getters.push((last_seg(cls), node.name().as_slice().to_vec(), iv, Some(node.location().start_offset())));
                }
            }
        }
        if self.dynamic == 0 && node.receiver().is_none() {
            if let Some(n) = fast_arity(&node.parameters()) {
                let name = node.name().as_slice().to_vec();
                let label = String::from_utf8_lossy(&name).into_owned();
                self.found.push((name, node.location().start_offset(), n, label.into_bytes(), self.lex.clone()));
            } else if let Some((npos, kws)) = kw_shape(&node.parameters()) {
                self.kw.push((node.name().as_slice().to_vec(), node.location().start_offset(), npos, kws, self.lex.clone()));
            }
        }
        self.dynamic += 1;
        visit_def_node(self, node);
        self.dynamic -= 1;
    }
}

impl<M: Module> Compiler<M> {
    /// Find the file's definitions and declare their fast entries (see the module docs).
    pub(crate) fn scan_defs(&mut self, root: &Node) {
        if self.in_dependency() {
            return;
        }
        let mut s = Scan { lex: Vec::new(), kw: Vec::new(), dynamic: 0, found: Vec::new(), all: Vec::new(), getters: Vec::new() };
        s.visit(root);
        for (cls, m, iv, off) in std::mem::take(&mut s.getters) {
            let g = self.class_getters.entry(cls).or_default();
            // (two different getters of that name: neither)
            match g.get(&m) {
                Some(prev) if *prev != (iv.clone(), off) => {
                    g.insert(m, (Vec::new(), None));
                }
                _ => {
                    g.insert(m, (iv, off));
                }
            }
        }
        for (o, lex) in s.all {
            self.def_lex.insert((self.file.clone(), o), lex);
        }
        let mut cands: Vec<(Vec<u8>, usize, Vec<Vec<u8>>)> = s.found.iter().map(|(n, o, _, _, l)| (n.clone(), *o, l.clone())).collect();
        for (name, offset, npos, kws, lex) in s.kw {
            cands.push((name.clone(), offset, lex.clone()));
            self.kw_defs.entry(name).or_default().push(KwDef { file: self.file.clone(), offset, npos, kws, lex });
        }
        self.collect_inline_defs(&cands);
        for (name, offset, arity, label, lex) in s.found {
            self.static_counter += 1;
            let l: String = String::from_utf8_lossy(&label).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            let sig = self.fast_sig(arity);
            let Ok(fid) = self.module.declare_function(&format!("__kn_s{}_{}", self.static_counter, l), Linkage::Local, &sig) else { continue };
            let d = StaticDef { file: self.file.clone(), offset, arity, fid, lex };
            self.static_defs.entry(name).or_default().push(d);
        }
    }

    /// The predeclared fast entry of the `def` at `offset` of the current file (once).
    pub(crate) fn static_fid_for_def(&mut self, name: &[u8], offset: usize) -> Option<FuncId> {
        let d = self.static_defs.get(name)?.iter().find(|d| d.offset == offset && d.file == self.file)?;
        let fid = d.fid;
        self.static_defined.insert(fid).then_some(fid)
    }

    /// The definition a receiver-less call `name` with `nargs` arguments most likely reaches.
    fn static_target(&self, fx: &Fx, name: &[u8], nargs: usize) -> Option<FuncId> {
        self.static_target_lex(fx, name, nargs)
    }
    fn static_target_lex(&self, fx: &Fx, name: &[u8], nargs: usize) -> Option<FuncId> {
        let defs = self.static_defs.get(name)?;
        let cands: Vec<&StaticDef> = defs.iter().filter(|d| d.arity == nargs).collect();
        let here = self.cur_lex(fx).unwrap_or_default();
        let local: Vec<&&StaticDef> = cands.iter().filter(|d| d.lex == here && d.file == self.file).collect();
        match (local.len(), cands.len()) {
            (1, _) => Some(local[0].fid),
            (0, 1) => Some(cands[0].fid),
            _ => None,
        }
    }

    /// A receiver-less call: straight to the fast entry of its static target when the
    /// site's slot says the receiver's class has that method (see the module docs), else
    /// `dynamic` (after asking the runtime to fill the slot).
    pub(crate) fn static_call(
        &mut self,
        fx: &mut Fx,
        recv: Value,
        name: &[u8],
        mid: Value,
        vals: &[Value],
        dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value,
    ) -> Option<Value> {
        if self.refine_scope.is_some() || !may_be_ruby_method(name) || self.in_dependency() {
            return None;
        }
        let fid = self.static_target(fx, name, vals.len())?;
        if fx.kind == ScopeKind::Method && fx.self_fid == Some(fid) && self.inline_ctx.is_empty() && self.claim_words.contains_key(&fid) {
            return Some(self.self_call(fx, fid, recv, vals, dynamic));
        }
        Some(self.static_guarded(fx, fid, recv, mid, vals, dynamic))
    }

    /// `static_call` to `fid`, behind the site's slot.
    fn static_guarded(&mut self, fx: &mut Fx, fid: FuncId, recv: Value, mid: Value, vals: &[Value], dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slot = self.cache_ptr(fx, 2);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        self.require_heap(fx, recv, slow);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let sk = fx.b.ins().load(I64, f, slot, 0);
        let ss = fx.b.ins().load(I64, f, slot, 8);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        // class and serial as one test
        let d1 = fx.b.ins().bxor(k, sk);
        let d2 = fx.b.ins().bxor(ss, cur);
        let d = fx.b.ins().bor(d1, d2);
        let bad = self.no_room(fx, dir);
        let d = fx.b.ins().bor(d, bad);
        self.require_zero(fx, d, slow);
        let mut args = vec![recv];
        args.extend_from_slice(vals);
        args.push(fx.b.ins().iconst(I64, QNIL));
        let r = self.call_fn(fx, fid, &args);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let fa = self.fn_addr(fx, fid);
        let tag = fx.b.ins().iconst(I64, ((vals.len() as i64) + 1) << 56);
        let expect = fx.b.ins().bor(fa, tag);
        self.call(fx, "kn_static_fill", &[slot, recv, mid, expect]);
        let r = dynamic(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
}

/// Whether a `def`'s body calls the method itself without a receiver (or with `self`),
/// with `n` plain arguments, outside blocks: those calls can be `self_call`s.
pub(crate) fn calls_itself(name: &[u8], n: usize, body: &Node) -> bool {
    struct S<'a> {
        name: &'a [u8],
        n: usize,
        found: bool,
    }
    impl<'pr> Visit<'pr> for S<'_> {
        fn visit_call_node(&mut self, node: &CallNode<'pr>) {
            let plain = node.receiver().is_none_or(|r| r.as_self_node().is_some());
            let args: Vec<Node> = node.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            let simple = args.iter().all(|a| {
                a.as_splat_node().is_none() && a.as_keyword_hash_node().is_none() && a.as_block_argument_node().is_none() && a.as_forwarding_arguments_node().is_none()
            });
            if plain && node.name().as_slice() == self.name && args.len() == self.n && simple && node.block().is_none() {
                self.found = true;
            }
            visit_call_node(self, node);
        }
        // (another `self`, or code that runs later)
        fn visit_block_node(&mut self, _: &BlockNode<'pr>) {}
        fn visit_lambda_node(&mut self, _: &LambdaNode<'pr>) {}
        fn visit_def_node(&mut self, _: &DefNode<'pr>) {}
        fn visit_class_node(&mut self, _: &ClassNode<'pr>) {}
        fn visit_module_node(&mut self, _: &ModuleNode<'pr>) {}
        fn visit_singleton_class_node(&mut self, _: &SingletonClassNode<'pr>) {}
    }
    let mut s = S { name, n, found: false };
    s.visit(body);
    s.found
}

impl<M: Module> Compiler<M> {
    /// The word of a `def` that calls itself (see `self_call`): 0 while the runtime vouches
    /// that this method is the only one of its name in any class (kenny-runtime abi.rs
    /// `kn_def`, core.rs `Claim`), else nonzero. Starts at 1: the `def` hands it over.
    pub(crate) fn claim_word_id(&mut self, fid: FuncId) -> DataId {
        if let Some(&id) = self.claim_words.get(&fid) {
            return id;
        }
        let id = self.module.declare_data(&format!("__kn_claim_{}", fid.as_u32()), Linkage::Local, true, false).expect("claim word");
        let mut d = DataDescription::new();
        d.define(Box::new(1u64.to_le_bytes()));
        d.set_align(8);
        self.module.define_data(id, &d).expect("claim word");
        self.claim_words.insert(fid, id);
        id
    }

    /// A method calling itself without a receiver, in its own body (not inlined, not in a
    /// block): `self` was dispatched to this method, so while it is the only method of its
    /// name in any method table (and defined in a class: a module's method may be bound
    /// to any object) the call reaches it. One word to check instead of the receiver's class
    /// and the serial; room on the stack and a pending collection are checked on entry
    /// (`entry_check`). Else `dynamic`: its one call keeps the slow path from holding values
    /// across calls (they would take callee-saved registers, saved on every call).
    fn self_call(&mut self, fx: &mut Fx, fid: FuncId, recv: Value, vals: &[Value], dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let id = self.claim_word_id(fid);
        let gv = self.module.declare_data_in_func(id, fx.b.func);
        let word = fx.b.ins().symbol_value(I64, gv);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        let w = fx.b.ins().load(I64, mem(), word, 0);
        // (on the word itself: a compare with 0 would bring a constant 0 that Cranelift
        // shares with the cold paths' and then keeps in a callee-saved register)
        let go = fx.b.create_block();
        fx.b.ins().brif(w, slow, &[], go, &[]);
        fx.b.switch_to_block(go);
        let mut args = vec![recv];
        args.extend_from_slice(vals);
        args.push(fx.b.ins().iconst(I64, QNIL));
        let r = self.call_fn(fx, fid, &args);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let r = dynamic(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// On entry to a method that calls itself (`self_call`s skip these checks): room on the
    /// stack (`KN_DIRECT.entry_limit`: 0 while several OS threads run Ruby code, their calls
    /// to themselves going through the runtime) and no collection pending.
    pub(crate) fn entry_check(&mut self, fx: &mut Fx) {
        let f = mem();
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let lim = fx.b.ins().load(I64, f, dir, KN_DIRECT_ENTRY_LIMIT);
        let sp = fx.b.ins().get_stack_pointer(I64);
        let low = fx.b.ins().icmp(IntCC::UnsignedLessThan, sp, lim);
        let low = fx.b.ins().uextend(I64, low);
        let gc = fx.b.ins().uload8(I64, f, fx.gcflag, 0);
        let bad = fx.b.ins().bor(low, gc);
        let slow = fx.b.create_block();
        let cont = fx.b.create_block();
        fx.b.ins().brif(bad, slow, &[], cont, &[]);
        fx.b.set_cold_block(slow);
        fx.b.switch_to_block(slow);
        let r = self.call(fx, "kn_entry_slow", &[]);
        self.check(fx, r);
        fx.b.ins().jump(cont, &[]);
        fx.b.switch_to_block(cont);
    }
}

// ---------- inlining ----------
//
// A call whose static target (above; for a call with a receiver, the program's only `def`
// of that name and arity) is a small method of this file with a simple body compiles that
// body in place, behind the same guard slot as a static call: `self` is the receiver and the
// method's locals are fresh variables. Calls made by the inlined code are recorded with an
// inline location (kenny-runtime backtrace.rs) so backtraces still show the method's frame.

impl InlineDef {
    pub(crate) fn params(&self) -> Option<ParametersNode<'static>> {
        match &self.node {
            DefSrc::Def(d) => d.parameters(),
            DefSrc::Block(c) => block_def(c).and_then(|(_, p, _)| p),
        }
    }
    pub(crate) fn body(&self) -> Option<Node<'static>> {
        match &self.node {
            DefSrc::Def(d) => d.body(),
            DefSrc::Block(c) => block_def(c).and_then(|(_, _, b)| b.body()),
        }
    }
    pub(crate) fn locals(&self) -> Vec<Vec<u8>> {
        let ids = match &self.node {
            DefSrc::Def(d) => d.locals(),
            DefSrc::Block(c) => match block_def(c) {
                Some((_, _, b)) => b.locals(),
                None => return Vec::new(),
            },
        };
        ids.iter().map(|l| l.as_slice().to_vec()).collect()
    }
}

/// The parts of `Fx` that belong to one Ruby scope: swapped out while an inlined method
/// body compiles, and back in for its caller's block at a `yield` (`inline_yield`).
pub(crate) struct ScopeState {
    self_v: Variable,
    locals: HashMap<Vec<u8>, Local>,
    num_locals: HashMap<Vec<u8>, (Variable, Variable)>,
    int_locals: HashSet<Vec<u8>>,
    flocals: HashMap<Vec<u8>, (Variable, Option<Variable>)>,
    int_sure: HashSet<Vec<u8>>,
    inline_levels: Vec<HashMap<Vec<u8>, Variable>>,
    vlocals: HashMap<Vec<u8>, std::rc::Rc<crate::vobj::VLocal>>,
    vself: Option<HashMap<Vec<u8>, Variable>>,
    /// (a speculated loop is about its own scope's locals)
    spec: Option<crate::spec::SpecCtx>,
    int_spec: HashSet<Vec<u8>>,
    cref_slot: usize,
    inl_self: Vec<crate::licm::Entry>,
    /// (an attr read being remembered: not the inlined body's)
    licm_pending: Option<crate::licm::Entry>,
    owned: HashMap<Vec<u8>, Variable>,
}
impl ScopeState {
    /// Move the current scope's state out of `fx` (leaving it empty).
    fn take(fx: &mut Fx) -> ScopeState {
        ScopeState {
            self_v: fx.self_v,
            locals: std::mem::take(&mut fx.locals),
            num_locals: std::mem::take(&mut fx.num_locals),
            int_locals: std::mem::take(&mut fx.int_locals),
            flocals: std::mem::take(&mut fx.flocals),
            int_sure: std::mem::take(&mut fx.int_sure),
            inline_levels: std::mem::take(&mut fx.inline_levels),
            vlocals: std::mem::take(&mut fx.vlocals),
            vself: fx.vself.take(),
            spec: fx.spec.take(),
            int_spec: std::mem::take(&mut fx.int_spec),
            cref_slot: fx.cref_slot,
            inl_self: std::mem::take(&mut fx.inl_self),
            licm_pending: fx.licm_pending.take(),
            owned: std::mem::take(&mut fx.owned),
        }
    }
    fn put(self, fx: &mut Fx) {
        fx.self_v = self.self_v;
        fx.locals = self.locals;
        fx.num_locals = self.num_locals;
        fx.int_locals = self.int_locals;
        fx.flocals = self.flocals;
        fx.int_sure = self.int_sure;
        fx.inline_levels = self.inline_levels;
        fx.vlocals = self.vlocals;
        fx.vself = self.vself;
        fx.spec = self.spec;
        fx.int_spec = self.int_spec;
        fx.cref_slot = self.cref_slot;
        fx.inl_self = self.inl_self;
        fx.licm_pending = self.licm_pending;
        fx.owned = self.owned;
    }
}

/// A call's literal block, compiled in place at each `yield` of the inlined method.
pub(crate) struct YieldBlock {
    pub blk: BlockNode<'static>,
    pub params: Vec<Vec<u8>>,
    pub locals: Vec<Vec<u8>>,
}

/// An inlined method body with a block to yield to (innermost last).
pub(crate) struct YieldTarget {
    yb: YieldBlock,
    /// `inline_ctx` depth of that body
    depth: usize,
    /// where `break` in the block goes: the call's result
    brk: Block,
    /// the caller's scope, while the body compiles
    caller: Option<ScopeState>,
}

/// How deep inlined code may itself inline.
const MAX_INLINE_DEPTH: usize = 2;
/// Most nodes an inlined body may have.
const MAX_INLINE_NODES: usize = 40;
/// kenny-runtime abi.rs `kn_static_fill`: the target must be public (a call with a receiver).
const FILL_PUBLIC: i64 = 1 << 63;

/// Where an inlinable method comes from: a `def`, or a `define_method` call compiled as one
/// (`block_def`).
pub(crate) enum DefSrc {
    Def(DefNode<'static>),
    Block(CallNode<'static>),
}

/// A `def` that may be inlined (its body re-parsed into a tree that lives forever).
pub(crate) struct InlineDef {
    pub node: DefSrc,
    pub name: Vec<u8>,
    /// how many `yield`s its body has (see `Simple::yields`)
    pub yields: usize,
    /// reads constants (only inlined where they resolve the same way)
    pub consts: bool,
    pub lex: Vec<Vec<u8>>,
    pub file: PathBuf,
    pub offset: usize,
}

/// One inlined method being compiled (innermost last).
pub(crate) struct InlineFrame {
    pub key: (PathBuf, usize),
    pub label_off: u32,
    pub file_off: u32,
    /// location of the call site (a line, or an inline record)
    pub call_loc: u32,
    pub lex: Vec<Vec<u8>>,
}

/// Is a method body simple enough to inline: only expressions on locals, instance
/// variables and literals, conditionals and plain calls (no blocks, no control flow out of
/// the method, nothing that looks at the method's own frame).
struct Simple {
    ok: bool,
    nodes: usize,
    consts: bool,
    /// `yield`s (with plain arguments): inlined only with a literal block (`inline_yield`)
    yields: usize,
}
impl<'pr> Visit<'pr> for Simple {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
}
impl Simple {
    fn check(&mut self, node: &Node) {
        self.nodes += 1;
        let ok = match node {
            Node::StatementsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::SelfNode { .. }
            | Node::SymbolNode { .. }
            | Node::StringNode { .. }
            | Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::ElseNode { .. }
            | Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::ArrayNode { .. } => true,
            Node::LocalVariableReadNode { .. } => node.as_local_variable_read_node().unwrap().depth() == 0,
            Node::LocalVariableWriteNode { .. } => node.as_local_variable_write_node().unwrap().depth() == 0,
            Node::LocalVariableOperatorWriteNode { .. } => node.as_local_variable_operator_write_node().unwrap().depth() == 0,
            Node::ConstantReadNode { .. } => {
                self.consts = true;
                true
            }
            Node::YieldNode { .. } => {
                self.yields += 1;
                let y = node.as_yield_node().unwrap();
                y.arguments().is_none_or(|a| {
                    a.arguments().iter().all(|x| {
                        x.as_splat_node().is_none() && x.as_block_argument_node().is_none() && x.as_keyword_hash_node().is_none() && x.as_forwarding_arguments_node().is_none()
                    })
                })
            }
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                !matches!(
                    c.name().as_slice(),
                    b"block_given?" | b"iterator?" | b"binding" | b"__method__" | b"__callee__" | b"__dir__" | b"caller" | b"caller_locations"
                        | b"local_variables" | b"eval" | b"instance_eval" | b"instance_exec" | b"class_eval" | b"module_eval" | b"class_exec"
                        | b"module_exec" | b"lambda" | b"proc" | b"loop" | b"require" | b"require_relative" | b"load" | b"autoload"
                        | b"define_method" | b"send" | b"__send__" | b"public_send" | b"method" | b"public_method" | b"catch" | b"throw"
                        | b"private" | b"public" | b"protected" | b"module_function" | b"using" | b"raise" | b"fail"
                ) && c.block().is_none()
            }
            _ => false,
        };
        if !ok || self.nodes > MAX_INLINE_NODES {
            self.ok = false;
        }
    }
}

/// The `def`s of a re-parsed file at these offsets.
struct FindDefs<'a> {
    want: &'a HashSet<usize>,
    found: Vec<DefSrc>,
}
impl<'a> Visit<'static> for FindDefs<'a> {
    fn visit_def_node(&mut self, node: &DefNode<'static>) {
        if self.want.contains(&node.location().start_offset()) {
            // (DefNode isn't Clone: look it up again through its own location)
            self.found.push(DefSrc::Def(unsafe { std::ptr::read(node) }));
        }
        visit_def_node(self, node);
    }
    fn visit_call_node(&mut self, node: &CallNode<'static>) {
        if self.want.contains(&node.location().start_offset()) && block_def(node).is_some() {
            self.found.push(DefSrc::Block(unsafe { std::ptr::read(node) }));
        }
        visit_call_node(self, node);
    }
}

impl<M: Module> Compiler<M> {
    /// Keep this file's inlinable `def`s (called by `scan_defs` with its candidates).
    pub(crate) fn collect_inline_defs(&mut self, cands: &[(Vec<u8>, usize, Vec<Vec<u8>>)]) {
        if cands.is_empty() || self.eval_unit {
            return;
        }
        let src: &'static [u8] = Box::leak(self.src.clone().into_boxed_slice());
        let pr: &'static ruby_prism::ParseResult<'static> = Box::leak(Box::new(ruby_prism::parse(src)));
        let want: HashSet<usize> = cands.iter().map(|c| c.1).collect();
        let mut fd = FindDefs { want: &want, found: Vec::new() };
        fd.visit(&pr.node());
        for d in fd.found {
            let off = match &d {
                DefSrc::Def(d) => d.location().start_offset(),
                DefSrc::Block(c) => c.location().start_offset(),
            };
            let Some((name, _, lex)) = cands.iter().find(|c| c.1 == off) else { continue };
            let idef = InlineDef { node: d, name: name.clone(), consts: false, yields: 0, lex: lex.clone(), file: self.file.clone(), offset: off };
            let params_ok = match idef.params() {
                None => true,
                Some(p) => {
                    p.block().is_none()
                        && p.requireds().iter().all(|r| r.as_required_parameter_node().is_some())
                        && p.keywords().iter().all(|k| k.as_required_keyword_parameter_node().is_some())
                }
            };
            if !params_ok {
                continue;
            }
            let mut s = Simple { ok: true, nodes: 0, consts: false, yields: 0 };
            if let Some(b) = idef.body() {
                s.visit(&b);
            }
            if !s.ok {
                continue;
            }
            let key = (self.file.clone(), off);
            let idef = InlineDef { consts: s.consts, yields: s.yields, ..idef };
            self.inline_defs.insert(key, std::rc::Rc::new(idef));
        }
    }

    /// Location bits for the current source offset: the line, or inside inlined code an
    /// inline record (kenny-runtime backtrace.rs).
    pub(crate) fn loc_bits(&mut self) -> u32 {
        let line = self.line_of(self.cur_offset) as u32;
        let Some(fr) = self.inline_ctx.last() else { return line };
        let (label, file, outer) = (fr.label_off, fr.file_off, fr.call_loc);
        let mut rec = Vec::with_capacity(16);
        for w in [line, label, file, outer] {
            rec.extend_from_slice(&w.to_le_bytes());
        }
        (1 << 31) | self.str_offset(&rec) as u32
    }

    /// The lexical class path of the code being compiled, if known.
    fn cur_lex(&self, fx: &Fx) -> Option<Vec<Vec<u8>>> {
        match self.inline_ctx.last() {
            Some(fr) => Some(fr.lex.clone()),
            None => fx.method.as_ref().and_then(|m| m.lex.clone()),
        }
    }

    /// Inline the target of `name(vals)` / `recv.name(vals)` (see above), with `dynamic`
    /// as the call when the guard fails. None: not inlinable here.
    pub(crate) fn inline_call(
        &mut self,
        fx: &mut Fx,
        recv: Value,
        name: &[u8],
        mid: Value,
        vals: &[Value],
        explicit: bool,
        dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value,
    ) -> Option<Value> {
        self.inline_call_with(fx, recv, name, mid, vals, explicit, None, dynamic)
    }

    /// The method `inline_call` would inline (with a literal block when `with_block`).
    pub(crate) fn inline_target(&self, fx: &Fx, name: &[u8], n: usize, explicit: bool, with_block: bool) -> Option<(StaticDef, std::rc::Rc<InlineDef>)> {
        if self.refine_scope.is_some() || self.in_dependency() || self.eval_unit || !may_be_ruby_method(name) || self.inline_ctx.len() >= MAX_INLINE_DEPTH {
            return None;
        }
        let sd = if explicit {
            let defs = self.static_defs.get(name)?;
            let mut c = defs.iter().filter(|d| d.arity == n);
            let d = c.next()?;
            if c.next().is_some() {
                return None;
            }
            d.clone()
        } else {
            let fid = self.static_target_lex(fx, name, n)?;
            self.static_defs.get(name)?.iter().find(|d| d.fid == fid)?.clone()
        };
        let key = (sd.file.clone(), sd.offset);
        let idef = self.inline_defs.get(&key)?.clone();
        if idef.file != self.file || self.inline_ctx.iter().any(|f| f.key == key) || self.def_stack.contains(&key) {
            return None;
        }
        if idef.consts && self.cur_lex(fx).as_ref() != Some(&idef.lex) && !self.def_cref.contains_key(&key) {
            return None;
        }
        // a `yield` needs the literal block (each one compiles it again: at most two)
        if if with_block { idef.yields > 2 } else { idef.yields > 0 } {
            return None;
        }
        Some((sd, idef))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inline_call_with(
        &mut self,
        fx: &mut Fx,
        recv: Value,
        name: &[u8],
        mid: Value,
        vals: &[Value],
        explicit: bool,
        yb: Option<YieldBlock>,
        dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value,
    ) -> Option<Value> {
        let n = vals.len();
        let (sd, idef) = self.inline_target(fx, name, n, explicit, yb.is_some())?;
        if std::env::var_os("KENNY_DEBUG_INLINE").is_some() {
            let line = self.line_of(self.cur_offset);
            eprintln!("kenny: inline {} into {}:{} (depth {})", String::from_utf8_lossy(name), self.file.display(), line, self.inline_ctx.len() + 1);
        }
        let f = mem();
        let slot = self.cache_ptr(fx, 2);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        self.require_heap(fx, recv, slow);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let sk = fx.b.ins().load(I64, f, slot, 0);
        let ss = fx.b.ins().load(I64, f, slot, 8);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        // class and serial as one test
        let d1 = fx.b.ins().bxor(k, sk);
        let d2 = fx.b.ins().bxor(ss, cur);
        let d = fx.b.ins().bor(d1, d2);
        self.require_zero(fx, d, slow);
        let r = self.inline_method_body(fx, &idef, recv, vals, &[], yb.map(|y| (y, done)));
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let fa = self.fn_addr(fx, sd.fid);
        let tag = ((n as i64) + 1) << 56 | if explicit { FILL_PUBLIC } else { 0 };
        let tag = fx.b.ins().iconst(I64, tag);
        let expect = fx.b.ins().bor(fa, tag);
        self.call(fx, "kn_static_fill", &[slot, recv, mid, expect]);
        let r = dynamic(self, fx);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        // (the inlined body's own calls are checked where they happen)
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        Some(r)
    }

    /// `yield` in an inlined method body called with a literal block: the block's body,
    /// here, in the caller's scope (`next` gives the yield's value, `break` the call's).
    pub(crate) fn inline_yield(&mut self, fx: &mut Fx, y: &YieldNode) -> Option<Value> {
        if self.yield_targets.last()?.depth != self.inline_ctx.len() {
            return None;
        }
        let vals: Vec<Value> = match y.arguments() {
            Some(a) => a.arguments().iter().map(|n| self.expr(fx, &n)).collect(),
            None => Vec::new(),
        };
        let at = self.cur_offset;
        let mut t = self.yield_targets.pop().unwrap();
        let frame = self.inline_ctx.pop().unwrap();
        let callee = ScopeState::take(fx);
        t.caller.take().expect("caller scope").put(fx);
        let next = fx.b.create_block();
        fx.b.append_block_param(next, I64);
        fx.loops.push(LoopCtx { next, exit: t.brk, redo: next, next_val: true, rescue_depth: fx.rescue_prev.len() });
        let v = self.inline_body_pub(fx, &t.yb.blk, &t.yb.params, &t.yb.locals, &vals, vals.len() == 1, true);
        fx.loops.pop();
        fx.b.ins().jump(next, &[v.into()]);
        fx.b.switch_to_block(next);
        let r = fx.b.block_params(next)[0];
        t.caller = Some(ScopeState::take(fx));
        callee.put(fx);
        self.inline_ctx.push(frame);
        self.yield_targets.push(t);
        self.cur_offset = at;
        self.set_srcloc(fx);
        Some(r)
    }

    /// `inline_method_body` for vobj.rs.
    pub(crate) fn inline_method_body_pub(&mut self, fx: &mut Fx, d: &InlineDef, recv: Value, vals: &[Value]) -> Value {
        self.inline_method_body(fx, d, recv, vals, &[], None)
    }

    /// Compile `d`'s body here with `recv` as self and `vals` as its parameters.
    /// `yb`: the call's literal block and the call's result block (for `break`).
    fn inline_method_body(&mut self, fx: &mut Fx, d: &InlineDef, recv: Value, vals: &[Value], kw: &[(Vec<u8>, Value)], yb: Option<(YieldBlock, Block)>) -> Value {
        let at = self.cur_offset;
        let call_loc = self.loc_bits();
        let mut label = d.name.clone();
        label.push(0);
        let label_off = self.str_offset(&label) as u32;
        let mut file = d.file.display().to_string().into_bytes();
        file.push(0);
        let file_off = self.str_offset(&file) as u32;
        // the callee's scope
        let mut caller = Some(ScopeState::take(fx));
        fx.inl_self = self.licm_inl_self(fx, recv);
        let sv = fx.b.declare_var(I64);
        fx.b.def_var(sv, recv);
        fx.self_v = sv;
        let nil = fx.b.ins().iconst(I64, QNIL);
        for n in d.locals() {
            if !fx.locals.contains_key(&n) {
                let v = fx.b.declare_var(I64);
                fx.b.def_var(v, nil);
                fx.locals.insert(n, Local::Var(v));
            }
        }
        if let Some(p) = d.params() {
            for (r, v) in p.requireds().iter().zip(vals) {
                if let Some(rp) = r.as_required_parameter_node() {
                    if let Some(Local::Var(var)) = fx.locals.get(rp.name().as_slice()) {
                        fx.b.def_var(*var, *v);
                    }
                }
            }
        }
        for (k, v) in kw {
            if let Some(Local::Var(var)) = fx.locals.get(k) {
                fx.b.def_var(*var, *v);
            }
        }
        let body = d.body();
        let mut analysis: Vec<&Node> = Vec::new();
        let params = d.params().map(|p| p.as_node());
        if let Some(p) = &params {
            analysis.push(p);
        }
        if let Some(b) = &body {
            analysis.push(b);
        }
        let (num, flo, ints, _) = crate::scopes::typed_setup(&mut fx.b, &fx.locals, &analysis, self.refine_scope.is_none());
        fx.num_locals = num;
        fx.flocals = flo;
        fx.int_sure = ints;
        fx.int_locals = crate::analysis::int_locals(&analysis);
        fx.vlocals = self.vobj_plan(&mut fx.b, &fx.locals, &analysis);
        // (a virtual object's `initialize`: its instance variables)
        fx.vself = fx.vself_next.take();
        // constants resolve in the method's own scope
        if let Some(&slot) = self.def_cref.get(&(d.file.clone(), d.offset)) {
            fx.cref_slot = slot;
        }
        self.inline_ctx.push(InlineFrame { key: (d.file.clone(), d.offset), label_off, file_off, call_loc, lex: d.lex.clone() });
        if let Some((yb, brk)) = yb {
            self.yield_targets.push(YieldTarget { yb, depth: self.inline_ctx.len(), brk, caller: caller.take() });
        }
        let r = match &body {
            Some(b) => self.expr(fx, b),
            None => self.nil(fx),
        };
        let caller = match caller {
            Some(c) => c,
            None => self.yield_targets.pop().and_then(|t| t.caller).expect("yield target"),
        };
        self.inline_ctx.pop();
        caller.put(fx);
        self.cur_offset = at;
        self.set_srcloc(fx);
        r
    }
}

impl<M: Module> Compiler<M> {
    /// `C.new(vals)` where C names a class of this file whose `initialize` can be inlined:
    /// while the runtime has found (`kn_new_fill`) that C's `new` is the builtin one, its
    /// `initialize` is that `def` and it makes plain objects, allocate here and inline the
    /// `initialize` body. Else `dynamic`.
    pub(crate) fn inline_new(&mut self, fx: &mut Fx, recv: Value, cname: &[u8], vals: &[Value], dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Option<Value> {
        if self.refine_scope.is_some() || self.in_dependency() || self.eval_unit || self.inline_ctx.len() >= MAX_INLINE_DEPTH {
            return None;
        }
        let n = vals.len();
        let defs = self.static_defs.get(&b"initialize"[..])?;
        let mut c = defs.iter().filter(|d| d.arity == n && d.lex.last().is_some_and(|l| l.as_slice() == cname || l.ends_with(&[b"::", cname].concat())));
        let sd = c.next()?.clone();
        if c.next().is_some() {
            return None;
        }
        let key = (sd.file.clone(), sd.offset);
        let idef = self.inline_defs.get(&key)?.clone();
        if idef.file != self.file || self.inline_ctx.iter().any(|f| f.key == key) || self.def_stack.contains(&key) {
            return None;
        }
        if idef.consts && self.cur_lex(fx).as_ref() != Some(&idef.lex) && !self.def_cref.contains_key(&key) {
            return None;
        }
        if idef.yields > 0 {
            return None;
        }
        if std::env::var_os("KENNY_DEBUG_INLINE").is_some() {
            let line = self.line_of(self.cur_offset);
            eprintln!("kenny: inline {}.new into {}:{} (depth {})", String::from_utf8_lossy(cname), self.file.display(), line, self.inline_ctx.len() + 1);
        }
        let f = mem();
        let slot = self.cache_ptr(fx, 2);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        let sk = fx.b.ins().load(I64, f, slot, 0);
        let ss = fx.b.ins().load(I64, f, slot, 8);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let d1 = fx.b.ins().bxor(recv, sk);
        let d2 = fx.b.ins().bxor(ss, cur);
        let d = fx.b.ins().bor(d1, d2);
        self.require_zero(fx, d, slow);
        let o = self.call(fx, "kn_obj_alloc", &[recv]);
        self.inline_method_body(fx, &idef, o, vals, &[], None);
        fx.b.ins().jump(done, &[o.into()]);
        fx.b.switch_to_block(slow);
        let fa = self.fn_addr(fx, sd.fid);
        let tag = fx.b.ins().iconst(I64, ((n as i64) + 1) << 56);
        let expect = fx.b.ins().bor(fa, tag);
        self.call(fx, "kn_new_fill", &[slot, recv, expect]);
        let r = dynamic(self, fx);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        Some(r)
    }
}

// ---------- keyword calls ----------
//
// `name(a, k: x, l: y)` / `recv.name(...)` with distinct literal Symbol keys, to a `def` of
// this file taking exactly those required keywords (in any order): the body is inlined (the
// keyword hash is only built when the guard fails). The guard slot is filled when the
// receiver's class has that very `def`: its parameter descriptor carries a marker unique to
// the definition (`kw_marker`), and the runtime compares the descriptor's address.

/// kenny-runtime abi.rs `kn_static_fill`: `expect` is a parameter descriptor's address.
const FILL_PARAMS: i64 = 1 << 62;

impl<M: Module> Compiler<M> {
    /// The marker appended to the parameter descriptor of a keyword `def` (none for others).
    pub(crate) fn kw_marker(&self, offset: usize) -> Option<Vec<u8>> {
        let defs = self.kw_defs.values().flatten();
        let _ = defs.clone().find(|d| d.offset == offset && d.file == self.file)?;
        Some(format!("kenny-inline:{}:{}\0", self.file.display(), offset).into_bytes())
    }

    /// The keyword `def` a call `name(npos positional, keys)` would inline, if any.
    pub(crate) fn kw_target(&self, fx: &Fx, name: &[u8], npos: usize, keys: &[Vec<u8>], explicit: bool) -> Option<(KwDef, std::rc::Rc<InlineDef>)> {
        if self.refine_scope.is_some() || self.in_dependency() || self.eval_unit || !may_be_ruby_method(name) || self.inline_ctx.len() >= MAX_INLINE_DEPTH {
            return None;
        }
        let defs = self.kw_defs.get(name)?;
        let fits = |d: &&KwDef| d.npos == npos && d.kws.len() == keys.len() && d.kws.iter().all(|k| keys.contains(k));
        let cands: Vec<&KwDef> = defs.iter().filter(fits).collect();
        let d = if explicit {
            match cands.as_slice() {
                [d] => *d,
                _ => return None,
            }
        } else {
            let here = self.cur_lex(fx).unwrap_or_default();
            let local: Vec<&&KwDef> = cands.iter().filter(|d| d.lex == here && d.file == self.file).collect();
            match (local.len(), cands.len()) {
                (1, _) => *local[0],
                (0, 1) => cands[0],
                _ => return None,
            }
        };
        let key = (d.file.clone(), d.offset);
        let idef = self.inline_defs.get(&key)?.clone();
        if idef.file != self.file || self.inline_ctx.iter().any(|f| f.key == key) || self.def_stack.contains(&key) {
            return None;
        }
        if idef.consts && self.cur_lex(fx).as_ref() != Some(&idef.lex) && !self.def_cref.contains_key(&key) {
            return None;
        }
        if idef.yields > 0 {
            return None;
        }
        Some((d.clone(), idef))
    }

    /// Inline the keyword call `kw_target` found (arguments evaluated: `pos`, and each
    /// keyword's value), with `dynamic` as the call when the guard fails.
    pub(crate) fn inline_kw_call(
        &mut self,
        fx: &mut Fx,
        target: (KwDef, std::rc::Rc<InlineDef>),
        recv: Value,
        mid: Value,
        pos: &[Value],
        kw: &[(Vec<u8>, Value)],
        explicit: bool,
        dynamic: impl FnOnce(&mut Self, &mut Fx) -> Value,
    ) -> Value {
        let (d, idef) = target;
        if std::env::var_os("KENNY_DEBUG_INLINE").is_some() {
            let line = self.line_of(self.cur_offset);
            eprintln!("kenny: inline {} (keywords) into {}:{} (depth {})", String::from_utf8_lossy(&idef.name), self.file.display(), line, self.inline_ctx.len() + 1);
        }
        let f = mem();
        let slot = self.cache_ptr(fx, 2);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        self.require_heap(fx, recv, slow);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let sk = fx.b.ins().load(I64, f, slot, 0);
        let ss = fx.b.ins().load(I64, f, slot, 8);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let d1 = fx.b.ins().bxor(k, sk);
        let d2 = fx.b.ins().bxor(ss, cur);
        let dd = fx.b.ins().bor(d1, d2);
        self.require_zero(fx, dd, slow);
        let r = self.inline_method_body(fx, &idef, recv, pos, kw, None);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let mut desc = crate::scopes::params_desc(idef.params()).into_bytes();
        desc.push(0);
        desc.extend(self.kw_marker(d.offset).unwrap_or_default());
        let (p, _) = self.str_ptr(fx, &desc);
        let tag = fx.b.ins().iconst(I64, FILL_PARAMS | if explicit { FILL_PUBLIC } else { 0 });
        let expect = fx.b.ins().bor(p, tag);
        self.call(fx, "kn_static_fill", &[slot, recv, mid, expect]);
        let r = dynamic(self, fx);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        r
    }
}
