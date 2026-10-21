//! Type-specialised code: inline Fixnum and Flonum arithmetic/comparisons, and the direct
//! `kn_send0/1/2` entry points for calls with few arguments and no block.
//!
//! Integer and Float operators can't be redefined (LANGUAGE.md rule 2), so `a + b` on two
//! Fixnums or Floats needs no guard beyond the tag checks. Every other specialisation is
//! guarded in the runtime by the call site's inline cache (see kenny-runtime/src/opt.rs).

use crate::*;
use ruby_prism::Node;
use cranelift_codegen::ir::{condcodes::FloatCC, types::F64, types::I32, types::I8};

/// Flonum tags (kenny-runtime/src/core.rs): low two bits 0b10; +0.0 has its own encoding.
const FLONUM_ZERO: i64 = 0x8000_0000_0000_0002u64 as i64;

/// The flonum for a compile-time Float, if it has one (same rule as the runtime's).
pub(crate) fn f64_to_flonum(d: f64) -> Option<i64> {
    let t = d.to_bits();
    let bits = ((t >> 60) & 7) as i64;
    if t != 0x3000_0000_0000_0000 && (bits - 3) & !1 == 0 {
        Some(((t.rotate_left(3) & !1) | 2) as i64)
    } else if t == 0 {
        Some(FLONUM_ZERO)
    } else {
        None
    }
}

/// The value of `v` if it is an `iconst` (a literal operand).
fn const_of(fx: &Fx, v: Value) -> Option<i64> {
    use cranelift_codegen::ir::{InstructionData, Opcode, ValueDef};
    let dfg = &fx.b.func.dfg;
    match dfg.value_def(v) {
        ValueDef::Result(inst, 0) => match dfg.insts[inst] {
            InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => Some(imm.bits()),
            _ => None,
        },
        _ => None,
    }
}

/// A numeric operand or result that may still be an unboxed Float.
#[derive(Clone, Copy)]
pub(crate) enum Num {
    Boxed(Value),
    /// `kind` (I8): 0 = only `boxed` is valid, 1 = only `raw` (an f64) is valid, 2 = both.
    Lazy { boxed: Value, raw: Value, kind: Value, lit: bool },
}
impl Num {
    fn boxed(&self) -> Value {
        match self {
            Num::Boxed(v) => *v,
            Num::Lazy { boxed, .. } => *boxed,
        }
    }
    fn is_float_lit(&self, fx: &Fx) -> bool {
        match self {
            Num::Boxed(v) => const_of(fx, *v).is_some_and(|c| c & 3 == 2),
            Num::Lazy { lit, .. } => *lit,
        }
    }
    fn is_int_lit(&self, fx: &Fx) -> bool {
        match self {
            Num::Boxed(v) => const_of(fx, *v).is_some_and(|c| c & 1 == 1),
            Num::Lazy { .. } => false,
        }
    }
    /// An Integer literal, or (most likely) an Integer: see `Fx::int_vals`.
    fn is_int_hint(&self, fx: &Fx) -> bool {
        self.is_int_lit(fx) || matches!(self, Num::Boxed(v) if fx.int_vals.contains(v))
    }
}

/// `recv op arg` for an operator the compiler inlines (one plain argument, no block).
pub(crate) fn numeric_call<'pr>(c: &ruby_prism::CallNode<'pr>) -> Option<(ruby_prism::Node<'pr>, ruby_prism::Node<'pr>)> {
    if !is_fast_op(c.name().as_slice()) || c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let recv = c.receiver()?;
    let args: Vec<ruby_prism::Node> = c.arguments()?.arguments().iter().collect();
    if args.len() != 1 || args[0].as_splat_node().is_some() || args[0].as_keyword_hash_node().is_some() || args[0].as_block_argument_node().is_some() || args[0].as_forwarding_arguments_node().is_some() {
        return None;
    }
    Some((recv, args.into_iter().next().unwrap()))
}

impl<M: Module> Compiler<M> {
    /// f64 of a flonum (the caller checked the tag).
    fn flo_decode(&mut self, fx: &mut Fx, v: Value) -> Value {
        let b63 = fx.b.ins().ushr_imm_s(v, 63);
        let two = fx.b.ins().iconst(I64, 2);
        let hi = fx.b.ins().isub(two, b63);
        let body = fx.b.ins().band_imm_s(v, !3i64);
        let t = fx.b.ins().bor(hi, body);
        let r = fx.b.ins().rotr_imm_s(t, 3);
        let is_zero = fx.b.ins().icmp_imm_s(IntCC::Equal, v, FLONUM_ZERO);
        let zero = fx.b.ins().iconst(I64, 0);
        let bits = fx.b.ins().select(is_zero, zero, r);
        fx.b.ins().bitcast(F64, MemFlagsData::new(), bits)
    }

    /// Tagged Float for an f64: a flonum inline, or a heap Float from the runtime (cold).
    /// Jumps to `done` with the value appended to `extra` block arguments.
    fn flo_encode_to(&mut self, fx: &mut Fx, f: Value, done: Block, extra: &[Value]) {
        let t = fx.b.ins().bitcast(I64, MemFlagsData::new(), f);
        let e = fx.b.ins().ushr_imm_s(t, 60);
        let e = fx.b.ins().band_imm_s(e, 7);
        let x = fx.b.ins().iadd_imm_s(e, -3);
        let in_range = fx.b.ins().icmp_imm_u(IntCC::UnsignedLessThanOrEqual, x, 1);
        let not_special = fx.b.ins().icmp_imm_s(IntCC::NotEqual, t, 0x3000_0000_0000_0000);
        let ok = fx.b.ins().band(in_range, not_special);
        let rot = fx.b.ins().rotl_imm_s(t, 3);
        let rot = fx.b.ins().band_imm_s(rot, !1i64);
        let enc = fx.b.ins().bor_imm_s(rot, 2);
        // +0.0 has its own flonum
        let is_zero = fx.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
        let zero = fx.b.ins().iconst(I64, FLONUM_ZERO);
        let enc = fx.b.ins().select(is_zero, zero, enc);
        let ok = fx.b.ins().bor(ok, is_zero);
        let boxb = fx.b.create_block();
        let mut args: Vec<cranelift_codegen::ir::BlockArg> = vec![enc.into()];
        args.extend(extra.iter().map(|v| cranelift_codegen::ir::BlockArg::from(*v)));
        fx.b.ins().brif(ok, done, &args, boxb, &[]);
        fx.b.set_cold_block(boxb);
        fx.b.switch_to_block(boxb);
        let v = self.call(fx, "kn_float_box", &[t]);
        let mut args: Vec<cranelift_codegen::ir::BlockArg> = vec![v.into()];
        args.extend(extra.iter().map(|v| cranelift_codegen::ir::BlockArg::from(*v)));
        fx.b.ins().jump(done, &args);
    }

    /// The tagged value of a `Num` (boxing a raw Float if that is all there is).
    pub(crate) fn materialize(&mut self, fx: &mut Fx, n: Num) -> Value {
        match n {
            Num::Boxed(v) => v,
            Num::Lazy { boxed, raw, kind, .. } => {
                if let Some(k) = const_of(fx, kind) {
                    if k != 1 {
                        return boxed;
                    }
                }
                let need = fx.b.ins().icmp_imm_s(IntCC::Equal, kind, 1);
                let enc = fx.b.create_block();
                let done = fx.b.create_block();
                fx.b.append_block_param(done, I64);
                fx.b.ins().brif(need, enc, &[], done, &[boxed.into()]);
                fx.b.switch_to_block(enc);
                self.flo_encode_to(fx, raw, done, &[]);
                fx.b.switch_to_block(done);
                fx.b.block_params(done)[0]
            }
        }
    }

