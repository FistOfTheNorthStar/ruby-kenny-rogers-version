//! Objects that never leave a method stay unallocated ("virtual"): for a local `x` assigned
//! `C.new(args)` and otherwise only read as the receiver of C's getters (`x.a`, where `a`
//! is an `attr_reader`/`attr_accessor` or a `def a; @a; end` of C's class body), the
//! assignment runs `initialize` inline with its instance variables as plain variables, and
//! the getters read those.
//!
//! Correctness doesn't depend on the analysis: a run-time flag (`isv`) says whether `x`
//! holds a virtual object. Anything else that reads `x` first builds the real object
//! (`materialize`: allocate, set the instance variables in `initialize`'s order) and from
//! then on `x` is that object; any other assignment to `x` clears the flag. A getter reads
//! its field only while the method serial is the one seen when the object was made (a
//! method definition anywhere may have changed the getter); otherwise it materializes and
//! makes the call.
//!
//! The assignment itself is guarded like an inlined `C.new` (statics.rs `inline_new`):
//! [C, serial] in a slot that `kn_vobj_fill` fills once C's `new` is the builtin one, its
//! `initialize` is that `def` and each getter is the one the analysis assumed. Else the
//! assignment makes a real object (`C.new` as usual).
//!
//! Only `initialize` bodies that just assign instance variables from expressions not
//! involving `self` qualify, so nothing can see the object while it is being built.

use crate::calls::Args;
use crate::statics::InlineDef;
use crate::*;
use ruby_prism::*;
use std::rc::Rc;

/// A getter of a class: the method name -> (instance variable read, offset of its `def`;
/// None for an attr_reader).
pub(crate) type Getters = HashMap<Vec<u8>, (Vec<u8>, Option<usize>)>;

/// A local that may hold a virtual object.
pub(crate) struct VLocal {
    /// the constant `C` of its `C.new`
    pub class: Vec<u8>,
    pub nargs: usize,
    /// `initialize`'s instance variables in the order it assigns them
    pub fields: Vec<(Vec<u8>, Variable)>,
    /// the getters its reads use: (method, instance variable, `def` offset)
    pub getters: Vec<(Vec<u8>, Vec<u8>, Option<usize>)>,
    /// 1 while the local holds a virtual object
    pub isv: Variable,
    /// the method serial when it was made
    pub ser0: Variable,
    /// its class (for `materialize`)
    pub cls: Variable,
}

/// Uses of the function's own (depth 0) locals.
#[derive(Default)]
struct Uses {
    /// `x = C.new(args)` statements: (C, argument count)
    news: HashMap<Vec<u8>, Vec<(Vec<u8>, usize)>>,
    /// `x.m` with no arguments: the method names
    gets: HashMap<Vec<u8>, Vec<Vec<u8>>>,
    /// any other reference (reads, other assignments, parameters)
    other: HashSet<Vec<u8>>,
    /// `binding`, `eval`, `local_variables`...: locals are visible to other code
    opaque: bool,
    nest: u32,
}

/// `C.new(a, b)`: (C, argument count) when it has only plain positional arguments.
fn plain_new(n: &Node) -> Option<(Vec<u8>, usize)> {
    let c = n.as_call_node()?;
    if c.name().as_slice() != b"new" || c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let k = c.receiver()?.as_constant_read_node()?;
    let mut n = 0;
    if let Some(a) = c.arguments() {
        for x in a.arguments().iter() {
            if x.as_splat_node().is_some() || x.as_keyword_hash_node().is_some() || x.as_block_argument_node().is_some() || x.as_forwarding_arguments_node().is_some() {
                return None;
            }
            n += 1;
        }
    }
    Some((k.name().as_slice().to_vec(), n))
}

impl Uses {
    fn statements(&mut self, s: &StatementsNode) {
        for n in s.body().iter() {
            if self.nest == 0 {
                if let Some(w) = n.as_local_variable_write_node() {
                    if w.depth() == 0 {
                        if let Some(cn) = plain_new(&w.value()) {
                            self.news.entry(w.name().as_slice().to_vec()).or_default().push(cn);
                            // (the arguments may use other locals)
                            if let Some(a) = w.value().as_call_node().and_then(|c| c.arguments()) {
                                self.visit(&a.as_node());
                            }
                            continue;
                        }
                    }
                }
            }
            self.visit(&n);
        }
    }
}

