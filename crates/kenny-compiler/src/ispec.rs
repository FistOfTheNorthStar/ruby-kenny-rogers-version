//! Integer-specialised copies of small recursive methods (`fib`, `tak`, `ackermann`...).
//!
//! A method qualifies when it calls itself (statics.rs `self_call`), takes only required
//! parameters and its body is integer arithmetic on them: literals, the parameters, `+ - *
//! / %`, comparisons in conditions (`if`/`unless`/ternary/`&&`/`||`/`!`), `return`, and calls
//! to itself. Besides the normal function it then gets a copy that assumes every value is a
//! Fixnum: no type checks, no slow paths, its self-calls straight to the copy.
//!
//! Anything the copy can't handle (a result beyond the Fixnums, a division by zero, a deep
//! stack, the method no longer the only one of its name) *bails*: it returns 0 and every
//! copy frame returns 0 too. The copy has no side effects, so the normal method then simply
//! runs the call from the start (and never tries the copy again: one flag per method).
//! A pending collection or thread switch is handled where the copy is (it holds no heap
//! values); an exception delivered there (Thread#raise / kill) returns UNDEF, as any call.

use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, AbiParam, Block, InstBuilder};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use ruby_prism::*;
use std::collections::HashMap;

/// Largest literal handled (a Fixnum with room for `2k` arithmetic on it).
const LIT_MAX: i64 = 1 << 40;

struct Elig<'a> {
    name: &'a [u8],
    params: &'a [Vec<u8>],
    ok: bool,
}

