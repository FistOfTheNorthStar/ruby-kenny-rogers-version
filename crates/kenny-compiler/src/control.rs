//! Control flow: if/unless, loops, case/when, begin/rescue/ensure, return/break/next.

use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, Block, InstBuilder};
use ruby_prism::*;

impl<M: Module> Compiler<M> {
    /// Branch to `t` when `node` is truthy, else to `e`. Comparisons, `!`, `&&`, `||` and
    /// parentheses branch directly (no Ruby true/false made and tested again).
    pub(crate) fn cond_br(&mut self, fx: &mut Fx, node: &Node, t: Block, e: Block) {
        if let Some(p) = node.as_parentheses_node() {
            if let Some(b) = p.body() {
                let single = match b.as_statements_node() {
                    Some(s) => {
                        let v: Vec<Node> = s.body().iter().collect();
                        if v.len() == 1 { v.into_iter().next() } else { None }
                    }
                    None => Some(b),
                };
                if let Some(inner) = single {
                    return self.cond_br(fx, &inner, t, e);
                }
            }
        }
        if let Some(a) = node.as_and_node() {
            let mid = fx.b.create_block();
            self.cond_br(fx, &a.left(), mid, e);
            fx.b.switch_to_block(mid);
            return self.cond_br(fx, &a.right(), t, e);
        }
        if let Some(o) = node.as_or_node() {
            let mid = fx.b.create_block();
            self.cond_br(fx, &o.left(), t, mid);
            fx.b.switch_to_block(mid);
            return self.cond_br(fx, &o.right(), t, e);
        }
        if let Some(c) = node.as_call_node() {
            let name = c.name().as_slice();
            // `!x` / `not x` (as `call_node` compiles it)
            if name == b"!" && c.arguments().is_none() && c.block().is_none() {
                if let Some(r) = c.receiver() {
                    return self.cond_br(fx, &r, e, t);
                }
            }
            // a numeric comparison (as `num_expr` compiles it, minus its special cases)
            if self.refine_scope.is_none() && matches!(name, b"<" | b"<=" | b">" | b">=" | b"==" | b"!=") && crate::fastpath::str_idx_eq_parts(&c).is_none() {
                if let Some((recv, arg)) = crate::fastpath::numeric_call(&c) {
                    let sym = recv.as_symbol_node().is_some() || arg.as_symbol_node().is_some();
                    if !(sym && matches!(name, b"==" | b"!=")) {
                        let op = name.to_vec();
                        let a = self.num_expr(fx, &recv, true);
                        let b = self.num_expr(fx, &arg, true);
                        self.cur_offset = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
                        return self.binop_branch(fx, &op, a, b, t, e);
                    }
                }
            }
        }
        let v = self.expr(fx, node);
        let c = self.truthy_br(fx, v);
        fx.b.ins().brif(c, t, &[], e, &[]);
    }