    /// An operand of an arithmetic/comparison operator, keeping Floats unboxed: arithmetic
    /// nodes, Float literals and locals with an unboxed shadow give `Num::Lazy`.
    /// `lazy`: the result may stay unboxed (it feeds another operator or a shadowed local);
    /// otherwise it is boxed on the Float path itself.
    pub(crate) fn num_expr(&mut self, fx: &mut Fx, node: &Node, lazy: bool) -> Num {
        if let Some(f) = node.as_float_node() {
            let f = f.value();
            let boxed = self.float_lit(fx, f);
            let raw = fx.b.ins().f64const(f);
            let kind = fx.b.ins().iconst(I8, 2);
            return Num::Lazy { boxed, raw, kind, lit: true };
        }
        if let Some(p) = node.as_parentheses_node() {
            if let Some(b) = p.body() {
                let single = match b.as_statements_node() {
                    Some(s) => {
                        let v: Vec<Node> = s.body().iter().collect();
                        if v.len() == 1 { Some(v.into_iter().next().unwrap()) } else { None }
                    }
                    None => Some(b),
                };
                if let Some(inner) = single {
                    return self.num_expr(fx, &inner, lazy);
                }
            }
        }
        if let Some(l) = node.as_local_variable_read_node() {
            return self.local_get_num(fx, l.name().as_slice(), l.depth());
        }
        if let Some(c) = node.as_call_node() {
            if let Some((recv, arg)) = numeric_call(&c) {
                let op = c.name().as_slice().to_vec();
                let (rs, as_) = (recv.as_symbol_node().is_some(), arg.as_symbol_node().is_some());
                if matches!(op.as_slice(), b"==" | b"!=") && (rs || as_) && self.refine_scope.is_none() {
                    let a = self.expr(fx, &recv);
                    let b = self.expr(fx, &arg);
                    self.cur_offset = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
                    return Num::Boxed(self.sym_eq(fx, &op, a, b, rs));
                }
                let a = self.num_expr(fx, &recv, true);
                let b = self.num_expr(fx, &arg, true);
                self.cur_offset = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
                return self.binop_num(fx, &op, a, b, lazy);
            }
        }
        Num::Boxed(self.expr(fx, node))
    }

    /// `a == b` / `a != b` with a Symbol literal on one side (`lit_recv`: on the left):
    /// compared by identity inline when the receiver is a Symbol (always, when it is the
    /// literal) and Symbol#== is the builtin (`KN_DIRECT.flags`), else a call.
    fn sym_eq(&mut self, fx: &mut Fx, op: &[u8], a: Value, b: Value, lit_recv: bool) -> Value {
        let f = mem();
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let flags = fx.b.ins().load(I64, f, dir, KD_FLAGS);
        let builtin = fx.b.ins().band_imm_s(flags, DIRECT_SYM_EQ);
        let same = fx.b.ins().icmp(IntCC::Equal, a, b);
        let ok = if lit_recv {
            fx.b.ins().icmp_imm_s(IntCC::NotEqual, builtin, 0)
        } else {
            let tag = fx.b.ins().band_imm_s(a, 0xff);
            let is_sym = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, SYM_TAG);
            let sym_recv = fx.b.ins().bor(is_sym, same);
            let bi = fx.b.ins().icmp_imm_s(IntCC::NotEqual, builtin, 0);
            fx.b.ins().band(bi, sym_recv)
        };
        let fast = fx.b.create_block();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        fx.b.ins().brif(ok, fast, &[], slow, &[]);
        fx.b.switch_to_block(fast);
        let cond = if op == b"!=" { fx.b.ins().bxor_imm_u(same, 1) } else { same };
        let r = self.bool_v(fx, cond);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let r = self.send(fx, a, op, &[b], None);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// `cur op rhs` for compound assignments (`@x += a * b`): the right side may be unboxed.
    pub(crate) fn binop_rhs(&mut self, fx: &mut Fx, op: &[u8], cur: Value, rhs: Num) -> Value {
        let n = self.binop_num(fx, op, Num::Boxed(cur), rhs, false);
        self.materialize(fx, n)
    }

    /// Binary operator on tagged values (see `binop_num`).
    pub(crate) fn binop(&mut self, fx: &mut Fx, op: &[u8], a: Value, b: Value) -> Value {
        let n = self.binop_num(fx, op, Num::Boxed(a), Num::Boxed(b), false);
        self.materialize(fx, n)
    }

