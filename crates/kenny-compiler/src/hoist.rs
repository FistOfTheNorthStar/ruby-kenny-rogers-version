//! Integer arithmetic computed once in the fast copy of a speculated loop (spec.rs):
//!
//! ```ruby
//! while j < n
//!   c[i * n + j] = c[i * n + j] + aik * b[k * n + j]
//! ```
//!
//! - **Out of a loop.** `i * n` and `k * n` don't change inside the `while j` loop: Fixnum-
//!   speculated locals the loop doesn't assign, and literals, with `+ - *`. They are
//!   computed before the loop (a loop nested in the fast copy, directly a statement). An
//!   overflow there hands the locals over to the generic copy, which runs the loop from
//!   the statement's start (felem.rs `resume_before`).
//! - **Within a statement.** In a statement of felem.rs (nothing in it assigns a local or
//!   runs Ruby code before its end), `i * n + j` is computed once.

use crate::*;
use cranelift_codegen::ir::InstBuilder;
use ruby_prism::*;
use std::collections::HashSet;

/// Every local the loop assigns (any depth: a name is enough).
#[derive(Default)]
struct Writes(HashSet<Vec<u8>>);
impl<'pr> Visit<'pr> for Writes {
    fn visit_local_variable_write_node(&mut self, n: &LocalVariableWriteNode<'pr>) {
        self.0.insert(n.name().as_slice().to_vec());
        visit_local_variable_write_node(self, n);
    }
    fn visit_local_variable_operator_write_node(&mut self, n: &LocalVariableOperatorWriteNode<'pr>) {
        self.0.insert(n.name().as_slice().to_vec());
        visit_local_variable_operator_write_node(self, n);
    }
    fn visit_local_variable_or_write_node(&mut self, n: &LocalVariableOrWriteNode<'pr>) {
        self.0.insert(n.name().as_slice().to_vec());
        visit_local_variable_or_write_node(self, n);
    }
    fn visit_local_variable_and_write_node(&mut self, n: &LocalVariableAndWriteNode<'pr>) {
        self.0.insert(n.name().as_slice().to_vec());
        visit_local_variable_and_write_node(self, n);
    }
    fn visit_local_variable_target_node(&mut self, n: &LocalVariableTargetNode<'pr>) {
        self.0.insert(n.name().as_slice().to_vec());
    }
}

/// `a op b` for `+ - *`: (op, a, b).
fn int_arith<'pr>(n: &Node<'pr>) -> Option<(Vec<u8>, Node<'pr>, Node<'pr>)> {
    let c = n.as_call_node()?;
    let (a, b) = crate::fastpath::numeric_call(&c)?;
    let op = c.name().as_slice();
    matches!(op, b"+" | b"-" | b"*").then(|| (op.to_vec(), a, b))
}

fn paren_inner<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
    let b = n.as_parentheses_node()?.body()?;
    match b.as_statements_node() {
        Some(s) => {
            let v: Vec<Node> = s.body().iter().collect();
            if v.len() == 1 { v.into_iter().next() } else { None }
        }
        None => Some(b),
    }
}

/// A tagged Fixnum literal.
fn fix_lit(n: &Node) -> Option<i64> {
    let i = crate::expr::int_value(&n.as_integer_node()?)?;
    (-(1i64 << 62)..(1i64 << 62)).contains(&i).then_some((i << 1) | 1)
}

/// The structural key of an Integer expression (`(* i n)`), None for anything else.
pub(crate) fn key(n: &Node) -> Option<String> {
    if let Some(v) = fix_lit(n) {
        return Some(format!("{}", v >> 1));
    }
    if let Some(r) = n.as_local_variable_read_node() {
        return (r.depth() == 0).then(|| String::from_utf8_lossy(r.name().as_slice()).into_owned());
    }
    if let Some(i) = paren_inner(n) {
        return key(&i);
    }
    let (op, a, b) = int_arith(n)?;
    Some(format!("({} {} {})", String::from_utf8_lossy(&op), key(&a)?, key(&b)?))
}

fn spec_int_local(fx: &Fx, name: &[u8]) -> bool {
    fx.int_spec.contains(name) && matches!(fx.locals.get(name), Some(Local::Var(_)))
}

/// Unchanged in the loop: literals, Fixnum locals it doesn't assign, `+ - *` on those.
fn invariant(fx: &Fx, w: &HashSet<Vec<u8>>, n: &Node) -> bool {
    if fix_lit(n).is_some() {
        return true;
    }
    if let Some(r) = n.as_local_variable_read_node() {
        let name = r.name().as_slice();
        return r.depth() == 0 && spec_int_local(fx, name) && !w.contains(name);
    }
    if let Some(i) = paren_inner(n) {
        return invariant(fx, w, &i);
    }
    match int_arith(n) {
        Some((_, a, b)) => invariant(fx, w, &a) && invariant(fx, w, &b),
        None => false,
    }
}

