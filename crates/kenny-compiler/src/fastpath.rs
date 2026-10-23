//! Type-specialised code: inline Fixnum and Flonum arithmetic/comparisons, and the direct
//! `kn_send0/1/2` entry points for calls with few arguments and no block.
//!
//! Integer and Float operators can't be redefined (LANGUAGE.md rule 2), so `a + b` on two
//! Fixnums or Floats needs no guard beyond the tag checks. Every other specialisation is
//! guarded in the runtime by the call site's inline cache (see kenny-runtime/src/opt.rs).

use crate::*;
use ruby_prism::Node;
use cranelift_codegen::ir::{condcodes::FloatCC, types::F64, types::I8};

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
pub(crate) fn const_of(fx: &Fx, v: Value) -> Option<i64> {
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
    /// Always a Float (an always-Float local, or arithmetic on such values): the f64.
    F64(Value),
}
impl Num {
    /// (only for `Boxed` / `Lazy`: `Compiler::lazy` turns `F64` into a `Lazy` first)
    fn boxed(&self) -> Value {
        match self {
            Num::Boxed(v) => *v,
            Num::Lazy { boxed, .. } => *boxed,
            Num::F64(_) => unreachable!("Num::F64 has no tagged value"),
        }
    }
    fn is_float_lit(&self, fx: &Fx) -> bool {
        match self {
            Num::Boxed(v) => const_of(fx, *v).is_some_and(|c| c & 3 == 2),
            Num::Lazy { lit, .. } => *lit,
            Num::F64(_) => false,
        }
    }
    fn is_int_lit(&self, fx: &Fx) -> bool {
        match self {
            Num::Boxed(v) => const_of(fx, *v).is_some_and(|c| c & 1 == 1),
            _ => false,
        }
    }
    /// The f64 when this is known to be a Float (an always-Float value or a literal).
    fn static_f64(&self) -> Option<Value> {
        match self {
            Num::F64(r) => Some(*r),
            Num::Lazy { raw, lit: true, .. } => Some(*raw),
            _ => None,
        }
    }
    /// Known to be an Integer (Fixnum or Bignum): a literal or an always-Integer value.
    fn is_sure_int(&self, fx: &Fx) -> bool {
        matches!(self, Num::Boxed(v) if fx.sure_ints.contains(v) || const_of(fx, *v).is_some_and(|c| c & 1 == 1))
    }
    /// An Integer literal, or (most likely) an Integer: see `Fx::int_vals`.
    fn is_int_hint(&self, fx: &Fx) -> bool {
        self.is_int_lit(fx) || matches!(self, Num::Boxed(v) if fx.int_vals.contains(v) || fx.sure_ints.contains(v))
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

/// kenny-runtime opt.rs `MATH_FNS` (same order: `kn_math1`'s `which`).
const MATH_FNS: [&[u8]; 15] = [b"sqrt", b"sin", b"cos", b"tan", b"atan", b"exp", b"log", b"log2", b"log10", b"cbrt", b"sinh", b"cosh", b"tanh", b"asin", b"acos"];

/// A numeric call computed inline while the methods are the builtin ones (`num_call`).
pub(crate) enum NumCall<'pr> {
    /// `Math.f(x)` (index into `MATH_FNS`)
    Math(usize, ruby_prism::Node<'pr>),
    /// `x.to_f`
    ToF(ruby_prism::Node<'pr>),
    /// `-x`
    Neg(ruby_prism::Node<'pr>),
    /// `x ** y`
    Pow(ruby_prism::Node<'pr>, ruby_prism::Node<'pr>),
    /// `x << y` / `x >> y` with an Integer literal shift (left: true for `<<`)
    Shift(ruby_prism::Node<'pr>, ruby_prism::Node<'pr>, bool),
}

/// `c` as a `NumCall`, if it is one.
pub(crate) fn num_call_of<'pr>(c: &ruby_prism::CallNode<'pr>) -> Option<NumCall<'pr>> {
    if c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let recv = c.receiver()?;
    let args: Vec<ruby_prism::Node> = match c.arguments() {
        Some(a) => a.arguments().iter().collect(),
        None => vec![],
    };
    if args.iter().any(|a| a.as_splat_node().is_some() || a.as_keyword_hash_node().is_some() || a.as_block_argument_node().is_some() || a.as_forwarding_arguments_node().is_some()) {
        return None;
    }
    let name = c.name().as_slice();
    match (name, args.len()) {
        (b"to_f", 0) => Some(NumCall::ToF(recv)),
        (b"-@", 0) => Some(NumCall::Neg(recv)),
        (b"**", 1) => Some(NumCall::Pow(recv, args.into_iter().next().unwrap())),
        // (`<<` is mostly Array/String append: only with a literal shift)
        (b"<<" | b">>", 1) if args[0].as_integer_node().is_some() => {
            Some(NumCall::Shift(recv, args.into_iter().next().unwrap(), name == b"<<"))
        }
        (_, 1) => {
            let which = MATH_FNS.iter().position(|f| *f == name)?;
            let math = recv.as_constant_read_node().is_some_and(|k| k.name().as_slice() == b"Math")
                || recv.as_constant_path_node().is_some_and(|k| k.parent().is_none() && k.name().is_some_and(|n| n.as_slice() == b"Math"));
            math.then(|| NumCall::Math(which, args.into_iter().next().unwrap()))
        }
        _ => None,
    }
}

impl<M: Module> Compiler<M> {
    /// A `NumCall` (see `num_call_of`): computed inline on Fixnum/Float operands (the
    /// result may stay unboxed) while the methods are the builtin ones (`KN_DIRECT.flags`
    /// `DIRECT_MATH` / `DIRECT_NUM`, and the receiver is the Math module) and the result is
    /// a plain Float: Math domain errors, `**` that would give a Complex, heap Floats and
    /// other receivers make the normal call.
    fn num_call(&mut self, fx: &mut Fx, c: &ruby_prism::CallNode, nc: NumCall) -> Num {
        let at = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
        let name = c.name().as_slice().to_vec();
        let (recv, a, b) = match &nc {
            NumCall::Math(_, arg) => {
                let r = self.expr(fx, &c.receiver().unwrap());
                let a = self.num_expr(fx, arg, true);
                (Num::Boxed(r), self.lazy(fx, a), None)
            }
            NumCall::ToF(r) | NumCall::Neg(r) => {
                let a = self.num_expr(fx, r, true);
                let a = self.lazy(fx, a);
                (a, a, None)
            }
            NumCall::Pow(r, arg) | NumCall::Shift(r, arg, _) => {
                let a = self.num_expr(fx, r, true);
                let b = self.num_expr(fx, arg, true);
                let (a, b) = (self.lazy(fx, a), self.lazy(fx, b));
                (a, a, Some(b))
            }
        };
        self.cur_offset = at;
        let f = mem();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.append_block_param(done, F64);
        fx.b.append_block_param(done, I8);
        fx.b.set_cold_block(slow);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let flags = fx.b.ins().load(I64, f, dir, KD_FLAGS);
        let bit = fx.b.ins().band_imm_s(flags, if matches!(nc, NumCall::Math(..)) { DIRECT_MATH } else { DIRECT_NUM });
        let mut ok = fx.b.ins().icmp_imm_s(IntCC::NotEqual, bit, 0);
        if let NumCall::Math(..) = nc {
            let mm = fx.b.ins().load(I64, f, dir, KD_MATH_MODULE);
            let is_math = fx.b.ins().icmp(IntCC::Equal, recv.boxed(), mm);
            ok = fx.b.ins().band(ok, is_math);
        }
        let nil = fx.b.ins().iconst(I64, QNIL);
        let one = fx.b.ins().iconst(I8, 1);
        match &nc {
            NumCall::Neg(_) => {
                // Fixnum: 2 - tagged is the tagged negation (overflow only for the minimum)
                let av = a.boxed();
                let fixb = fx.b.ins().band_imm_s(av, 1);
                let is_fix = fx.b.ins().icmp_imm_s(IntCC::NotEqual, fixb, 0);
                let fix_ok = fx.b.ins().band(ok, is_fix);
                let fb = fx.b.create_block();
                let flo_b = fx.b.create_block();
                fx.b.ins().brif(fix_ok, fb, &[], flo_b, &[]);
                fx.b.switch_to_block(fb);
                let two = fx.b.ins().iconst(I64, 2);
                let (r, of) = fx.b.ins().ssub_overflow(two, av);
                let z = fx.b.ins().f64const(0.0);
                let k0 = fx.b.ins().iconst(I8, 0);
                fx.b.ins().brif(of, slow, &[], done, &[r.into(), z.into(), k0.into()]);
                fx.b.switch_to_block(flo_b);
                let (sel, _) = self.float_operand(fx, &a, false);
                let is_flo = match sel.0 {
                    Some(hr) => fx.b.ins().bor(hr, sel.1),
                    None => sel.1,
                };
                let ok2 = fx.b.ins().band(ok, is_flo);
                let b2 = fx.b.create_block();
                fx.b.ins().brif(ok2, b2, &[], slow, &[]);
                fx.b.switch_to_block(b2);
                let x = self.float_value(fx, &a, sel);
                let r = fx.b.ins().fneg(x);
                fx.b.ins().jump(done, &[nil.into(), r.into(), one.into()]);
            }
            NumCall::ToF(_) | NumCall::Math(..) => {
                let (sel, num_ok) = self.float_operand(fx, &a, false);
                let ok = fx.b.ins().band(ok, num_ok);
                let b2 = fx.b.create_block();
                fx.b.ins().brif(ok, b2, &[], slow, &[]);
                fx.b.switch_to_block(b2);
                let x = self.float_value(fx, &a, sel);
                match nc {
                    NumCall::Math(which, _) => {
                        let zero = fx.b.ins().f64const(0.0);
                        let bad = match which {
                            // sqrt, log, log2, log10: not below zero
                            0 | 6 | 7 | 8 => Some(fx.b.ins().fcmp(FloatCC::LessThan, x, zero)),
                            // asin, acos: within [-1, 1]
                            13 | 14 => {
                                let ax = fx.b.ins().fabs(x);
                                let onef = fx.b.ins().f64const(1.0);
                                Some(fx.b.ins().fcmp(FloatCC::GreaterThan, ax, onef))
                            }
                            _ => None,
                        };
                        if let Some(bad) = bad {
                            let b3 = fx.b.create_block();
                            fx.b.ins().brif(bad, slow, &[], b3, &[]);
                            fx.b.switch_to_block(b3);
                        }
                        let r = if which == 0 {
                            let r = fx.b.ins().sqrt(x);
                            fx.b.ins().fadd(r, zero)
                        } else {
                            let w = fx.b.ins().iconst(I64, which as i64);
                            let call = self.rt_ref(fx, "kn_math1");
                            let inst = fx.b.ins().call(call, &[w, x]);
                            fx.b.inst_results(inst)[0]
                        };
                        fx.b.ins().jump(done, &[nil.into(), r.into(), one.into()]);
                    }
                    _ => {
                        fx.b.ins().jump(done, &[nil.into(), x.into(), one.into()]);
                    }
                }
            }
            NumCall::Shift(_, _, left) => {
                // two Fixnums, a shift by 0..=62 that stays a Fixnum
                let (av, bv) = (a.boxed(), b.unwrap().boxed());
                let both = fx.b.ins().band(av, bv);
                let fixb = fx.b.ins().band_imm_s(both, 1);
                let is_fix = fx.b.ins().icmp_imm_s(IntCC::NotEqual, fixb, 0);
                let n = fx.b.ins().sshr_imm_s(bv, 1);
                let small = fx.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, n, 63);
                let o1 = fx.b.ins().band(ok, is_fix);
                let o2 = fx.b.ins().band(o1, small);
                let b2 = fx.b.create_block();
                fx.b.ins().brif(o2, b2, &[], slow, &[]);
                fx.b.switch_to_block(b2);
                let x = fx.b.ins().sshr_imm_s(av, 1);
                let z = fx.b.ins().f64const(0.0);
                let k0 = fx.b.ins().iconst(I8, 0);
                if *left {
                    // tagged: (x << n) << 1 | 1, which must shift back to x
                    let n1 = fx.b.ins().iadd_imm_s(n, 1);
                    let t = fx.b.ins().ishl(x, n1);
                    let back = fx.b.ins().sshr(t, n1);
                    let same = fx.b.ins().icmp(IntCC::Equal, back, x);
                    let r = fx.b.ins().bor_imm_s(t, 1);
                    fx.b.ins().brif(same, done, &[r.into(), z.into(), k0.into()], slow, &[]);
                } else {
                    let s = fx.b.ins().sshr(x, n);
                    let t = fx.b.ins().ishl_imm_s(s, 1);
                    let r = fx.b.ins().bor_imm_s(t, 1);
                    fx.b.ins().jump(done, &[r.into(), z.into(), k0.into()]);
                }
            }
            NumCall::Pow(..) => {
                // a Float base (an Integer's ** is another method), a Fixnum/Float exponent
                let bn = b.unwrap();
                let (sa, _) = self.float_operand(fx, &a, false);
                let a_flo = match sa.0 {
                    Some(hr) => fx.b.ins().bor(hr, sa.1),
                    None => sa.1,
                };
                let (sb, b_ok) = self.float_operand(fx, &bn, false);
                let o1 = fx.b.ins().band(ok, a_flo);
                let o2 = fx.b.ins().band(o1, b_ok);
                let b2 = fx.b.create_block();
                fx.b.ins().brif(o2, b2, &[], slow, &[]);
                fx.b.switch_to_block(b2);
                let x = self.float_value(fx, &a, sa);
                let y = self.float_value(fx, &bn, sb);
                // a negative base with a fractional exponent gives a Complex
                let zero = fx.b.ins().f64const(0.0);
                let nonneg = fx.b.ins().fcmp(FloatCC::GreaterThanOrEqual, x, zero);
                let fl = fx.b.ins().floor(y);
                let integral = fx.b.ins().fcmp(FloatCC::Equal, fl, y);
                let real = fx.b.ins().bor(nonneg, integral);
                let b3 = fx.b.create_block();
                fx.b.ins().brif(real, b3, &[], slow, &[]);
                fx.b.switch_to_block(b3);
                let call = self.rt_ref(fx, "kn_pow");
                let inst = fx.b.ins().call(call, &[x, y]);
                let r = fx.b.inst_results(inst)[0];
                fx.b.ins().jump(done, &[nil.into(), r.into(), one.into()]);
            }
        }
        fx.b.switch_to_block(slow);
        let rv = self.materialize(fx, recv);
        let args: Vec<Value> = match (&nc, b) {
            (NumCall::Math(..), _) => vec![self.materialize(fx, a)],
            (NumCall::Pow(..) | NumCall::Shift(..), Some(bn)) => vec![self.materialize(fx, bn)],
            _ => vec![],
        };
        self.cur_offset = at;
        let v = self.send(fx, rv, &name, &args, None);
        let z = fx.b.ins().f64const(0.0);
        let k0 = fx.b.ins().iconst(I8, 0);
        fx.b.ins().jump(done, &[v.into(), z.into(), k0.into()]);
        fx.b.switch_to_block(done);
        let p = fx.b.block_params(done).to_vec();
        Num::Lazy { boxed: p[0], raw: p[1], kind: p[2], lit: false }
    }

    /// f64 of a flonum (the caller checked the tag).
    pub(crate) fn flo_decode(&mut self, fx: &mut Fx, v: Value) -> Value {
        let b63 = fx.b.ins().ushr_imm_s(v, 63);
        let two = fx.b.ins().iconst(I64, 2);
        let hi = fx.b.ins().isub(two, b63);
        let body = fx.b.ins().band_imm_s(v, !3i64);
        let t = fx.b.ins().bor(hi, body);
        let r = fx.b.ins().rotr_imm_s(t, 3);
        let is_zero = fx.b.ins().icmp_imm_s(IntCC::Equal, v, FLONUM_ZERO);
        // (0 then, but not a constant: one would be materialized once for the whole function
        // and kept across calls in a callee-saved register)
        let zero = fx.b.ins().bxor_imm_s(v, FLONUM_ZERO);
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
            Num::F64(r) => {
                let done = fx.b.create_block();
                fx.b.append_block_param(done, I64);
                self.flo_encode_to(fx, r, done, &[]);
                fx.b.switch_to_block(done);
                fx.b.block_params(done)[0]
            }
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

    /// `F64` as a `Lazy` (only the raw f64 valid), for the paths that handle both kinds.
    fn lazy(&mut self, fx: &mut Fx, n: Num) -> Num {
        match n {
            Num::F64(raw) => {
                let boxed = fx.b.ins().iconst(I64, 0);
                let kind = fx.b.ins().iconst(I8, 1);
                Num::Lazy { boxed, raw, kind, lit: false }
            }
            n => n,
        }
    }

    /// The f64 of a value that is a Float: a flonum inline, a heap Float in the runtime.
    pub(crate) fn float_unbox(&mut self, fx: &mut Fx, v: Value) -> Value {
        let tag = fx.b.ins().band_imm_s(v, 3);
        let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
        let fb = fx.b.create_block();
        let hb = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, F64);
        fx.b.set_cold_block(hb);
        fx.b.ins().brif(flo, fb, &[], hb, &[]);
        fx.b.switch_to_block(fb);
        let d = self.flo_decode(fx, v);
        fx.b.ins().jump(done, &[d.into()]);
        fx.b.switch_to_block(hb);
        let call = self.rt_ref(fx, "kn_float_unbox");
        let inst = fx.b.ins().call(call, &[v]);
        let r = fx.b.inst_results(inst)[0];
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// Can f64 `f` be a flonum (else boxing it makes a heap Float)? Same rule as
    /// `flo_encode_to`.
    fn flonum_ok(&mut self, fx: &mut Fx, f: Value) -> (Value, Value) {
        let t = fx.b.ins().bitcast(I64, MemFlagsData::new(), f);
        let e = fx.b.ins().ushr_imm_s(t, 60);
        let e = fx.b.ins().band_imm_s(e, 7);
        let x = fx.b.ins().iadd_imm_s(e, -3);
        let in_range = fx.b.ins().icmp_imm_u(IntCC::UnsignedLessThanOrEqual, x, 1);
        let not_special = fx.b.ins().icmp_imm_s(IntCC::NotEqual, t, 0x3000_0000_0000_0000);
        let ok = fx.b.ins().band(in_range, not_special);
        let is_zero = fx.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
        (fx.b.ins().bor(ok, is_zero), t)
    }

    /// An always-Float local's tagged value: its box when it has one, else a flonum, else a
    /// new heap Float that becomes its box.
    pub(crate) fn flocal_box(&mut self, fx: &mut Fx, raw: Variable, cache: Option<Variable>) -> Value {
        let Some(cache) = cache else {
            let r = fx.b.use_var(raw);
            return self.materialize(fx, Num::F64(r));
        };
        let c = fx.b.use_var(cache);
        let r = fx.b.use_var(raw);
        let enc = fx.b.create_block();
        let hb = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let has = fx.b.ins().icmp_imm_s(IntCC::NotEqual, c, 0);
        fx.b.ins().brif(has, done, &[c.into()], enc, &[]);
        fx.b.switch_to_block(enc);
        let (ok, t) = self.flonum_ok(fx, r);
        let rot = fx.b.ins().rotl_imm_s(t, 3);
        let rot = fx.b.ins().band_imm_s(rot, !1i64);
        let v = fx.b.ins().bor_imm_s(rot, 2);
        let is_zero = fx.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
        let zero = fx.b.ins().iconst(I64, FLONUM_ZERO);
        let v = fx.b.ins().select(is_zero, zero, v);
        fx.b.set_cold_block(hb);
        fx.b.ins().brif(ok, done, &[v.into()], hb, &[]);
        fx.b.switch_to_block(hb);
        let v = self.call(fx, "kn_float_box", &[t]);
        fx.b.def_var(cache, v);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// An always-Float local's box for a copy to another one (`x = y`): its box, boxing it
    /// now when it is a heap Float without one (so both name the same object); 0 when it
    /// is a flonum that has no box yet.
    pub(crate) fn flocal_pin(&mut self, fx: &mut Fx, raw: Variable, cache: Variable) -> Value {
        let c = fx.b.use_var(cache);
        let r = fx.b.use_var(raw);
        let (ok, t) = self.flonum_ok(fx, r);
        let has = fx.b.ins().icmp_imm_s(IntCC::NotEqual, c, 0);
        let fine = fx.b.ins().bor(ok, has);
        let hb = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(hb);
        fx.b.ins().brif(fine, done, &[c.into()], hb, &[]);
        fx.b.switch_to_block(hb);
        let v = self.call(fx, "kn_float_box", &[t]);
        fx.b.def_var(cache, v);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// (f64, box or 0) of a numeric value assigned to an always-Float local: a value that
    /// arrives boxed keeps its box (its identity).
    pub(crate) fn num_f64_boxed(&mut self, fx: &mut Fx, n: Num) -> (Value, Value) {
        match n {
            Num::F64(r) => (r, fx.b.ins().iconst(I64, 0)),
            Num::Lazy { raw, boxed, lit: true, .. } => (raw, boxed),
            Num::Lazy { boxed, raw, kind, .. } => {
                // kind 1: only the f64; otherwise the box is valid (and is a Float)
                let only_raw = fx.b.ins().icmp_imm_s(IntCC::Equal, kind, 1);
                let rb = fx.b.create_block();
                let bb = fx.b.create_block();
                let done = fx.b.create_block();
                fx.b.append_block_param(done, F64);
                fx.b.append_block_param(done, I64);
                fx.b.ins().brif(only_raw, rb, &[], bb, &[]);
                fx.b.switch_to_block(rb);
                let z = fx.b.ins().iconst(I64, 0);
                fx.b.ins().jump(done, &[raw.into(), z.into()]);
                fx.b.switch_to_block(bb);
                let f = self.float_unbox(fx, boxed);
                fx.b.ins().jump(done, &[f.into(), boxed.into()]);
                fx.b.switch_to_block(done);
                let p = fx.b.block_params(done);
                (p[0], p[1])
            }
            Num::Boxed(v) => (self.float_unbox(fx, v), v),
        }
    }

    /// `a op b` for an arithmetic or comparison operator where one operand is known to be a
    /// Float and the other a Float or an Integer (`Num::static_f64` / `is_sure_int`): no
    /// type checks beyond Fixnum for the Integer side (a Bignum, or one too large to
    /// compare exactly as a double, makes the call). Arithmetic stays an unboxed `F64`.
    fn typed_float_op(&mut self, fx: &mut Fx, op: &[u8], a: Num, b: Num) -> Num {
        let arith = matches!(op, b"+" | b"-" | b"*" | b"/");
        let mut slow: Option<Block> = None;
        let operand = |fx: &mut Fx, n: &Num, slow: &mut Option<Block>| -> Value {
            if let Some(r) = n.static_f64() {
                return r;
            }
            let v = n.boxed();
            if let Some(k) = const_of(fx, v) {
                let i = k >> 1;
                if arith || i.unsigned_abs() < (1u64 << 53) {
                    return fx.b.ins().f64const(i as f64);
                }
            }
            let low = fx.b.ins().band_imm_s(v, 1);
            let mut ok = fx.b.ins().icmp_imm_s(IntCC::NotEqual, low, 0);
            let i = fx.b.ins().sshr_imm_s(v, 1);
            if !arith {
                let x = fx.b.ins().iadd_imm_s(i, 1i64 << 53);
                let small = fx.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, x, 1i64 << 54);
                ok = fx.b.ins().band(ok, small);
            }
            let sb = *slow.get_or_insert_with(|| {
                let b = fx.b.create_block();
                fx.b.set_cold_block(b);
                b
            });
            let cont = fx.b.create_block();
            fx.b.ins().brif(ok, cont, &[], sb, &[]);
            fx.b.switch_to_block(cont);

            fx.b.ins().fcvt_from_sint(F64, i)
        };
        let fa = operand(fx, &a, &mut slow);
        let fb = operand(fx, &b, &mut slow);
        let r = match op {
            b"+" => fx.b.ins().fadd(fa, fb),
            b"-" => fx.b.ins().fsub(fa, fb),
            b"*" => fx.b.ins().fmul(fa, fb),
            b"/" => fx.b.ins().fdiv(fa, fb),
            _ => {
                let cc = match op {
                    b"<" => FloatCC::LessThan,
                    b"<=" => FloatCC::LessThanOrEqual,
                    b">" => FloatCC::GreaterThan,
                    b">=" => FloatCC::GreaterThanOrEqual,
                    b"==" => FloatCC::Equal,
                    _ => FloatCC::NotEqual,
                };
                fx.b.ins().fcmp(cc, fa, fb)
            }
        };
        let Some(sb) = slow else {
            return if arith { Num::F64(r) } else { Num::Boxed(self.bool_v(fx, r)) };
        };
        let done = fx.b.create_block();
        fx.b.append_block_param(done, if arith { F64 } else { I8 });
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(sb);
        let av = self.materialize(fx, a);
        let bv = self.materialize(fx, b);
        let v = self.send(fx, av, op, &[bv], None);
        let r = if arith { self.float_unbox(fx, v) } else { self.truthy(fx, v) };
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        if arith { Num::F64(r) } else { Num::Boxed(self.bool_v(fx, r)) }
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
            if let Some(nc) = num_call_of(&c).filter(|_| self.refine_scope.is_none()) {
                return self.num_call(fx, &c, nc);
            }
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
        if self.refine_scope.is_none()
            && matches!(op, b"+" | b"-" | b"*" | b"/" | b"<" | b"<=" | b">" | b">=" | b"==" | b"!=")
            && (matches!(a, Num::F64(_)) || matches!(b, Num::F64(_)) || (a.static_f64().is_some() && b.static_f64().is_some()))
            && (a.static_f64().is_some() || a.is_sure_int(fx))
            && (b.static_f64().is_some() || b.is_sure_int(fx))
        {
            return self.typed_float_op(fx, op, a, b);
        }
        let a = self.lazy(fx, a);
        let b = self.lazy(fx, b);
        // two Fixnums for sure (spec.rs): comparisons and bit operations need nothing else
        let fixv = |fx: &Fx, n: &Num| matches!(n, Num::Boxed(v) if fx.sure_fix.contains(v) || const_of(fx, *v).is_some_and(|c| c & 1 == 1));
        let both_fix = self.refine_scope.is_none() && fixv(fx, &a) && fixv(fx, &b);
        if both_fix {
            let (x, y) = (a.boxed(), b.boxed());
            match op {
                b"<" | b"<=" | b">" | b">=" | b"==" | b"!=" => {
                    let c = fx.b.ins().icmp(int_cc(op), x, y);
                    return Num::Boxed(self.bool_v(fx, c));
                }
                b"&" | b"|" => {
                    let r = if op == b"&" { fx.b.ins().band(x, y) } else { fx.b.ins().bor(x, y) };
                    fx.sure_fix.insert(r);
                    fx.sure_ints.insert(r);
                    return Num::Boxed(r);
                }
                b"^" => {
                    let t = fx.b.ins().bxor(x, y);
                    let r = fx.b.ins().bor_imm_s(t, 1);
                    fx.sure_fix.insert(r);
                    fx.sure_ints.insert(r);
                    return Num::Boxed(r);
                }
                _ => {}
            }
        }
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
        let both_sure = a.is_sure_int(fx) && b.is_sure_int(fx);
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
            if both_fix {
                fx.b.ins().jump(fast, &[]);
            } else {
                let both = fx.b.ins().band(ab, bb);
                let bit = fx.b.ins().band_imm_s(both, 1);
                fx.b.ins().brif(bit, fast, &[], fchk, &[]);
            }
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
            if both_sure {
                fx.sure_ints.insert(p[0]);
            }
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
        let cold = fx.b.func.layout.is_cold(fchk);
        fx.b.switch_to_block(fchk);
        // two Floats (flonums or raw f64s) first: no Fixnum conversions
        let (a_f, a_ok) = self.float_only(fx, a, !arith);
        let (b_f, b_ok) = self.float_only(fx, b, !arith);
        let ok = fx.b.ins().band(a_ok, b_ok);
        let ffast = fx.b.create_block();
        fx.b.append_block_param(ffast, F64);
        fx.b.append_block_param(ffast, F64);
        let mixed = fx.b.create_block();
        if cold {
            fx.b.set_cold_block(ffast);
        }
        fx.b.set_cold_block(mixed);
        let fa = a_f.unwrap_or_else(|| self.flo_decode(fx, a.boxed()));
        let fb = b_f.unwrap_or_else(|| self.flo_decode(fx, b.boxed()));
        fx.b.ins().brif(ok, ffast, &[fa.into(), fb.into()], mixed, &[]);
        // a Fixnum mixed in (converted as Ruby does)
        fx.b.switch_to_block(mixed);
        let (a_sel, a_ok) = self.float_operand(fx, a, !arith);
        let (b_sel, b_ok) = self.float_operand(fx, b, !arith);
        let ok = fx.b.ins().band(a_ok, b_ok);
        let ma = self.float_value(fx, a, a_sel);
        let mb = self.float_value(fx, b, b_sel);
        fx.b.ins().brif(ok, ffast, &[ma.into(), mb.into()], slow, &[]);
        fx.b.switch_to_block(ffast);
        let fa = fx.b.block_params(ffast)[0];
        let fb = fx.b.block_params(ffast)[1];
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
    /// (the f64 when it is always valid, usable inline) for one operand of a Float operation
    /// when only Floats count: a flonum, or a raw f64 (ordered, for comparisons). With no
    /// f64 given, the caller decodes the operand as a flonum.
    fn float_only(&mut self, fx: &mut Fx, n: &Num, exact: bool) -> (Option<Value>, Value) {
        let v = n.boxed();
        match n {
            Num::Lazy { raw, kind, .. } => {
                if let Some(k) = const_of(fx, *kind) {
                    if k != 0 {
                        let ok = if exact { fx.b.ins().fcmp(FloatCC::Ordered, *raw, *raw) } else { fx.b.ins().iconst(I8, 1) };
                        return (Some(*raw), ok);
                    }
                }
                let hr = fx.b.ins().icmp_imm_s(IntCC::NotEqual, *kind, 0);
                let mut raw_ok = hr;
                if exact {
                    let ordered = fx.b.ins().fcmp(FloatCC::Ordered, *raw, *raw);
                    raw_ok = fx.b.ins().band(hr, ordered);
                }
                let tag = fx.b.ins().band_imm_s(v, 3);
                let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
                // (a raw value that is NaN must not use the tagged slot either)
                let not_raw = fx.b.ins().icmp_imm_s(IntCC::Equal, *kind, 0);
                let flo = fx.b.ins().band(flo, not_raw);
                let ok = fx.b.ins().bor(flo, raw_ok);
                let d = self.flo_decode(fx, v);
                (Some(fx.b.ins().select(hr, *raw, d)), ok)
            }
            _ => {
                let tag = fx.b.ins().band_imm_s(v, 3);
                (None, fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2))
            }
        }
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
        {
            let vs = vals.to_vec();
            let nm = name.to_vec();
            let inl = self.inline_call(fx, recv, name, mid, vals, explicit, |c, fx| {
                if explicit { c.send_simple_dyn(fx, cache, recv, &nm, mid, &vs, explicit) } else { c.send_simple_static(fx, cache, recv, &nm, mid, &vs) }
            });
            if let Some(r) = inl {
                return r;
            }
        }
        if !explicit {
            let vs = vals.to_vec();
            let nm = name.to_vec();
            if let Some(r) = self.static_call(fx, recv, name, mid, vals, |c, fx| c.send_simple_dyn(fx, cache, recv, &nm, mid, &vs, explicit)) {
                return r;
            }
        }
        self.send_simple_dyn(fx, cache, recv, name, mid, vals, explicit)
    }
    /// A receiver-less call: static if it can be, else dynamic.
    fn send_simple_static(&mut self, fx: &mut Fx, cache: Value, recv: Value, name: &[u8], mid: Value, vals: &[Value]) -> Value {
        let vs = vals.to_vec();
        let nm = name.to_vec();
        if let Some(r) = self.static_call(fx, recv, name, mid, vals, |c, fx| c.send_simple_dyn(fx, cache, recv, &nm, mid, &vs, false)) {
            return r;
        }
        self.send_simple_dyn(fx, cache, recv, name, mid, vals, false)
    }
    fn send_simple_dyn(&mut self, fx: &mut Fx, cache: Value, recv: Value, name: &[u8], mid: Value, vals: &[Value], explicit: bool) -> Value {
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
        // `a[i]` / `a[i] = v` on a plain Array (the app's code, or any loop)
        if self.refine_scope.is_none() && (in_loop || !self.in_dependency()) {
            match (name, vals) {
                (b"[]", [i]) => {
                    let i = *i;
                    return self.ary_index_fast(fx, recv, i, None, |c, fx| plain(c, fx, "kn_send1", &[cache, recv, mid, i, e]));
                }
                (b"[]=", [i, v]) => {
                    let (i, v) = (*i, *v);
                    return self.ary_index_fast(fx, recv, i, Some(v), |c, fx| plain(c, fx, "kn_send2", &[cache, recv, mid, i, v, e]));
                }
                (b"getbyte", [i]) => {
                    let i = *i;
                    return self.str_byte_fast(fx, recv, i, None, |c, fx| plain(c, fx, "kn_send1", &[cache, recv, mid, i, e]));
                }
                (b"setbyte", [i, v]) => {
                    let (i, v) = (*i, *v);
                    return self.str_byte_fast(fx, recv, i, Some(v), |c, fx| plain(c, fx, "kn_send2", &[cache, recv, mid, i, v, e]));
                }
                _ => {}
            }
        }
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
        let remember = if val.is_none() { self.licm_take(fx, false) } else { None };
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);

        // a heap object (not an immediate, nil or false) of the recorded class (an empty
        // cache word matches none), a plain Object (for a write also not frozen: kind and
        // flags are one word): one test
        self.require_heap(fx, recv, slow_b);
        let word = fx.b.ins().load(I64, f, cache, CC_ATTR);
        let klass = fx.b.ins().load(I64, f, recv, 0);
        let kf = fx.b.ins().load(I64, f, recv, OBJ_KIND);
        let mask = if val.is_some() { 0xFFFF_FFFF | (FL_FROZEN << 32) } else { 0xFFFF_FFFF };
        let kfm = fx.b.ins().band_imm_s(kf, mask);
        let wk = fx.b.ins().ushr_imm_u(word, 16);
        let diff = fx.b.ins().bxor(klass, wk);
        let bad = fx.b.ins().bor(diff, kfm);
        self.require_zero(fx, bad, slow_b);

        // the slot is within the object's ivar buffer (capacity first, as SVec::snap does)
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
                match &remember {
                    None => {
                        fx.b.ins().brif(unset, slow_b, &[], done, &[v.into()]);
                    }
                    Some(e) => {
                        let b4 = fx.b.create_block();
                        fx.b.ins().brif(unset, slow_b, &[], b4, &[]);
                        fx.b.switch_to_block(b4);
                        self.licm_record(fx, e, v);
                        fx.b.ins().jump(done, &[v.into()]);
                    }
                }
            }
            Some(a) => {
                // no write barrier due (an old buffer not yet remembered takes the runtime
                // path, which remembers it)
                let old_bits = fx.b.ins().band_imm_s(cap, SVEC_FLAGS);
                let no_wb = fx.b.ins().icmp_imm_s(IntCC::NotEqual, old_bits, SVEC_OLD);
                let w = fx.b.ins().band(inb, no_wb);
                fx.b.ins().brif(w, b3, &[], slow_b, &[]);
                fx.b.switch_to_block(b3);
                fx.b.ins().store(f, a, addr, 0);
                self.licm_kill(fx);
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
    /// `recv[i]` (`val` None) or `recv[i] = val` without a call when `recv` is a plain Array
    /// (its class is Array itself) whose `[]` / `[]=` is the builtin (`KN_DIRECT.flags`) and
    /// `i` an Integer: an element (nil past the end), or for a write an element that exists,
    /// of an unfrozen Array with no write barrier due (an immediate value never needs one).
    /// Anything else runs `slow`.
    pub(crate) fn ary_index_fast(&mut self, fx: &mut Fx, recv: Value, idx: Value, val: Option<Value>, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);
        // the index is a Fixnum
        let not_fix = fx.b.ins().bnot(idx);
        let not_fix = fx.b.ins().band_imm_s(not_fix, 1);
        self.ary_guard(fx, recv, val.is_some(), not_fix, slow_b, &[]);
        // the element (capacity first, as SVec::snap does); a negative index counts from the end
        let cap_addr = fx.b.ins().iadd_imm_s(recv, ARY_CAP as i64);
        let cap = fx.b.ins().atomic_load(I64, f, cap_addr);
        let ptr = fx.b.ins().load(I64, f, recv, ARY_PTR);
        let len = fx.b.ins().load(I64, f, recv, ARY_LEN);
        let capn = fx.b.ins().band_imm_s(cap, SVEC_CAP_MASK);
        let n = fx.b.ins().umin(len, capn);
        let i = fx.b.ins().sshr_imm_s(idx, 1);
        // (n when i < 0, else 0: a mask, not a select against a 0 that ends up spilled)
        let sign = fx.b.ins().sshr_imm_s(i, 63);
        let add = fx.b.ins().band(sign, n);
        let i = fx.b.ins().iadd(i, add);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let off = fx.b.ins().ishl_imm_u(i, 3);
        let addr = fx.b.ins().iadd(ptr, off);
        match val {
            None => {
                let b3 = fx.b.create_block();
                let nil = fx.b.ins().iconst(I64, QNIL);
                fx.b.ins().brif(inb, b3, &[], done, &[nil.into()]);
                fx.b.switch_to_block(b3);
                let v = fx.b.ins().load(I64, f, addr, 0);
                fx.b.ins().jump(done, &[v.into()]);
            }
            Some(v) => {
                self.require(fx, inb, slow_b);
                // no write barrier due: the buffer isn't old-and-unremembered, or v is immediate
                let old_bits = fx.b.ins().band_imm_s(cap, SVEC_FLAGS);
                let wb_maybe = fx.b.ins().icmp_imm_s(IntCC::Equal, old_bits, SVEC_OLD);
                let store_b = fx.b.create_block();
                let old_b = fx.b.create_block();
                fx.b.ins().brif(wb_maybe, old_b, &[], store_b, &[]);
                fx.b.switch_to_block(old_b);
                let r = fx.b.ins().rotr_imm_s(v, 3);
                let r = fx.b.ins().iadd_imm_s(r, -2);
                let heap = fx.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, r, (1i64 << 61) - 2);
                fx.b.ins().brif(heap, slow_b, &[], store_b, &[]);
                fx.b.switch_to_block(store_b);
                fx.b.ins().store(f, v, addr, 0);
                fx.b.ins().jump(done, &[v.into()]);
            }
        }
        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// `recv.getbyte(i)` (`val` None) or `recv.setbyte(i, val)` without a call when `recv` is
    /// a plain String (its class is String itself) whose getbyte and setbyte are the builtins
    /// (`KN_DIRECT.flags`) and `i` (and `val`) Integers: the byte (nil past the end), or for a
    /// write a byte that exists of an unfrozen String. Anything else runs `slow`.
    pub(crate) fn str_byte_fast(&mut self, fx: &mut Fx, recv: Value, idx: Value, val: Option<Value>, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);
        let fixes = match val {
            Some(v) => fx.b.ins().band(idx, v),
            None => idx,
        };
        let not_fix = fx.b.ins().bnot(fixes);
        let not_fix = fx.b.ins().band_imm_s(not_fix, 1);
        let r = fx.b.ins().rotr_imm_s(recv, 3);
        let r = fx.b.ins().iadd_imm_s(r, -2);
        let heap = fx.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, r, (1i64 << 61) - 2);
        let b0 = fx.b.create_block();
        fx.b.ins().brif(heap, b0, &[], slow_b, &[]);
        fx.b.switch_to_block(b0);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let sk = fx.b.ins().load(I64, f, dir, KD_STRING_CLASS);
        let flags = fx.b.ins().load(I64, f, dir, KD_FLAGS);
        let bits = fx.b.ins().band_imm_s(flags, DIRECT_STR_BYTE);
        let no_bits = fx.b.ins().bxor_imm_s(bits, DIRECT_STR_BYTE);
        let other = fx.b.ins().bxor(k, sk);
        let mut bad = fx.b.ins().bor(other, no_bits);
        bad = fx.b.ins().bor(bad, not_fix);
        if val.is_some() {
            let fl = fx.b.ins().uload32(f, recv, OBJ_FLAGS);
            let fr = fx.b.ins().band_imm_s(fl, FL_FROZEN);
            bad = fx.b.ins().bor(bad, fr);
        }
        let cont = fx.b.create_block();
        fx.b.ins().brif(bad, slow_b, &[], cont, &[]);
        fx.b.switch_to_block(cont);
        // the byte (capacity first, as SVec::snap does); a negative index counts from the end
        let cap_addr = fx.b.ins().iadd_imm_s(recv, STR_CAP as i64);
        let cap = fx.b.ins().atomic_load(I64, f, cap_addr);
        let ptr = fx.b.ins().load(I64, f, recv, STR_PTR);
        let len = fx.b.ins().load(I64, f, recv, STR_LEN);
        let capn = fx.b.ins().band_imm_s(cap, SVEC_CAP_MASK);
        let n = fx.b.ins().umin(len, capn);
        let i = fx.b.ins().sshr_imm_s(idx, 1);
        let sign = fx.b.ins().sshr_imm_s(i, 63);
        let add = fx.b.ins().band(sign, n);
        let i = fx.b.ins().iadd(i, add);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let addr = fx.b.ins().iadd(ptr, i);
        match val {
            None => {
                let b3 = fx.b.create_block();
                let nil = fx.b.ins().iconst(I64, QNIL);
                fx.b.ins().brif(inb, b3, &[], done, &[nil.into()]);
                fx.b.switch_to_block(b3);
                let byte = fx.b.ins().uload8(I64, f, addr, 0);
                let t = fx.b.ins().ishl_imm_u(byte, 1);
                let r = fx.b.ins().bor_imm_s(t, 1);
                fx.b.ins().jump(done, &[r.into()]);
            }
            Some(v) => {
                self.require(fx, inb, slow_b);
                let byte = fx.b.ins().ushr_imm_u(v, 1);
                fx.b.ins().istore8(f, byte, addr, 0);
                fx.b.ins().jump(done, &[v.into()]);
            }
        }
        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// The checks of an inline Array index path, falling through when they pass, else to
    /// `slow_b` (with `slow_args`): `recv` is a heap object whose class is Array itself, its
    /// `[]` is the builtin and for a `write` also `[]=`, and not frozen; and `not_fix`
    /// (index checks the caller computed) is 0.
    ///
    /// When `recv` was read from an Array local that a loop doesn't assign (licm.rs), the
    /// first set of checks runs only until it passes, then again after anything that could
    /// change it (a call); `not_fix` is checked every time.
    pub(crate) fn ary_guard(&mut self, fx: &mut Fx, recv: Value, write: bool, not_fix: Value, slow_b: Block, slow_args: &[Value]) {
        let f = mem();
        let args: Vec<cranelift_codegen::ir::BlockArg> = slow_args.iter().map(|&v| v.into()).collect();
        let fact = self.licm_ary(fx, recv, write);
        // the checks on recv alone, as one word that must be 0
        let recv_bad = |c: &mut Self, fx: &mut Fx| -> Value {
            let gv = c.module.declare_data_in_func(c.direct_id, fx.b.func);
            let dir = fx.b.ins().symbol_value(I64, gv);
            let k = fx.b.ins().load(I64, f, recv, 0);
            let ak = fx.b.ins().load(I64, f, dir, KD_ARRAY_CLASS);
            let flags = fx.b.ins().load(I64, f, dir, KD_FLAGS);
            let want = if write { DIRECT_ARY_AREF | DIRECT_ARY_ASET } else { DIRECT_ARY_AREF };
            let bits = fx.b.ins().band_imm_s(flags, want);
            let no_bits = fx.b.ins().bxor_imm_s(bits, want);
            let other = fx.b.ins().bxor(k, ak);
            let mut bad = fx.b.ins().bor(other, no_bits);
            if write {
                let fl = fx.b.ins().uload32(f, recv, OBJ_FLAGS);
                let fr = fx.b.ins().band_imm_s(fl, FL_FROZEN);
                bad = fx.b.ins().bor(bad, fr);
            }
            bad
        };
        let heap_check = |fx: &mut Fx, args: &[cranelift_codegen::ir::BlockArg]| {
            let r = fx.b.ins().rotr_imm_s(recv, 3);
            let r = fx.b.ins().iadd_imm_s(r, -2);
            let heap = fx.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, r, (1i64 << 61) - 2);
            let b0 = fx.b.create_block();
            fx.b.ins().brif(heap, b0, &[], slow_b, args);
            fx.b.switch_to_block(b0);
        };
        let cont = fx.b.create_block();
        match fact {
            None => {
                heap_check(fx, &args);
                let bad = recv_bad(self, fx);
                let bad = fx.b.ins().bor(bad, not_fix);
                fx.b.ins().brif(bad, slow_b, &args, cont, &[]);
            }
            Some(e) => {
                let known = fx.b.create_block();
                let chk = fx.b.create_block();
                let valid = fx.b.use_var(e.valid);
                fx.b.ins().brif(valid, known, &[], chk, &[]);
                fx.b.switch_to_block(chk);
                heap_check(fx, &args);
                let bad = recv_bad(self, fx);
                let rec = fx.b.create_block();
                fx.b.ins().brif(bad, slow_b, &args, rec, &[]);
                fx.b.switch_to_block(rec);
                self.licm_record(fx, &e, recv);
                fx.b.ins().jump(known, &[]);
                fx.b.switch_to_block(known);
                fx.b.ins().brif(not_fix, slow_b, &args, cont, &[]);
            }
        }
        fx.b.switch_to_block(cont);
    }