    /// Binary operator: inline Fixnum path (overflow-checked), inline Float path for
    /// Flonum/Fixnum/unboxed mixes, else a call. Arithmetic results stay unboxed
    /// (`Num::Lazy`) until something needs the tagged value.
    pub(crate) fn binop_num(&mut self, fx: &mut Fx, op: &[u8], a: Num, b: Num, lazy: bool) -> Num {
        if !is_fast_op(op) {
            let av = self.materialize(fx, a);
            let bv = self.materialize(fx, b);
            return Num::Boxed(self.send(fx, av, op, &[bv], None));
        }
        let arith = matches!(op, b"+" | b"-" | b"*" | b"/");
        // the result stays unboxed (done gets raw f64 + kind parameters)
        let unboxed = arith && lazy;
        // a literal operand tells which path is the likely one
        let lit_float = a.is_float_lit(fx) || b.is_float_lit(fx);
        let lit_int = a.is_int_hint(fx) || b.is_int_hint(fx);
        let both_int = a.is_int_hint(fx) && b.is_int_hint(fx);
        // Float % has CRuby's sign rules and NaN cases (left to the runtime); & | ^ are
        // Integer-only; == and != only get the inline Float path when an operand is known
        // to be a Float (they are everywhere in non-numeric code, mostly on other types)
        let known_float = lit_float || matches!(a, Num::Lazy { .. }) || matches!(b, Num::Lazy { .. });
        // (after `using`, Float operators may be refined: no inline Float path there)
        let int_only = matches!(op, b"%" | b"&" | b"|" | b"^")
            || (matches!(op, b"==" | b"!=") && !known_float)
            || self.refine_scope.is_some();
        let fast = fx.b.create_block();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let extra: Vec<Value> = if unboxed {
            fx.b.append_block_param(done, F64);
            fx.b.append_block_param(done, I8);
            vec![fx.b.ins().f64const(0.0), fx.b.ins().iconst(I8, 0)]
        } else {
            vec![]
        };
        let fchk = if int_only { slow } else { fx.b.create_block() };
        let (ab, bb) = (a.boxed(), b.boxed());
        if lit_float && !int_only {
            // `x * 2.0`: never two Fixnums
            fx.b.ins().jump(fchk, &[]);
        } else {
            // an unboxed-only operand (kind 1) has the even dummy 0 in its tagged slot, so
            // this test also rules it out
            let both = fx.b.ins().band(ab, bb);
            let bit = fx.b.ins().band_imm_s(both, 1);
            fx.b.ins().brif(bit, fast, &[], fchk, &[]);
        }
        if lit_int && fchk != slow {
            fx.b.set_cold_block(fchk);
        }
        if lit_int || lit_float {
            fx.b.set_cold_block(slow);
        }
        fx.b.switch_to_block(fast);
        let jump_int = |fx: &mut Fx, cond: Option<Value>, r: Value| {
            let mut args: Vec<cranelift_codegen::ir::BlockArg> = vec![r.into()];
            args.extend(extra.iter().map(|v| cranelift_codegen::ir::BlockArg::from(*v)));
            match cond {
                Some(of) => {
                    fx.b.ins().brif(of, slow, &[], done, &args);
                }
                None => {
                    fx.b.ins().jump(done, &args);
                }
            }
        };
        let (a1, b1) = (ab, bb);
        match op {
            b"+" => {
                let bm1 = fx.b.ins().iadd_imm_s(b1, -1);
                let (r, of) = fx.b.ins().sadd_overflow(a1, bm1);
                jump_int(fx, Some(of), r);
            }
            b"-" => {
                let (d, of) = fx.b.ins().ssub_overflow(a1, b1);
                let r = fx.b.ins().iadd_imm_s(d, 1);
                jump_int(fx, Some(of), r);
            }
            b"*" => {
                let x = fx.b.ins().sshr_imm_s(a1, 1);
                let y = fx.b.ins().iadd_imm_s(b1, -1);
                let (p, of) = fx.b.ins().smul_overflow(x, y);
                let r = fx.b.ins().iadd_imm_s(p, 1);
                jump_int(fx, Some(of), r);
            }
            b"/" | b"%" => {
                let x = fx.b.ins().sshr_imm_s(a1, 1);
                let y = fx.b.ins().sshr_imm_s(b1, 1);
                let nz = fx.b.create_block();
                let yz = fx.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                fx.b.ins().brif(yz, slow, &[], nz, &[]);
                fx.b.switch_to_block(nz);
                let q = fx.b.ins().sdiv(x, y);
                let rem = fx.b.ins().srem(x, y);
                // floor semantics: adjust when remainder is non-zero and signs differ
                let rnz = fx.b.ins().icmp_imm_s(IntCC::NotEqual, rem, 0);
                let sx = fx.b.ins().bxor(rem, y);
                let neg = fx.b.ins().icmp_imm_s(IntCC::SignedLessThan, sx, 0);
                let adj = fx.b.ins().band(rnz, neg);
                let adj64 = fx.b.ins().uextend(I64, adj);
                if op == b"/" {
                    let q2 = fx.b.ins().isub(q, adj64);
                    let too_big = fx.b.ins().icmp_imm_s(IntCC::Equal, q2, 1i64 << 62);
                    let sh = fx.b.ins().ishl_imm_s(q2, 1);
                    let r = fx.b.ins().iadd_imm_s(sh, 1);
                    jump_int(fx, Some(too_big), r);
                } else {
                    let zero = fx.b.ins().iconst(I64, 0);
                    let addy = fx.b.ins().select(adj, y, zero);
                    let m = fx.b.ins().iadd(rem, addy);
                    let sh = fx.b.ins().ishl_imm_s(m, 1);
                    let r = fx.b.ins().iadd_imm_s(sh, 1);
                    jump_int(fx, None, r);
                }
            }
            b"&" | b"|" => {
                // the tag bit survives
                let r = if op == b"&" { fx.b.ins().band(a1, b1) } else { fx.b.ins().bor(a1, b1) };
                jump_int(fx, None, r);
            }
            b"^" => {
                let x = fx.b.ins().bxor(a1, b1);
                let r = fx.b.ins().bor_imm_s(x, 1);
                jump_int(fx, None, r);
            }
            _ => {
                let cc = int_cc(op);
                let c = fx.b.ins().icmp(cc, a1, b1);
                let r = self.bool_v(fx, c);
                jump_int(fx, None, r);
            }
        }
        if !int_only {
            self.float_binop(fx, op, &a, &b, fchk, slow, done, unboxed);
        }
        fx.b.switch_to_block(slow);
        let av = self.materialize(fx, a);
        let bv = self.materialize(fx, b);
        let r = self.send(fx, av, op, &[bv], None);
        let mut args: Vec<cranelift_codegen::ir::BlockArg> = vec![r.into()];
        args.extend(extra.iter().map(|v| cranelift_codegen::ir::BlockArg::from(*v)));
        fx.b.ins().jump(done, &args);
        fx.b.switch_to_block(done);
        let p = fx.b.block_params(done).to_vec();
        if both_int && matches!(op, b"+" | b"-" | b"*" | b"/" | b"%" | b"&" | b"|" | b"^") {
            fx.int_vals.insert(p[0]);
        }
        if unboxed {
            Num::Lazy { boxed: p[0], raw: p[1], kind: p[2], lit: false }
        } else {
            Num::Boxed(p[0])
        }
    }

    /// The Float half of `binop_num`, entered at `fchk` when the operands aren't both
    /// Fixnums: unboxed Floats, Flonums and Fixnums are computed inline; heap Floats (values
    /// beyond ~1e±77, -0.0, Infinity, NaN), Bignums and everything else go to `slow`.
    #[allow(clippy::too_many_arguments)]
    fn float_binop(&mut self, fx: &mut Fx, op: &[u8], a: &Num, b: &Num, fchk: Block, slow: Block, done: Block, unboxed: bool) {
        let arith = matches!(op, b"+" | b"-" | b"*" | b"/");
        fx.b.switch_to_block(fchk);
        let (a_sel, a_ok) = self.float_operand(fx, a, !arith);
        let (b_sel, b_ok) = self.float_operand(fx, b, !arith);
        let ok = fx.b.ins().band(a_ok, b_ok);
        let ffast = fx.b.create_block();
        if fx.b.func.layout.is_cold(fchk) {
            fx.b.set_cold_block(ffast);
        }
        fx.b.ins().brif(ok, ffast, &[], slow, &[]);
        fx.b.switch_to_block(ffast);
        let fa = self.float_value(fx, a, a_sel);
        let fb = self.float_value(fx, b, b_sel);
        if arith {
            let r = match op {
                b"+" => fx.b.ins().fadd(fa, fb),
                b"-" => fx.b.ins().fsub(fa, fb),
                b"*" => fx.b.ins().fmul(fa, fb),
                _ => fx.b.ins().fdiv(fa, fb),
            };
            if unboxed {
                // boxed later, if at all
                let dummy = fx.b.ins().iconst(I64, 0);
                let k1 = fx.b.ins().iconst(I8, 1);
                fx.b.ins().jump(done, &[dummy.into(), r.into(), k1.into()]);
            } else {
                self.flo_encode_to(fx, r, done, &[]);
            }
        } else {
            // flonums are never NaN, so the ordered comparisons are exact
            let cc = match op {
                b"<" => FloatCC::LessThan,
                b"<=" => FloatCC::LessThanOrEqual,
                b">" => FloatCC::GreaterThan,
                b">=" => FloatCC::GreaterThanOrEqual,
                b"==" => FloatCC::Equal,
                _ => FloatCC::NotEqual,
            };
            let c = fx.b.ins().fcmp(cc, fa, fb);
            let r = self.bool_v(fx, c);
            fx.b.ins().jump(done, &[r.into()]);
        }
    }

