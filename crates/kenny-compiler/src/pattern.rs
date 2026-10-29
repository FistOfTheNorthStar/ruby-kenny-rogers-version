//! Pattern matching: `case/in`, `expr in pattern`, `expr => pattern`.
//!
//! A pattern compiles to code that falls through when it matches and jumps to `fail`
//! otherwise. Bindings are assigned as matching proceeds (like CRuby).

use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, Block, InstBuilder};
use ruby_prism::*;

impl<M: Module> Compiler<M> {
    fn fail_unless(&mut self, fx: &mut Fx, cond: Value, fail: Block) {
        let ok = fx.b.create_block();
        fx.b.ins().brif(cond, ok, &[], fail, &[]);
        fx.b.switch_to_block(ok);
    }
    fn fail_unless_truthy(&mut self, fx: &mut Fx, v: Value, fail: Block) {
        let t = self.truthy(fx, v);
        self.fail_unless(fx, t, fail);
    }
    fn case_eq(&mut self, fx: &mut Fx, pat: Value, v: Value, fail: Block) {
        let r = self.send(fx, pat, b"===", &[v], None);
        self.fail_unless_truthy(fx, r, fail);
    }

    pub(crate) fn pattern(&mut self, fx: &mut Fx, pat: &Node, v: Value, fail: Block) {
        let at = pat.location().start_offset();
        if let Some(t) = pat.as_local_variable_target_node() {
            self.local_set(fx, t.name().as_slice(), t.depth(), v);
            return;
        }
        if let Some(i) = pat.as_implicit_node() {
            // `in {name:}` binds a local named after the key
            self.pattern(fx, &i.value(), v, fail);
            return;
        }
        if let Some(c) = pat.as_capture_pattern_node() {
            self.pattern(fx, &c.value(), v, fail);
            let t = c.target();
            self.local_set(fx, t.name().as_slice(), t.depth(), v);
            return;
        }
        if let Some(a) = pat.as_alternation_pattern_node() {
            let ok = fx.b.create_block();
            let try_right = fx.b.create_block();
            self.pattern(fx, &a.left(), v, try_right);
            fx.b.ins().jump(ok, &[]);
            fx.b.switch_to_block(try_right);
            self.pattern(fx, &a.right(), v, fail);
            fx.b.ins().jump(ok, &[]);
            fx.b.switch_to_block(ok);
            return;
        }
        if let Some(p) = pat.as_pinned_variable_node() {
            let pv = self.expr(fx, &p.variable());
            self.case_eq(fx, pv, v, fail);
            return;
        }
        if let Some(p) = pat.as_pinned_expression_node() {
            let pv = self.expr(fx, &p.expression());
            self.case_eq(fx, pv, v, fail);
            return;
        }
        if let Some(ap) = pat.as_array_pattern_node() {
            if let Some(c) = ap.constant() {
                let cv = self.expr(fx, &c);
                self.case_eq(fx, cv, v, fail);
            }
            let d = self.callc(fx, "kn_pm_deconstruct", &[v]);
            let is_nil = fx.b.ins().icmp_imm_s(IntCC::NotEqual, d, QNIL);
            self.fail_unless(fx, is_nil, fail);
            let reqs: Vec<Node> = ap.requireds().iter().collect();
            let posts: Vec<Node> = ap.posts().iter().collect();
            let len = self.call(fx, "kn_ary_len", &[d]);
            let n = (reqs.len() + posts.len()) as i64;
            let ok = match ap.rest() {
                Some(_) => fx.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, len, n),
                None => fx.b.ins().icmp_imm_s(IntCC::Equal, len, n),
            };
            self.fail_unless(fx, ok, fail);
            for (i, r) in reqs.iter().enumerate() {
                let iv = self.iconst(fx, i as i64);
                let e = self.call(fx, "kn_ary_ref", &[d, iv]);
                self.pattern(fx, r, e, fail);
            }
            if let Some(rest) = ap.rest() {
                if let Some(sp) = rest.as_splat_node() {
                    if let Some(t) = sp.expression() {
                        let st = self.iconst(fx, reqs.len() as i64);
                        let po = self.iconst(fx, posts.len() as i64);
                        let slice = self.call(fx, "kn_ary_slice_rest", &[d, st, po]);
                        self.pattern(fx, &t, slice, fail);
                    }
                }
            }
            for (k, p) in posts.iter().enumerate() {
                let pre = self.iconst(fx, reqs.len() as i64);
                let po = self.iconst(fx, posts.len() as i64);
                let kk = self.iconst(fx, k as i64);
                let e = self.call(fx, "kn_ary_post_ref", &[d, pre, po, kk]);
                self.pattern(fx, p, e, fail);
            }
            return;
        }
        if let Some(fp) = pat.as_find_pattern_node() {
            if let Some(c) = fp.constant() {
                let cv = self.expr(fx, &c);
                self.case_eq(fx, cv, v, fail);
            }
            let d = self.callc(fx, "kn_pm_deconstruct", &[v]);
            let not_nil = fx.b.ins().icmp_imm_s(IntCC::NotEqual, d, QNIL);
            self.fail_unless(fx, not_nil, fail);
            let reqs: Vec<Node> = fp.requireds().iter().collect();
            let n = reqs.len() as i64;
            let len = self.call(fx, "kn_ary_len", &[d]);
            // try each offset i in 0..=len-n
            let i = fx.b.declare_var(I64);
            let zero = self.iconst(fx, 0);
            fx.b.def_var(i, zero);
            let head = fx.b.create_block();
            let attempt = fx.b.create_block();
            let next = fx.b.create_block();
            let found = fx.b.create_block();
            fx.b.ins().jump(head, &[]);
            fx.b.switch_to_block(head);
            let iv = fx.b.use_var(i);
            let room = fx.b.ins().isub(len, iv);
            let fits = fx.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, room, n);
            fx.b.ins().brif(fits, attempt, &[], fail, &[]);
            fx.b.switch_to_block(attempt);
            for (k, r) in reqs.iter().enumerate() {
                let iv = fx.b.use_var(i);
                let idx = fx.b.ins().iadd_imm_s(iv, k as i64);
                let e = self.call(fx, "kn_ary_ref", &[d, idx]);
                self.pattern(fx, r, e, next);
            }
            fx.b.ins().jump(found, &[]);
            fx.b.switch_to_block(next);
            let iv = fx.b.use_var(i);
            let i2 = fx.b.ins().iadd_imm_s(iv, 1);
            fx.b.def_var(i, i2);
            fx.b.ins().jump(head, &[]);
            fx.b.switch_to_block(found);
            // bind the leading/trailing splats
            if let Some(t) = fp.left().expression() {
                let iv = fx.b.use_var(i);
                let zero = self.iconst(fx, 0);
                let slice = self.call(fx, "kn_pm_slice", &[d, zero, iv]);
                self.pattern(fx, &t, slice, fail);
            }
            if let Some(t) = fp.right().as_splat_node().and_then(|s| s.expression()) {
                let iv = fx.b.use_var(i);
                let st = fx.b.ins().iadd_imm_s(iv, n);
                let slice = self.call(fx, "kn_pm_slice", &[d, st, len]);
                self.pattern(fx, &t, slice, fail);
            }
            return;
        }
        if let Some(hp) = pat.as_hash_pattern_node() {
            if let Some(c) = hp.constant() {
                let cv = self.expr(fx, &c);
                self.case_eq(fx, cv, v, fail);
            }
            let elems: Vec<Node> = hp.elements().iter().collect();
            let keys: Vec<Vec<u8>> = elems
                .iter()
                .filter_map(|e| e.as_assoc_node().and_then(|a| a.key().as_symbol_node().map(|s| s.unescaped().to_vec())))
                .collect();
            let key_vals: Vec<Value> = keys.iter().map(|k| self.sym_val(fx, k)).collect();
            let keys_ary = if hp.rest().is_some_and(|r| r.as_assoc_splat_node().is_some()) {
                self.nil(fx)
            } else {
                let n = self.iconst(fx, key_vals.len() as i64);
                let buf = self.args_buf(fx, &key_vals);
                self.call(fx, "kn_ary_new", &[n, buf])
            };
            let d = self.callc(fx, "kn_pm_deconstruct_keys", &[v, keys_ary]);
            let not_nil = fx.b.ins().icmp_imm_s(IntCC::NotEqual, d, QNIL);
            self.fail_unless(fx, not_nil, fail);
            for (e, kv) in elems.iter().zip(key_vals.iter()) {
                let Some(a) = e.as_assoc_node() else { continue };
                let val = self.call(fx, "kn_pm_key", &[d, *kv]);
                let present = fx.b.ins().icmp_imm_s(IntCC::NotEqual, val, KW_MISSING);
                self.fail_unless(fx, present, fail);
                self.pattern(fx, &a.value(), val, fail);
            }
            if let Some(rest) = hp.rest() {
                let n = self.iconst(fx, key_vals.len() as i64);
                let buf = self.args_buf(fx, &key_vals);
                let ka = self.call(fx, "kn_ary_new", &[n, buf]);
                if let Some(s) = rest.as_assoc_splat_node() {
                    if let Some(t) = s.value() {
                        let r = self.call(fx, "kn_pm_hash_rest", &[d, ka]);
                        self.pattern(fx, &t, r, fail);
                    }
                } else if rest.as_no_keywords_parameter_node().is_some() {
                    let r = self.call(fx, "kn_pm_hash_rest", &[d, ka]);
                    let len = self.call(fx, "kn_pm_size", &[r]);
                    let empty = fx.b.ins().icmp_imm_s(IntCC::Equal, len, 0);
                    self.fail_unless(fx, empty, fail);
                }
            }
            return;
        }
        if pat.as_splat_node().is_some() {
            self.error_at(at, "splat pattern outside an array pattern");
            return;
        }
        // value pattern: pattern === value
        if pat.as_symbol_node().is_some() || pat.as_nil_node().is_some() || pat.as_true_node().is_some() || pat.as_false_node().is_some() {
            let pv = self.expr(fx, pat);
            let eq = fx.b.ins().icmp(IntCC::Equal, pv, v);
            self.fail_unless(fx, eq, fail);
            return;
        }
        let pv = self.expr(fx, pat);
        self.case_eq(fx, pv, v, fail);
    }

    /// One `in` clause: pattern plus optional `if`/`unless` guard.
    fn in_clause(&mut self, fx: &mut Fx, pat: &Node, v: Value, fail: Block) {
        if let Some(g) = pat.as_if_node() {
            if let Some(inner) = g.statements().and_then(|s| s.body().iter().next()) {
                self.pattern(fx, &inner, v, fail);
                let c = self.expr(fx, &g.predicate());
                self.fail_unless_truthy(fx, c, fail);
                return;
            }
        }
        if let Some(g) = pat.as_unless_node() {
            if let Some(inner) = g.statements().and_then(|s| s.body().iter().next()) {
                self.pattern(fx, &inner, v, fail);
                let c = self.expr(fx, &g.predicate());
                let t = self.truthy_br(fx, c);
                let ok = fx.b.create_block();
                fx.b.ins().brif(t, fail, &[], ok, &[]);
                fx.b.switch_to_block(ok);
                return;
            }
        }
        self.pattern(fx, pat, v, fail);
    }

    pub(crate) fn case_match(&mut self, fx: &mut Fx, n: &CaseMatchNode) -> Value {
        let subj = match n.predicate() {
            Some(p) => self.expr(fx, &p),
            None => self.nil(fx),
        };
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let mut in_i = 0u8;
        for c in n.conditions().iter() {
            let Some(inn) = c.as_in_node() else { continue };
            let fail = fx.b.create_block();
            self.in_clause(fx, &inn.pattern(), subj, fail);
            self.cov_branch(fx, &n.as_node(), in_i);
            in_i += 1;
            let v = self.stmts(fx, inn.statements());
            fx.b.ins().jump(done, &[v.into()]);
            fx.b.switch_to_block(fail);
        }
        self.cov_branch(fx, &n.as_node(), in_i);
        let v = match n.else_clause() {
            Some(e) => self.stmts(fx, e.statements()),
            None => {
                self.callc(fx, "kn_pm_nomatch", &[subj]);
                self.nil(fx)
            }
        };
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    pub(crate) fn match_predicate(&mut self, fx: &mut Fx, value: &Node, pat: &Node, required: bool) -> Value {
        let v = self.expr(fx, value);
        let fail = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        self.in_clause(fx, pat, v, fail);
        let t = self.iconst(fx, if required { QNIL } else { QTRUE });
        fx.b.ins().jump(done, &[t.into()]);
        fx.b.switch_to_block(fail);
        if required {
            self.callc(fx, "kn_pm_nomatch", &[v]);
        }
        let f = self.iconst(fx, if required { QNIL } else { QFALSE });
        fx.b.ins().jump(done, &[f.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
}
