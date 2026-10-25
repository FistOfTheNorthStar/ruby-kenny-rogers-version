//! Float elements read unboxed in the fast copy of a speculated loop (spec.rs):
//!
//! ```ruby
//! c[i * n + j] = c[i * n + j] + aik * b[k * n + j]
//! ```
//!
//! With `c` an Array of Floats (analysis.rs `float_arrays`) and `aik` a speculated Float,
//! both element reads give an f64 directly, and the arithmetic needs no type checks.
//!
//! - **Which statements.** `x[e] = v`, `v = e` and `v op= e` (`v` a speculated local) whose
//!   values are all numbers for sure: literals, speculated locals, the elements described
//!   below, and `+ - * /` on those; indexes from Fixnum locals and literals. Nothing in
//!   them runs Ruby code before the final store or assignment.
//! - **Which elements.** `x[e]` of an Array of Floats, the value assigned to a Float local,
//!   or an operand of arithmetic whose other operand is a Float. The first two must be
//!   Floats. The last may also be a Fixnum, converted to a double as Ruby does.
//! - **Otherwise.** When `x` isn't a plain Array (with the builtin `[]`), the index is not
//!   a Fixnum within it, or the element is something else, the statement hasn't done
//!   anything yet: the locals are handed over to the generic copy, which runs the
//!   statement from its start (`resume_before`).

use crate::fastpath::Num;
use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::F64, InstBuilder};
use ruby_prism::*;

#[derive(Clone, Copy, PartialEq)]
enum K {
    Int,
    Float,
}

/// `x[e]` with `x` a local of the scope and one plain argument: (x, e).
pub(crate) fn elem_read<'pr>(n: &Node<'pr>) -> Option<(Vec<u8>, Node<'pr>)> {
    let c = n.as_call_node()?;
    if c.name().as_slice() != b"[]" || c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let r = c.receiver()?.as_local_variable_read_node().filter(|r| r.depth() == 0)?;
    let mut args: Vec<Node> = c.arguments()?.arguments().iter().collect();
    let plain = |a: &Node| a.as_splat_node().is_none() && a.as_keyword_hash_node().is_none() && a.as_forwarding_arguments_node().is_none();
    if args.len() != 1 || !plain(&args[0]) {
        return None;
    }
    Some((r.name().as_slice().to_vec(), args.pop().unwrap()))
}

/// The one expression in parentheses.
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

/// `a op b` for `+ - * /`: (op, a, b).
fn arith<'pr>(n: &Node<'pr>) -> Option<(Vec<u8>, Node<'pr>, Node<'pr>)> {
    let c = n.as_call_node()?;
    let (a, b) = crate::fastpath::numeric_call(&c)?;
    let op = c.name().as_slice();
    matches!(op, b"+" | b"-" | b"*" | b"/").then(|| (op.to_vec(), a, b))
}

fn reg_local(fx: &Fx, name: &[u8]) -> bool {
    matches!(fx.locals.get(name), Some(Local::Var(_)))
}

/// An Integer for sure, computed without running Ruby code: Fixnum-speculated locals,
/// literals, `+ - *` on those.
pub(crate) fn sure_int(fx: &Fx, n: &Node) -> bool {
    if n.as_integer_node().is_some() {
        return true;
    }
    if let Some(r) = n.as_local_variable_read_node() {
        let name = r.name().as_slice();
        return r.depth() == 0 && fx.int_spec.contains(name) && reg_local(fx, name);
    }
    if let Some(i) = paren_inner(n) {
        return sure_int(fx, &i);
    }
    match arith(n) {
        Some((op, a, b)) => op != b"/" && sure_int(fx, &a) && sure_int(fx, &b),
        None => false,
    }
}