impl Elig<'_> {
    fn lit(n: &Node) -> Option<i64> {
        let i = n.as_integer_node()?;
        let v = crate::expr::int_value(&i)?;
        (v.abs() <= LIT_MAX).then_some(v)
    }
    /// An integer-valued expression.
    fn int(&mut self, n: &Node) {
        if !self.ok {
            return;
        }
        if Self::lit(n).is_some() {
            return;
        }
        if let Some(l) = n.as_local_variable_read_node() {
            if l.depth() != 0 || !self.params.iter().any(|p| p.as_slice() == l.name().as_slice()) {
                self.ok = false;
            }
            return;
        }
        if let Some(p) = n.as_parentheses_node() {
            match p.body() {
                Some(b) => self.stmts_value(&b),
                None => self.ok = false,
            }
            return;
        }
        if let Some(i) = n.as_if_node() {
            self.cond(&i.predicate());
            match (i.statements(), i.subsequent()) {
                (Some(s), Some(e)) => {
                    self.stmts_value(&s.as_node());
                    self.else_value(&e);
                }
                _ => self.ok = false,
            }
            return;
        }
        if let Some(u) = n.as_unless_node() {
            self.cond(&u.predicate());
            match (u.statements(), u.else_clause()) {
                (Some(s), Some(e)) => {
                    self.stmts_value(&s.as_node());
                    match e.statements() {
                        Some(es) => self.stmts_value(&es.as_node()),
                        None => self.ok = false,
                    }
                }
                _ => self.ok = false,
            }
            return;
        }
        if let Some(c) = n.as_call_node() {
            if c.block().is_some() || c.is_safe_navigation() {
                self.ok = false;
                return;
            }
            let args: Vec<Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            let name = c.name().as_slice();
            match c.receiver() {
                Some(r) if r.as_self_node().is_none() => {
                    if matches!(name, b"+" | b"-" | b"*" | b"/" | b"%") && args.len() == 1 {
                        self.int(&r);
                        self.int(&args[0]);
                        return;
                    }
                    self.ok = false;
                }
                _ => {
                    if name == self.name && args.len() == self.params.len() {
                        for a in &args {
                            self.int(a);
                        }
                        return;
                    }
                    self.ok = false;
                }
            }
            return;
        }
        if let Some(r) = n.as_return_node() {
            let args: Vec<Node> = r.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            match args.as_slice() {
                [a] => self.int(a),
                _ => self.ok = false,
            }
            return;
        }
        self.ok = false;
    }
    fn else_value(&mut self, e: &Node) {
        if let Some(el) = e.as_else_node() {
            match el.statements() {
                Some(s) => self.stmts_value(&s.as_node()),
                None => self.ok = false,
            }
        } else {
            // elsif
            self.int(e);
        }
    }
    /// Statements whose last one gives the value (the others: integer expressions too, for
    /// their effects, which are only returns and self-calls).
    fn stmts_value(&mut self, n: &Node) {
        match n.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                if v.is_empty() {
                    self.ok = false;
                }
                for x in &v {
                    self.stmt(x);
                }
            }
            None => self.int(n),
        }
    }
    /// A statement: an integer expression, or an `if`/`unless` (maybe without else) of them.
    fn stmt(&mut self, n: &Node) {
        if let Some(i) = n.as_if_node() {
            self.cond(&i.predicate());
            if let Some(s) = i.statements() {
                for x in s.body().iter() {
                    self.stmt(&x);
                }
            }
            if let Some(e) = i.subsequent() {
                match e.as_else_node() {
                    Some(el) => {
                        if let Some(s) = el.statements() {
                            for x in s.body().iter() {
                                self.stmt(&x);
                            }
                        }
                    }
                    None => self.stmt(&e),
                }
            }
            return;
        }
        if let Some(u) = n.as_unless_node() {
            self.cond(&u.predicate());
            if let Some(s) = u.statements() {
                for x in s.body().iter() {
                    self.stmt(&x);
                }
            }
            if let Some(s) = u.else_clause().and_then(|e| e.statements()) {
                for x in s.body().iter() {
                    self.stmt(&x);
                }
            }
            return;
        }
        self.int(n);
    }
    fn cond(&mut self, n: &Node) {
        if !self.ok {
            return;
        }
        if let Some(p) = n.as_parentheses_node() {
            let single = p.body().and_then(|b| match b.as_statements_node() {
                Some(s) => {
                    let v: Vec<Node> = s.body().iter().collect();
                    if v.len() == 1 { v.into_iter().next() } else { None }
                }
                None => Some(b),
            });
            match single {
                Some(x) => self.cond(&x),
                None => self.ok = false,
            }
            return;
        }
        if let Some(a) = n.as_and_node() {
            self.cond(&a.left());
            self.cond(&a.right());
            return;
        }
        if let Some(o) = n.as_or_node() {
            self.cond(&o.left());
            self.cond(&o.right());
            return;
        }
        if let Some(c) = n.as_call_node() {
            let name = c.name().as_slice();
            let args: Vec<Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            if c.block().is_some() || c.is_safe_navigation() {
                self.ok = false;
                return;
            }
            if let Some(r) = c.receiver() {
                if name == b"!" && args.is_empty() {
                    return self.cond(&r);
                }
                if matches!(name, b"<" | b"<=" | b">" | b">=" | b"==" | b"!=") && args.len() == 1 {
                    self.int(&r);
                    self.int(&args[0]);
                    return;
                }
            }
        }
        self.ok = false;
    }
}

/// Parameter names of a `def` that qualifies (see the module docs).
pub(crate) fn eligible(name: &[u8], params: &Option<ParametersNode>, body: &Node) -> Option<Vec<Vec<u8>>> {
    if std::env::var_os("KENNY_NO_ISPEC").is_some() {
        return None;
    }
    let mut ps = Vec::new();
    if let Some(p) = params {
        if p.block().is_some() {
            return None;
        }
        for r in p.requireds().iter() {
            ps.push(r.as_required_parameter_node()?.name().as_slice().to_vec());
        }
    }
    let mut e = Elig { name, params: &ps, ok: true };
    e.stmts_value(body);
    e.ok.then_some(ps)
}

/// While compiling the copy.
struct Sx<'f> {
    b: FunctionBuilder<'f>,
    vars: HashMap<Vec<u8>, Value>,
    bail: Block,
    me: ir::FuncRef,
}