    pub(crate) fn if_node(&mut self, fx: &mut Fx, n: &IfNode) -> Value {
        let then_b = fx.b.create_block();
        let else_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        self.cond_br(fx, &n.predicate(), then_b, else_b);
        fx.b.switch_to_block(then_b);
        let v = self.stmts(fx, n.statements());
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(else_b);
        let v = match n.subsequent() {
            Some(s) => self.expr(fx, &s),
            None => self.nil(fx),
        };
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// `if` as a statement (its branches' statements too: `expr_void`).
    pub(crate) fn if_void(&mut self, fx: &mut Fx, n: &IfNode) -> Value {
        let then_b = fx.b.create_block();
        let else_b = fx.b.create_block();
        let done = fx.b.create_block();
        self.cond_br(fx, &n.predicate(), then_b, else_b);
        fx.b.switch_to_block(then_b);
        self.stmts_void(fx, n.statements());
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(else_b);
        if let Some(s) = n.subsequent() {
            match s.as_else_node() {
                Some(e) => self.stmts_void(fx, e.statements()),
                None => {
                    self.expr_void(fx, &s);
                }
            }
        }
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
        self.nil(fx)
    }
    pub(crate) fn unless_void(&mut self, fx: &mut Fx, n: &UnlessNode) -> Value {
        let then_b = fx.b.create_block();
        let else_b = fx.b.create_block();
        let done = fx.b.create_block();
        self.cond_br(fx, &n.predicate(), else_b, then_b);
        fx.b.switch_to_block(then_b);
        self.stmts_void(fx, n.statements());
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(else_b);
        if let Some(e) = n.else_clause() {
            self.stmts_void(fx, e.statements());
        }
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
        self.nil(fx)
    }

    pub(crate) fn unless_node(&mut self, fx: &mut Fx, n: &UnlessNode) -> Value {
        let then_b = fx.b.create_block();
        let else_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        self.cond_br(fx, &n.predicate(), else_b, then_b);
        fx.b.switch_to_block(then_b);
        let v = self.stmts(fx, n.statements());
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(else_b);
        let v = match n.else_clause() {
            Some(e) => self.stmts(fx, e.statements()),
            None => self.nil(fx),
        };
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// `lp`: the WhileNode / UntilNode, when the loop may be speculated on (spec.rs).
    pub(crate) fn while_loop(&mut self, fx: &mut Fx, lp: Option<&Node>, pred: &Node, body: Option<StatementsNode>, until: bool, do_while: bool) -> Value {
        if let (Some(lp), false) = (lp, do_while) {
            let mut core = |c: &mut Self, fx: &mut Fx, exit: Block| {
                let (p, b) = match lp.as_while_node() {
                    Some(w) => (w.predicate(), w.statements()),
                    None => {
                        let u = lp.as_until_node().unwrap();
                        (u.predicate(), u.statements())
                    }
                };
                c.while_core(fx, &p, b, until, false, exit);
            };
            if let Some(v) = self.spec_loop(fx, lp, &mut core) {
                return v;
            }
        }
        let exit = fx.b.create_block();
        fx.b.append_block_param(exit, I64);
        let saved = self.hoist_enter(fx, lp);
        self.while_core(fx, pred, body, until, do_while, exit);
        self.hoist_leave(fx, saved);
        fx.b.switch_to_block(exit);
        fx.b.block_params(exit)[0]
    }

    /// The loop itself, leaving through `exit` (an I64 parameter: the loop's value).
    fn while_core(&mut self, fx: &mut Fx, pred: &Node, body: Option<StatementsNode>, until: bool, do_while: bool, exit: Block) {
        let header = fx.b.create_block();
        let body_b = fx.b.create_block();
        let remembered = self.licm_enter(fx, pred, body.as_ref());
        fx.b.ins().jump(if do_while { body_b } else { header }, &[]);
        fx.b.switch_to_block(header);
        let out = fx.b.create_block();
        if until {
            self.cond_br(fx, pred, out, body_b);
        } else {
            self.cond_br(fx, pred, body_b, out);
        }
        fx.b.switch_to_block(out);
        let nil = self.nil(fx);
        fx.b.ins().jump(exit, &[nil.into()]);
        fx.b.switch_to_block(body_b);
        fx.loops.push(LoopCtx { next: header, exit, redo: body_b, next_val: false, rescue_depth: fx.rescue_prev.len() });
        self.stmts_void(fx, body);
        fx.loops.pop();
        self.safepoint(fx);
        fx.b.ins().jump(header, &[]);
        self.licm_leave(fx, remembered);
    }

    pub(crate) fn for_loop(&mut self, fx: &mut Fx, f: &ForNode) -> Value {
        let coll = self.expr(fx, &f.collection());
        let ary = self.callc(fx, "kn_splat_to_ary", &[coll]);
        let i = fx.b.declare_var(I64);
        let zero = self.iconst(fx, 0);
        fx.b.def_var(i, zero);
        let header = fx.b.create_block();
        let body_b = fx.b.create_block();
        let incr = fx.b.create_block();
        let exit = fx.b.create_block();
        fx.b.append_block_param(exit, I64);
        fx.b.ins().jump(header, &[]);
        fx.b.switch_to_block(header);
        let iv = fx.b.use_var(i);
        let len = self.call(fx, "kn_ary_len", &[ary]);
        let c = fx.b.ins().icmp(IntCC::SignedLessThan, iv, len);
        fx.b.ins().brif(c, body_b, &[], exit, &[coll.into()]);
        fx.b.switch_to_block(body_b);
        let iv = fx.b.use_var(i);
        let x = self.call(fx, "kn_ary_ref", &[ary, iv]);
        self.assign_target(fx, &f.index(), x);
        let body_start = fx.b.create_block();
        fx.b.ins().jump(body_start, &[]);
        fx.b.switch_to_block(body_start);
        fx.loops.push(LoopCtx { next: incr, exit, redo: body_start, next_val: false, rescue_depth: fx.rescue_prev.len() });
        self.stmts_void(fx, f.statements());
        fx.loops.pop();
        fx.b.ins().jump(incr, &[]);
        fx.b.switch_to_block(incr);
        let iv = fx.b.use_var(i);
        let n = fx.b.ins().iadd_imm_s(iv, 1);
        fx.b.def_var(i, n);
        self.safepoint(fx);
        fx.b.ins().jump(header, &[]);
        fx.b.switch_to_block(exit);
        fx.b.block_params(exit)[0]
    }

    pub(crate) fn case_node(&mut self, fx: &mut Fx, n: &CaseNode) -> Value {
        let subj = n.predicate().map(|p| self.expr(fx, &p));
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        for w in n.conditions().iter() {
            let Some(w) = w.as_when_node() else { continue };
            let body_b = fx.b.create_block();
            for cond in w.conditions().iter() {
                let at = cond.location().start_offset();
                let t = if let Some(sp) = cond.as_splat_node() {
                    // when *list  ->  list.any? { |x| x === subj }
                    let list = match sp.expression() {
                        Some(e) => self.expr(fx, &e),
                        None => self.nil(fx),
                    };
                    let ary = self.callc(fx, "kn_splat_to_ary", &[list]);
                    let s = match subj {
                        Some(s) => s,
                        None => {
                            self.error_at(at, "splat in `when` without a case subject");
                            self.nil(fx)
                        }
                    };
                    let r = self.send(fx, ary, b"__kn_case_any", &[s], None);
                    self.truthy(fx, r)
                } else {
                    match subj {
                        Some(s) => {
                            if cond.as_symbol_node().is_some() {
                                let cv = self.expr(fx, &cond);
                                fx.b.ins().icmp(IntCC::Equal, cv, s)
                            } else {
                                // `when SomeClass`: the constant through the inline cache
                                let r = match cond.as_constant_read_node() {
                                    Some(c) if self.refine_scope.is_none() => {
                                        let crefp = self.slot_ptr(fx, fx.cref_slot);
                                        let sym = self.sym_val(fx, c.name().as_slice());
                                        let cache = self.owned_cache_ptr(fx, OWNED_CONST);
                                        let cv = self.const_cached(fx, cache, crefp, sym);
                                        self.case_class_eqq(fx, cv, s)
                                    }
                                    _ => {
                                        let cv = self.expr(fx, &cond);
                                        self.send(fx, cv, b"===", &[s], None)
                                    }
                                };
                                self.truthy(fx, r)
                            }
                        }
                        None => {
                            let cv = self.expr(fx, &cond);
                            self.truthy(fx, cv)
                        }
                    }
                };
                let next = fx.b.create_block();
                fx.b.ins().brif(t, body_b, &[], next, &[]);
                fx.b.switch_to_block(next);
            }
            let fallthrough = fx.b.current_block().unwrap();
            fx.b.switch_to_block(body_b);
            let v = self.stmts(fx, w.statements());
            fx.b.ins().jump(done, &[v.into()]);
            fx.b.switch_to_block(fallthrough);
        }
        let v = match n.else_clause() {
            Some(e) => self.stmts(fx, e.statements()),
            None => self.nil(fx),
        };
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// Exception classes of a rescue clause (default StandardError).
    fn rescue_classes(&mut self, fx: &mut Fx, r: &RescueNode) -> Vec<Value> {
        let ex: Vec<Node> = r.exceptions().iter().collect();
        if ex.is_empty() {
            let cref = self.cref(fx);
            let s = self.sym_val(fx, b"StandardError");
            return vec![self.callc(fx, "kn_const_get", &[cref, s])];
        }
        ex.iter()
            .map(|e| {
                if let Some(sp) = e.as_splat_node() {
                    let v = match sp.expression() {
                        Some(x) => self.expr(fx, &x),
                        None => self.nil(fx),
                    };
                    self.callc(fx, "kn_splat_to_ary", &[v])
                } else {
                    self.expr(fx, e)
                }
            })
            .collect()
    }

    pub(crate) fn begin_node(&mut self, fx: &mut Fx, n: &BeginNode) -> Value {
        let rescue = n.rescue_clause();
        let ensure = n.ensure_clause();
        if rescue.is_none() && ensure.is_none() {
            let v = self.stmts(fx, n.statements());
            return match n.else_clause() {
                Some(e) => self.stmts(fx, e.statements()),
                None => v,
            };
        }
        let after = fx.b.create_block();
        fx.b.append_block_param(after, I64);
        let ens_unwind = ensure.as_ref().map(|_| fx.b.create_block());
        if let Some(eu) = ens_unwind {
            fx.unwind.push(eu);
            fx.ensure_depth += 1;
        }

        let v = match rescue {
            Some(first) => {
                let body_start = fx.b.create_block();
                let normal = fx.b.create_block();
                fx.b.append_block_param(normal, I64);
                fx.b.ins().jump(body_start, &[]);
                fx.b.switch_to_block(body_start);
                let resc_bb = fx.b.create_block();
                fx.unwind.push(resc_bb);
                let v = self.stmts(fx, n.statements());
                fx.unwind.pop();
                let v = match n.else_clause() {
                    Some(e) => self.stmts(fx, e.statements()),
                    None => v,
                };
                fx.b.ins().jump(normal, &[v.into()]);

                fx.b.switch_to_block(resc_bb);
                let exc = self.call(fx, "kn_pending_exc", &[]);
                let is_nil = fx.b.ins().icmp_imm_s(IntCC::Equal, exc, QNIL);
                let check_bb = fx.b.create_block();
                let outer = *fx.unwind.last().unwrap();
                fx.b.ins().brif(is_nil, outer, &[], check_bb, &[]);
                fx.b.switch_to_block(check_bb);
                // matching rescue clauses in order
                let mut clause = Some(first);
                while let Some(r) = clause {
                    let classes = self.rescue_classes(fx, &r);
                    let body_bb = fx.b.create_block();
                    for cls in classes {
                        let m = self.callc(fx, "kn_exc_match", &[exc, cls]);
                        let t = self.truthy_br(fx, m);
                        let next = fx.b.create_block();
                        fx.b.ins().brif(t, body_bb, &[], next, &[]);
                        fx.b.switch_to_block(next);
                    }
                    let nomatch = fx.b.current_block().unwrap();
                    fx.b.switch_to_block(body_bb);
                    let prev = self.call(fx, "kn_errinfo", &[]);
                    self.call(fx, "kn_rescued", &[exc]);
                    // an exception (or a throw) leaving the rescue body restores `$!` first
                    let restore_bb = fx.b.create_block();
                    fx.unwind.push(restore_bb);
                    fx.rescue_prev.push(prev);
                    if let Some(reference) = r.reference() {
                        self.assign_target(fx, &reference, exc);
                    }
                    fx.retry.push(body_start);
                    let rv = self.stmts(fx, r.statements());
                    fx.retry.pop();
                    fx.rescue_prev.pop();
                    fx.unwind.pop();
                    self.call(fx, "kn_errinfo_restore", &[prev]);
                    fx.b.ins().jump(normal, &[rv.into()]);
                    fx.b.switch_to_block(restore_bb);
                    self.call(fx, "kn_errinfo_restore", &[prev]);
                    let outer = *fx.unwind.last().unwrap();
                    fx.b.ins().jump(outer, &[]);
                    fx.b.switch_to_block(nomatch);
                    clause = r.subsequent();
                }
                let outer = *fx.unwind.last().unwrap();
                fx.b.ins().jump(outer, &[]);
                fx.b.switch_to_block(normal);
                fx.b.block_params(normal)[0]
            }
            None => self.stmts(fx, n.statements()),
        };

        match (ensure, ens_unwind) {
            (Some(e), Some(eu)) => {
                fx.unwind.pop();
                fx.ensure_depth -= 1;
                self.stmts(fx, e.statements());
                fx.b.ins().jump(after, &[v.into()]);
                // unwinding path: run ensure, then keep unwinding
                fx.b.switch_to_block(eu);
                let h = self.call(fx, "kn_save_pending", &[]);
                self.stmts(fx, e.statements());
                let again = self.call(fx, "kn_restore_pending", &[h]);
                let outer = *fx.unwind.last().unwrap();
                let nil = self.nil(fx);
                fx.b.ins().brif(again, outer, &[], after, &[nil.into()]);
            }
            _ => {
                fx.b.ins().jump(after, &[v.into()]);
            }
        }
        fx.b.switch_to_block(after);
        fx.b.block_params(after)[0]
    }

    pub(crate) fn rescue_modifier(&mut self, fx: &mut Fx, n: &RescueModifierNode) -> Value {
        let normal = fx.b.create_block();
        fx.b.append_block_param(normal, I64);
        let resc_bb = fx.b.create_block();
        fx.unwind.push(resc_bb);
        let v = self.expr(fx, &n.expression());
        fx.unwind.pop();
        fx.b.ins().jump(normal, &[v.into()]);
        fx.b.switch_to_block(resc_bb);
        let exc = self.call(fx, "kn_pending_exc", &[]);
        let is_nil = fx.b.ins().icmp_imm_s(IntCC::Equal, exc, QNIL);
        let check_bb = fx.b.create_block();
        let outer = *fx.unwind.last().unwrap();
        fx.b.ins().brif(is_nil, outer, &[], check_bb, &[]);
        fx.b.switch_to_block(check_bb);
        let cref = self.cref(fx);
        let s = self.sym_val(fx, b"StandardError");
        let cls = self.callc(fx, "kn_const_get", &[cref, s]);
        let m = self.callc(fx, "kn_exc_match", &[exc, cls]);
        let t = self.truthy_br(fx, m);
        let body_bb = fx.b.create_block();
        let outer = *fx.unwind.last().unwrap();
        fx.b.ins().brif(t, body_bb, &[], outer, &[]);
        fx.b.switch_to_block(body_bb);
        let prev = self.call(fx, "kn_errinfo", &[]);
        self.call(fx, "kn_rescued", &[exc]);
        let restore_bb = fx.b.create_block();
        fx.unwind.push(restore_bb);
        fx.rescue_prev.push(prev);
        let rv = self.expr(fx, &n.rescue_expression());
        fx.rescue_prev.pop();
        fx.unwind.pop();
        self.call(fx, "kn_errinfo_restore", &[prev]);
        fx.b.ins().jump(normal, &[rv.into()]);
        fx.b.switch_to_block(restore_bb);
        self.call(fx, "kn_errinfo_restore", &[prev]);
        let outer = *fx.unwind.last().unwrap();
        fx.b.ins().jump(outer, &[]);
        fx.b.switch_to_block(normal);
        fx.b.block_params(normal)[0]
    }

    fn jump_value(&mut self, fx: &mut Fx, args: Option<ArgumentsNode>) -> Value {
        let items: Vec<Node> = match &args {
            Some(a) => a.arguments().iter().collect(),
            None => vec![],
        };
        if items.len() == 1 && items[0].as_splat_node().is_none() {
            return self.expr(fx, &items[0]);
        }
        if items.is_empty() {
            return self.nil(fx);
        }
        let a = self.eval_args(fx, args);
        match a.splat {
            Some(ary) => ary,
            None => {
                let n = self.iconst(fx, a.vals.len() as i64);
                let buf = self.args_buf(fx, &a.vals);
                self.call(fx, "kn_ary_new", &[n, buf])
            }
        }
    }

    pub(crate) fn return_node(&mut self, fx: &mut Fx, args: Option<ArgumentsNode>) -> Value {
        let v = self.jump_value(fx, args);
        match fx.kind {
            // through the `ensure`s to this function's own unwind path
            ScopeKind::Method if fx.ensure_depth > 0 => {
                let tag = fx.b.use_var(fx.tag_v);
                self.call(fx, "kn_throw_return", &[tag, v]);
                self.jump_unwind(fx);
            }
            ScopeKind::Block if fx.ret_here && fx.ensure_depth > 0 => {
                let tag = fx.b.use_var(fx.tag_v);
                self.call(fx, "kn_throw_return", &[tag, v]);
                self.jump_unwind(fx);
            }
            ScopeKind::Block if !fx.lambda && !fx.ret_here => {
                // to the enclosing lambda / method body block, else the method
                let tag = match fx.ret_hops {
                    Some(h) => {
                        let mut p = fx.b.use_var(fx.penv_v.expect("a block's parent env"));
                        for _ in 0..h {
                            p = fx.b.ins().load(I64, mem(), p, ENV_PARENT);
                        }
                        p
                    }
                    None => match self.method_env(fx) {
                        Some(e) => e,
                        None => self.iconst(fx, 0),
                    },
                };
                self.call(fx, "kn_throw_return", &[tag, v]);
                self.jump_unwind(fx);
            }
            _ => {
                self.restore_errinfo_to(fx, 0);
                fx.b.ins().return_(&[v]);
                self.dead_block(fx);
            }
        }
        self.nil(fx)
    }

    /// Before a jump out of the rescue bodies entered since `depth`: restore `$!`.
    pub(crate) fn restore_errinfo_to(&mut self, fx: &mut Fx, depth: usize) {
        if let Some(&prev) = fx.rescue_prev.get(depth) {
            self.call(fx, "kn_errinfo_restore", &[prev]);
        }
    }

    pub(crate) fn break_node(&mut self, fx: &mut Fx, args: Option<ArgumentsNode>, at: usize) -> Value {
        let v = self.jump_value(fx, args);
        if let Some(l) = fx.loops.last() {
            let (exit, depth) = (l.exit, l.rescue_depth);
            self.restore_errinfo_to(fx, depth);
            fx.b.ins().jump(exit, &[v.into()]);
            self.dead_block(fx);
        } else if fx.kind == ScopeKind::Block {
            if fx.lambda {
                self.restore_errinfo_to(fx, 0);
                fx.b.ins().return_(&[v]);
                self.dead_block(fx);
            } else {
                let p = fx.b.use_var(fx.proc_v.unwrap());
                self.call(fx, "kn_throw_break", &[p, v]);
                self.jump_unwind(fx);
            }
        } else {
            self.error_at(at, "break outside of a loop or block");
        }
        self.nil(fx)
    }

    pub(crate) fn next_node(&mut self, fx: &mut Fx, args: Option<ArgumentsNode>, at: usize) -> Value {
        let v = self.jump_value(fx, args);
        if let Some(l) = fx.loops.last() {
            let (next, depth, next_val) = (l.next, l.rescue_depth, l.next_val);
            self.restore_errinfo_to(fx, depth);
            if next_val {
                fx.b.ins().jump(next, &[v.into()]);
            } else {
                fx.b.ins().jump(next, &[]);
            }
            self.dead_block(fx);
        } else if fx.kind == ScopeKind::Block {
            self.restore_errinfo_to(fx, 0);
            fx.b.ins().return_(&[v]);
            self.dead_block(fx);
        } else {
            self.error_at(at, "next outside of a loop or block");
        }
        self.nil(fx)
    }
}