    /// (selector, usable inline) for one operand of a Float operation. The selector is
    /// (has a raw f64, is a flonum). A Fixnum is usable for arithmetic (Ruby converts it to
    /// a double the same way); for comparisons only below 2**53, where that is exact. A raw
    /// f64 may be NaN or infinite, so comparisons only take it when it is ordered.
    fn float_operand(&mut self, fx: &mut Fx, n: &Num, exact: bool) -> ((Option<Value>, Value), Value) {
        let v = n.boxed();
        let tag = fx.b.ins().band_imm_s(v, 3);
        let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
        let low = fx.b.ins().band_imm_s(v, 1);
        let mut fixok = fx.b.ins().icmp_imm_s(IntCC::NotEqual, low, 0);
        if exact {
            let x = fx.b.ins().sshr_imm_s(v, 1);
            let x = fx.b.ins().iadd_imm_s(x, 1i64 << 53);
            let small = fx.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, x, 1i64 << 54);
            fixok = fx.b.ins().band(fixok, small);
        }
        let mut ok = fx.b.ins().bor(flo, fixok);
        let mut has_raw = None;
        if let Num::Lazy { raw, kind, .. } = n {
            let hr = fx.b.ins().icmp_imm_s(IntCC::NotEqual, *kind, 0);
            let mut raw_ok = hr;
            if exact {
                let ordered = fx.b.ins().fcmp(FloatCC::Ordered, *raw, *raw);
                raw_ok = fx.b.ins().band(hr, ordered);
                // an unordered raw value (NaN) must not use the boxed slot either
                let not_raw = fx.b.ins().icmp_imm_s(IntCC::Equal, *kind, 0);
                ok = fx.b.ins().band(ok, not_raw);
            } else {
                let not_raw = fx.b.ins().icmp_imm_s(IntCC::Equal, *kind, 0);
                ok = fx.b.ins().band(ok, not_raw);
            }
            ok = fx.b.ins().bor(ok, raw_ok);
            has_raw = Some(hr);
        }
        ((has_raw, flo), ok)
    }
    fn float_value(&mut self, fx: &mut Fx, n: &Num, sel: (Option<Value>, Value)) -> Value {
        let v = n.boxed();
        let d = self.flo_decode(fx, v);
        let i = fx.b.ins().sshr_imm_s(v, 1);
        let fi = fx.b.ins().fcvt_from_sint(F64, i);
        let x = fx.b.ins().select(sel.1, d, fi);
        match (n, sel.0) {
            (Num::Lazy { raw, .. }, Some(hr)) => fx.b.ins().select(hr, *raw, x),
            _ => x,
        }
    }

    /// A Float literal: a constant when it has a flonum encoding.
    pub(crate) fn float_lit(&mut self, fx: &mut Fx, f: f64) -> Value {
        match f64_to_flonum(f) {
            Some(v) => self.iconst(fx, v),
            None => {
                let slot = self.cache_ptr(fx, 1);
                let bits = self.iconst(fx, f.to_bits() as i64);
                self.call(fx, "kn_float_lit", &[slot, bits])
            }
        }
    }

    /// A call with 0-2 positional arguments and no block: `kn_send0/1/2`, which handle the
    /// hot core methods directly (guarded by the inline cache) and skip the argument buffer.
    /// Inside a loop, a call that may reach an attr_reader / attr_writer first tries the
    /// site's recorded slot inline (`attr_fast`).
    pub(crate) fn send_simple(&mut self, fx: &mut Fx, cache: Value, recv: Value, name: &[u8], mid: Value, vals: &[Value], explicit: bool) -> Value {
        let e = self.iconst(fx, explicit as i64);
        let in_loop = !fx.loops.is_empty();
        let (inline, stub) = direct_plan(name, in_loop);
        let vs = vals.to_vec();
        let reader = attr_name(name);
        let writer = name.last() == Some(&b'=') && attr_name(&name[..name.len() - 1]);
        let plain = move |c: &mut Self, fx: &mut Fx, rt: &'static str, args: &[Value]| -> Value {
            let kind = match rt {
                "kn_send0" => Some(if reader { STUB_SEND0 } else { 6 }),
                "kn_send1" => Some(if writer { 1 } else { 7 }),
                "kn_send2" => Some(2),
                "kn_attr_get" => Some(STUB_ATTR_GET),
                "kn_attr_set" => Some(STUB_ATTR_SET),
                _ => None,
            };
            let attr_rt = matches!(rt, "kn_attr_get" | "kn_attr_set");
            if inline && !attr_rt {
                c.direct_call(fx, cache, recv, &vs, explicit, |c, fx| c.call(fx, rt, args))
            } else if let (true, Some(k)) = (stub || (inline && attr_rt), kind) {
                c.call_stub(fx, k, args)
            } else {
                c.call(fx, rt, args)
            }
        };
        match (name, vals) {
            (b"nil?", []) => return self.call_stub(fx, STUB_NIL_P, &[cache, recv, mid, e]),
            (b"is_a?" | b"kind_of?", [a]) => return self.call_stub(fx, STUB_IS_A, &[cache, recv, mid, *a, e]),
            _ => {}
        }
        match vals {
            // (not names that are mostly core methods: `words.size` is a plain call)
            [] if in_loop && attr_name(name) && may_be_ruby_method(name) => self.attr_fast(fx, cache, recv, None, |c, fx| plain(c, fx, "kn_attr_get", &[cache, recv, mid, e])),
            [] => plain(self, fx, "kn_send0", &[cache, recv, mid, e]),
            [a] if in_loop && name.last() == Some(&b'=') && attr_name(&name[..name.len() - 1]) => {
                let a = *a;
                self.attr_fast(fx, cache, recv, Some(a), |c, fx| plain(c, fx, "kn_attr_set", &[cache, recv, mid, a, e]))
            }
            [a] => plain(self, fx, "kn_send1", &[cache, recv, mid, *a, e]),
            [a, b] => plain(self, fx, "kn_send2", &[cache, recv, mid, *a, *b, e]),
            _ => unreachable!(),
        }
    }

    /// An attribute read (`val` None) or write without a runtime call, once the runtime has
    /// recorded in the call site's cache that this site reaches an attr_reader/attr_writer
    /// for receivers of one class (kenny-runtime `CallCache::attr`: class << 16 | ivar index;
    /// cleared when any method changes). Anything else runs `slow`, `kn_attr_get` or
    /// `kn_attr_set`, which records the class (other classes: the direct-call stub's ways).
    ///
    /// Used only inside loops (`while`/`until`/`for` and inlined `each`/`times`/... blocks):
    /// elsewhere the call itself is most of the cost, and inlining at every site would grow
    /// a Rails binary's code by a quarter. (Checking several classes here, from the cache
    /// entry, made monomorphic loops like nbody's 3-5% slower.)
    pub(crate) fn attr_fast(&mut self, fx: &mut Fx, cache: Value, recv: Value, val: Option<Value>, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);

        // a recorded attribute (most sites never record one: one load and a branch), and a
        // heap object (not an immediate, nil or false)
        let word = fx.b.ins().load(I64, f, cache, CC_ATTR);
        let b0 = fx.b.create_block();
        fx.b.ins().brif(word, b0, &[], slow_b, &[]);
        fx.b.switch_to_block(b0);
        let low = fx.b.ins().band_imm_s(recv, 7);
        let aligned = fx.b.ins().icmp_imm_s(IntCC::Equal, low, 0);
        let not_nil = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
        let heap = fx.b.ins().band(aligned, not_nil);
        let b1 = fx.b.create_block();
        fx.b.ins().brif(heap, b1, &[], slow_b, &[]);

        // of the recorded class, a plain Object
        fx.b.switch_to_block(b1);
        let klass = fx.b.ins().load(I64, f, recv, 0);
        let kind = fx.b.ins().load(I32, f, recv, OBJ_KIND);
        let wk = fx.b.ins().ushr_imm_u(word, 16);
        let same = fx.b.ins().icmp(IntCC::Equal, klass, wk);
        let is_obj = fx.b.ins().icmp_imm_s(IntCC::Equal, kind, 0);
        let ok = fx.b.ins().band(same, is_obj);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(ok, b2, &[], slow_b, &[]);

        // the slot is within the object's ivar buffer (capacity first, as SVec::snap does)
        fx.b.switch_to_block(b2);
        let cap_addr = fx.b.ins().iadd_imm_s(recv, OBJ_IVARS_CAP as i64);
        let cap = fx.b.ins().atomic_load(I64, f, cap_addr);
        let ptr = fx.b.ins().load(I64, f, recv, OBJ_IVARS_PTR);
        let len = fx.b.ins().load(I64, f, recv, OBJ_IVARS_LEN);
        let capn = fx.b.ins().band_imm_s(cap, SVEC_CAP_MASK);
        let n = fx.b.ins().umin(len, capn);
        let idx = fx.b.ins().band_imm_s(word, 0xFFFF);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, idx, n);
        let off = fx.b.ins().ishl_imm_u(idx, 3);
        let addr = fx.b.ins().iadd(ptr, off);
        let b3 = fx.b.create_block();
        match val {
            None => {
                fx.b.ins().brif(inb, b3, &[], slow_b, &[]);
                // an ivar removed with remove_instance_variable reads as nil (slow path)
                fx.b.switch_to_block(b3);
                let v = fx.b.ins().load(I64, f, addr, 0);
                let unset = fx.b.ins().icmp_imm_s(IntCC::Equal, v, UNDEF);
                fx.b.ins().brif(unset, slow_b, &[], done, &[v.into()]);
            }
            Some(a) => {
                // not frozen, and no write barrier due (an old buffer not yet remembered
                // takes the runtime path, which remembers it)
                let flags = fx.b.ins().load(I32, f, recv, OBJ_FLAGS);
                let frozen = fx.b.ins().band_imm_s(flags, FL_FROZEN);
                let not_frozen = fx.b.ins().icmp_imm_s(IntCC::Equal, frozen, 0);
                let old_bits = fx.b.ins().band_imm_s(cap, SVEC_FLAGS);
                let no_wb = fx.b.ins().icmp_imm_s(IntCC::NotEqual, old_bits, SVEC_OLD);
                let w1 = fx.b.ins().band(inb, not_frozen);
                let w = fx.b.ins().band(w1, no_wb);
                fx.b.ins().brif(w, b3, &[], slow_b, &[]);
                fx.b.switch_to_block(b3);
                fx.b.ins().store(f, a, addr, 0);
                fx.b.ins().jump(done, &[a.into()]);
            }
        }

        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
}