impl<M: Module> Compiler<M> {
    /// Compile the copy of method `fid` (named `name`, parameters `params`): (its function,
    /// its off flag).
    pub(crate) fn ispec_build(&mut self, fid: FuncId, name: &[u8], params: &[Vec<u8>], body: &Node) -> (FuncId, DataId) {
        let n = params.len();
        let mut sig = self.module.make_signature();
        for _ in 0..n {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        let label: String = String::from_utf8_lossy(name).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let id = self.module.declare_function(&format!("__kn_ispec_{}_{}", fid.as_u32(), label), Linkage::Local, &sig).expect("ispec fn");
        let off = self.module.declare_data(&format!("__kn_ispec_off_{}", fid.as_u32()), Linkage::Local, true, false).expect("ispec flag");
        let mut d = DataDescription::new();
        d.define(Box::new(0u64.to_le_bytes()));
        d.set_align(8);
        self.module.define_data(off, &d).expect("ispec flag");
        let claim_id = self.claim_word_id(fid);
        let poll_id = self.rt_id("kn_ispec_poll");

        let mut func = ir::Function::with_name_signature(ir::UserFuncName::user(0, id.as_u32()), sig);
        let mut fctx = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry).to_vec();
            let f = mem();
            // room on the stack (`KN_DIRECT.entry_limit`), no collection or switch pending
            let dg = self.module.declare_data_in_func(self.direct_id, b.func);
            let dir = b.ins().symbol_value(I64, dg);
            let lim = b.ins().load(I64, f, dir, KN_DIRECT_ENTRY_LIMIT);
            let gg = self.module.declare_data_in_func(self.gc_flag_id, b.func);
            let gcp = b.ins().symbol_value(I64, gg);
            let gc = b.ins().uload8(I64, f, gcp, 0);
            let sp = b.ins().get_stack_pointer(I64);
            let low = b.ins().icmp(IntCC::UnsignedLessThan, sp, lim);
            let low = b.ins().uextend(I64, low);
            let bad = b.ins().bor(low, gc);
            let poll = b.create_block();
            let go = b.create_block();
            let bail = b.create_block();
            b.set_cold_block(poll);
            b.set_cold_block(bail);
            b.ins().brif(bad, poll, &[], go, &[]);
            b.switch_to_block(poll);
            let pr = self.module.declare_func_in_func(poll_id, b.func);
            let inst = b.ins().call(pr, &[]);
            let r = b.inst_results(inst)[0];
            // 0: go on; 1: bail; else UNDEF (an exception)
            let ok = b.create_block();
            let not0 = b.create_block();
            b.set_cold_block(not0);
            b.ins().brif(r, not0, &[], ok, &[]);
            b.switch_to_block(not0);
            let one = b.ins().icmp_imm_s(IntCC::Equal, r, 1);
            let exc = b.create_block();
            b.set_cold_block(exc);
            b.ins().brif(one, bail, &[], exc, &[]);
            b.switch_to_block(exc);
            let u = b.ins().iconst(I64, UNDEF);
            b.ins().return_(&[u]);
            // (Ruby code may have run there: still the only method of its name?)
            b.switch_to_block(ok);
            let cg = self.module.declare_data_in_func(claim_id, b.func);
            let claim = b.ins().symbol_value(I64, cg);
            let w = b.ins().load(I64, f, claim, 0);
            b.ins().brif(w, bail, &[], go, &[]);
            b.switch_to_block(go);
            let me = self.module.declare_func_in_func(id, b.func);
            let mut vars = HashMap::new();
            for (i, nm) in params.iter().enumerate() {
                vars.insert(nm.clone(), p[i]);
            }
            let mut sx = Sx { b, vars, bail, me };
            let v = sx.stmts_value(body, name);
            if let Some(v) = v {
                sx.b.ins().return_(&[v]);
            }
            sx.b.switch_to_block(bail);
            let z = sx.b.ins().iconst(I64, 0);
            sx.b.ins().return_(&[z]);
            sx.b.seal_all_blocks();
            let cfg = self.module.isa().frontend_config();
            sx.b.finalize(cfg);
        }
        let mut ctx = cranelift_codegen::Context::for_function(func);
        if let Err(e) = self.module.define_function(id, &mut ctx) {
            self.errors.push(format!("internal codegen error in integer copy of {}: {e:?}", String::from_utf8_lossy(name)));
        }
        (id, off)
    }

    /// In the normal method `fid` (its first statements, `args` its parameters): when the
    /// copy is on, its word says the method is still the only one of its name and the
    /// arguments are Fixnums, return the copy's result; on a bail turn the copy off and go
    /// on with the normal body.
    pub(crate) fn ispec_enter(&mut self, fx: &mut Fx, fid: FuncId, copy: (FuncId, DataId), args: &[Value]) {
        let f = mem();
        let og = self.module.declare_data_in_func(copy.1, fx.b.func);
        let offp = fx.b.ins().symbol_value(I64, og);
        let off = fx.b.ins().load(I64, f, offp, 0);
        let cid = self.claim_word_id(fid);
        let cg = self.module.declare_data_in_func(cid, fx.b.func);
        let cp = fx.b.ins().symbol_value(I64, cg);
        let cw = fx.b.ins().load(I64, f, cp, 0);
        let mut bad = fx.b.ins().bor(off, cw);
        // (several OS threads running Ruby code: no stack check in the copy)
        let dg = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, dg);
        let lim = fx.b.ins().load(I64, f, dir, KN_DIRECT_ENTRY_LIMIT);
        let nolim = fx.b.ins().icmp_imm_s(IntCC::Equal, lim, 0);
        let nolim = fx.b.ins().uextend(I64, nolim);
        bad = fx.b.ins().bor(bad, nolim);
        if !args.is_empty() {
            let mut all = args[0];
            for a in &args[1..] {
                all = fx.b.ins().band(all, *a);
            }
            let fix = fx.b.ins().band_imm_s(all, 1);
            let notfix = fx.b.ins().bxor_imm_s(fix, 1);
            bad = fx.b.ins().bor(bad, notfix);
        }
        let try_b = fx.b.create_block();
        let normal = fx.b.create_block();
        fx.b.ins().brif(bad, normal, &[], try_b, &[]);
        fx.b.switch_to_block(try_b);
        let fr = self.module.declare_func_in_func(copy.0, fx.b.func);
        let inst = fx.b.ins().call(fr, args);
        let r = fx.b.inst_results(inst)[0];
        let ret = fx.b.create_block();
        let bailed = fx.b.create_block();
        fx.b.set_cold_block(bailed);
        fx.b.ins().brif(r, ret, &[], bailed, &[]);
        fx.b.switch_to_block(ret);
        fx.b.ins().return_(&[r]);
        fx.b.switch_to_block(bailed);
        let one = fx.b.ins().iconst(I64, 1);
        fx.b.ins().store(f, one, offp, 0);
        fx.b.ins().jump(normal, &[]);
        fx.b.switch_to_block(normal);
    }
}

