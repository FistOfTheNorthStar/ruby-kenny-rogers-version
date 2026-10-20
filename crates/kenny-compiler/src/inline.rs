//! Block inlining: `n.times { |i| ... }`, `ary.each { |x| ... }`, `ary.map { ... }`,
//! `a.upto(b)`, `(a..b).each` and friends with a literal block become loops in the caller.
//!
//! At runtime a guard (`kn_opt_check`, through the call site's inline cache) checks that the
//! receiver's method is the builtin one; if not (a subclass or monkey patch overriding it,
//! `method_missing`, a receiver of another type) the call is made normally with the block
//! compiled as a closure as usual. The block body is therefore compiled twice; it is only
//! inlined when it has no nested closures, `return`, `redo`, `retry`, `super`, `ensure` or
//! other constructs whose meaning depends on running in its own frame.
//!
//! In the inlined loop the block's own locals are fresh Cranelift variables (re-initialised
//! to nil on every iteration, as each block call starts with fresh locals); variables of
//! enclosing scopes are reached by subtracting one from Prism's scope depth. `next` continues
//! the loop (with its value for `map`/`select`/...), `break` leaves it with its value.

use crate::*;
use ruby_prism::*;

/// Largest block body (in source bytes) that is inlined.
const MAX_INLINE_BODY: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Times,
    Upto,
    Downto,
    RangeEach,
    Each,
    EachWithIndex,
    EachIndex,
    Map,
    Select,
    Reject,
    Any,
    All,
}

/// Runtime opt tags (kenny-runtime/src/opt.rs).
fn opt_tag(k: Kind) -> i64 {
    match k {
        Kind::Times => 17,
        Kind::Upto => 18,
        Kind::Downto => 19,
        Kind::Each => 20,
        Kind::EachWithIndex => 21,
        Kind::Map => 22,
        Kind::RangeEach => 23,
        Kind::Select => 24,
        Kind::Reject => 25,
        Kind::EachIndex => 26,
        Kind::Any => 27,
        Kind::All => 28,
    }
}

/// Which loop a call with a literal block can become (name and argument count).
fn kind_of(name: &[u8], argc: usize) -> Option<Kind> {
    Some(match (name, argc) {
        (b"times", 0) => Kind::Times,
        (b"upto", 1) => Kind::Upto,
        (b"downto", 1) => Kind::Downto,
        (b"each", 0) => Kind::Each, // Array#each or Range#each, decided at runtime
        (b"each_with_index", 0) => Kind::EachWithIndex,
        (b"each_index", 0) => Kind::EachIndex,
        (b"map" | b"collect", 0) => Kind::Map,
        (b"select" | b"filter", 0) => Kind::Select,
        (b"reject", 0) => Kind::Reject,
        (b"any?", 0) => Kind::Any,
        (b"all?", 0) => Kind::All,
        _ => return None,
    })
}