impl<M: Module> Compiler<M> {
    /// `@x` (`val` None) or `@x = val` on `recv` (self) without a runtime call, through the
    /// site's ivar cache (kenny-runtime abi.rs `IvCache`: shape | slot << 48, filled by
    /// `kn_ivar_get`/`kn_ivar_set`) when `recv` is a plain Object of that shape whose ivar
    /// buffer already has the slot. Anything else (first run, another class, a frozen
    /// object, a buffer to grow or to remember for the GC) runs `slow`.
    pub(crate) fn ivar_fast(&mut self, fx: &mut Fx, cache: Value, recv: Value, val: Option<Value>, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);

        // a filled cache, and a heap object
        let word = fx.b.ins().load(I64, f, cache, 0);
        let b0 = fx.b.create_block();
        fx.b.ins().brif(word, b0, &[], slow_b, &[]);
        fx.b.switch_to_block(b0);
        let low = fx.b.ins().band_imm_s(recv, 7);
        let aligned = fx.b.ins().icmp_imm_s(IntCC::Equal, low, 0);
        let not_nil = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
        let heap = fx.b.ins().band(aligned, not_nil);
        let b1 = fx.b.create_block();
        fx.b.ins().brif(heap, b1, &[], slow_b, &[]);

        // a plain Object (kind 0; for a write also not frozen: kind and flags are one word)
        // of the cached shape
        fx.b.switch_to_block(b1);
        let kf = fx.b.ins().load(I64, f, recv, OBJ_KIND);
        let mask = if val.is_some() { 0xFFFF_FFFF | (FL_FROZEN << 32) } else { 0xFFFF_FFFF };
        let kfm = fx.b.ins().band_imm_s(kf, mask);
        let plain = fx.b.ins().icmp_imm_s(IntCC::Equal, kfm, 0);
        let shape = fx.b.ins().load(I64, f, recv, OBJ_SHAPE);
        let wshape = fx.b.ins().band_imm_s(word, IV_PTR);
        let same = fx.b.ins().icmp(IntCC::Equal, shape, wshape);
        let ok = fx.b.ins().band(plain, same);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(ok, b2, &[], slow_b, &[]);

        // the slot is within the buffer (capacity first, as SVec::snap does)
        fx.b.switch_to_block(b2);
        let cap_addr = fx.b.ins().iadd_imm_s(recv, OBJ_IVARS_CAP as i64);
        let cap = fx.b.ins().atomic_load(I64, f, cap_addr);
        let ptr = fx.b.ins().load(I64, f, recv, OBJ_IVARS_PTR);
        let len = fx.b.ins().load(I64, f, recv, OBJ_IVARS_LEN);
        let capn = fx.b.ins().band_imm_s(cap, SVEC_CAP_MASK);
        let n = fx.b.ins().umin(len, capn);
        let idx = fx.b.ins().ushr_imm_u(word, 48);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, idx, n);
        let off = fx.b.ins().ishl_imm_u(idx, 3);
        let addr = fx.b.ins().iadd(ptr, off);
        let b3 = fx.b.create_block();
        match val {
            None => {
                fx.b.ins().brif(inb, b3, &[], slow_b, &[]);
                // a removed ivar (UNDEF) reads as nil
                fx.b.switch_to_block(b3);
                let v = fx.b.ins().load(I64, f, addr, 0);
                let unset = fx.b.ins().icmp_imm_s(IntCC::Equal, v, UNDEF);
                let nil = fx.b.ins().iconst(I64, QNIL);
                let r = fx.b.ins().select(unset, nil, v);
                fx.b.ins().jump(done, &[r.into()]);
            }
            Some(a) => {
                // no write barrier due (an old buffer not yet remembered: the runtime path)
                let old_bits = fx.b.ins().band_imm_s(cap, SVEC_FLAGS);
                let no_wb = fx.b.ins().icmp_imm_s(IntCC::NotEqual, old_bits, SVEC_OLD);
                let w = fx.b.ins().band(inb, no_wb);
                fx.b.ins().brif(w, b3, &[], slow_b, &[]);
                fx.b.switch_to_block(b3);
                fx.b.ins().store(f, a, addr, 0);
                fx.b.ins().jump(done, &[a.into()]);
            }
        }

        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
}