impl Sx<'_> {
    fn bail_if(&mut self, c: Value) {
        let cont = self.b.create_block();
        self.b.ins().brif(c, self.bail, &[], cont, &[]);
        self.b.switch_to_block(cont);
    }
    /// A tagged Fixnum literal.
    fn lit(&mut self, k: i64) -> Value {
        self.b.ins().iconst(I64, (k << 1) | 1)
    }
    /// Statements: the last one's value (None: the block ended in a `return`).
    fn stmts_value(&mut self, n: &Node, name: &[u8]) -> Option<Value> {
        match n.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                let mut last = None;
                for (i, x) in v.iter().enumerate() {
                    if i + 1 < v.len() {
                        if !self.stmt(x, name) {
                            return None;
                        }
                    } else {
                        last = self.int(x, name);
                    }
                }
                last
            }
            None => self.int(n, name),
        }
    }
    /// A statement; false when it ended the block (a `return`).
    fn stmt(&mut self, n: &Node, name: &[u8]) -> bool {
        if n.as_if_node().is_some() || n.as_unless_node().is_some() {
            let (pred, then_s, else_s): (Node, Option<StatementsNode>, Option<Node>) = match n.as_if_node() {
                Some(i) => (i.predicate(), i.statements(), i.subsequent()),
                None => {
                    let u = n.as_unless_node().unwrap();
                    (u.predicate(), u.else_clause().and_then(|e| e.statements()), u.statements().map(|s| s.as_node()))
                }
            };
            let (tb, eb, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            self.cond(&pred, tb, eb, name);
            self.b.switch_to_block(tb);
            let mut open = true;
            if let Some(s) = then_s {
                for x in s.body().iter() {
                    if !self.stmt(&x, name) {
                        open = false;
                        break;
                    }
                }
            }
            if open {
                self.b.ins().jump(done, &[]);
            }
            self.b.switch_to_block(eb);
            let mut open = true;
            if let Some(e) = else_s {
                let stmts: Vec<Node> = match (e.as_else_node(), e.as_statements_node()) {
                    (Some(el), _) => el.statements().map(|s| s.body().iter().collect()).unwrap_or_default(),
                    (None, Some(s)) => s.body().iter().collect(),
                    _ => vec![e],
                };
                for x in &stmts {
                    if !self.stmt(x, name) {
                        open = false;
                        break;
                    }
                }
            }
            if open {
                self.b.ins().jump(done, &[]);
            }
            self.b.switch_to_block(done);
            return true;
        }
        self.int(n, name).is_some()
    }
    /// An integer expression: its tagged value (None: it was a `return`).
    fn int(&mut self, n: &Node, name: &[u8]) -> Option<Value> {
        if let Some(k) = Elig::lit(n) {
            return Some(self.lit(k));
        }
        if let Some(l) = n.as_local_variable_read_node() {
            return Some(self.vars[l.name().as_slice()]);
        }
        if let Some(p) = n.as_parentheses_node() {
            return self.stmts_value(&p.body().unwrap(), name);
        }
        if let Some(r) = n.as_return_node() {
            let a = r.arguments().unwrap().arguments().iter().next().unwrap();
            let v = self.int(&a, name)?;
            self.b.ins().return_(&[v]);
            let dead = self.b.create_block();
            self.b.switch_to_block(dead);
            return None;
        }
        if n.as_if_node().is_some() || n.as_unless_node().is_some() {
            let (pred, then_s, else_n): (Node, Node, Node) = match n.as_if_node() {
                Some(i) => (i.predicate(), i.statements().unwrap().as_node(), i.subsequent().unwrap()),
                None => {
                    let u = n.as_unless_node().unwrap();
                    (u.predicate(), u.else_clause().unwrap().statements().unwrap().as_node(), u.statements().unwrap().as_node())
                }
            };
            let (tb, eb, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            self.b.append_block_param(done, I64);
            self.cond(&pred, tb, eb, name);
            self.b.switch_to_block(tb);
            if let Some(v) = self.stmts_value(&then_s, name) {
                self.b.ins().jump(done, &[v.into()]);
            }
            self.b.switch_to_block(eb);
            let ev = match else_n.as_else_node() {
                Some(el) => self.stmts_value(&el.statements().unwrap().as_node(), name),
                None => self.stmts_value(&else_n, name),
            };
            if let Some(v) = ev {
                self.b.ins().jump(done, &[v.into()]);
            }
            self.b.switch_to_block(done);
            return Some(self.b.block_params(done)[0]);
        }
        let c = n.as_call_node().unwrap();
        let args: Vec<Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
        match c.receiver() {
            Some(r) if r.as_self_node().is_none() => {
                let op = c.name().as_slice();
                let lit_b = Elig::lit(&args[0]);
                let a = self.int(&r, name)?;
                let bv = self.int(&args[0], name)?;
                Some(self.arith(op, a, bv, lit_b))
            }
            _ => {
                let mut vals = Vec::new();
                for a in &args {
                    vals.push(self.int(a, name)?);
                }
                // (still the only method of its name, statics.rs `self_call`: checked on
                // entry to the normal method and after a poll, the only places Ruby code runs)
                let inst = self.b.ins().call(self.me, &vals);
                let r = self.b.inst_results(inst)[0];
                // 0 (a bail) and UNDEF (an exception) are the even results: pass them up
                let odd = self.b.ins().band_imm_s(r, 1);
                let cont = self.b.create_block();
                let up = self.b.create_block();
                self.b.set_cold_block(up);
                self.b.ins().brif(odd, cont, &[], up, &[]);
                self.b.switch_to_block(up);
                self.b.ins().return_(&[r]);
                self.b.switch_to_block(cont);
                Some(r)
            }
        }
    }
    /// `a op b` on tagged Fixnums (`lit`: `b` is that literal): bails when the result isn't
    /// a Fixnum (or on a division by zero).
    fn arith(&mut self, op: &[u8], a: Value, b: Value, lit: Option<i64>) -> Value {
        match (op, lit) {
            (b"+" | b"-", Some(k)) => {
                // a ± 2k, overflowing exactly when it moves the wrong way
                let k2 = if op == b"+" { 2 * k } else { -2 * k };
                let r = self.b.ins().iadd_imm_s(a, k2);
                if k2 != 0 {
                    let cc = if k2 > 0 { IntCC::SignedLessThan } else { IntCC::SignedGreaterThan };
                    let of = self.b.ins().icmp(cc, r, a);
                    self.bail_if(of);
                }
                r
            }
            (b"+", None) => {
                let bm = self.b.ins().iadd_imm_s(b, -1);
                let (r, of) = self.b.ins().sadd_overflow(a, bm);
                self.bail_if(of);
                r
            }
            (b"-", None) => {
                let (d, of) = self.b.ins().ssub_overflow(a, b);
                self.bail_if(of);
                self.b.ins().iadd_imm_s(d, 1)
            }
            (b"*", _) => {
                let x = self.b.ins().sshr_imm_s(a, 1);
                let y = self.b.ins().iadd_imm_s(b, -1);
                let (p, of) = self.b.ins().smul_overflow(x, y);
                self.bail_if(of);
                self.b.ins().iadd_imm_s(p, 1)
            }
            _ => {
                // / and % (floored, as Integer's)
                let x = self.b.ins().sshr_imm_s(a, 1);
                let y = self.b.ins().sshr_imm_s(b, 1);
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                self.bail_if(z);
                let q = self.b.ins().sdiv(x, y);
                let qy = self.b.ins().imul(q, y);
                let rem = self.b.ins().isub(x, qy);
                let rnz = self.b.ins().icmp_imm_s(IntCC::NotEqual, rem, 0);
                let sx = self.b.ins().bxor(rem, y);
                let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, sx, 0);
                let adj = self.b.ins().band(rnz, neg);
                let v = if op == b"/" {
                    let adj64 = self.b.ins().uextend(I64, adj);
                    let q2 = self.b.ins().isub(q, adj64);
                    // (only MIN / -1 leaves the Fixnums)
                    let big = self.b.ins().icmp_imm_s(IntCC::Equal, q2, 1i64 << 62);
                    self.bail_if(big);
                    q2
                } else {
                    let zero = self.b.ins().iconst(I64, 0);
                    let addy = self.b.ins().select(adj, y, zero);
                    self.b.ins().iadd(rem, addy)
                };
                let sh = self.b.ins().ishl_imm_s(v, 1);
                self.b.ins().iadd_imm_s(sh, 1)
            }
        }
    }
    /// Branch to `t` when the condition holds, else `e`.
    fn cond(&mut self, n: &Node, t: Block, e: Block, name: &[u8]) {
        if let Some(p) = n.as_parentheses_node() {
            let b = p.body().unwrap();
            let x = match b.as_statements_node() {
                Some(s) => s.body().iter().next().unwrap(),
                None => b,
            };
            return self.cond(&x, t, e, name);
        }
        if let Some(a) = n.as_and_node() {
            let mid = self.b.create_block();
            self.cond(&a.left(), mid, e, name);
            self.b.switch_to_block(mid);
            return self.cond(&a.right(), t, e, name);
        }
        if let Some(o) = n.as_or_node() {
            let mid = self.b.create_block();
            self.cond(&o.left(), t, mid, name);
            self.b.switch_to_block(mid);
            return self.cond(&o.right(), t, e, name);
        }
        let c = n.as_call_node().unwrap();
        let r = c.receiver().unwrap();
        let op = c.name().as_slice();
        if op == b"!" {
            return self.cond(&r, e, t, name);
        }
        let arg = c.arguments().unwrap().arguments().iter().next().unwrap();
        let (Some(a), Some(b)) = (self.int(&r, name), self.int(&arg, name)) else {
            return;
        };
        let cc = match op {
            b"<" => IntCC::SignedLessThan,
            b"<=" => IntCC::SignedLessThanOrEqual,
            b">" => IntCC::SignedGreaterThan,
            b">=" => IntCC::SignedGreaterThanOrEqual,
            b"==" => IntCC::Equal,
            _ => IntCC::NotEqual,
        };
        let v = self.b.ins().icmp(cc, a, b);
        self.b.ins().brif(v, t, &[], e, &[]);
    }
}