impl<'pr> Visit<'pr> for Uses {
    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        self.statements(node);
    }
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if matches!(node.name().as_slice(), b"binding" | b"eval" | b"local_variables" | b"instance_eval" | b"instance_exec" | b"class_eval" | b"module_eval") {
            self.opaque = true;
        }
        if self.nest == 0 && node.arguments().is_none() && node.block().is_none() && !node.is_safe_navigation() {
            if let Some(l) = node.receiver().and_then(|r| r.as_local_variable_read_node()) {
                if l.depth() == 0 {
                    self.gets.entry(l.name().as_slice().to_vec()).or_default().push(node.name().as_slice().to_vec());
                    return;
                }
            }
        }
        visit_call_node(self, node);
    }
    fn visit_local_variable_read_node(&mut self, node: &LocalVariableReadNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
    }
    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_write_node(self, node);
    }
    fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
    }
    fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_operator_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        if node.depth() == self.nest {
            self.other.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_and_write_node(self, node);
    }
    fn visit_required_parameter_node(&mut self, node: &RequiredParameterNode<'pr>) {
        self.other.insert(node.name().as_slice().to_vec());
    }
    fn visit_optional_parameter_node(&mut self, node: &OptionalParameterNode<'pr>) {
        self.other.insert(node.name().as_slice().to_vec());
        visit_optional_parameter_node(self, node);
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
    fn visit_def_node(&mut self, _node: &DefNode<'pr>) {}
    fn visit_class_node(&mut self, _node: &ClassNode<'pr>) {}
    fn visit_module_node(&mut self, _node: &ModuleNode<'pr>) {}
    fn visit_singleton_class_node(&mut self, _node: &SingletonClassNode<'pr>) {}
}

/// Does an `initialize` body only assign instance variables from expressions that don't
/// involve `self`? Then its instance variables, in assignment order.
fn init_fields(d: &InlineDef) -> Option<Vec<Vec<u8>>> {
    struct NoSelf {
        ok: bool,
    }
    impl<'pr> Visit<'pr> for NoSelf {
        fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
            self.check(&node);
        }
        fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
            self.check(&node);
        }
    }
    impl NoSelf {
        fn check(&mut self, node: &Node) {
            let bad = match node {
                Node::SelfNode { .. }
                | Node::InstanceVariableReadNode { .. }
                | Node::InstanceVariableWriteNode { .. }
                | Node::InstanceVariableOperatorWriteNode { .. }
                | Node::InstanceVariableOrWriteNode { .. }
                | Node::InstanceVariableAndWriteNode { .. }
                | Node::InstanceVariableTargetNode { .. }
                | Node::YieldNode { .. }
                | Node::SuperNode { .. }
                | Node::ForwardingSuperNode { .. }
                | Node::ReturnNode { .. } => true,
                Node::CallNode { .. } => node.as_call_node().unwrap().receiver().is_none(),
                _ => false,
            };
            if bad {
                self.ok = false;
            }
        }
    }
    let mut fields: Vec<Vec<u8>> = Vec::new();
    let Some(body) = d.node.body() else { return Some(fields) };
    let s = body.as_statements_node()?;
    for st in s.body().iter() {
        let w = st.as_instance_variable_write_node()?;
        let mut ns = NoSelf { ok: true };
        ns.visit(&w.value());
        if !ns.ok {
            return None;
        }
        let name = w.name().as_slice().to_vec();
        if !fields.contains(&name) {
            fields.push(name);
        }
    }
    Some(fields)
}

impl<M: Module> Compiler<M> {
    /// The `initialize` an inlined `C.new` with `n` arguments would run (statics.rs
    /// `inline_new` picks the same one).
    pub(crate) fn init_target(&self, cname: &[u8], n: usize) -> Option<(crate::statics::StaticDef, Rc<InlineDef>)> {
        let defs = self.static_defs.get(&b"initialize"[..])?;
        let mut c = defs.iter().filter(|d| d.arity == n && d.lex.last().is_some_and(|l| l.as_slice() == cname || l.ends_with(&[b"::", cname].concat())));
        let sd = c.next()?.clone();
        if c.next().is_some() {
            return None;
        }
        let idef = self.inline_defs.get(&(sd.file.clone(), sd.offset))?.clone();
        if idef.file != self.file || idef.consts {
            return None;
        }
        Some((sd, idef))
    }

    /// The locals of a function (its analysis nodes) that may hold virtual objects, with
    /// their variables declared (and defined: not virtual) at the current point of `b`.
    pub(crate) fn vobj_plan(&self, b: &mut FunctionBuilder, locals: &HashMap<Vec<u8>, Local>, nodes: &[&Node]) -> HashMap<Vec<u8>, Rc<VLocal>> {
        let mut out = HashMap::new();
        if self.refine_scope.is_some() || self.in_dependency() || self.eval_unit || std::env::var_os("KENNY_NO_VOBJ").is_some() {
            return out;
        }
        let mut u = Uses::default();
        for n in nodes {
            u.visit(n);
        }
        if u.opaque {
            return out;
        }
        for (name, news) in &u.news {
            if u.other.contains(name) || !matches!(locals.get(name), Some(Local::Var(_))) {
                continue;
            }
            let (cname, nargs) = news[0].clone();
            if news.iter().any(|x| *x != (cname.clone(), nargs)) {
                continue;
            }
            let Some(gets) = u.gets.get(name) else { continue };
            let Some(cg) = self.class_getters.get(&cname) else { continue };
            let mut getters: Vec<(Vec<u8>, Vec<u8>, Option<usize>)> = Vec::new();
            let mut ok = true;
            for m in gets {
                match cg.get(m) {
                    Some((iv, off)) if !iv.is_empty() => {
                        if !getters.iter().any(|g| &g.0 == m) {
                            getters.push((m.clone(), iv.clone(), *off));
                        }
                    }
                    _ => ok = false,
                }
            }
            if !ok {
                continue;
            }
            let Some((_, idef)) = self.init_target(&cname, nargs) else { continue };
            let Some(fnames) = init_fields(&idef) else { continue };
            let nil = b.ins().iconst(I64, QNIL);
            let zero = b.ins().iconst(I64, 0);
            let mut fields = Vec::new();
            for f in fnames {
                let v = b.declare_var(I64);
                b.def_var(v, nil);
                fields.push((f, v));
            }
            let mk = |b: &mut FunctionBuilder, init: Value| {
                let v = b.declare_var(I64);
                b.def_var(v, init);
                v
            };
            let isv = mk(b, zero);
            let ser0 = mk(b, zero);
            let cls = mk(b, nil);
            if std::env::var_os("KENNY_DEBUG_VOBJ").is_some() {
                eprintln!("kenny: virtual {} = {}.new in {}", String::from_utf8_lossy(name), String::from_utf8_lossy(&cname), self.file.display());
            }
            out.insert(name.clone(), Rc::new(VLocal { class: cname, nargs, fields, getters, isv, ser0, cls }));
        }
        out
    }

    /// `x = C.new(args)` as a statement, for a local of the plan.
    pub(crate) fn vobj_new(&mut self, fx: &mut Fx, name: &[u8], call: &CallNode) {
        let vl = fx.vlocals.get(name).unwrap().clone();
        let at = call.location().start_offset();
        let recv = self.expr(fx, &call.receiver().unwrap());
        let mut vals = Vec::new();
        if let Some(a) = call.arguments() {
            for x in a.arguments().iter() {
                vals.push(self.expr(fx, &x));
            }
        }
        self.cur_offset = at;
        let target = (vals.len() == vl.nargs && self.inline_ctx.len() < 2).then(|| self.init_target(&vl.class, vals.len())).flatten();
        let Some((sd, idef)) = target else {
            let r = self.send_raw(fx, recv, b"new", &Args { vals: vals.clone(), splat: None }, None, true);
            self.check(fx, r);
            self.local_set(fx, name, 0, r);
            return;
        };
        let f = mem();
        let slot = self.cache_ptr(fx, 2);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.set_cold_block(slow);
        let sk = fx.b.ins().load(I64, f, slot, 0);
        let ss = fx.b.ins().load(I64, f, slot, 8);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let d1 = fx.b.ins().bxor(recv, sk);
        let d2 = fx.b.ins().bxor(ss, cur);
        let d = fx.b.ins().bor(d1, d2);
        let same = fx.b.ins().icmp_imm_s(IntCC::Equal, d, 0);
        self.require(fx, same, slow);
        // virtual: `initialize` with its instance variables as variables
        let nil = self.nil(fx);
        for (_, v) in &vl.fields {
            fx.b.def_var(*v, nil);
        }
        fx.vself_next = Some(vl.fields.iter().cloned().collect());
        self.inline_method_body_pub(fx, &idef, nil, &vals);
        fx.vself_next = None;
        let one = fx.b.ins().iconst(I64, 1);
        fx.b.def_var(vl.isv, one);
        fx.b.def_var(vl.ser0, cur);
        fx.b.def_var(vl.cls, recv);
        fx.b.ins().jump(done, &[]);
        // a real object (and a chance to fill the slot)
        fx.b.switch_to_block(slow);
        let mut desc = Vec::new();
        for (m, iv, off) in &vl.getters {
            desc.push(self.sym_val(fx, m));
            let e = match off.and_then(|o| self.static_defs.get(m).and_then(|ds| ds.iter().find(|d| d.offset == o && d.file == self.file && d.arity == 0)).map(|d| d.fid)) {
                Some(fid) => {
                    let fa = self.fn_addr(fx, fid);
                    let tag = fx.b.ins().iconst(I64, 1 << 56);
                    fx.b.ins().bor(fa, tag)
                }
                // an attr_reader of that instance variable (a Symbol, never an address)
                None => self.sym_val(fx, iv),
            };
            desc.push(e);
        }
        let buf = self.args_buf(fx, &desc);
        let n = fx.b.ins().iconst(I64, vl.getters.len() as i64);
        let fa = self.fn_addr(fx, sd.fid);
        let tag = fx.b.ins().iconst(I64, ((vals.len() as i64) + 1) << 56);
        let expect = fx.b.ins().bor(fa, tag);
        self.call(fx, "kn_vobj_fill", &[slot, recv, expect, buf, n]);
        self.cur_offset = at;
        let r = self.send_raw(fx, recv, b"new", &Args { vals: vals.clone(), splat: None }, None, true);
        self.check(fx, r);
        self.local_set(fx, name, 0, r);
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
    }

    /// The real object of a planned local (building it first if it is virtual).
    pub(crate) fn vobj_materialize(&mut self, fx: &mut Fx, name: &[u8]) {
        let vl = fx.vlocals.get(name).unwrap().clone();
        let Some(Local::Var(xv)) = fx.locals.get(name) else { return };
        let xv = *xv;
        let isv = fx.b.use_var(vl.isv);
        let mb = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.set_cold_block(mb);
        fx.b.ins().brif(isv, mb, &[], done, &[]);
        fx.b.switch_to_block(mb);
        let cls = fx.b.use_var(vl.cls);
        let o = self.call(fx, "kn_obj_alloc", &[cls]);
        for (iv, var) in &vl.fields {
            let v = fx.b.use_var(*var);
            let id = self.sym_val(fx, iv);
            let cache = self.cache_ptr(fx, 2);
            self.call(fx, "kn_ivar_set", &[o, id, v, cache]);
        }
        fx.b.def_var(xv, o);
        let zero = fx.b.ins().iconst(I64, 0);
        fx.b.def_var(vl.isv, zero);
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
    }

    /// `x.m` for a planned local and one of its getters: the field while `x` is virtual and
    /// the getter can't have changed, else the call on the real object.
    pub(crate) fn vobj_get(&mut self, fx: &mut Fx, name: &[u8], m: &[u8]) -> Option<Value> {
        let vl = fx.vlocals.get(name)?.clone();
        let (_, iv, _) = vl.getters.iter().find(|g| g.0 == m)?.clone();
        let f = mem();
        let isv = fx.b.use_var(vl.isv);
        let ser0 = fx.b.use_var(vl.ser0);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let a = fx.b.ins().bxor_imm_s(isv, 1);
        let b2 = fx.b.ins().bxor(cur, ser0);
        let bad = fx.b.ins().bor(a, b2);
        let fast = fx.b.create_block();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        fx.b.ins().brif(bad, slow, &[], fast, &[]);
        fx.b.switch_to_block(fast);
        let v = match vl.fields.iter().find(|(f, _)| *f == iv) {
            Some((_, var)) => fx.b.use_var(*var),
            None => self.nil(fx),
        };
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(slow);
        let o = self.local_get(fx, name, 0);
        let r = self.send_raw(fx, o, m, &Args { vals: Vec::new(), splat: None }, None, true);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        Some(r)
    }
}