impl<M: Module> Compiler<M> {
    /// A constant read in a loop: the site's cache entry (kenny-runtime abi.rs `ConstEntry`
    /// {serial, cref, val}) read inline when it is current (`KN_DIRECT.const_serial`) for
    /// this lexical scope, else `kn_const_get_l`, which looks it up and fills it.
    pub(crate) fn const_cached(&mut self, fx: &mut Fx, cache: Value, crefp: Value, id: Value) -> Value {
        let f = mem();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        let e = fx.b.ins().atomic_load(I64, f, cache);
        let b0 = fx.b.create_block();
        fx.b.ins().brif(e, b0, &[], slow, &[]);
        fx.b.switch_to_block(b0);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let cur = fx.b.ins().load(I64, f, dir, KD_CONST_SERIAL);
        let ser = fx.b.ins().load(I64, f, e, 0);
        let ecref = fx.b.ins().load(I64, f, e, 8);
        let cref = fx.b.ins().load(I64, f, crefp, 0);
        let s1 = fx.b.ins().icmp(IntCC::Equal, ser, cur);
        let s2 = fx.b.ins().icmp(IntCC::Equal, ecref, cref);
        let ok = fx.b.ins().band(s1, s2);
        let hit = fx.b.create_block();
        fx.b.ins().brif(ok, hit, &[], slow, &[]);
        fx.b.switch_to_block(hit);
        let v = fx.b.ins().load(I64, f, e, 16);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(slow);
        let v = self.callc(fx, "kn_const_get_l", &[cache, crefp, id]);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// A call to a compiled Ruby method without going through the runtime: when the
    /// site's inline cache (kenny-runtime core.rs `CacheEntry`) is current and maps the
    /// receiver's class to a compiled method (`direct`), call its function here. Also
    /// required: a heap receiver, a public method for a call with an explicit receiver, no
    /// collection pending, and the stack above `KN_DIRECT.call_limit` (only one OS thread
    /// runs Ruby code). Anything else runs `slow` (`kn_send0`...), which also fills the
    /// cache.
    pub(crate) fn direct_call(&mut self, fx: &mut Fx, cache: Value, recv: Value, vals: &[Value], explicit: bool, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);

        // a filled cache with a compiled method in a checked way (sites of core methods,
        // most of them, leave here)
        let e = fx.b.ins().load(I64, f, cache, 0);
        let b0 = fx.b.create_block();
        fx.b.ins().brif(e, b0, &[], slow_b, &[]);
        fx.b.switch_to_block(b0);
        let ways = WAYS;
        let fs: Vec<Value> = (0..ways).map(|w| fx.b.ins().load(I64, f, e, CE_DIRECT + 8 * w as i32)).collect();
        let mut any = fs[0];
        for fw in &fs[1..] {
            any = fx.b.ins().bor(any, *fw);
        }
        let b1 = fx.b.create_block();
        fx.b.ins().brif(any, b1, &[], slow_b, &[]);

        // a heap receiver whose class is in one of those ways, under the current serial
        fx.b.switch_to_block(b1);
        let low = fx.b.ins().band_imm_s(recv, 7);
        let aligned = fx.b.ins().icmp_imm_s(IntCC::Equal, low, 0);
        let not_nil = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
        let heap = fx.b.ins().band(aligned, not_nil);
        let b2h = fx.b.create_block();
        fx.b.ins().brif(heap, b2h, &[], slow_b, &[]);
        fx.b.switch_to_block(b2h);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let ser = fx.b.ins().load(I64, f, e, CE_SERIAL);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let mut ok = fx.b.ins().icmp(IntCC::Equal, ser, cur);
        let mut func = fx.b.ins().iconst(I64, 0);
        for w in (0..ways).rev() {
            let kw = fx.b.ins().load(I64, f, e, CE_KLASS + 8 * w as i32);
            let hit = fx.b.ins().icmp(IntCC::Equal, kw, k);
            func = fx.b.ins().select(hit, fs[w], func);
        }
        // a compiled method (public, with an explicit receiver: the low bit marks private)
        let callable = if explicit {
            let priv_bit = fx.b.ins().band_imm_s(func, 1);
            let public = fx.b.ins().icmp_imm_s(IntCC::Equal, priv_bit, 0);
            let some = fx.b.ins().icmp_imm_s(IntCC::NotEqual, func, 0);
            fx.b.ins().band(public, some)
        } else {
            func = fx.b.ins().band_imm_s(func, !1);
            fx.b.ins().icmp_imm_s(IntCC::NotEqual, func, 0)
        };
        ok = fx.b.ins().band(ok, callable);
        // room on the stack (one OS thread), no collection pending
        let sp = fx.b.ins().get_stack_pointer(I64);
        let lim = fx.b.ins().load(I64, f, dir, 8);
        let room = fx.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, sp, lim);
        let gc = fx.b.ins().load(I8, f, fx.gcflag, 0);
        let calm = fx.b.ins().icmp_imm_s(IntCC::Equal, gc, 0);
        let ok2 = fx.b.ins().band(room, calm);
        ok = fx.b.ins().band(ok, ok2);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(ok, b2, &[], slow_b, &[]);

        fx.b.switch_to_block(b2);
        let argv = if vals.is_empty() { fx.b.ins().iconst(I64, 8) } else { self.args_buf(fx, vals) };
        let argc = fx.b.ins().iconst(I64, vals.len() as i64);
        let nil = fx.b.ins().iconst(I64, QNIL);
        let sig = self.method_sig();
        let sr = fx.b.import_signature(sig);
        let call = fx.b.ins().call_indirect(sr, func, &[recv, argc, argv, nil]);
        let r = fx.b.inst_results(call)[0];
        fx.b.ins().jump(done, &[r.into()]);

        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
}

/// The direct-call stub kinds: the runtime function each one falls back to, its number of
/// method arguments, whether it takes `explicit` (or `mode`, see opt.rs `kn_attr_get`), and
/// the attribute access it also does inline (0 none, 1 reader, 2 writer), or the builtin
/// method it answers (3 nil?, 4 is_a?/kind_of?).
const STUBS: [(&str, usize, bool, u8); 10] = [
    ("kn_send0", 0, true, 1),
    ("kn_send1", 1, true, 2),
    ("kn_send2", 2, true, 0),
    ("kn_vcall", 0, false, 1),
    ("kn_attr_get", 0, true, 1),
    ("kn_attr_set", 1, true, 2),
    // for names that can't be attributes
    ("kn_send0", 0, true, 0),
    ("kn_send1", 1, true, 0),
    ("kn_send0", 0, true, 3),
    ("kn_send1", 1, true, 4),
];
pub(crate) const STUB_SEND0: u8 = 0;
pub(crate) const STUB_VCALL: u8 = 3;
pub(crate) const STUB_ATTR_GET: u8 = 4;
pub(crate) const STUB_ATTR_SET: u8 = 5;
const STUB_NIL_P: u8 = 8;
const STUB_IS_A: u8 = 9;

impl<M: Module> Compiler<M> {
    /// A call through this module's direct-call stub of `kind` (see `STUBS`), which makes
    /// the call without the runtime when the site's inline cache allows (see `direct_stub`),
    /// else calls the runtime function. One shared stub instead of checks at every call
    /// site keeps code small.
    pub(crate) fn call_stub(&mut self, fx: &mut Fx, kind: u8, args: &[Value]) -> Value {
        let id = self.direct_stub(kind);
        self.call_fn(fx, id, args)
    }