/// The type `n` has for sure in a statement of this kind (see the module docs), None when
/// it isn't one. `fctx`: `n` is an operand of arithmetic whose other operand is a Float.
/// `found`: set when an element read is speculated.
fn kind(fx: &Fx, n: &Node, fctx: bool, found: &mut bool) -> Option<K> {
    if n.as_integer_node().is_some() {
        return Some(K::Int);
    }
    if n.as_float_node().is_some() {
        return Some(K::Float);
    }
    if let Some(r) = n.as_local_variable_read_node() {
        let name = r.name().as_slice();
        if r.depth() != 0 || !reg_local(fx, name) {
            return None;
        }
        if fx.flocals.contains_key(name) {
            return Some(K::Float);
        }
        return fx.int_spec.contains(name).then_some(K::Int);
    }
    if let Some(i) = paren_inner(n) {
        return kind(fx, &i, fctx, found);
    }
    if let Some((x, e)) = elem_read(n) {
        if !reg_local(fx, &x) || !sure_int(fx, &e) {
            return None;
        }
        if fctx || fx.typed_info.float_arrays.contains(&x) {
            *found = true;
            return Some(K::Float);
        }
        return None;
    }
    let (_, a, b) = arith(n)?;
    let (fa, fb) = (static_float(fx, &a), static_float(fx, &b));
    let ka = kind(fx, &a, fb, found)?;
    let kb = kind(fx, &b, fa, found)?;
    Some(if ka == K::Float || kb == K::Float { K::Float } else { K::Int })
}

/// `n` is a Float for sure (in such a statement), its context aside.
pub(crate) fn static_float(fx: &Fx, n: &Node) -> bool {
    kind(fx, n, false, &mut false) == Some(K::Float)
}

/// Statement `n` of the fast copy reads elements speculated as Floats (module docs).
fn elem_stmt(fx: &Fx, n: &Node) -> bool {
    let Some(spec) = fx.spec.as_ref() else { return false };
    let speculated = |name: &[u8]| spec.locals.iter().any(|l| l.name == name) && reg_local(fx, name);
    let mut found = false;
    let ok = if let Some(w) = n.as_local_variable_write_node() {
        let name = w.name().as_slice();
        let v = w.value();
        w.depth() == 0
            && speculated(name)
            && match elem_read(&v) {
                // `aik = a[i * n + k]` for a Float local: the element must be a Float
                Some((x, e)) if spec.fvars.contains_key(name) => {
                    found = reg_local(fx, &x) && sure_int(fx, &e);
                    found
                }
                _ => kind(fx, &v, false, &mut found).is_some(),
            }
    } else if let Some(w) = n.as_local_variable_operator_write_node() {
        let name = w.name().as_slice();
        let op = w.binary_operator().as_slice();
        let fctx = spec.fvars.contains_key(name) && matches!(op, b"+" | b"-" | b"*" | b"/");
        w.depth() == 0 && speculated(name) && matches!(op, b"+" | b"-" | b"*" | b"/") && kind(fx, &w.value(), fctx, &mut found).is_some()
    } else if let Some(c) = n.as_call_node() {
        let args: Vec<Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
        let recv_ok = c.receiver().and_then(|r| r.as_local_variable_read_node()).is_some_and(|r| r.depth() == 0 && reg_local(fx, r.name().as_slice()));
        c.name().as_slice() == b"[]="
            && c.block().is_none()
            && !c.is_safe_navigation()
            && recv_ok
            && args.len() == 2
            && sure_int(fx, &args[0])
            && kind(fx, &args[1], false, &mut found).is_some()
    } else {
        false
    };
    ok && found
}