/// Rejects block bodies that can't run inside the caller's frame.
struct Check {
    ok: bool,
}
impl<'pr> Visit<'pr> for Check {
    fn visit_block_node(&mut self, _: &BlockNode<'pr>) {
        self.ok = false;
    }
    fn visit_lambda_node(&mut self, _: &LambdaNode<'pr>) {
        self.ok = false;
    }
    fn visit_def_node(&mut self, _: &DefNode<'pr>) {
        self.ok = false;
    }
    fn visit_class_node(&mut self, _: &ClassNode<'pr>) {
        self.ok = false;
    }
    fn visit_module_node(&mut self, _: &ModuleNode<'pr>) {
        self.ok = false;
    }
    fn visit_singleton_class_node(&mut self, _: &SingletonClassNode<'pr>) {
        self.ok = false;
    }
    fn visit_return_node(&mut self, _: &ReturnNode<'pr>) {
        self.ok = false;
    }
    fn visit_redo_node(&mut self, _: &RedoNode<'pr>) {
        self.ok = false;
    }
    fn visit_retry_node(&mut self, _: &RetryNode<'pr>) {
        self.ok = false;
    }
    fn visit_super_node(&mut self, _: &SuperNode<'pr>) {
        self.ok = false;
    }
    fn visit_forwarding_super_node(&mut self, _: &ForwardingSuperNode<'pr>) {
        self.ok = false;
    }
    fn visit_ensure_node(&mut self, _: &EnsureNode<'pr>) {
        self.ok = false;
    }
    fn visit_forwarding_arguments_node(&mut self, _: &ForwardingArgumentsNode<'pr>) {
        self.ok = false;
    }
    fn visit_it_local_variable_read_node(&mut self, _: &ItLocalVariableReadNode<'pr>) {
        self.ok = false;
    }
    fn visit_post_execution_node(&mut self, _: &PostExecutionNode<'pr>) {
        self.ok = false;
    }
    fn visit_pre_execution_node(&mut self, _: &PreExecutionNode<'pr>) {
        self.ok = false;
    }
    fn visit_block_argument_node(&mut self, node: &BlockArgumentNode<'pr>) {
        // anonymous `&` forwards the method's block through a hidden local
        if node.expression().is_none() {
            self.ok = false;
        }
        visit_block_argument_node(self, node);
    }
    fn visit_splat_node(&mut self, node: &SplatNode<'pr>) {
        if node.expression().is_none() {
            self.ok = false;
        }
        visit_splat_node(self, node);
    }
    fn visit_assoc_splat_node(&mut self, node: &AssocSplatNode<'pr>) {
        if node.value().is_none() {
            self.ok = false;
        }
        visit_assoc_splat_node(self, node);
    }
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if node.receiver().is_none()
            && matches!(node.name().as_slice(), b"binding" | b"local_variables" | b"eval" | b"proc" | b"lambda" | b"block_given?" | b"iterator?")
        {
            self.ok = false;
        }
        visit_call_node(self, node);
    }
}

/// The names of a block's parameters when they are all plain required ones (`|a, b|`).
fn simple_params(params: Option<Node>) -> Option<Vec<Vec<u8>>> {
    let Some(pn) = params else { return Some(vec![]) };
    let bp = pn.as_block_parameters_node()?;
    let Some(p) = bp.parameters() else { return Some(vec![]) };
    if p.optionals().iter().next().is_some()
        || p.rest().is_some()
        || p.posts().iter().next().is_some()
        || p.keywords().iter().next().is_some()
        || p.keyword_rest().is_some()
        || p.block().is_some()
    {
        return None;
    }
    let mut out = Vec::new();
    for r in p.requireds().iter() {
        out.push(r.as_required_parameter_node()?.name().as_slice().to_vec());
    }
    Some(out)
}

