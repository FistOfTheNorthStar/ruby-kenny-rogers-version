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
                let a = self.num_expr(fx, &recv, true);
                let b = self.num_expr(fx, &arg, true);
                self.cur_offset = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
                return self.binop_num(fx, &op, a, b, lazy);
            }
        }
        Num::Boxed(self.expr(fx, node))
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
        let lit_int = a.is_int_lit(fx) || b.is_int_lit(fx);
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
        match vals {
            [] if in_loop && attr_name(name) => self.attr_fast(fx, cache, recv, None, |c, fx| c.call(fx, "kn_attr_get", &[cache, recv, mid, e])),
            [] => self.call(fx, "kn_send0", &[cache, recv, mid, e]),
            [a] if in_loop && name.last() == Some(&b'=') && attr_name(&name[..name.len() - 1]) => {
                let a = *a;
                self.attr_fast(fx, cache, recv, Some(a), |c, fx| c.call(fx, "kn_attr_set", &[cache, recv, mid, a, e]))
            }
            [a] => self.call(fx, "kn_send1", &[cache, recv, mid, *a, e]),
            [a, b] => self.call(fx, "kn_send2", &[cache, recv, mid, *a, *b, e]),
            _ => unreachable!(),
        }
    }

    /// An attribute read (`val` None) or write without a runtime call, once the runtime has
    /// recorded in the call site's cache that this site reaches an attr_reader/attr_writer
    /// for receivers of one class (kenny-runtime `CallCache::attr`: class << 16 | ivar index;
    /// cleared when any method changes). Anything else runs `slow`, `kn_attr_get` or
    /// `kn_attr_set`, which records the class.
    ///
    /// Used only inside loops (`while`/`until`/`for` and inlined `each`/`times`/... blocks):
    /// elsewhere the call itself is most of the cost, and inlining at every site would grow
    /// a Rails binary's code by a quarter.
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