    /// The stub: when the site's cache entry (kenny-runtime core.rs `CacheEntry`) is
    /// current and one of its first ways is the receiver's class (a heap object), either
    /// - calls the compiled method (`direct`): public, or the call has no explicit
    ///   receiver; no collection pending; the stack above `KN_DIRECT.call_limit`, or
    /// - reads/writes the attribute's slot (`attr`): a plain Object allocated from that
    ///   class, the slot within its buffer; a write needs no write barrier and an unfrozen
    ///   object, or
    /// - answers the builtin nil? or is_a? (`attr` tags `ATTR_NIL_P`, `ATTR_IS_A`; for
    ///   immediate receivers `KN_DIRECT.flags` says whether they are the builtin ones).
    fn direct_stub(&mut self, kind: u8) -> FuncId {
        if let Some(&id) = self.direct_stubs.get(&kind) {
            return id;
        }
        let (rt, nargs, has_explicit, attr) = STUBS[kind as usize];
        let mut sig = self.module.make_signature();
        for _ in 0..3 + nargs + has_explicit as usize {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        let id = self.module.declare_function(&format!("__kn_direct_{kind}"), Linkage::Local, &sig).expect("declare stub");
        self.direct_stubs.insert(kind, id);
        let mut func = ir::Function::with_name_signature(ir::UserFuncName::user(0, id.as_u32()), sig);
        let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
        let rt_id = self.rt_id(rt);
        let msig = self.method_sig();
        {
            let mut b = cranelift_frontend::FunctionBuilder::new(&mut func, &mut fctx);
            let f = mem();
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry).to_vec();
            let (cache, recv) = (p[0], p[1]);
            let args = p[3..3 + nargs].to_vec();
            let explicit = if has_explicit { Some(p[3 + nargs]) } else { None };
            let slow = b.create_block();
            b.set_cold_block(slow);
            let rt_is_a = self.rt_id("kn_is_a");
            let gv = self.module.declare_data_in_func(self.direct_id, b.func);
            let dir = b.ins().symbol_value(I64, gv);
            let builtin = attr >= 3;
            if builtin {
                // an immediate receiver: the builtin method when the flag says so
                let low = b.ins().band_imm_s(recv, 7);
                let aligned = b.ins().icmp_imm_s(IntCC::Equal, low, 0);
                let not_nil = b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
                let heap = b.ins().band(aligned, not_nil);
                let imm = b.create_block();
                let on = b.create_block();
                b.ins().brif(heap, on, &[], imm, &[]);
                b.switch_to_block(imm);
                let flags = b.ins().load(I64, f, dir, KD_FLAGS);
                let bit = b.ins().band_imm_s(flags, if attr == 3 { DIRECT_NIL_P } else { DIRECT_IS_A });
                let go = b.create_block();
                b.ins().brif(bit, go, &[], slow, &[]);
                b.switch_to_block(go);
                if attr == 3 {
                    let is_nil = b.ins().icmp_imm_s(IntCC::Equal, recv, QNIL);
                    let t = b.ins().iconst(I64, QTRUE);
                    let fl = b.ins().iconst(I64, QFALSE);
                    let r = b.ins().select(is_nil, t, fl);
                    b.ins().return_(&[r]);
                } else {
                    let rf = self.module.declare_func_in_func(rt_is_a, b.func);
                    let call = b.ins().call(rf, &[recv, args[0]]);
                    let r = b.inst_results(call)[0];
                    b.ins().return_(&[r]);
                }
                b.switch_to_block(on);
            }

            // a filled cache with a compiled method or an attribute in a checked way
            let e = b.ins().load(I64, f, cache, 0);
            let b0 = b.create_block();
            b.ins().brif(e, b0, &[], slow, &[]);
            b.switch_to_block(b0);
            let ways = WAYS;
            let fs: Vec<Value> = (0..ways).map(|w| b.ins().load(I64, f, e, CE_DIRECT + 8 * w as i32)).collect();
            let at: Vec<Value> = if attr != 0 { (0..ways).map(|w| b.ins().load(I64, f, e, CE_ATTR + 8 * w as i32)).collect() } else { vec![] };
            let mut any = fs[0];
            for v in fs[1..].iter().chain(at.iter()) {
                any = b.ins().bor(any, *v);
            }
            let b1 = b.create_block();
            b.ins().brif(any, b1, &[], slow, &[]);

            // a heap receiver of a class in those ways, under the current serial
            b.switch_to_block(b1);
            let low = b.ins().band_imm_s(recv, 7);
            let aligned = b.ins().icmp_imm_s(IntCC::Equal, low, 0);
            let not_nil = b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
            let heap = b.ins().band(aligned, not_nil);
            let b2 = b.create_block();
            b.ins().brif(heap, b2, &[], slow, &[]);
            b.switch_to_block(b2);
            let k = b.ins().load(I64, f, recv, 0);
            let ser = b.ins().load(I64, f, e, CE_SERIAL);
            let cur = b.ins().load(I64, f, dir, 0);
            let current = b.ins().icmp(IntCC::Equal, ser, cur);
            let mut fsel = b.ins().iconst(I64, 0);
            let mut asel = b.ins().iconst(I64, 0);
            for w in (0..ways).rev() {
                let kw = b.ins().load(I64, f, e, CE_KLASS + 8 * w as i32);
                let hit = b.ins().icmp(IntCC::Equal, kw, k);
                fsel = b.ins().select(hit, fs[w], fsel);
                if attr != 0 {
                    asel = b.ins().select(hit, at[w], asel);
                }
            }
            // private (low bit) only without an explicit receiver (explicit/mode 1)
            let allowed = |b: &mut cranelift_frontend::FunctionBuilder, v: Value| match explicit {
                Some(x) => {
                    let pb = b.ins().band_imm_s(v, 1);
                    let denied = b.ins().band(pb, x);
                    b.ins().icmp_imm_s(IntCC::Equal, denied, 0)
                }
                None => b.ins().iconst(I8, 1),
            };

            // an attribute site: the attribute first (the right kind, reader/writer)
            let attr_b = b.create_block();
            if matches!(attr, 1 | 2) {
                let tagged = b.ins().band_imm_s(asel, 4 | 2);
                let want = if attr == 2 { 6 } else { 4 };
                let kind_ok = b.ins().icmp_imm_s(IntCC::Equal, tagged, want);
                let mut ok = b.ins().band(current, kind_ok);
                let al = allowed(&mut b, asel);
                ok = b.ins().band(ok, al);
                let callchk = b.create_block();
                b.ins().brif(ok, attr_b, &[], callchk, &[]);
                b.switch_to_block(callchk);
            }

            // the compiled method, with room on the stack and no collection pending
            let clean = b.ins().band_imm_s(fsel, !1);
            let some = b.ins().icmp_imm_s(IntCC::NotEqual, clean, 0);
            let mut ok = b.ins().band(current, some);
            let al = allowed(&mut b, fsel);
            ok = b.ins().band(ok, al);
            let sp = b.ins().get_stack_pointer(I64);
            let lim = b.ins().load(I64, f, dir, 8);
            let room = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, sp, lim);
            let gfv = self.module.declare_data_in_func(self.gc_flag_id, b.func);
            let gflag = b.ins().symbol_value(I64, gfv);
            let gc = b.ins().load(I8, f, gflag, 0);
            let calm = b.ins().icmp_imm_s(IntCC::Equal, gc, 0);
            let ok2 = b.ins().band(room, calm);
            ok = b.ins().band(ok, ok2);
            let call_b = b.create_block();
            let not_call = if builtin { b.create_block() } else { slow };
            b.ins().brif(ok, call_b, &[], not_call, &[]);

            b.switch_to_block(call_b);
            let argv = if nargs == 0 {
                b.ins().iconst(I64, 8)
            } else {
                let ss = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (nargs * 8) as u32, 3));
                for (i, v) in args.iter().enumerate() {
                    b.ins().stack_store(I64, *v, ss, (i * 8) as i32);
                }
                b.ins().stack_addr(I64, ss, 0)
            };
            let argc = b.ins().iconst(I64, nargs as i64);
            let nil = b.ins().iconst(I64, QNIL);
            let sr = b.import_signature(msig);
            let call = b.ins().call_indirect(sr, clean, &[recv, argc, argv, nil]);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);

            if builtin {
                // the builtin method for this class (a heap object here: never nil)
                b.switch_to_block(not_call);
                let tagged = b.ins().band_imm_s(asel, 0x1e);
                let want = if attr == 3 { ATTR_NIL_P } else { ATTR_IS_A };
                let kind_ok = b.ins().icmp_imm_s(IntCC::Equal, tagged, want);
                let mut ok = b.ins().band(current, kind_ok);
                let al = allowed(&mut b, asel);
                ok = b.ins().band(ok, al);
                let go = b.create_block();
                b.ins().brif(ok, go, &[], slow, &[]);
                b.switch_to_block(go);
                if attr == 3 {
                    let r = b.ins().iconst(I64, QFALSE);
                    b.ins().return_(&[r]);
                } else {
                    // the answer recorded for this argument (kenny-runtime abi.rs
                    // `kn_is_a_fill`), else work it out and record it
                    let rc = b.ins().ushr_imm_u(asel, 16);
                    let same = b.ins().icmp(IntCC::Equal, rc, args[0]);
                    let known = b.create_block();
                    let walk = b.create_block();
                    b.ins().brif(same, known, &[], walk, &[]);
                    b.switch_to_block(known);
                    let bit = b.ins().band_imm_s(asel, 32);
                    let t = b.ins().iconst(I64, QTRUE);
                    let fl = b.ins().iconst(I64, QFALSE);
                    let r = b.ins().select(bit, t, fl);
                    b.ins().return_(&[r]);
                    b.switch_to_block(walk);
                    let fill = self.rt_id("kn_is_a_fill");
                    let rf = self.module.declare_func_in_func(fill, b.func);
                    let call = b.ins().call(rf, &[cache, recv, args[0]]);
                    let r = b.inst_results(call)[0];
                    b.ins().return_(&[r]);
                }
            }
            if matches!(attr, 1 | 2) {
                // the attribute: a plain Object (kind 0; for a write also not frozen)
                // allocated from its class
                b.switch_to_block(attr_b);
                let kf = b.ins().load(I64, f, recv, OBJ_KIND);
                let mask = if attr == 2 { 0xFFFF_FFFF | (FL_FROZEN << 32) } else { 0xFFFF_FFFF };
                let kfm = b.ins().band_imm_s(kf, mask);
                let plain = b.ins().icmp_imm_s(IntCC::Equal, kfm, 0);
                let shape = b.ins().load(I64, f, recv, OBJ_SHAPE);
                let own = b.ins().icmp(IntCC::Equal, shape, k);
                let cap_addr = b.ins().iadd_imm_s(recv, OBJ_IVARS_CAP as i64);
                let cap = b.ins().atomic_load(I64, f, cap_addr);
                let ptr = b.ins().load(I64, f, recv, OBJ_IVARS_PTR);
                let len = b.ins().load(I64, f, recv, OBJ_IVARS_LEN);
                let capn = b.ins().band_imm_s(cap, SVEC_CAP_MASK);
                let n = b.ins().umin(len, capn);
                let idx = b.ins().ushr_imm_u(asel, 8);
                let inb = b.ins().icmp(IntCC::UnsignedLessThan, idx, n);
                let o1 = b.ins().band(plain, own);
                let mut ok = b.ins().band(o1, inb);
                if attr == 2 {
                    let old_bits = b.ins().band_imm_s(cap, SVEC_FLAGS);
                    let no_wb = b.ins().icmp_imm_s(IntCC::NotEqual, old_bits, SVEC_OLD);
                    ok = b.ins().band(ok, no_wb);
                }
                let off = b.ins().ishl_imm_u(idx, 3);
                let addr = b.ins().iadd(ptr, off);
                let go = b.create_block();
                b.ins().brif(ok, go, &[], slow, &[]);
                b.switch_to_block(go);
                if attr == 2 {
                    b.ins().store(f, args[0], addr, 0);
                    b.ins().return_(&[args[0]]);
                } else {
                    let v = b.ins().load(I64, f, addr, 0);
                    let unset = b.ins().icmp_imm_s(IntCC::Equal, v, UNDEF);
                    let nil = b.ins().iconst(I64, QNIL);
                    let r = b.ins().select(unset, nil, v);
                    b.ins().return_(&[r]);
                }
            }

            b.switch_to_block(slow);
            let rf = self.module.declare_func_in_func(rt_id, b.func);
            let call = b.ins().call(rf, &p);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.seal_all_blocks();
            b.finalize(self.module.isa().frontend_config());
        }
        let mut ctx = cranelift_codegen::Context::for_function(func);
        if let Err(e) = self.module.define_function(id, &mut ctx) {
            self.errors.push(format!("internal codegen error in direct-call stub: {e:?}"));
        }
        id
    }
}