    /// `recv[idx] += rhs` (`sub`: `-=`) with no call when `recv` is a plain Array whose `[]`
    /// and `[]=` are the builtins, unfrozen, `idx` a Fixnum within it, and the element and
    /// `rhs` Fixnums whose sum is one (an immediate: no write barrier). Else `slow`, given the
    /// element (read only once).
    pub(crate) fn ary_add_fast(
        &mut self,
        fx: &mut Fx,
        recv: Value,
        idx: Value,
        rhs: Value,
        sub: bool,
        slow: impl FnOnce(&mut Self, &mut Fx, Value) -> Value,
    ) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.append_block_param(slow_b, I64);
        fx.b.set_cold_block(slow_b);
        let unread = fx.b.ins().iconst(I64, UNDEF);
        // idx and rhs Fixnums: both low bits set
        let both = fx.b.ins().band(idx, rhs);
        let not_fix = fx.b.ins().bnot(both);
        let not_fix = fx.b.ins().band_imm_s(not_fix, 1);
        self.ary_guard(fx, recv, true, not_fix, slow_b, &[unread]);
        let cap_addr = fx.b.ins().iadd_imm_s(recv, ARY_CAP as i64);
        let cap = fx.b.ins().atomic_load(I64, f, cap_addr);
        let ptr = fx.b.ins().load(I64, f, recv, ARY_PTR);
        let len = fx.b.ins().load(I64, f, recv, ARY_LEN);
        let capn = fx.b.ins().band_imm_s(cap, SVEC_CAP_MASK);
        let n = fx.b.ins().umin(len, capn);
        let i = fx.b.ins().sshr_imm_s(idx, 1);
        // (n when i < 0, else 0: a mask, not a select against a 0 that ends up spilled)
        let sign = fx.b.ins().sshr_imm_s(i, 63);
        let add = fx.b.ins().band(sign, n);
        let i = fx.b.ins().iadd(i, add);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(inb, b2, &[], slow_b, &[unread.into()]);
        fx.b.switch_to_block(b2);
        let off = fx.b.ins().ishl_imm_u(i, 3);
        let addr = fx.b.ins().iadd(ptr, off);
        let cur = fx.b.ins().load(I64, f, addr, 0);
        // a Fixnum element: tagged sum = cur + (rhs - 1), tagged difference = cur - (rhs - 1)
        let b3 = fx.b.create_block();
        let cfix = fx.b.ins().band_imm_s(cur, 1);
        fx.b.ins().brif(cfix, b3, &[], slow_b, &[cur.into()]);
        fx.b.switch_to_block(b3);
        let r1 = fx.b.ins().iadd_imm_s(rhs, -1);
        let (v, of) = if sub { fx.b.ins().ssub_overflow(cur, r1) } else { fx.b.ins().sadd_overflow(cur, r1) };
        let b4 = fx.b.create_block();
        fx.b.ins().brif(of, slow_b, &[cur.into()], b4, &[]);
        fx.b.switch_to_block(b4);
        fx.b.ins().store(f, v, addr, 0);
        fx.b.ins().jump(done, &[v.into()]);
        // the element: read here already, else (UNDEF) read now
        fx.b.switch_to_block(slow_b);
        let got = fx.b.block_params(slow_b)[0];
        let was_read = fx.b.ins().icmp_imm_s(IntCC::NotEqual, got, UNDEF);
        let read_now = fx.b.create_block();
        let have = fx.b.create_block();
        fx.b.append_block_param(have, I64);
        fx.b.set_cold_block(read_now);
        fx.b.set_cold_block(have);
        fx.b.ins().brif(was_read, have, &[got.into()], read_now, &[]);
        fx.b.switch_to_block(read_now);
        let cur = self.send(fx, recv, b"[]", &[idx], None);
        fx.b.ins().jump(have, &[cur.into()]);
        fx.b.switch_to_block(have);
        let cur = fx.b.block_params(have)[0];
        let r = slow(self, fx, cur);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

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