impl<M: Module> Compiler<M> {
    /// Try to compile `recv.name(args) { block }` as an inline loop with a guarded fallback.
    /// Returns None (having emitted nothing) when the call doesn't qualify.
    pub(crate) fn inline_block_call(&mut self, fx: &mut Fx, c: &CallNode, recv: Value) -> Option<Value> {
        if self.refine_scope.is_some() || c.is_safe_navigation() || std::env::var_os("KENNY_NO_INLINE").is_some() {
            return None;
        }
        let bn = c.block()?.as_block_node()?;
        let args: Vec<Node> = match c.arguments() {
            Some(a) => a.arguments().iter().collect(),
            None => vec![],
        };
        if args.iter().any(|a| a.as_splat_node().is_some() || a.as_keyword_hash_node().is_some() || a.as_forwarding_arguments_node().is_some()) {
            return None;
        }
        let kind = kind_of(c.name().as_slice(), args.len())?;
        let params = simple_params(bn.parameters())?;
        let mut chk = Check { ok: true };
        if let Some(b) = bn.body() {
            // the body is compiled twice (loop + fallback closure): only small ones, where the
            // per-iteration call overhead matters most
            let l = b.location();
            if l.end_offset() - l.start_offset() > MAX_INLINE_BODY {
                return None;
            }
            chk.visit(&b);
        }
        if !chk.ok {
            return None;
        }
        let name = c.name().as_slice().to_vec();
        let at_loc = c.message_loc().map(|l| l.start_offset()).unwrap_or(c.location().start_offset());
        let arg = if args.len() == 1 { Some(self.expr(fx, &args[0])) } else { None };
        self.cur_offset = at_loc;
        self.mark_loc(fx);

        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let inl = fx.b.create_block();
        let fallback = fx.b.create_block();
        let cache = self.owned_cache_ptr(fx, OWNED_CALL);
        let mid = self.sym_val(fx, &name);
        // Range#each and Array#each share the call: try Array first, then Range
        let (range_inl, guard_kind) = if kind == Kind::Each { (Some(fx.b.create_block()), Kind::Each) } else { (None, kind) };
        let tag = self.iconst(fx, opt_tag(guard_kind));
        let ok = self.call(fx, "kn_opt_check", &[cache, recv, mid, tag]);
        let mut ok = fx.b.ins().icmp_imm_s(IntCC::NotEqual, ok, 0);
        if let Some(a) = arg {
            let fixa = fx.b.ins().band_imm_s(a, 1);
            let fixa = fx.b.ins().icmp_imm_s(IntCC::NotEqual, fixa, 0);
            ok = fx.b.ins().band(ok, fixa);
        }
        match range_inl {
            Some(rb) => {
                let try_range = fx.b.create_block();
                fx.b.ins().brif(ok, inl, &[], try_range, &[]);
                fx.b.switch_to_block(try_range);
                let tag = self.iconst(fx, opt_tag(Kind::RangeEach));
                let ok = self.call(fx, "kn_opt_check", &[cache, recv, mid, tag]);
                fx.b.ins().brif(ok, rb, &[], fallback, &[]);
            }
            None => {
                fx.b.ins().brif(ok, inl, &[], fallback, &[]);
            }
        }

        // the normal call, with the block as a closure
        fx.b.switch_to_block(fallback);
        let locals: Vec<Vec<u8>> = bn.locals().iter().map(|c| c.as_slice().to_vec()).collect();
        let p = self.block_proc(fx, bn.parameters(), bn.body(), locals.clone(), false);
        self.cur_offset = at_loc;
        let r = self.send_with_block(fx, recv, &name, arg, p);
        fx.b.ins().jump(done, &[r.into()]);

        // the loops (errors/warnings were reported by the closure compilation already)
        let (nerr, nwarn) = (self.errors.len(), self.warnings.len());
        fx.b.switch_to_block(inl);
        let v = self.inline_loop(fx, kind, recv, arg, &bn, &params, &locals);
        fx.b.ins().jump(done, &[v.into()]);
        if let Some(rb) = range_inl {
            fx.b.switch_to_block(rb);
            let v = self.inline_loop(fx, Kind::RangeEach, recv, None, &bn, &params, &locals);
            fx.b.ins().jump(done, &[v.into()]);
        }
        self.errors.truncate(nerr);
        self.warnings.truncate(nwarn);
        fx.b.switch_to_block(done);
        Some(fx.b.block_params(done)[0])
    }