/// Worth checking for a direct call: not an operator or a name that is almost always a
/// core method (those sites go straight to the runtime, which handles them directly).
pub(crate) fn may_be_ruby_method(name: &[u8]) -> bool {
    if !name.iter().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    !matches!(
        name,
        b"size" | b"length" | b"count" | b"empty?" | b"nil?" | b"first" | b"last" | b"push" | b"pop" | b"shift" | b"unshift"
            | b"key?" | b"has_key?" | b"include?" | b"fetch" | b"dig" | b"delete" | b"keys" | b"values" | b"to_s" | b"to_sym"
            | b"to_i" | b"to_f" | b"to_a" | b"to_h" | b"dup" | b"freeze" | b"frozen?" | b"is_a?" | b"kind_of?"
            | b"instance_of?" | b"respond_to?" | b"class" | b"equal?" | b"eql?" | b"hash" | b"abs" | b"floor"
            | b"ceil" | b"round" | b"zero?" | b"even?" | b"odd?" | b"chr" | b"ord" | b"succ" | b"pred" | b"min"
            | b"max" | b"sum" | b"sort" | b"reverse" | b"join" | b"split" | b"strip" | b"upcase" | b"downcase"
            | b"start_with?" | b"end_with?" | b"sub" | b"gsub" | b"match?" | b"concat" | b"append" | b"merge"
            | b"compact" | b"flatten" | b"uniq" | b"sqrt" | b"new" | b"format" | b"sprintf" | b"puts" | b"print"
            | b"p" | b"raise" | b"require" | b"require_relative" | b"send" | b"public_send" | b"__send__"
            | b"instance_variable_get" | b"instance_variable_set" | b"lambda" | b"proc" | b"block_given?"
            | b"object_id" | b"inspect" | b"then" | b"tap" | b"itself" | b"at" | b"values_at" | b"slice" | b"store"
    )
}

/// How many ways of a cache entry compiled code checks (of kenny-runtime's `IC_WAYS`):
/// checking 3 or 4 made the method benchmarks slower.
const WAYS: usize = 2;
/// (check inline at the call site, call through the stub) for a call of `name`: inline in
/// loops; elsewhere the shared stub (inline at every site grew a Rails binary's code from
/// 60 to 92 MB).
pub(crate) fn direct_plan(name: &[u8], in_loop: bool) -> (bool, bool) {
    if !may_be_ruby_method(name) {
        return (false, false);
    }
    (in_loop, !in_loop)
}

/// kenny-runtime core.rs `CacheEntry` (repr(C)): serial, n, klass[4], method[4], direct[4],
/// attr[4].
const CE_SERIAL: i32 = 0;
const CE_KLASS: i32 = 16;
const CE_DIRECT: i32 = 80;
const CE_ATTR: i32 = 112;

const OBJ_SHAPE: i32 = 16;
/// kenny-runtime core.rs `KnDirect::flags` and its bits, `SYM_TAG`.
const KD_FLAGS: i32 = 16;
const KD_CONST_SERIAL: i32 = 24;
const DIRECT_SYM_EQ: i64 = 1;
const DIRECT_NIL_P: i64 = 2;
const DIRECT_IS_A: i64 = 4;
/// kenny-runtime core.rs `ATTR_NIL_P`, `ATTR_IS_A`.
const ATTR_NIL_P: i64 = 8;
const ATTR_IS_A: i64 = 16;
const SYM_TAG: i64 = 0x0c;
/// kenny-runtime abi.rs `IV_PTR`: the shape bits of an `IvCache` word.
const IV_PTR: i64 = (1 << 48) - 1;

/// Layout read by the attribute fast path (kenny-runtime core.rs `Header`, `RObject` and
/// `CallCache`, svec.rs `SVec`).
const OBJ_KIND: i32 = 8;
const OBJ_FLAGS: i32 = 12;
const OBJ_IVARS_PTR: i32 = 24;
const OBJ_IVARS_LEN: i32 = 32;
const OBJ_IVARS_CAP: i32 = 40;
const FL_FROZEN: i64 = 1;
const SVEC_CAP_MASK: i64 = (1 << 56) - 1;
const SVEC_OLD: i64 = 1 << 62;
const SVEC_FLAGS: i64 = (1 << 62) | (1 << 61);
const CC_ATTR: i32 = 16;

/// A name `attr_reader`/`attr_writer` can define (a valid instance variable name minus `@`).
pub(crate) fn attr_name(name: &[u8]) -> bool {
    match name.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == b'_' || *c >= 0x80 => {}
        _ => return false,
    }
    name.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 0x80)
}

fn int_cc(op: &[u8]) -> IntCC {
    match op {
        b"<" => IntCC::SignedLessThan,
        b"<=" => IntCC::SignedLessThanOrEqual,
        b">" => IntCC::SignedGreaterThan,
        b">=" => IntCC::SignedGreaterThanOrEqual,
        b"==" => IntCC::Equal,
        _ => IntCC::NotEqual,
    }
}

pub(crate) fn is_fast_op(op: &[u8]) -> bool {
    matches!(op, b"+" | b"-" | b"*" | b"/" | b"%" | b"<" | b"<=" | b">" | b">=" | b"==" | b"!=" | b"&" | b"|" | b"^")
}