        // a heap object
        self.require_heap(fx, recv, slow_b);
        // a plain Object (kind 0; for a write also not frozen: kind and flags are one word)
        // of the cached shape (an empty cache word matches no shape; an object without a
        // shape yet has no ivars, so the bounds check below sends it to the runtime)
        let word = fx.b.ins().load(I64, f, cache, 0);
        let kf = fx.b.ins().load(I64, f, recv, OBJ_KIND);
        let mask = if val.is_some() { 0xFFFF_FFFF | (FL_FROZEN << 32) } else { 0xFFFF_FFFF };
        let kfm = fx.b.ins().band_imm_s(kf, mask);
        let shape = fx.b.ins().load(I64, f, recv, OBJ_SHAPE);
        let wshape = fx.b.ins().band_imm_s(word, IV_PTR);
        let diff = fx.b.ins().bxor(shape, wshape);
        let bad = fx.b.ins().bor(kfm, diff);
        self.require_zero(fx, bad, slow_b);

        // the slot is within the buffer (capacity first, as SVec::snap does)
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
                if let Some(e) = self.licm_take(fx, true) {
                    self.licm_record(fx, &e, r);
                }
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
                self.licm_kill(fx);
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
    /// A class's or module's own instance variable read (`@x` in `def self.x`), without a
    /// call once the site's cache (kenny-runtime abi.rs `IvCache::class_pos`: class |
    /// position << 48) has this receiver: the entry at that position of its ivar map
    /// (core.rs `CLASS_CIVARS`) still has the key. Anything else runs `slow`.
    pub(crate) fn civar_fast(&mut self, fx: &mut Fx, cache: Value, recv: Value, id: Value, slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let f = mem();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);
        let word = fx.b.ins().load(I64, f, cache, 8);
        let cls = fx.b.ins().band_imm_s(word, IV_PTR);
        let same = fx.b.ins().icmp(IntCC::Equal, cls, recv);
        let filled = fx.b.ins().icmp_imm_s(IntCC::NotEqual, word, 0);
        let ok = fx.b.ins().band(same, filled);
        self.require(fx, ok, slow_b);
        let mp = fx.b.ins().iadd_imm_s(recv, CLASS_CIVARS as i64);
        let slab = fx.b.ins().atomic_load(I64, f, mp);
        self.require_nonzero(fx, slab, slow_b);
        let len = fx.b.ins().atomic_load(I64, f, slab);
        let cap = fx.b.ins().load(I64, f, slab, 8);
        let n = fx.b.ins().umin(len, cap);
        let i = fx.b.ins().ushr_imm_u(word, 48);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
        self.require(fx, inb, slow_b);
        let off = fx.b.ins().ishl_imm_u(i, 4);
        let e = fx.b.ins().iadd(slab, off);
        let key = fx.b.ins().uload32(f, e, 16);
        let want = fx.b.ins().ushr_imm_u(id, 8);
        let want = fx.b.ins().band_imm_s(want, 0xFFFF_FFFF);
        let kok = fx.b.ins().icmp(IntCC::Equal, key, want);
        self.require(fx, kok, slow_b);
        let v = fx.b.ins().load(I64, f, e, 24);
        if let Some(en) = self.licm_take(fx, true) {
            self.licm_record(fx, &en, v);
        }
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// `pat === subj` for a `when` clause: answered inline while every class's === is the
    /// builtin Module#=== (`DIRECT_MOD_EQQ_ALL`), pat is a class (not a module: those have
    /// no superclass chain) and subj a heap object: subj's class has pat at pat's depth in
    /// its superclass chain. Else a call.
    pub(crate) fn case_class_eqq(&mut self, fx: &mut Fx, pat: Value, subj: Value) -> Value {
        self.mark_loc(fx);
        let cache = self.owned_cache_ptr(fx, OWNED_CALL);
        let mid = self.sym_val(fx, b"===");
        let f = mem();
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow);
        let heap = |c: &mut Self, fx: &mut Fx, v: Value| {
            let low = fx.b.ins().band_imm_s(v, 7);
            let aligned = fx.b.ins().icmp_imm_s(IntCC::Equal, low, 0);
            let not_nil = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, v, QNIL);
            let _ = c;
            fx.b.ins().band(aligned, not_nil)
        };
        let h1 = heap(self, fx, pat);
        let h2 = heap(self, fx, subj);
        let hh = fx.b.ins().band(h1, h2);
        let b1 = fx.b.create_block();
        fx.b.ins().brif(hh, b1, &[], slow, &[]);
        fx.b.switch_to_block(b1);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let flags = fx.b.ins().load(I64, f, dir, KD_FLAGS);
        let off_bit = fx.b.ins().band_imm_s(flags, DIRECT_MOD_EQQ_ALL);
        let off = fx.b.ins().bxor_imm_s(off_bit, DIRECT_MOD_EQQ_ALL);
        let kind = fx.b.ins().uload32(f, pat, HDR_KIND);
        let not_cls = fx.b.ins().bxor_imm_s(kind, KIND_CLASS);
        let bad = fx.b.ins().bor(off, not_cls);
        let b15 = fx.b.create_block();
        fx.b.ins().brif(bad, slow, &[], b15, &[]);
        // (pat is a class or module)
        fx.b.switch_to_block(b15);
        let cs_len = fx.b.ins().load(I64, f, pat, CLASS_SUP_LEN);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(cs_len, b2, &[], slow, &[]);
        fx.b.switch_to_block(b2);
        let k = fx.b.ins().load(I64, f, subj, 0);
        let ks_len = fx.b.ins().load(I64, f, k, CLASS_SUP_LEN);
        let d = fx.b.ins().iadd_imm_s(cs_len, -1);
        let inb = fx.b.ins().icmp(IntCC::UnsignedLessThan, d, ks_len);
        let b3 = fx.b.create_block();
        let no = fx.b.ins().iconst(I64, QFALSE);
        fx.b.ins().brif(inb, b3, &[], done, &[no.into()]);
        fx.b.switch_to_block(b3);
        let ksp = fx.b.ins().load(I64, f, k, CLASS_SUP_PTR);
        let off = fx.b.ins().ishl_imm_u(d, 3);
        let addr = fx.b.ins().iadd(ksp, off);
        let x = fx.b.ins().load(I64, f, addr, 0);
        let same = fx.b.ins().icmp(IntCC::Equal, x, pat);
        let t = fx.b.ins().iconst(I64, QTRUE);
        let fl = fx.b.ins().iconst(I64, QFALSE);
        let r = fx.b.ins().select(same, t, fl);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(slow);
        let one = fx.b.ins().iconst(I64, 1);
        let r = self.call(fx, "kn_send1", &[cache, pat, mid, subj, one]);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

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

    /// Go on in a new block only when `c` holds, else to `slow`. (A branch per test: tests
    /// combined with `band` would each be materialized as a flag first.)
    pub(crate) fn require(&mut self, fx: &mut Fx, c: Value, slow: Block) {
        let b = fx.b.create_block();
        fx.b.ins().brif(c, b, &[], slow, &[]);
        fx.b.switch_to_block(b);
    }
    /// ... when `v` is 0. (A branch on `v` itself: `v == 0` would bring a constant 0, which
    /// Cranelift shares with the other uses of 0, cold calls' arguments among them, without
    /// rematerializing it: kept across calls in a callee-saved register, saved and restored
    /// on every call of the function.)
    pub(crate) fn require_zero(&mut self, fx: &mut Fx, v: Value, slow: Block) {
        let b = fx.b.create_block();
        fx.b.ins().brif(v, slow, &[], b, &[]);
        fx.b.switch_to_block(b);
    }
    /// ... when `v` is not 0 (see `require_zero`).
    pub(crate) fn require_nonzero(&mut self, fx: &mut Fx, v: Value, slow: Block) {
        let b = fx.b.create_block();
        fx.b.ins().brif(v, b, &[], slow, &[]);
        fx.b.switch_to_block(b);
    }
    /// ... when `v` is a heap object (not an immediate, nil or false): rotated right by 3,
    /// an aligned word is below 2^61 and the heap ones are 2 and up. (One test and branch.)
    pub(crate) fn require_heap(&mut self, fx: &mut Fx, v: Value, slow: Block) {
        let r = fx.b.ins().rotr_imm_s(v, 3);
        let r = fx.b.ins().iadd_imm_s(r, -2);
        let heap = fx.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, r, (1i64 << 61) - 2);
        self.require(fx, heap, slow);
    }
    /// Nonzero when there is no room on the stack for a call (one OS thread) or a collection
    /// is pending (`dir`: KN_DIRECT), to be or-ed into a guard word that must be 0.
    pub(crate) fn no_room(&mut self, fx: &mut Fx, dir: Value) -> Value {
        let f = mem();
        let sp = fx.b.ins().get_stack_pointer(I64);
        let lim = fx.b.ins().load(I64, f, dir, 8);
        // stack addresses are far below 2^63: sp < lim exactly when sp - lim is negative
        let d = fx.b.ins().isub(sp, lim);
        let below = fx.b.ins().ushr_imm_u(d, 63);
        let gc = fx.b.ins().uload8(I64, f, fx.gcflag, 0);
        fx.b.ins().bor(below, gc)
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
        self.require_heap(fx, recv, slow_b);
        let k = fx.b.ins().load(I64, f, recv, 0);
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let ser = fx.b.ins().load(I64, f, e, CE_SERIAL);
        let cur = fx.b.ins().load(I64, f, dir, 0);
        let stale = fx.b.ins().bxor(ser, cur);
        let bad = self.no_room(fx, dir);
        let mut bad = fx.b.ins().bor(stale, bad);
        let mut func = fx.b.ins().iconst(I64, 0);
        let mut msel = fx.b.ins().iconst(I64, 0);
        for w in (0..ways).rev() {
            let kw = fx.b.ins().load(I64, f, e, CE_KLASS + 8 * w as i32);
            let hit = fx.b.ins().icmp(IntCC::Equal, kw, k);
            func = fx.b.ins().select(hit, fs[w], func);
            let mw = fx.b.ins().load(I64, f, e, CE_METHOD + 8 * w as i32);
            msel = fx.b.ins().select(hit, mw, msel);
        }
        // a compiled method (public, with an explicit receiver: the low bit marks private),
        // the current serial, room for the call: one branch on (all fine ? func : 0)
        if explicit {
            let priv_bit = fx.b.ins().band_imm_s(func, 1);
            bad = fx.b.ins().bor(bad, priv_bit);
        } else {
            func = fx.b.ins().band_imm_s(func, !1);
        }
        let fine = fx.b.ins().icmp_imm_s(IntCC::Equal, bad, 0);
        let zero = fx.b.ins().iconst(I64, 0);
        let go = fx.b.ins().select(fine, func, zero);
        let b2 = fx.b.create_block();
        fx.b.ins().brif(go, b2, &[], slow_b, &[]);
        fx.b.switch_to_block(b2);

        // the fast entry when the method takes exactly these arguments
        let fw = fx.b.ins().load(I64, f, msel, METHOD_FAST);
        let top = fx.b.ins().ushr_imm_u(fw, 56);
        let fits = fx.b.ins().icmp_imm_s(IntCC::Equal, top, vals.len() as i64 + 1);
        let fast_b = fx.b.create_block();
        let gen_b = fx.b.create_block();
        fx.b.ins().brif(fits, fast_b, &[], gen_b, &[]);
        fx.b.switch_to_block(fast_b);
        let fp = fx.b.ins().band_imm_s(fw, (1i64 << 56) - 1);
        let mut fargs = vec![recv];
        fargs.extend_from_slice(vals);
        fargs.push(fx.b.ins().iconst(I64, QNIL));
        let fsig = self.fast_sig(vals.len());
        let fsr = fx.b.import_signature(fsig);
        let call = fx.b.ins().call_indirect(fsr, fp, &fargs);
        self.licm_kill(fx);
        let r = fx.b.inst_results(call)[0];
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(gen_b);
        let argv = if vals.is_empty() { fx.b.ins().iconst(I64, 8) } else { self.args_buf(fx, vals) };
        let argc = fx.b.ins().iconst(I64, vals.len() as i64);
        let nil = fx.b.ins().iconst(I64, QNIL);
        let sig = self.method_sig();
        let sr = fx.b.import_signature(sig);
        let call = fx.b.ins().call_indirect(sr, func, &[recv, argc, argv, nil]);
        self.licm_kill(fx);
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
/// method it answers (3 nil?, 4 is_a?/kind_of?). `VARARGS`: any number of arguments in a
/// buffer plus a block, the runtime function's own parameters `(cache, recv, mid, argc,
/// argv, blk)`.
const VARARGS: usize = usize::MAX;
const STUBS: [(&str, usize, bool, u8); 12] = [
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
    ("kn_is_a_send", 1, true, 4),
    ("kn_send", VARARGS, false, 0),
    ("kn_send_pub", VARARGS, false, 0),
];
pub(crate) const STUB_SEND_N: u8 = 10;
pub(crate) const STUB_SEND_PUB_N: u8 = 11;
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
        let var = nargs == VARARGS;
        let mut sig = self.module.make_signature();
        for _ in 0..if var { 6 } else { 3 + nargs + has_explicit as usize } {
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
            let args = if var { vec![] } else { p[3..3 + nargs].to_vec() };
            let explicit = if var {
                (rt == "kn_send_pub").then(|| b.ins().iconst(I64, 1))
            } else if has_explicit {
                Some(p[3 + nargs])
            } else {
                None
            };
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
                if attr == 4 {
                    // every heap object's is_a? is the builtin one and the argument a class
                    // (not a module): the receiver's class has it at its depth in its
                    // superclass chain (kenny-runtime core.rs `is_a`)
                    let cache_b = b.create_block();
                    let flags = b.ins().load(I64, f, dir, KD_FLAGS);
                    let bit = b.ins().band_imm_s(flags, DIRECT_IS_A_HEAP);
                    let c1 = b.create_block();
                    b.ins().brif(bit, c1, &[], cache_b, &[]);
                    b.switch_to_block(c1);
                    let a = args[0];
                    let low = b.ins().band_imm_s(a, 7);
                    let aligned = b.ins().icmp_imm_s(IntCC::Equal, low, 0);
                    let not_nil = b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, a, QNIL);
                    let heap = b.ins().band(aligned, not_nil);
                    let c2 = b.create_block();
                    b.ins().brif(heap, c2, &[], cache_b, &[]);
                    b.switch_to_block(c2);
                    let kind = b.ins().uload32(f, a, HDR_KIND);
                    let is_cls = b.ins().icmp_imm_s(IntCC::Equal, kind, KIND_CLASS);
                    let c3 = b.create_block();
                    b.ins().brif(is_cls, c3, &[], cache_b, &[]);
                    b.switch_to_block(c3);
                    let cs_len = b.ins().load(I64, f, a, CLASS_SUP_LEN);
                    let c4 = b.create_block();
                    b.ins().brif(cs_len, c4, &[], cache_b, &[]);
                    b.switch_to_block(c4);
                    let k = b.ins().load(I64, f, recv, 0);
                    let ks_len = b.ins().load(I64, f, k, CLASS_SUP_LEN);
                    let d = b.ins().iadd_imm_s(cs_len, -1);
                    let inb = b.ins().icmp(IntCC::UnsignedLessThan, d, ks_len);
                    let c5 = b.create_block();
                    let no_b = b.create_block();
                    b.ins().brif(inb, c5, &[], no_b, &[]);
                    b.switch_to_block(no_b);
                    let no = b.ins().iconst(I64, QFALSE);
                    b.ins().return_(&[no]);
                    b.switch_to_block(c5);
                    let ksp = b.ins().load(I64, f, k, CLASS_SUP_PTR);
                    let off = b.ins().ishl_imm_u(d, 3);
                    let addr = b.ins().iadd(ksp, off);
                    let x = b.ins().load(I64, f, addr, 0);
                    let same = b.ins().icmp(IntCC::Equal, x, a);
                    let t = b.ins().iconst(I64, QTRUE);
                    let fl = b.ins().iconst(I64, QFALSE);
                    let r = b.ins().select(same, t, fl);
                    b.ins().return_(&[r]);
                    b.switch_to_block(cache_b);
                }
            }

            if attr == 1 && !var {
                // the common case first: way 0 is an attribute reader of the receiver's
                // class (a plain Object of that class, the slot within its buffer)
                let general = b.create_block();
                let e = b.ins().load(I64, f, cache, 0);
                let w0 = b.create_block();
                b.ins().brif(e, w0, &[], general, &[]);
                // (a site that calls a method: to the general path after one check)
                b.switch_to_block(w0);
                let a0 = b.ins().load(I64, f, e, CE_ATTR);
                let ak = b.ins().band_imm_s(a0, 6);
                let da = b.ins().bxor_imm_s(ak, 4);
                let w1 = b.create_block();
                b.ins().brif(da, general, &[], w1, &[]);
                b.switch_to_block(w1);
                let low = b.ins().band_imm_s(recv, 7);
                let aligned = b.ins().icmp_imm_s(IntCC::Equal, low, 0);
                let not_nil = b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, recv, QNIL);
                let heap = b.ins().band(aligned, not_nil);
                let w2 = b.create_block();
                b.ins().brif(heap, w2, &[], general, &[]);
                b.switch_to_block(w2);
                let k = b.ins().load(I64, f, recv, 0);
                let k0 = b.ins().load(I64, f, e, CE_KLASS);
                let ser = b.ins().load(I64, f, e, CE_SERIAL);
                let cur = b.ins().load(I64, f, dir, 0);
                let kf = b.ins().load(I64, f, recv, OBJ_KIND);
                let shape = b.ins().load(I64, f, recv, OBJ_SHAPE);
                // differences, all 0 when it is that case
                let dk = b.ins().bxor(k, k0);
                let ds = b.ins().bxor(ser, cur);
                let dshape = b.ins().bxor(shape, k);
                let kfm = b.ins().band_imm_s(kf, 0xFFFF_FFFF);
                let mut bad = b.ins().bor(dk, ds);
                bad = b.ins().bor(bad, dshape);
                bad = b.ins().bor(bad, kfm);
                if let Some(x) = explicit {
                    let pb = b.ins().band_imm_s(a0, 1);
                    let denied = b.ins().band(pb, x);
                    bad = b.ins().bor(bad, denied);
                }
                let w3 = b.create_block();
                b.ins().brif(bad, general, &[], w3, &[]);
                b.switch_to_block(w3);
                let cap_addr = b.ins().iadd_imm_s(recv, OBJ_IVARS_CAP as i64);
                let cap = b.ins().atomic_load(I64, f, cap_addr);
                let ptr = b.ins().load(I64, f, recv, OBJ_IVARS_PTR);
                let len = b.ins().load(I64, f, recv, OBJ_IVARS_LEN);
                let capn = b.ins().band_imm_s(cap, SVEC_CAP_MASK);
                let n = b.ins().umin(len, capn);
                let idx = b.ins().ushr_imm_u(a0, 8);
                let inb = b.ins().icmp(IntCC::UnsignedLessThan, idx, n);
                let w4 = b.create_block();
                b.ins().brif(inb, w4, &[], general, &[]);
                b.switch_to_block(w4);
                let off = b.ins().ishl_imm_u(idx, 3);
                let addr = b.ins().iadd(ptr, off);
                let v = b.ins().load(I64, f, addr, 0);
                let unset = b.ins().icmp_imm_s(IntCC::Equal, v, UNDEF);
                let nil = b.ins().iconst(I64, QNIL);
                let r = b.ins().select(unset, nil, v);
                b.ins().return_(&[r]);
                b.switch_to_block(general);
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
            let mut msel = b.ins().iconst(I64, 0);
            for w in (0..ways).rev() {
                let kw = b.ins().load(I64, f, e, CE_KLASS + 8 * w as i32);
                let hit = b.ins().icmp(IntCC::Equal, kw, k);
                fsel = b.ins().select(hit, fs[w], fsel);
                if !var {
                    let mw = b.ins().load(I64, f, e, CE_METHOD + 8 * w as i32);
                    msel = b.ins().select(hit, mw, msel);
                }
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
            if !var {
                // the method's fast entry when it takes exactly these arguments
                // (kenny-runtime core.rs `Method::fast`)
                let fw = b.ins().load(I64, f, msel, METHOD_FAST);
                let top = b.ins().ushr_imm_u(fw, 56);
                let fits = b.ins().icmp_imm_s(IntCC::Equal, top, nargs as i64 + 1);
                let fast_b = b.create_block();
                let gen_b = b.create_block();
                b.ins().brif(fits, fast_b, &[], gen_b, &[]);
                b.switch_to_block(fast_b);
                let fp = b.ins().band_imm_s(fw, (1i64 << 56) - 1);
                let mut fargs = vec![recv];
                fargs.extend_from_slice(&args);
                fargs.push(b.ins().iconst(I64, QNIL));
                let fsig = self.fast_sig(nargs);
                let fsr = b.import_signature(fsig);
                let call = b.ins().call_indirect(fsr, fp, &fargs);
                let r = b.inst_results(call)[0];
                b.ins().return_(&[r]);
                b.switch_to_block(gen_b);
            }
            let (argc, argv, blk) = if var {
                (p[3], p[4], p[5])
            } else {
                let argv = if nargs == 0 {
                b.ins().iconst(I64, 8)
            } else {
                let ss = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (nargs * 8) as u32, 3));
                for (i, v) in args.iter().enumerate() {
                    b.ins().stack_store(I64, *v, ss, (i * 8) as i32);
                }
                b.ins().stack_addr(I64, ss, 0)
                };
                (b.ins().iconst(I64, nargs as i64), argv, b.ins().iconst(I64, QNIL))
            };
            let sr = b.import_signature(msig);
            let call = b.ins().call_indirect(sr, clean, &[recv, argc, argv, blk]);
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
/// kenny-runtime core.rs `CLASS_CIVARS`
const CLASS_CIVARS: i32 = 144;
/// kenny-runtime core.rs `KnDirect::flags` and its bits, `SYM_TAG`.
const KD_FLAGS: i32 = 16;
const KD_CONST_SERIAL: i32 = 24;
const KD_MATH_MODULE: i32 = 40;
const DIRECT_MATH: i64 = 32;
const DIRECT_NUM: i64 = 64;
const KD_ARRAY_CLASS: i32 = 48;
/// kenny-runtime core.rs `KnDirect::string_class`, `DIRECT_STR_BYTE`, `RString::b`.
const KD_STRING_CLASS: i32 = 64;
const DIRECT_STR_BYTE: i64 = 512;
const STR_PTR: i32 = 16;
const STR_LEN: i32 = 24;
const STR_CAP: i32 = 32;
const DIRECT_ARY_AREF: i64 = 8;
const DIRECT_ARY_ASET: i64 = 16;
/// kenny-runtime core.rs `RArray::v` (an `SVec`: ptr, len, cap).
const ARY_PTR: i32 = 16;
const ARY_LEN: i32 = 24;
const ARY_CAP: i32 = 32;
/// kenny-runtime core.rs `Class::sup_ptr` / `sup_len` (`CLASS_SUP_PTR`).
const CLASS_SUP_PTR: i32 = 272;
/// kenny-runtime core.rs `Header::kind`, `Kind::Class`, `DIRECT_IS_A_HEAP`.
const HDR_KIND: i32 = 8;
const KIND_CLASS: i64 = 7;
const DIRECT_IS_A_HEAP: i64 = 128;
const DIRECT_MOD_EQQ_ALL: i64 = 256;
const CLASS_SUP_LEN: i32 = 280;
const CE_METHOD: i32 = 48;
/// kenny-runtime core.rs `Method::fast`
const METHOD_FAST: i32 = 0;
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