impl<M: Module> Compiler<M> {
    /// Before statement `n`: in the generic copy of a speculated loop, the place the fast
    /// copy may continue at; in the fast copy, the statement's hand-over when it reads
    /// speculated elements. Returns what `felem_end` restores.
    pub(crate) fn felem_begin(&mut self, fx: &mut Fx, n: &Node) -> Option<Block> {
        let at = n.location().start_offset();
        if let Some(b) = fx.resume_before.remove(&at) {
            fx.b.ins().jump(b, &[]);
            fx.b.switch_to_block(b);
        }
        let prev = fx.spec_stmt.take();
        fx.spec_stmt_used = false;
        fx.stmt_memo.clear();
        fx.stmt_at = if fx.spec.is_some() { at } else { usize::MAX };
        if fx.spec.is_some() && fx.inline_levels.is_empty() && self.inline_ctx.is_empty() && std::env::var_os("KENNY_NO_FELEM").is_none() && elem_stmt(fx, n) {
            fx.spec_stmt = Some(fx.b.create_block());
            if std::env::var_os("KENNY_DEBUG_SPEC").is_some() {
                let line = self.line_of(at);
                eprintln!("felem {}:{}", self.file.display(), line);
            }
        }
        prev
    }

    /// After statement `n` (see `felem_begin`): fill the hand-over if anything uses it.
    pub(crate) fn felem_end(&mut self, fx: &mut Fx, n: &Node, prev: Option<Block>) {
        if let (Some(d), true) = (fx.spec_stmt.take(), fx.spec_stmt_used) {
            let at = n.location().start_offset();
            let cont = fx.b.current_block();
            fx.b.switch_to_block(d);
            fx.b.set_cold_block(d);
            self.spec_handover(fx, None, |_| true);
            let target = *fx.resume_before_req.entry(at).or_insert_with(|| fx.b.create_block());
            fx.b.ins().jump(target, &[]);
            if let Some(c) = cont {
                fx.b.switch_to_block(c);
            }
        }
        fx.spec_stmt = prev;
        fx.spec_stmt_used = false;
        fx.stmt_memo.clear();
    }

    /// Element read `node` (`elem_read`) in such a statement as an f64. `arith`: a Fixnum
    /// element is taken too (converted).
    pub(crate) fn felem_read(&mut self, fx: &mut Fx, node: &Node, arith: bool) -> Num {
        let c = node.as_call_node().unwrap();
        let (_, e) = elem_read(node).unwrap();
        let recv = self.expr(fx, &c.receiver().unwrap());
        let idx = self.expr(fx, &e);
        self.cur_offset = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
        let deopt = fx.spec_stmt.expect("felem_read outside a speculated statement");
        fx.spec_stmt_used = true;
        let v = self.ary_load_or(fx, recv, idx, deopt);
        let done = fx.b.create_block();
        fx.b.append_block_param(done, F64);
        let fb = fx.b.create_block();
        let other = fx.b.create_block();
        fx.b.set_cold_block(other);
        let tag = fx.b.ins().band_imm_s(v, 3);
        let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
        fx.b.ins().brif(flo, fb, &[], other, &[]);
        fx.b.switch_to_block(fb);
        let d = self.flo_decode(fx, v);
        fx.b.ins().jump(done, &[d.into()]);
        fx.b.switch_to_block(other);
        if arith {
            let fixb = fx.b.create_block();
            let heap = fx.b.create_block();
            fx.b.set_cold_block(fixb);
            fx.b.set_cold_block(heap);
            let low = fx.b.ins().band_imm_s(v, 1);
            fx.b.ins().brif(low, fixb, &[], heap, &[]);
            fx.b.switch_to_block(fixb);
            let i = fx.b.ins().sshr_imm_s(v, 1);
            let f = fx.b.ins().fcvt_from_sint(F64, i);
            fx.b.ins().jump(done, &[f.into()]);
            fx.b.switch_to_block(heap);
        }
        // a heap Float (else hand over)
        let is = self.call(fx, "kn_is_float", &[v]);
        let hb = fx.b.create_block();
        fx.b.set_cold_block(hb);
        fx.b.ins().brif(is, hb, &[], deopt, &[]);
        fx.b.switch_to_block(hb);
        let call = self.rt_ref(fx, "kn_float_unbox");
        let inst = fx.b.ins().call(call, &[v]);
        let r = fx.b.inst_results(inst)[0];
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        Num::F64(fx.b.block_params(done)[0])
    }
}
