//! Expressions: literals, variables, assignment, strings, collections, `defined?`.

use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, InstBuilder};
use ruby_prism::*;

pub(crate) fn int_value(n: &IntegerNode) -> Option<i64> {
    let value = n.value();
    let (neg, digits) = value.to_u32_digits();
    if digits.len() > 2 {
        return None;
    }
    let mut v: u64 = 0;
    for (i, d) in digits.iter().enumerate() {
        v |= (*d as u64) << (32 * i);
    }
    if v > (1u64 << 62) {
        return None;
    }
    let v = v as i64;
    let v = if neg { -v } else { v };
    if v >= -(1i64 << 62) && v < (1i64 << 62) { Some(v) } else { None }
}
fn int_decimal(n: &IntegerNode) -> String {
    let value = n.value();
    let (neg, digits) = value.to_u32_digits();
    let mut big = num_from_digits(digits);
    if big.is_empty() {
        big = "0".into();
    }
    if neg { format!("-{big}") } else { big }
}
fn num_from_digits(d: &[u32]) -> String {
    // base 2^32 -> decimal via repeated division
    let mut limbs: Vec<u32> = d.to_vec();
    let mut out = Vec::new();
    while limbs.iter().any(|x| *x != 0) {
        let mut rem: u64 = 0;
        for l in limbs.iter_mut().rev() {
            let cur = (rem << 32) | *l as u64;
            *l = (cur / 10) as u32;
            rem = cur % 10;
        }
        out.push(b'0' + rem as u8);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Regex literal source as Onigmo sees it (only `\/` is unescaped).
fn regex_source(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' && raw.get(i + 1) == Some(&b'/') {
            out.push(b'/');
            i += 2;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

impl<M: Module> Compiler<M> {
    /// An integer literal value of any size as a Ruby Integer.
    fn big_int_value(&mut self, fx: &mut Fx, v: &ruby_prism::Integer) -> Value {
        let (neg, digits) = v.to_u32_digits();
        let mut mag: u64 = 0;
        let small = digits.len() <= 2;
        if small {
            for (i, d) in digits.iter().enumerate() {
                mag |= (*d as u64) << (32 * i);
            }
        }
        if small && mag < (1u64 << 61) {
            let x = if neg { -(mag as i64) } else { mag as i64 };
            return self.iconst(fx, (x << 1) | 1);
        }
        let mut txt = num_from_digits(digits);
        if txt.is_empty() {
            txt = "0".into();
        }
        if neg {
            txt.insert(0, '-');
        }
        let slot = self.cache_ptr(fx, 1);
        let (p, l) = self.str_ptr(fx, txt.as_bytes());
        self.call(fx, "kn_int_lit", &[slot, p, l])
    }

    pub(crate) fn stmts(&mut self, fx: &mut Fx, s: Option<StatementsNode>) -> Value {
        match s {
            Some(s) => self.expr(fx, &s.as_node()),
            None => self.nil(fx),
        }
    }

    /// A statement whose value is not used: `a, b = x, y` assigns without building the
    /// right-hand side Array.
    /// `x = v` / `x op= v`, keeping a Float result unboxed when `x` has a shadow.
    fn local_write_num(&mut self, fx: &mut Fx, node: &Node) -> Option<crate::fastpath::Num> {
        let at = node.location().start_offset();
        let r = self.local_write_num_inner(fx, node)?;
        // the generic copy of a speculated loop: where its fast copy may continue here
        if let Some(b) = fx.resume.remove(&at) {
            let v = self.materialize(fx, r);
            fx.b.ins().jump(b, &[v.into()]);
            fx.b.switch_to_block(b);
            return Some(crate::fastpath::Num::Boxed(fx.b.block_params(b)[0]));
        }
        Some(r)
    }
    fn local_write_num_inner(&mut self, fx: &mut Fx, node: &Node) -> Option<crate::fastpath::Num> {
        // the fast copy of a speculated loop: a checked assignment
        if let Some(spec) = &fx.spec {
            let target = node
                .as_local_variable_write_node()
                .map(|n| (n.name().as_slice().to_vec(), n.depth()))
                .or_else(|| node.as_local_variable_operator_write_node().map(|n| (n.name().as_slice().to_vec(), n.depth())));
            // (a block inlined into the loop has locals of its own at depth 0)
            if let Some((name, _)) = target.filter(|t| t.1 as usize == fx.inline_levels.len()) {
                if spec.locals.iter().any(|l| l.name == name) {
                    let at = node.location().start_offset();
                    self.cur_offset = at;
                    // `i = i + 1` / `i += 1` on a Fixnum local
                    let arith = match node.as_local_variable_write_node() {
                        Some(n) => n.value().as_call_node().and_then(|c| {
                            let (a, b) = crate::fastpath::numeric_call(&c)?;
                            Some((c.name().as_slice().to_vec(), a, b))
                        }),
                        None => None,
                    };
                    if let Some((op, a, b)) = arith {
                        if let Some(r) = self.spec_int_arith(fx, &name, &op, Some(&a), &b, at) {
                            return Some(r);
                        }
                    }
                    if let Some(n) = node.as_local_variable_operator_write_node() {
                        let op = n.binary_operator().as_slice().to_vec();
                        if let Some(r) = self.spec_int_arith(fx, &name, &op, None, &n.value(), at) {
                            return Some(r);
                        }
                    }
                    let v = if let Some(n) = node.as_local_variable_write_node() {
                        self.num_expr(fx, &n.value(), true)
                    } else {
                        let n = node.as_local_variable_operator_write_node().unwrap();
                        let cur = self.local_get_num(fx, &name, 0);
                        let rhs = self.num_expr(fx, &n.value(), true);
                        self.cur_offset = at;
                        self.binop_num(fx, n.binary_operator().as_slice(), cur, rhs, true)
                    };
                    return Some(self.spec_assign(fx, &name, v, at));
                }
            }
        }
        if let Some(n) = node.as_local_variable_write_node() {
            self.cur_offset = node.location().start_offset();
            let (name, depth) = (n.name().as_slice().to_vec(), n.depth());
            // `x = y` between always-Float locals: the same object, if it is a heap Float
            let level = fx.inline_levels.len() as u32;
            if let (Some(&(rx, cx)), Some(r)) = (fx.flocals.get(&name), n.value().as_local_variable_read_node()) {
                if let (true, Some(&(ry, cy))) = (depth == level && r.depth() == level, fx.flocals.get(r.name().as_slice())) {
                    if let Some(cx) = cx {
                        let boxed = match cy {
                            Some(cy) => self.flocal_pin(fx, ry, cy),
                            None => fx.b.ins().iconst(I64, 0),
                        };
                        fx.b.def_var(cx, boxed);
                    }
                    let f = fx.b.use_var(ry);
                    fx.b.def_var(rx, f);
                    return Some(crate::fastpath::Num::F64(f));
                }
            }
            if self.has_num_shadow(fx, &name, depth) {
                let v = self.num_expr(fx, &n.value(), true);
                self.local_set_num(fx, &name, depth, v);
                return Some(v);
            }
            let v = self.expr(fx, &n.value());
            self.local_set(fx, &name, depth, v);
            // a new String only this local has (owned.rs)
            if depth == 0 && fx.inline_levels.is_empty() && crate::owned::fresh_str(&n.value()) {
                if let Some(&ov) = fx.owned.get(&name) {
                    let one = fx.b.ins().iconst(cranelift_codegen::ir::types::I8, 1);
                    fx.b.def_var(ov, one);
                }
            }
            return Some(crate::fastpath::Num::Boxed(v));
        }
        if let Some(n) = node.as_local_variable_operator_write_node() {
            self.cur_offset = node.location().start_offset();
            let (name, depth) = (n.name().as_slice().to_vec(), n.depth());
            let cur = self.local_get_num(fx, &name, depth);
            let rhs = self.num_expr(fx, &n.value(), true);
            self.cur_offset = node.location().start_offset();
            let lazy = self.has_num_shadow(fx, &name, depth);
            let v = self.binop_num(fx, n.binary_operator().as_slice(), cur, rhs, lazy);
            self.local_set_num(fx, &name, depth, v);
            return Some(v);
        }
        None
    }

    /// `s = s + x` / `s += x` as a statement, `s` a local of `fx.owned` (owned.rs):
    /// appended to in place while its owned register is set.
    fn append_own(&mut self, fx: &mut Fx, node: &Node, name: &[u8], ov: Variable) -> Value {
        use cranelift_codegen::ir::types::I8;
        self.cur_offset = node.location().start_offset();
        let arg = crate::owned::append_arg(node, name).unwrap();
        let Some(&Local::Var(var)) = fx.locals.get(name) else { unreachable!() };
        let cur = fx.b.use_var(var);
        let e = self.expr(fx, &arg);
        self.cur_offset = match node.as_local_variable_write_node() {
            Some(w) => w.value().as_call_node().and_then(|c| c.message_loc()).map(|l| l.start_offset()),
            None => node.as_local_variable_operator_write_node().map(|w| w.binary_operator_loc().start_offset()),
        }
        .unwrap_or(node.location().start_offset());
        let own8 = fx.b.use_var(ov);
        let own = fx.b.ins().uextend(I64, own8);
        let r = self.call(fx, "kn_str_append_own", &[cur, e, own]);
        let undef = fx.b.ins().icmp_imm_s(IntCC::Equal, r, UNDEF);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.append_block_param(done, I8);
        fx.b.set_cold_block(slow);
        let one = fx.b.ins().iconst(I8, 1);
        fx.b.ins().brif(undef, slow, &[], done, &[r.into(), one.into()]);
        fx.b.switch_to_block(slow);
        let v = self.send(fx, cur, b"+", &[e], None);
        let zero = fx.b.ins().iconst(I8, 0);
        fx.b.ins().jump(done, &[v.into(), zero.into()]);
        fx.b.switch_to_block(done);
        let (v, o) = (fx.b.block_params(done)[0], fx.b.block_params(done)[1]);
        fx.b.def_var(var, v);
        fx.b.def_var(ov, o);
        self.nil(fx)
    }

    /// Statements whose values are not used (loop bodies).
    pub(crate) fn stmts_void(&mut self, fx: &mut Fx, s: Option<StatementsNode>) {
        if let Some(s) = s {
            for n in s.body().iter() {
                self.expr_void(fx, &n);
            }
        }
    }

    pub(crate) fn expr_void(&mut self, fx: &mut Fx, node: &Node) -> Value {
        if !fx.owned.is_empty() && fx.inline_levels.is_empty() {
            if let Some(name) = crate::owned::append_target(node) {
                if let Some(&ov) = fx.owned.get(&name) {
                    return self.append_own(fx, node, &name, ov);
                }
            }
            // (so that appends in its branches are statements too)
            if let Some(n) = node.as_if_node() {
                return self.if_void(fx, &n);
            }
            if let Some(n) = node.as_unless_node() {
                return self.unless_void(fx, &n);
            }
        }
        if let Some(w) = node.as_local_variable_write_node() {
            if w.depth() == 0 && fx.inline_levels.is_empty() && fx.vlocals.contains_key(w.name().as_slice()) {
                if let Some(c) = w.value().as_call_node() {
                    self.cur_offset = node.location().start_offset();
                    self.vobj_new(fx, w.name().as_slice(), &c);
                    return self.nil(fx);
                }
            }
        }
        if node.as_local_variable_write_node().is_some() || node.as_local_variable_operator_write_node().is_some() {
            self.local_write_num(fx, node);
            return self.nil(fx);
        }
        if let Some(m) = node.as_multi_write_node() {
            let simple = |n: &Node| {
                n.as_local_variable_target_node().is_some() || n.as_instance_variable_target_node().is_some()
            };
            if let Some(a) = m.value().as_array_node() {
                let elems: Vec<Node> = a.elements().iter().collect();
                let lefts: Vec<Node> = m.lefts().iter().collect();
                if m.rest().is_none()
                    && m.rights().iter().next().is_none()
                    && lefts.iter().all(simple)
                    && elems.iter().all(|e| e.as_splat_node().is_none() && e.as_keyword_hash_node().is_none())
                {
                    self.cur_offset = node.location().start_offset();
                    // a local with an unboxed shadow keeps a Float result unboxed
                    let shadow = |s: &Self, fx: &Fx, t: Option<&Node>| {
                        t.and_then(|t| t.as_local_variable_target_node()).filter(|l| s.has_num_shadow(fx, l.name().as_slice(), l.depth())).map(|l| (l.name().as_slice().to_vec(), l.depth()))
                    };
                    let mut vals: Vec<crate::fastpath::Num> = Vec::new();
                    for (i, e) in elems.iter().enumerate() {
                        let v = if shadow(self, fx, lefts.get(i)).is_some() { self.num_expr(fx, e, true) } else { crate::fastpath::Num::Boxed(self.expr(fx, e)) };
                        vals.push(v);
                    }
                    for (i, t) in lefts.iter().enumerate() {
                        let v = match vals.get(i) {
                            Some(v) => *v,
                            None => crate::fastpath::Num::Boxed(self.nil(fx)),
                        };
                        match shadow(self, fx, Some(t)) {
                            Some((name, depth)) => self.local_set_num(fx, &name, depth, v),
                            None => {
                                let v = self.materialize(fx, v);
                                self.assign_target(fx, t, v);
                            }
                        }
                    }
                    return self.nil(fx);
                }
            }
        }
        self.expr(fx, node)
    }

    pub(crate) fn expr(&mut self, fx: &mut Fx, node: &Node) -> Value {
        let at = node.location().start_offset();
        self.cur_offset = at;
        self.set_srcloc(fx);
        match node {
            Node::StatementsNode { .. } => {
                let s = node.as_statements_node().unwrap();
                let body: Vec<Node> = s.body().iter().collect();
                let mut last = None;
                for (i, n) in body.iter().enumerate() {
                    last = Some(if i + 1 < body.len() { self.expr_void(fx, n) } else { self.expr(fx, n) });
                }
                match last {
                    Some(v) => v,
                    None => self.nil(fx),
                }
            }
            Node::ParenthesesNode { .. } => match node.as_parentheses_node().unwrap().body() {
                Some(b) => self.expr(fx, &b),
                None => self.nil(fx),
            },
            Node::NilNode { .. } => self.nil(fx),
            Node::TrueNode { .. } => self.iconst(fx, QTRUE),
            Node::FalseNode { .. } => self.iconst(fx, QFALSE),
            Node::SelfNode { .. } => self.self_val(fx),
            Node::IntegerNode { .. } => {
                let n = node.as_integer_node().unwrap();
                match int_value(&n) {
                    Some(v) => self.iconst(fx, (v << 1) | 1),
                    None => {
                        let txt = int_decimal(&n);
                        let slot = self.cache_ptr(fx, 1);
                        let (p, l) = self.str_ptr(fx, txt.as_bytes());
                        self.call(fx, "kn_int_lit", &[slot, p, l])
                    }
                }
            }
            Node::FloatNode { .. } => {
                let f = node.as_float_node().unwrap().value();
                self.float_lit(fx, f)
            }
            Node::RationalNode { .. } => {
                let r = node.as_rational_node().unwrap();
                let (nv, dv) = (r.numerator(), r.denominator());
                let n = self.big_int_value(fx, &nv);
                let d = self.big_int_value(fx, &dv);
                self.callc(fx, "kn_rational_lit", &[n, d])
            }
            Node::ImaginaryNode { .. } => {
                // 2i, 1.5i, 3ri  ->  Complex(0, numeric)
                let n = node.as_imaginary_node().unwrap();
                let im = self.expr(fx, &n.numeric());
                let zero = self.iconst(fx, 1); // tagged Fixnum 0
                let s = self.self_val(fx);
                self.send(fx, s, b"Complex", &[zero, im], None)
            }
            Node::StringNode { .. } => {
                let s = node.as_string_node().unwrap();
                let bytes = s.unescaped().to_vec();
                if s.is_mutable() {
                    let (p, l) = self.str_ptr(fx, &bytes);
                    self.call(fx, "kn_str_new", &[p, l])
                } else {
                    self.str_lit(fx, &bytes)
                }
            }
            Node::InterpolatedStringNode { .. } => {
                let s = node.as_interpolated_string_node().unwrap();
                self.interpolate(fx, &s.parts())
            }
            Node::XStringNode { .. } => {
                let x = node.as_x_string_node().unwrap();
                let (p, l) = self.str_ptr(fx, &x.unescaped().to_vec());
                let cmd = self.call(fx, "kn_str_new", &[p, l]);
                let s = self.self_val(fx);
                self.send(fx, s, b"`", &[cmd], None)
            }
            Node::InterpolatedXStringNode { .. } => {
                let x = node.as_interpolated_x_string_node().unwrap();
                let cmd = self.interpolate(fx, &x.parts());
                let s = self.self_val(fx);
                self.send(fx, s, b"`", &[cmd], None)
            }
            Node::SymbolNode { .. } => {
                let s = node.as_symbol_node().unwrap();
                self.sym_val(fx, &s.unescaped().to_vec())
            }
            Node::InterpolatedSymbolNode { .. } => {
                let s = node.as_interpolated_symbol_node().unwrap();
                let st = self.interpolate(fx, &s.parts());
                self.call(fx, "kn_str_to_sym", &[st])
            }
            Node::RegularExpressionNode { .. } => {
                let r = node.as_regular_expression_node().unwrap();
                let src = regex_source(r.content_loc().as_slice());
                // 32: /n (ASCII-8BIT), so `\xHH` escapes match bytes
                let opts = (r.is_ignore_case() as i64) | ((r.is_extended() as i64) << 1) | ((r.is_multi_line() as i64) << 2) | ((r.is_ascii_8bit() as i64) << 5);
                let slot = self.cache_ptr(fx, 1);
                let (p, l) = self.str_ptr(fx, &src);
                let o = self.iconst(fx, opts);
                self.callc(fx, "kn_regexp_lit", &[slot, p, l, o])
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let r = node.as_interpolated_regular_expression_node().unwrap();
                let (p, l) = self.str_ptr(fx, b"");
                let st = self.call(fx, "kn_str_new", &[p, l]);
                for part in r.parts().iter() {
                    if let Some(sn) = part.as_string_node() {
                        let src = regex_source(sn.content_loc().as_slice());
                        let (p, l) = self.str_ptr(fx, &src);
                        self.call(fx, "kn_str_append_raw", &[st, p, l]);
                    } else {
                        let v = self.expr(fx, &part);
                        self.callc(fx, "kn_str_append", &[st, v]);
                    }
                }
                let opts = (r.is_ignore_case() as i64) | ((r.is_extended() as i64) << 1) | ((r.is_multi_line() as i64) << 2) | ((r.is_ascii_8bit() as i64) << 5);
                let o = self.iconst(fx, opts);
                self.callc(fx, "kn_regexp_new", &[st, o])
            }
            Node::MatchWriteNode { .. } => {
                let m = node.as_match_write_node().unwrap();
                let r = self.call_node(fx, &m.call());
                for t in m.targets().iter() {
                    if let Some(lt) = t.as_local_variable_target_node() {
                        let s = self.sym_val(fx, lt.name().as_slice());
                        let v = self.call(fx, "kn_match_named", &[s]);
                        self.local_set(fx, lt.name().as_slice(), lt.depth(), v);
                    }
                }
                r
            }
            Node::NumberedReferenceReadNode { .. } => {
                let n = node.as_numbered_reference_read_node().unwrap().number();
                let nv = self.iconst(fx, n as i64);
                self.call(fx, "kn_nth_ref", &[nv])
            }
            Node::BackReferenceReadNode { .. } => {
                let b = node.as_back_reference_read_node().unwrap();
                let c = b.name().as_slice().get(1).copied().unwrap_or(b'&');
                let cv = self.iconst(fx, c as i64);
                self.call(fx, "kn_backref", &[cv])
            }
            Node::MatchLastLineNode { .. } | Node::InterpolatedMatchLastLineNode { .. } => {
                self.unsupported(fx, at, "matching a bare regex against $_ is not supported by Kenny");
                self.nil(fx)
            }
            Node::ArrayNode { .. } => {
                let a = node.as_array_node().unwrap();
                self.array_literal(fx, &a.elements())
            }
            Node::HashNode { .. } => {
                let h = node.as_hash_node().unwrap();
                self.hash_literal(fx, &h.elements(), false)
            }
            Node::KeywordHashNode { .. } => {
                let h = node.as_keyword_hash_node().unwrap();
                self.hash_literal(fx, &h.elements(), true)
            }
            Node::RangeNode { .. } => {
                let r = node.as_range_node().unwrap();
                let lo = match r.left() {
                    Some(n) => self.expr(fx, &n),
                    None => self.nil(fx),
                };
                let hi = match r.right() {
                    Some(n) => self.expr(fx, &n),
                    None => self.nil(fx),
                };
                let ex = self.iconst(fx, r.is_exclude_end() as i64);
                self.call(fx, "kn_range_new", &[lo, hi, ex])
            }
            Node::SourceFileNode { .. } => {
                let f = self.file.display().to_string();
                self.str_lit(fx, f.as_bytes())
            }
            Node::SourceLineNode { .. } => {
                let l = self.line_of(at) as i64;
                self.iconst(fx, (l << 1) | 1)
            }
            Node::SourceEncodingNode { .. } => {
                let cref = self.cref(fx);
                let enc = self.sym_val(fx, b"Encoding");
                let e = self.callc(fx, "kn_const_get", &[cref, enc]);
                let u = self.sym_val(fx, b"UTF_8");
                self.callc(fx, "kn_const_get_under", &[e, u])
            }

            // ---- variables ----
            Node::LocalVariableReadNode { .. } => {
                let n = node.as_local_variable_read_node().unwrap();
                self.local_get(fx, n.name().as_slice(), n.depth())
            }
            Node::ItLocalVariableReadNode { .. } => self.local_get(fx, b"it", 0),
            Node::LocalVariableWriteNode { .. } => {
                let n = self.local_write_num(fx, node).unwrap();
                self.materialize(fx, n)
            }
            Node::LocalVariableOperatorWriteNode { .. } => {
                let n = self.local_write_num(fx, node).unwrap();
                self.materialize(fx, n)
            }
            Node::LocalVariableOrWriteNode { .. } => {
                let n = node.as_local_variable_or_write_node().unwrap();
                let (name, depth) = (n.name().as_slice().to_vec(), n.depth());
                let cur = self.local_get(fx, &name, depth);
                let value = n.value();
                self.or_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.local_set(fx, &name, depth, v);
                    v
                })
            }
            Node::LocalVariableAndWriteNode { .. } => {
                let n = node.as_local_variable_and_write_node().unwrap();
                let (name, depth) = (n.name().as_slice().to_vec(), n.depth());
                let cur = self.local_get(fx, &name, depth);
                let value = n.value();
                self.and_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.local_set(fx, &name, depth, v);
                    v
                })
            }
            Node::InstanceVariableReadNode { .. } => {
                let n = node.as_instance_variable_read_node().unwrap();
                self.ivar_get(fx, n.name().as_slice())
            }
            Node::InstanceVariableWriteNode { .. } => {
                let n = node.as_instance_variable_write_node().unwrap();
                let v = self.expr(fx, &n.value());
                self.ivar_set(fx, n.name().as_slice(), v)
            }
            Node::InstanceVariableOperatorWriteNode { .. } => {
                let n = node.as_instance_variable_operator_write_node().unwrap();
                let name = n.name().as_slice().to_vec();
                let cur = self.ivar_get(fx, &name);
                let rhs = self.num_expr(fx, &n.value(), true);
                let v = self.binop_rhs(fx, n.binary_operator().as_slice(), cur, rhs);
                self.ivar_set(fx, &name, v)
            }
            Node::InstanceVariableOrWriteNode { .. } => {
                let n = node.as_instance_variable_or_write_node().unwrap();
                let name = n.name().as_slice().to_vec();
                let cur = self.ivar_get(fx, &name);
                let value = n.value();
                self.or_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.ivar_set(fx, &name, v)
                })
            }
            Node::InstanceVariableAndWriteNode { .. } => {
                let n = node.as_instance_variable_and_write_node().unwrap();
                let name = n.name().as_slice().to_vec();
                let cur = self.ivar_get(fx, &name);
                let value = n.value();
                self.and_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.ivar_set(fx, &name, v)
                })
            }
            Node::GlobalVariableReadNode { .. } => {
                let n = node.as_global_variable_read_node().unwrap();
                let s = self.sym_val(fx, n.name().as_slice());
                self.call(fx, "kn_gvar_get", &[s])
            }
            Node::GlobalVariableWriteNode { .. } => {
                let n = node.as_global_variable_write_node().unwrap();
                let v = self.expr(fx, &n.value());
                let s = self.sym_val(fx, n.name().as_slice());
                self.call(fx, "kn_gvar_set", &[s, v])
            }
            Node::GlobalVariableOperatorWriteNode { .. } => {
                let n = node.as_global_variable_operator_write_node().unwrap();
                let s = self.sym_val(fx, n.name().as_slice());
                let cur = self.call(fx, "kn_gvar_get", &[s]);
                let rhs = self.num_expr(fx, &n.value(), true);
                let v = self.binop_rhs(fx, n.binary_operator().as_slice(), cur, rhs);
                self.call(fx, "kn_gvar_set", &[s, v])
            }
            Node::GlobalVariableOrWriteNode { .. } => {
                let n = node.as_global_variable_or_write_node().unwrap();
                let s = self.sym_val(fx, n.name().as_slice());
                let cur = self.call(fx, "kn_gvar_get", &[s]);
                let value = n.value();
                self.or_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.call(fx, "kn_gvar_set", &[s, v])
                })
            }
            Node::ClassVariableReadNode { .. } => {
                let n = node.as_class_variable_read_node().unwrap();
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                self.callc(fx, "kn_cvar_get", &[cref, s])
            }
            Node::ClassVariableWriteNode { .. } => {
                let n = node.as_class_variable_write_node().unwrap();
                let v = self.expr(fx, &n.value());
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                self.call(fx, "kn_cvar_set", &[cref, s, v])
            }
            Node::ClassVariableOperatorWriteNode { .. } => {
                let n = node.as_class_variable_operator_write_node().unwrap();
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                let cur = self.callc(fx, "kn_cvar_get", &[cref, s]);
                let rhs = self.num_expr(fx, &n.value(), true);
                let v = self.binop_rhs(fx, n.binary_operator().as_slice(), cur, rhs);
                self.call(fx, "kn_cvar_set", &[cref, s, v])
            }
            Node::ClassVariableOrWriteNode { .. } => {
                let n = node.as_class_variable_or_write_node().unwrap();
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                let cur = self.call(fx, "kn_cvar_get_soft", &[cref, s]);
                let value = n.value();
                self.or_assign(fx, cur, |c, fx| {
                    let v = c.expr(fx, &value);
                    c.call(fx, "kn_cvar_set", &[cref, s, v])
                })
            }
            Node::ConstantReadNode { .. } => {
                let n = node.as_constant_read_node().unwrap();
                let crefp = self.slot_ptr(fx, fx.cref_slot);
                let s = self.sym_val(fx, n.name().as_slice());
                let cache = self.owned_cache_ptr(fx, OWNED_CONST);
                // inline in loops and the app's own code (in every method of the gems too:
                // +4.5% code in a Rails app)
                if fx.loops.is_empty() && self.in_dependency() {
                    self.callc(fx, "kn_const_get_l", &[cache, crefp, s])
                } else {
                    self.const_cached(fx, cache, crefp, s)
                }
            }
            Node::ConstantPathNode { .. } => {
                let n = node.as_constant_path_node().unwrap();
                let parent = match n.parent() {
                    Some(p) => self.expr(fx, &p),
                    None => self.call(fx, "kn_object_class", &[]),
                };
                let name = n.name().map(|x| x.as_slice().to_vec()).unwrap_or_default();
                let s = self.sym_val(fx, &name);
                self.callc(fx, "kn_const_get_under", &[parent, s])
            }
            Node::ConstantWriteNode { .. } => {
                let n = node.as_constant_write_node().unwrap();
                let v = self.expr(fx, &n.value());
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                self.callc(fx, "kn_const_set", &[cref, s, v])
            }
            Node::ConstantPathWriteNode { .. } => {
                let n = node.as_constant_path_write_node().unwrap();
                let t = n.target();
                let parent = match t.parent() {
                    Some(p) => self.expr(fx, &p),
                    None => self.call(fx, "kn_object_class", &[]),
                };
                let v = self.expr(fx, &n.value());
                let name = t.name().map(|x| x.as_slice().to_vec()).unwrap_or_default();
                let s = self.sym_val(fx, &name);
                self.callc(fx, "kn_const_set", &[parent, s, v])
            }
            Node::ConstantOrWriteNode { .. } => {
                let n = node.as_constant_or_write_node().unwrap();
                let cref = self.cref(fx);
                let s = self.sym_val(fx, n.name().as_slice());
                let def = self.call(fx, "kn_const_defined", &[cref, s]);
                let yes = fx.b.create_block();
                let no = fx.b.create_block();
                let done = fx.b.create_block();
                fx.b.append_block_param(done, I64);
                fx.b.ins().brif(def, yes, &[], no, &[]);
                fx.b.switch_to_block(yes);
                let cur = self.callc(fx, "kn_const_get", &[cref, s]);
                fx.b.ins().jump(done, &[cur.into()]);
                fx.b.switch_to_block(no);
                let v = self.expr(fx, &n.value());
                self.callc(fx, "kn_const_set", &[cref, s, v]);
                fx.b.ins().jump(done, &[v.into()]);
                fx.b.switch_to_block(done);
                fx.b.block_params(done)[0]
            }
            Node::MultiWriteNode { .. } => {
                let m = node.as_multi_write_node().unwrap();
                let value = m.value();
                // the expression's value is the right-hand side as written (`while (a, b = q.shift)`
                // stops at nil), not the array it was converted to
                let (rhs, result) = if let Some(a) = value.as_array_node() {
                    let r = self.array_literal(fx, &a.elements());
                    (r, r)
                } else {
                    let v = self.expr(fx, &value);
                    (self.callc(fx, "kn_masgn_ary", &[v]), v)
                };
                self.destructure(fx, &m.lefts(), m.rest(), &m.rights(), rhs);
                result
            }

            // ---- definitions ----
            Node::DefNode { .. } => self.def_node(fx, &node.as_def_node().unwrap()),
            Node::ClassNode { .. } => self.class_node(fx, &node.as_class_node().unwrap()),
            Node::ModuleNode { .. } => self.module_node(fx, &node.as_module_node().unwrap()),
            Node::SingletonClassNode { .. } => self.sclass_node(fx, &node.as_singleton_class_node().unwrap()),
            Node::AliasMethodNode { .. } => {
                let a = node.as_alias_method_node().unwrap();
                let name_of = |n: Node| n.as_symbol_node().map(|s| s.unescaped().to_vec());
                match (name_of(a.new_name()), name_of(a.old_name())) {
                    (Some(new), Some(old)) => {
                        let s = self.self_val(fx);
                        let nv = self.sym_val(fx, &new);
                        let ov = self.sym_val(fx, &old);
                        self.callc(fx, "kn_alias", &[s, nv, ov])
                    }
                    _ => {
                        self.unsupported(fx, at, "dynamic alias names are not supported");
                        self.nil(fx)
                    }
                }
            }
            Node::AliasGlobalVariableNode { .. } => {
                let a = node.as_alias_global_variable_node().unwrap();
                let new = a.new_name().location().as_slice().to_vec();
                let old = a.old_name().location().as_slice().to_vec();
                let nv = self.sym_val(fx, &new);
                let ov = self.sym_val(fx, &old);
                self.call(fx, "kn_gvar_alias", &[nv, ov])
            }
            Node::UndefNode { .. } => {
                let u = node.as_undef_node().unwrap();
                let s = self.self_val(fx);
                for n in u.names().iter() {
                    if let Some(sn) = n.as_symbol_node() {
                        let v = self.sym_val(fx, &sn.unescaped().to_vec());
                        self.call(fx, "kn_undef", &[s, v]);
                    }
                }
                self.nil(fx)
            }
            Node::LambdaNode { .. } => {
                let l = node.as_lambda_node().unwrap();
                let locals = l.locals().iter().map(|c| c.as_slice().to_vec()).collect();
                self.block_proc(fx, l.parameters(), l.body(), locals, true)
            }

            // ---- control flow, calls ----
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                if crate::fastpath::numeric_call(&c).is_some() || (crate::fastpath::num_call_of(&c).is_some() && self.refine_scope.is_none()) {
                    let n = self.num_expr(fx, node, false);
                    self.materialize(fx, n)
                } else {
                    self.call_node(fx, &c)
                }
            }
            Node::CallOperatorWriteNode { .. } => self.call_op_write(fx, &node.as_call_operator_write_node().unwrap()),
            Node::CallOrWriteNode { .. } => self.call_or_write(fx, &node.as_call_or_write_node().unwrap(), true),
            Node::CallAndWriteNode { .. } => self.call_and_write(fx, &node.as_call_and_write_node().unwrap()),
            Node::IndexOperatorWriteNode { .. } => self.index_op_write(fx, &node.as_index_operator_write_node().unwrap()),
            Node::IndexOrWriteNode { .. } => self.index_or_write(fx, &node.as_index_or_write_node().unwrap()),
            Node::IndexAndWriteNode { .. } => self.index_and_write(fx, &node.as_index_and_write_node().unwrap()),
            Node::SuperNode { .. } => self.super_node(fx, &node.as_super_node().unwrap()),
            Node::ForwardingSuperNode { .. } => self.zsuper_node(fx, &node.as_forwarding_super_node().unwrap()),
            Node::YieldNode { .. } => self.yield_node(fx, &node.as_yield_node().unwrap()),
            Node::IfNode { .. } => self.if_node(fx, &node.as_if_node().unwrap()),
            Node::UnlessNode { .. } => self.unless_node(fx, &node.as_unless_node().unwrap()),
            Node::ElseNode { .. } => self.stmts(fx, node.as_else_node().unwrap().statements()),
            Node::AndNode { .. } => {
                let a = node.as_and_node().unwrap();
                let l = self.expr(fx, &a.left());
                let right = a.right();
                self.and_assign(fx, l, |c, fx| c.expr(fx, &right))
            }
            Node::OrNode { .. } => {
                let o = node.as_or_node().unwrap();
                let l = self.expr(fx, &o.left());
                let right = o.right();
                self.or_assign(fx, l, |c, fx| c.expr(fx, &right))
            }
            Node::WhileNode { .. } => {
                let w = node.as_while_node().unwrap();
                self.while_loop(fx, Some(node), &w.predicate(), w.statements(), false, w.is_begin_modifier())
            }
            Node::UntilNode { .. } => {
                let w = node.as_until_node().unwrap();
                self.while_loop(fx, Some(node), &w.predicate(), w.statements(), true, w.is_begin_modifier())
            }
            Node::ForNode { .. } => self.for_loop(fx, &node.as_for_node().unwrap()),
            Node::CaseNode { .. } => self.case_node(fx, &node.as_case_node().unwrap()),
            Node::BeginNode { .. } => self.begin_node(fx, &node.as_begin_node().unwrap()),
            Node::RescueModifierNode { .. } => self.rescue_modifier(fx, &node.as_rescue_modifier_node().unwrap()),
            Node::ReturnNode { .. } => self.return_node(fx, node.as_return_node().unwrap().arguments()),
            Node::BreakNode { .. } => self.break_node(fx, node.as_break_node().unwrap().arguments(), at),
            Node::NextNode { .. } => self.next_node(fx, node.as_next_node().unwrap().arguments(), at),
            Node::RetryNode { .. } => {
                match fx.retry.last().copied() {
                    Some(t) => {
                        fx.b.ins().jump(t, &[]);
                        self.dead_block(fx);
                    }
                    None => {
                        self.unsupported(fx, at, "retry outside of rescue");
                    }
                }
                self.nil(fx)
            }
            Node::RedoNode { .. } => {
                let target = fx.loops.last().map(|l| l.redo).or(fx.redo_block);
                match target {
                    Some(t) => {
                        fx.b.ins().jump(t, &[]);
                        self.dead_block(fx);
                    }
                    None => {
                        self.unsupported(fx, at, "redo outside of a loop or block");
                    }
                }
                self.nil(fx)
            }
            Node::DefinedNode { .. } => self.defined_node(fx, &node.as_defined_node().unwrap()),
            Node::ImplicitNode { .. } => self.expr(fx, &node.as_implicit_node().unwrap().value()),
            Node::EmbeddedStatementsNode { .. } => self.stmts(fx, node.as_embedded_statements_node().unwrap().statements()),
            Node::SplatNode { .. } => {
                let s = node.as_splat_node().unwrap();
                match s.expression() {
                    Some(e) => {
                        let v = self.expr(fx, &e);
                        self.callc(fx, "kn_splat_to_ary", &[v])
                    }
                    None => self.nil(fx),
                }
            }
            Node::FlipFlopNode { .. } => {
                self.unsupported(fx, at, "flip-flops are not supported by Kenny (see docs/LANGUAGE.md)");
                self.nil(fx)
            }
            Node::PreExecutionNode { .. } | Node::PostExecutionNode { .. } => {
                self.unsupported(fx, at, "BEGIN/END blocks are not supported by Kenny (see docs/LANGUAGE.md)");
                self.nil(fx)
            }
            Node::CaseMatchNode { .. } => self.case_match(fx, &node.as_case_match_node().unwrap()),
            Node::MatchPredicateNode { .. } => {
                let m = node.as_match_predicate_node().unwrap();
                self.match_predicate(fx, &m.value(), &m.pattern(), false)
            }
            Node::MatchRequiredNode { .. } => {
                let m = node.as_match_required_node().unwrap();
                self.match_predicate(fx, &m.value(), &m.pattern(), true)
            }
            Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. } => {
                self.unsupported(fx, at, "this compound constant/global assignment is not supported yet");
                self.nil(fx)
            }
            // `#$var` / `#{...}` parts of an interpolated literal (e.g. a regexp's)
            Node::EmbeddedVariableNode { .. } => {
                let ev = node.as_embedded_variable_node().unwrap();
                self.expr(fx, &ev.variable())
            }
            Node::ShareableConstantNode { .. } => {
                let s = node.as_shareable_constant_node().unwrap();
                self.expr(fx, &s.write())
            }
            _ => {
                let snippet = String::from_utf8_lossy(node.location().as_slice());
                let snippet: String = snippet.chars().take(40).collect();
                self.unsupported(fx, at, &format!("unsupported syntax: `{snippet}`"));
                self.nil(fx)
            }
        }
    }

    // ---------- helpers ----------

    pub(crate) fn or_assign<F>(&mut self, fx: &mut Fx, cur: Value, f: F) -> Value
    where
        F: FnOnce(&mut Compiler<M>, &mut Fx) -> Value,
    {
        let t = self.truthy_br(fx, cur);
        let no = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(t, done, &[cur.into()], no, &[]);
        fx.b.switch_to_block(no);
        let v = f(self, fx);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
    pub(crate) fn and_assign<F>(&mut self, fx: &mut Fx, cur: Value, f: F) -> Value
    where
        F: FnOnce(&mut Compiler<M>, &mut Fx) -> Value,
    {
        let t = self.truthy_br(fx, cur);
        let yes = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(t, yes, &[], done, &[cur.into()]);
        fx.b.switch_to_block(yes);
        let v = f(self, fx);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    pub(crate) fn ivar_get(&mut self, fx: &mut Fx, name: &[u8]) -> Value {
        if let Some(vs) = &fx.vself {
            // (a virtual object's `initialize`, vobj.rs)
            return match vs.get(name) {
                Some(&v) => fx.b.use_var(v),
                None => self.nil(fx),
            };
        }
        // remembered across a loop's iterations (licm.rs)
        if !fx.licm.is_empty() {
            let nm = name.to_vec();
            if let Some(v) = self.licm_ivar(fx, name, |c, fx| c.ivar_get(fx, &nm)) {
                return v;
            }
        }
        let s = self.self_val(fx);
        let id = self.sym_val(fx, name);
        let cache = self.cache_ptr(fx, 2);
        // `self` is a class or module: in `def self.x` (and its blocks), a class body
        if self.inline_ivars(fx) && (fx.kind == ScopeKind::Class || fx.method.as_ref().is_some_and(|m| m.singleton)) {
            return self.civar_fast(fx, cache, s, id, |c, fx| c.call(fx, "kn_ivar_get", &[s, id, cache]));
        }
        if self.inline_ivars(fx) {
            return self.ivar_fast(fx, cache, s, None, |c, fx| c.call(fx, "kn_ivar_get", &[s, id, cache]));
        }
        self.call(fx, "kn_ivar_get", &[s, id, cache])
    }
    pub(crate) fn ivar_set(&mut self, fx: &mut Fx, name: &[u8], v: Value) -> Value {
        if let Some(&var) = fx.vself.as_ref().and_then(|vs| vs.get(name)) {
            fx.b.def_var(var, v);
            return v;
        }
        let s = self.self_val(fx);
        let id = self.sym_val(fx, name);
        let cache = self.cache_ptr(fx, 2);
        if self.inline_ivars(fx) {
            return self.ivar_fast(fx, cache, s, Some(v), |c, fx| c.callc(fx, "kn_ivar_set", &[s, id, v, cache]));
        }
        self.callc(fx, "kn_ivar_set", &[s, id, v, cache])
    }
    /// Instance variables are read/written inline in loops and in the app's own code; in the
    /// gems' other code the call is most of the cost and inlining every access would grow a
    /// Rails binary's code by ~9%.
    fn inline_ivars(&self, fx: &Fx) -> bool {
        !fx.loops.is_empty() || !self.in_dependency()
    }

    /// A String with interpolation: built in a buffer sized for the literal parts plus 16
    /// bytes per embedded value, which only this code sees until it is done (kenny-runtime
    /// abi.rs `kn_str_interp`).
    fn interpolate(&mut self, fx: &mut Fx, parts: &NodeList) -> Value {
        let cap: usize = parts.iter().map(|p| p.as_string_node().map_or(16, |sn| sn.unescaped().len())).sum();
        let cap = self.iconst(fx, cap.min(1 << 16) as i64);
        let s = self.call(fx, "kn_str_buf", &[cap]);
        for part in parts.iter() {
            if let Some(sn) = part.as_string_node() {
                let (p, l) = self.str_ptr(fx, &sn.unescaped().to_vec());
                self.call(fx, "kn_str_interp_raw", &[s, p, l]);
            } else if let Some(es) = part.as_embedded_statements_node() {
                let v = self.stmts(fx, es.statements());
                self.callc(fx, "kn_str_interp", &[s, v]);
            } else if let Some(ev) = part.as_embedded_variable_node() {
                let v = self.expr(fx, &ev.variable());
                self.callc(fx, "kn_str_interp", &[s, v]);
            } else {
                let v = self.expr(fx, &part);
                self.callc(fx, "kn_str_interp", &[s, v]);
            }
        }
        s
    }

    pub(crate) fn array_literal(&mut self, fx: &mut Fx, elems: &NodeList) -> Value {
        let items: Vec<Node> = elems.iter().collect();
        if items.iter().any(|n| n.as_splat_node().is_some()) {
            let zero = self.iconst(fx, 0);
            let a = self.call(fx, "kn_ary_new", &[zero, zero]);
            for n in &items {
                if let Some(s) = n.as_splat_node() {
                    let v = match s.expression() {
                        Some(e) => self.expr(fx, &e),
                        None => self.nil(fx),
                    };
                    self.callc(fx, "kn_ary_concat_splat", &[a, v]);
                } else {
                    let v = self.expr(fx, n);
                    self.call(fx, "kn_ary_push", &[a, v]);
                }
            }
            return a;
        }
        let vals: Vec<Value> = items.iter().map(|n| self.expr(fx, n)).collect();
        let n = self.iconst(fx, vals.len() as i64);
        let buf = self.args_buf(fx, &vals);
        self.call(fx, "kn_ary_new", &[n, buf])
    }

    pub(crate) fn hash_literal(&mut self, fx: &mut Fx, elems: &NodeList, kw: bool) -> Value {
        let mut vals = Vec::new();
        let mut splats = Vec::new();
        for e in elems.iter() {
            if let Some(a) = e.as_assoc_node() {
                let k = self.expr(fx, &a.key());
                let v = self.expr(fx, &a.value());
                if !splats.is_empty() {
                    splats.push((Some((k, v)), None));
                } else {
                    vals.push(k);
                    vals.push(v);
                }
            } else if let Some(s) = e.as_assoc_splat_node() {
                let v = match s.value() {
                    Some(x) => self.expr(fx, &x),
                    None => self.forwarded_local(fx, b"**"),
                };
                splats.push((None, Some(v)));
            }
        }
        // keys that are distinct Symbol literals need no copies or duplicate checks
        let mut syms: Vec<Vec<u8>> = Vec::new();
        let distinct_syms = splats.is_empty()
            && elems.iter().all(|e| {
                let Some(s) = e.as_assoc_node().and_then(|a| a.key().as_symbol_node()) else { return false };
                let b = s.unescaped().to_vec();
                let fresh = !syms.contains(&b);
                syms.push(b);
                fresh
            });
        let n = self.iconst(fx, vals.len() as i64);
        let buf = self.args_buf(fx, &vals);
        let k = self.iconst(fx, kw as i64);
        let h = self.call(fx, if distinct_syms && !vals.is_empty() { "kn_hash_new_syms" } else { "kn_hash_new" }, &[n, buf, k]);
        for (pair, splat) in splats {
            if let Some((k, v)) = pair {
                let two = self.iconst(fx, 2);
                let buf = self.args_buf(fx, &[k, v]);
                let zero = self.iconst(fx, 0);
                let one = self.call(fx, "kn_hash_new", &[two, buf, zero]);
                self.callc(fx, "kn_hash_merge_splat", &[h, one]);
            }
            if let Some(s) = splat {
                self.callc(fx, "kn_hash_merge_splat", &[h, s]);
            }
        }
        h
    }

    /// Assign `v` to an assignment target node (masgn / for / rescue => e / block params).
    pub(crate) fn assign_target(&mut self, fx: &mut Fx, target: &Node, v: Value) {
        let at = target.location().start_offset();
        if let Some(t) = target.as_local_variable_target_node() {
            self.local_set(fx, t.name().as_slice(), t.depth(), v);
        } else if let Some(t) = target.as_required_parameter_node() {
            self.local_set(fx, t.name().as_slice(), 0, v);
        } else if let Some(t) = target.as_instance_variable_target_node() {
            self.ivar_set(fx, t.name().as_slice(), v);
        } else if let Some(t) = target.as_global_variable_target_node() {
            let s = self.sym_val(fx, t.name().as_slice());
            self.call(fx, "kn_gvar_set", &[s, v]);
        } else if let Some(t) = target.as_class_variable_target_node() {
            let cref = self.cref(fx);
            let s = self.sym_val(fx, t.name().as_slice());
            self.call(fx, "kn_cvar_set", &[cref, s, v]);
        } else if let Some(t) = target.as_constant_target_node() {
            let cref = self.cref(fx);
            let s = self.sym_val(fx, t.name().as_slice());
            self.callc(fx, "kn_const_set", &[cref, s, v]);
        } else if let Some(t) = target.as_index_target_node() {
            let recv = self.expr(fx, &t.receiver());
            let mut args: Vec<Value> = match t.arguments() {
                Some(a) => a.arguments().iter().map(|n| self.expr(fx, &n)).collect(),
                None => vec![],
            };
            args.push(v);
            self.send(fx, recv, b"[]=", &args, None);
        } else if let Some(t) = target.as_call_target_node() {
            let recv = self.expr(fx, &t.receiver());
            let name = t.name().as_slice().to_vec();
            self.send(fx, recv, &name, &[v], None);
        } else if let Some(m) = target.as_multi_target_node() {
            let ary = self.callc(fx, "kn_masgn_ary", &[v]);
            self.destructure(fx, &m.lefts(), m.rest(), &m.rights(), ary);
        } else if let Some(s) = target.as_splat_node() {
            if let Some(e) = s.expression() {
                self.assign_target(fx, &e, v);
            }
        } else {
            self.unsupported(fx, at, "unsupported assignment target");
        }
    }

    pub(crate) fn destructure(&mut self, fx: &mut Fx, lefts: &NodeList, rest: Option<Node>, rights: &NodeList, ary: Value) {
        let lefts: Vec<Node> = lefts.iter().collect();
        let rights: Vec<Node> = rights.iter().collect();
        for (i, t) in lefts.iter().enumerate() {
            let iv = self.iconst(fx, i as i64);
            let v = self.call(fx, "kn_ary_ref", &[ary, iv]);
            self.assign_target(fx, t, v);
        }
        if let Some(r) = rest {
            if let Some(s) = r.as_splat_node() {
                if let Some(e) = s.expression() {
                    let st = self.iconst(fx, lefts.len() as i64);
                    let po = self.iconst(fx, rights.len() as i64);
                    let v = self.call(fx, "kn_ary_slice_rest", &[ary, st, po]);
                    self.assign_target(fx, &e, v);
                }
            }
        }
        for (k, t) in rights.iter().enumerate() {
            let pre = self.iconst(fx, lefts.len() as i64);
            let po = self.iconst(fx, rights.len() as i64);
            let kk = self.iconst(fx, k as i64);
            let v = self.call(fx, "kn_ary_post_ref", &[ary, pre, po, kk]);
            self.assign_target(fx, t, v);
        }
    }

    fn defined_node(&mut self, fx: &mut Fx, d: &DefinedNode) -> Value {
        let v = d.value();
        let lit = |c: &mut Compiler<M>, fx: &mut Fx, s: &str| c.str_lit(fx, s.as_bytes());
        let cond_str = |c: &mut Compiler<M>, fx: &mut Fx, cond: Value, s: &str| {
            let yes = lit(c, fx, s);
            let nil = c.nil(fx);
            let z = fx.b.ins().icmp_imm_s(IntCC::NotEqual, cond, 0);
            fx.b.ins().select(z, yes, nil)
        };
        if v.as_local_variable_read_node().is_some() {
            return lit(self, fx, "local-variable");
        }
        if let Some(iv) = v.as_instance_variable_read_node() {
            let s = self.self_val(fx);
            let id = self.sym_val(fx, iv.name().as_slice());
            let r = self.call(fx, "kn_ivar_defined", &[s, id]);
            return cond_str(self, fx, r, "instance-variable");
        }
        if let Some(gv) = v.as_global_variable_read_node() {
            let id = self.sym_val(fx, gv.name().as_slice());
            let r = self.call(fx, "kn_gvar_defined", &[id]);
            return cond_str(self, fx, r, "global-variable");
        }
        if let Some(cv) = v.as_class_variable_read_node() {
            let cref = self.cref(fx);
            let id = self.sym_val(fx, cv.name().as_slice());
            let r = self.call(fx, "kn_cvar_defined", &[cref, id]);
            return cond_str(self, fx, r, "class variable");
        }
        if let Some(cv) = v.as_constant_read_node() {
            let crefp = self.slot_ptr(fx, fx.cref_slot);
            let id = self.sym_val(fx, cv.name().as_slice());
            let r = self.call(fx, "kn_const_defined_l", &[crefp, id]);
            return cond_str(self, fx, r, "constant");
        }
        if let Some(call) = v.as_call_node() {
            // evaluate the receiver; any exception means "not defined"
            let recv = match call.receiver() {
                Some(r) => {
                    let handler = fx.b.create_block();
                    let done = fx.b.create_block();
                    fx.b.append_block_param(done, I64);
                    fx.b.append_block_param(done, I64);
                    fx.unwind.push(handler);
                    let rv = self.expr(fx, &r);
                    fx.unwind.pop();
                    let one = self.iconst(fx, 1);
                    fx.b.ins().jump(done, &[rv.into(), one.into()]);
                    fx.b.switch_to_block(handler);
                    self.call(fx, "kn_clear_pending", &[]);
                    let nil = self.nil(fx);
                    let zero = self.iconst(fx, 0);
                    fx.b.ins().jump(done, &[nil.into(), zero.into()]);
                    fx.b.switch_to_block(done);
                    let ok = fx.b.block_params(done)[1];
                    let rv = fx.b.block_params(done)[0];
                    let id = self.sym_val(fx, call.name().as_slice());
                    let r = self.call(fx, "kn_respond_to", &[rv, id]);
                    let r = fx.b.ins().band(r, ok);
                    return cond_str(self, fx, r, "method");
                }
                None => self.self_val(fx),
            };
            let id = self.sym_val(fx, call.name().as_slice());
            let r = self.call(fx, "kn_respond_to", &[recv, id]);
            return cond_str(self, fx, r, "method");
        }
        if let Some(cp) = v.as_constant_path_node() {
            let parent_ok = match cp.parent() {
                Some(p) => {
                    let handler = fx.b.create_block();
                    let done = fx.b.create_block();
                    fx.b.append_block_param(done, I64);
                    fx.b.append_block_param(done, I64);
                    fx.unwind.push(handler);
                    let pv = self.expr(fx, &p);
                    fx.unwind.pop();
                    let one = self.iconst(fx, 1);
                    fx.b.ins().jump(done, &[pv.into(), one.into()]);
                    fx.b.switch_to_block(handler);
                    self.call(fx, "kn_clear_pending", &[]);
                    let nil = self.nil(fx);
                    let zero = self.iconst(fx, 0);
                    fx.b.ins().jump(done, &[nil.into(), zero.into()]);
                    fx.b.switch_to_block(done);
                    (fx.b.block_params(done)[0], fx.b.block_params(done)[1])
                }
                None => {
                    let o = self.call(fx, "kn_object_class", &[]);
                    let one = self.iconst(fx, 1);
                    (o, one)
                }
            };
            let id = self.sym_val(fx, cp.name().map(|n| n.as_slice().to_vec()).unwrap_or_default().as_slice());
            let r = self.call(fx, "kn_const_defined_under", &[parent_ok.0, id]);
            let r = fx.b.ins().band(r, parent_ok.1);
            return cond_str(self, fx, r, "constant");
        }
        if v.as_yield_node().is_some() {
            let b = self.method_block(fx);
            let r = fx.b.ins().icmp_imm_s(IntCC::NotEqual, b, QNIL);
            let r = fx.b.ins().uextend(I64, r);
            return cond_str(self, fx, r, "yield");
        }
        if v.as_self_node().is_some() {
            return lit(self, fx, "self");
        }
        if v.as_nil_node().is_some() || v.as_true_node().is_some() || v.as_false_node().is_some() {
            return lit(self, fx, "expression");
        }
        if v.as_super_node().is_some() || v.as_forwarding_super_node().is_some() {
            // is there a method for `super` to call (e.g. in a module extended onto an object)
            if let Some(m) = fx.method.clone() {
                let slot = self.slot_ptr(fx, m.owner_slot);
                let s = self.self_val(fx);
                let mid = self.sym_val(fx, &m.name);
                let r = self.call(fx, "kn_super_defined", &[slot, s, mid]);
                return cond_str(self, fx, r, "super");
            }
            return lit(self, fx, "super");
        }
        if v.as_local_variable_write_node().is_some()
            || v.as_instance_variable_write_node().is_some()
            || v.as_global_variable_write_node().is_some()
            || v.as_class_variable_write_node().is_some()
            || v.as_constant_write_node().is_some()
            || v.as_multi_write_node().is_some()
            || v.as_local_variable_operator_write_node().is_some()
            || v.as_local_variable_or_write_node().is_some()
            || v.as_instance_variable_or_write_node().is_some()
        {
            return lit(self, fx, "assignment");
        }
        lit(self, fx, "expression")
    }
}