/// The largest invariant operations in the loop (not inside blocks).
struct Cands<'a, 'b, 'pr> {
    fx: &'a Fx<'b>,
    w: &'a HashSet<Vec<u8>>,
    out: Vec<Node<'pr>>,
}
impl<'a, 'b, 'pr> Visit<'pr> for Cands<'a, 'b, 'pr> {
    fn visit_call_node(&mut self, n: &CallNode<'pr>) {
        let node = n.as_node();
        if int_arith(&node).is_some() && invariant(self.fx, self.w, &node) {
            self.out.push(node);
            return;
        }
        visit_call_node(self, n);
    }
    fn visit_block_node(&mut self, _: &BlockNode<'pr>) {}
    fn visit_lambda_node(&mut self, _: &LambdaNode<'pr>) {}
    fn visit_def_node(&mut self, _: &DefNode<'pr>) {}
}

impl<M: Module> Compiler<M> {
    fn hoist_on(&self, fx: &Fx) -> bool {
        fx.spec.is_some() && fx.inline_levels.is_empty() && self.inline_ctx.is_empty()
    }

    /// Before loop `lp` (compiled in the current block): its invariant operations, when it
    /// is a statement of a speculated loop's fast copy. Returns what `hoist_leave` restores.
    pub(crate) fn hoist_enter(&mut self, fx: &mut Fx, lp: Option<&Node>) -> Option<HashMap<String, Value>> {
        let lp = lp?;
        let at = lp.location().start_offset();
        if !self.hoist_on(fx) || fx.stmt_at != at || std::env::var_os("KENNY_NO_HOIST").is_some() {
            return None;
        }
        let mut w = Writes::default();
        w.visit(lp);
        let nodes = {
            let mut c = Cands { fx, w: &w.0, out: Vec::new() };
            c.visit(lp);
            c.out
        };
        let saved = fx.hoisted.clone();
        let deopt = fx.b.create_block();
        let mut used = false;
        for n in &nodes {
            let Some(k) = key(n) else { continue };
            if fx.hoisted.contains_key(&k) {
                continue;
            }
            let v = self.hoist_eval(fx, n, deopt, &mut used);
            if std::env::var_os("KENNY_DEBUG_SPEC").is_some() {
                let line = self.line_of(at);
                eprintln!("hoist {}:{}: {}", self.file.display(), line, k);
            }
            fx.hoisted.insert(k, v);
        }
        if used {
            // an overflow: the generic copy runs the loop
            let cont = fx.b.current_block().unwrap();
            fx.b.switch_to_block(deopt);
            fx.b.set_cold_block(deopt);
            self.spec_handover(fx, None, |_| true);
            let target = *fx.resume_before_req.entry(at).or_insert_with(|| fx.b.create_block());
            fx.b.ins().jump(target, &[]);
            fx.b.switch_to_block(cont);
        }
        Some(saved)
    }

    pub(crate) fn hoist_leave(&mut self, fx: &mut Fx, saved: Option<HashMap<String, Value>>) {
        if let Some(s) = saved {
            fx.hoisted = s;
        }
    }

    /// Fixnum operations on Fixnums, to `deopt` on overflow.
    fn hoist_eval(&mut self, fx: &mut Fx, n: &Node, deopt: Block, used: &mut bool) -> Value {
        if let Some(v) = key(n).and_then(|k| fx.hoisted.get(&k).copied()) {
            return v;
        }
        if let Some(v) = fix_lit(n) {
            return fx.b.ins().iconst(I64, v);
        }
        if let Some(r) = n.as_local_variable_read_node() {
            return self.local_get(fx, r.name().as_slice(), 0);
        }
        if let Some(i) = paren_inner(n) {
            return self.hoist_eval(fx, &i, deopt, used);
        }
        let (op, a, b) = int_arith(n).unwrap();
        let x = self.hoist_eval(fx, &a, deopt, used);
        let y = self.hoist_eval(fx, &b, deopt, used);
        let (r, of) = match op.as_slice() {
            b"+" => {
                let ym1 = fx.b.ins().iadd_imm_s(y, -1);
                fx.b.ins().sadd_overflow(x, ym1)
            }
            b"-" => {
                let (d, of) = fx.b.ins().ssub_overflow(x, y);
                (fx.b.ins().iadd_imm_s(d, 1), of)
            }
            _ => {
                let xs = fx.b.ins().sshr_imm_s(x, 1);
                let ym1 = fx.b.ins().iadd_imm_s(y, -1);
                let (p, of) = fx.b.ins().smul_overflow(xs, ym1);
                (fx.b.ins().iadd_imm_s(p, 1), of)
            }
        };
        *used = true;
        let ok = fx.b.create_block();
        fx.b.ins().brif(of, deopt, &[], ok, &[]);
        fx.b.switch_to_block(ok);
        fx.sure_fix.insert(r);
        fx.sure_ints.insert(r);
        r
    }

    /// An operation computed before the loop, or already in this statement.
    pub(crate) fn hoisted_get(&self, fx: &Fx, n: &Node) -> Option<crate::fastpath::Num> {
        if (fx.hoisted.is_empty() && fx.stmt_memo.is_empty()) || !self.hoist_on(fx) || int_arith(n).is_none() {
            return None;
        }
        let k = key(n)?;
        if let Some(&v) = fx.hoisted.get(&k) {
            return Some(crate::fastpath::Num::Boxed(v));
        }
        fx.stmt_memo.get(&k).copied()
    }

    /// Remember Integer operation `n`'s value for the rest of the statement (felem.rs).
    pub(crate) fn stmt_memo_put(&mut self, fx: &mut Fx, n: &Node, v: crate::fastpath::Num) {
        if fx.spec_stmt.is_none() || !self.hoist_on(fx) || int_arith(n).is_none() || !crate::felem::sure_int(fx, n) {
            return;
        }
        if let Some(k) = key(n) {
            fx.stmt_memo.insert(k, v);
        }
    }
}