    /// One iteration's block call, inline: fresh locals, parameters bound to `vals` (with
    /// autosplat of a single Array value when the block takes several parameters), body.
    #[allow(clippy::too_many_arguments)]
    fn inline_body(&mut self, fx: &mut Fx, bn: &BlockNode, params: &[Vec<u8>], locals: &[Vec<u8>], vals: &[Value], splat: bool, value_used: bool) -> Value {
        let nil = self.nil(fx);
        let mut level = HashMap::new();
        for n in locals {
            let v = fx.b.declare_var(I64);
            fx.b.def_var(v, nil);
            level.insert(n.clone(), v);
        }
        fx.inline_levels.push(level);
        if splat && params.len() > 1 {
            // |a, b| on one yielded value: destructure an Array (or #to_ary), else a = value
            let ary = self.callc(fx, "kn_autosplat", &[vals[0]]);
            let is_nil = fx.b.ins().icmp_imm_s(IntCC::Equal, ary, QNIL);
            let plain = fx.b.create_block();
            let spl = fx.b.create_block();
            let join = fx.b.create_block();
            fx.b.ins().brif(is_nil, plain, &[], spl, &[]);
            fx.b.switch_to_block(plain);
            self.local_set(fx, &params[0], 0, vals[0]);
            fx.b.ins().jump(join, &[]);
            fx.b.switch_to_block(spl);
            for (i, p) in params.iter().enumerate() {
                let iv = self.iconst(fx, i as i64);
                let x = self.call(fx, "kn_ary_ref", &[ary, iv]);
                self.local_set(fx, p, 0, x);
            }
            fx.b.ins().jump(join, &[]);
            fx.b.switch_to_block(join);
        } else {
            for (i, p) in params.iter().enumerate() {
                let x = vals.get(i).copied().unwrap_or(nil);
                self.local_set(fx, p, 0, x);
            }
        }
        let v = match bn.body() {
            // the block's value only matters to map/select/...
            Some(b) if !value_used => {
                match b.as_statements_node() {
                    Some(s) => self.stmts_void(fx, Some(s)),
                    None => {
                        self.expr_void(fx, &b);
                    }
                }
                self.nil(fx)
            }
            Some(b) => self.expr(fx, &b),
            None => self.nil(fx),
        };
        fx.inline_levels.pop();
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn inline_loop(&mut self, fx: &mut Fx, kind: Kind, recv: Value, arg: Option<Value>, bn: &BlockNode, params: &[Vec<u8>], locals: &[Vec<u8>]) -> Value {
        let i = fx.b.declare_var(I64);
        let header = fx.b.create_block();
        let body_b = fx.b.create_block();
        let exit = fx.b.create_block();
        fx.b.append_block_param(exit, I64);
        // `next` lands in `step` (with the block's value where the loop uses it)
        let step = fx.b.create_block();
        let collects = matches!(kind, Kind::Map | Kind::Select | Kind::Reject | Kind::Any | Kind::All);
        if collects {
            fx.b.append_block_param(step, I64);
        }
        let zero = self.iconst(fx, 0);
        let (start, out) = match kind {
            Kind::Times => (zero, None),
            Kind::Upto | Kind::Downto => (fx.b.ins().sshr_imm_s(recv, 1), None),
            Kind::RangeEach => {
                let lo = self.call(fx, "kn_range_first", &[recv]);
                (fx.b.ins().sshr_imm_s(lo, 1), None)
            }
            Kind::Map | Kind::Select | Kind::Reject => (zero, Some(self.call(fx, "kn_ary_new", &[zero, zero]))),
            _ => (zero, None),
        };
        fx.b.def_var(i, start);
        // loop bound for the integer loops (untagged, inclusive except for times)
        let bound = match kind {
            Kind::Times => Some(fx.b.ins().sshr_imm_s(recv, 1)),
            Kind::Upto | Kind::Downto => Some(fx.b.ins().sshr_imm_s(arg.unwrap(), 1)),
            Kind::RangeEach => {
                let hi = self.call(fx, "kn_range_last", &[recv]);
                Some(fx.b.ins().sshr_imm_s(hi, 1))
            }
            _ => None,
        };
        let finished = match kind {
            Kind::Map | Kind::Select | Kind::Reject => out.unwrap(),
            Kind::Any => self.iconst(fx, QFALSE),
            Kind::All => self.iconst(fx, QTRUE),
            _ => recv,
        };
        fx.b.ins().jump(header, &[]);

        fx.b.switch_to_block(header);
        let iv = fx.b.use_var(i);
        let elem = match kind {
            Kind::Times | Kind::Upto | Kind::Downto | Kind::RangeEach => {
                let cc = match kind {
                    Kind::Times => IntCC::SignedLessThan,
                    Kind::Downto => IntCC::SignedGreaterThanOrEqual,
                    _ => IntCC::SignedLessThanOrEqual,
                };
                let c = fx.b.ins().icmp(cc, iv, bound.unwrap());
                fx.b.ins().brif(c, body_b, &[], exit, &[finished.into()]);
                None
            }
            Kind::EachIndex => {
                let len = self.call(fx, "kn_ary_len", &[recv]);
                let c = fx.b.ins().icmp(IntCC::SignedLessThan, iv, len);
                fx.b.ins().brif(c, body_b, &[], exit, &[finished.into()]);
                None
            }
            _ => {
                let x = self.call(fx, "kn_ary_at", &[recv, iv]);
                let end = fx.b.ins().icmp_imm_s(IntCC::Equal, x, UNDEF);
                fx.b.ins().brif(end, exit, &[finished.into()], body_b, &[]);
                Some(x)
            }
        };

        fx.b.switch_to_block(body_b);
        let iv = fx.b.use_var(i);
        let idx = fx.b.ins().ishl_imm_s(iv, 1);
        let idx = fx.b.ins().iadd_imm_s(idx, 1);
        let (vals, splat): (Vec<Value>, bool) = match kind {
            Kind::Times | Kind::Upto | Kind::Downto | Kind::RangeEach | Kind::EachIndex => (vec![idx], false),
            Kind::EachWithIndex => (vec![elem.unwrap(), idx], false),
            _ => (vec![elem.unwrap()], true),
        };
        fx.loops.push(LoopCtx { next: step, exit, redo: body_b, next_val: collects });
        let v = self.inline_body(fx, bn, params, locals, &vals, splat, collects);
        fx.loops.pop();
        if collects {
            fx.b.ins().jump(step, &[v.into()]);
        } else {
            fx.b.ins().jump(step, &[]);
        }

        fx.b.switch_to_block(step);
        if collects {
            let r = fx.b.block_params(step)[0];
            // the element again: `elem` is defined in the header, which dominates `step`
            let x = elem.unwrap();
            match kind {
                Kind::Map => {
                    self.call(fx, "kn_ary_push", &[out.unwrap(), r]);
                }
                Kind::Select | Kind::Reject => {
                    let t = self.truthy(fx, r);
                    let push = fx.b.create_block();
                    let cont = fx.b.create_block();
                    if kind == Kind::Select {
                        fx.b.ins().brif(t, push, &[], cont, &[]);
                    } else {
                        fx.b.ins().brif(t, cont, &[], push, &[]);
                    }
                    fx.b.switch_to_block(push);
                    self.call(fx, "kn_ary_push", &[out.unwrap(), x]);
                    fx.b.ins().jump(cont, &[]);
                    fx.b.switch_to_block(cont);
                }
                Kind::Any => {
                    let t = self.truthy(fx, r);
                    let cont = fx.b.create_block();
                    let tr = self.iconst(fx, QTRUE);
                    fx.b.ins().brif(t, exit, &[tr.into()], cont, &[]);
                    fx.b.switch_to_block(cont);
                }
                _ => {
                    let t = self.truthy(fx, r);
                    let cont = fx.b.create_block();
                    let fa = self.iconst(fx, QFALSE);
                    fx.b.ins().brif(t, cont, &[], exit, &[fa.into()]);
                    fx.b.switch_to_block(cont);
                }
            }
        }
        let iv = fx.b.use_var(i);
        let n = fx.b.ins().iadd_imm_s(iv, if kind == Kind::Downto { -1 } else { 1 });
        fx.b.def_var(i, n);
        self.safepoint(fx);
        fx.b.ins().jump(header, &[]);

        fx.b.switch_to_block(exit);
        fx.b.block_params(exit)[0]
    }
}
