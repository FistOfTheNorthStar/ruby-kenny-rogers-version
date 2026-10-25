//! Scopes become functions: top level, methods (`def`), blocks/lambdas and class bodies.

use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, AbiParam, InstBuilder, StackSlotData, StackSlotKind};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use ruby_prism::*;

pub(crate) struct FnSpec<'a, 'pr> {
    pub kind: ScopeKind,
    pub lambda: bool,
    pub label: String,
    pub locals: Vec<Vec<u8>>,
    pub analysis: Vec<&'a Node<'pr>>,
    /// Lexical scope: a cref slot is two cache words, [class, address of the enclosing
    /// scope's cref slot] (the second is written when a class body starts; 0 = none).
    pub cref_slot: usize,
    pub method: Option<MethodCtx>,
    /// A method's fast entry: `(self, a0.., a(n-1), blk)` for n required parameters,
    /// instead of `(self, argc, argv, blk)` (see `fast_wrapper`).
    pub fast_arity: Option<usize>,
    /// The function, when declared ahead (statics.rs)
    pub predeclared: Option<FuncId>,
    /// Its name in backtraces, when not the usual one for its kind (a `define_method`
    /// block compiled as a method keeps its "block in ..." label)
    pub bt_label: Option<String>,
    /// A block that becomes a method's body (`define_method(:m) { }`): its `return`
    /// returns from it, as a lambda's does
    pub bmethod: bool,
    /// For a block: env hops from its parent env to the env of the lambda (or method
    /// body block) that its `return` returns from; None: the method's (`method_env`)
    pub ret_hops: Option<u32>,
    /// statics.rs `entry_check` (a method that calls itself)
    pub entry_check: bool,
}

pub(crate) struct BlockArity {
    pub min: i64,
    pub max: i64,
    pub autosplat: bool,
}

/// What `def_parts` compiles: a `def`, or a `define_method` block that is one.
pub(crate) struct DefParts<'pr> {
    pub name: Vec<u8>,
    /// where it goes, and how (kenny-runtime abi.rs `kn_def`)
    pub target: Value,
    pub kind: i64,
    pub params: Option<ParametersNode<'pr>>,
    pub body: Option<Node<'pr>>,
    pub locals: Vec<Vec<u8>>,
    pub offset: usize,
    pub singleton: bool,
    pub bt_label: Option<String>,
}

/// Method#parameters descriptor: "req:a,opt:b,rest:c,keyreq:d,key:e,keyrest:f,block:g".
pub(crate) fn params_desc(p: Option<ParametersNode>) -> String {
    let Some(p) = p else { return String::new() };
    let name = |n: &Node| -> String {
        n.as_required_parameter_node().map(|r| String::from_utf8_lossy(r.name().as_slice()).into_owned()).unwrap_or_default()
    };
    let mut out: Vec<String> = Vec::new();
    for r in p.requireds().iter() {
        out.push(format!("req:{}", name(&r)));
    }
    for o in p.optionals().iter() {
        if let Some(o) = o.as_optional_parameter_node() {
            out.push(format!("opt:{}", String::from_utf8_lossy(o.name().as_slice())));
        }
    }
    let kwrest = p.keyword_rest();
    let forwarding = kwrest.as_ref().is_some_and(|k| k.as_forwarding_parameter_node().is_some());
    if let Some(r) = p.rest().and_then(|r| r.as_rest_parameter_node().map(|rp| rp.name().map(|n| String::from_utf8_lossy(n.as_slice()).into_owned()))) {
        out.push(format!("rest:{}", r.unwrap_or_else(|| "*".into())));
    }
    for r in p.posts().iter() {
        out.push(format!("req:{}", name(&r)));
    }
    for k in p.keywords().iter() {
        if let Some(r) = k.as_required_keyword_parameter_node() {
            out.push(format!("keyreq:{}", String::from_utf8_lossy(r.name().as_slice()).trim_end_matches(':')));
        } else if let Some(o) = k.as_optional_keyword_parameter_node() {
            out.push(format!("key:{}", String::from_utf8_lossy(o.name().as_slice()).trim_end_matches(':')));
        }
    }
    if forwarding {
        out.push("rest:*".into());
        out.push("keyrest:**".into());
        out.push("block:&".into());
    } else if let Some(k) = kwrest {
        if let Some(kr) = k.as_keyword_rest_parameter_node() {
            out.push(format!("keyrest:{}", kr.name().map(|n| String::from_utf8_lossy(n.as_slice()).into_owned()).unwrap_or_else(|| "**".into())));
        } else if k.as_no_keywords_parameter_node().is_some() {
            out.push("nokey:".into());
        }
    }
    if let Some(b) = p.block() {
        out.push(format!("block:{}", b.name().map(|n| String::from_utf8_lossy(n.as_slice()).into_owned()).unwrap_or_else(|| "&".into())));
    }
    out.join(",")
}

/// Proc#parameters descriptor of a block (as for a lambda: Proc#parameters makes a proc's
/// required parameters optional).
fn block_params_desc(params: Option<&Node>) -> String {
    let Some(pn) = params else { return String::new() };
    if let Some(np) = pn.as_numbered_parameters_node() {
        return (1..=np.maximum()).map(|i| format!("req:_{i}")).collect::<Vec<_>>().join(",");
    }
    if pn.as_it_parameters_node().is_some() {
        return "req:".into();
    }
    pn.as_block_parameters_node().map(|bp| params_desc(bp.parameters())).unwrap_or_default()
}

fn names(list: &ConstantList) -> Vec<Vec<u8>> {
    list.iter().map(|c| c.as_slice().to_vec()).collect()
}

impl<M: Module> Compiler<M> {
    pub(crate) fn compile_program(&mut self, src: &[u8], path: &Path) -> Result<FuncId, String> {
        self.compile_unit(src, path, ScopeKind::Top)
    }

    /// Compile a whole source unit as a top-level (or class-body) function.
    pub(crate) fn compile_unit(&mut self, src: &[u8], path: &Path, kind: ScopeKind) -> Result<FuncId, String> {
        let result = ruby_prism::parse(src);
        let mut had_err = false;
        for e in result.errors() {
            self.error_at(e.location().start_offset(), &format!("syntax error: {}", e.message()));
            had_err = true;
        }
        if had_err {
            return Err("syntax errors".into());
        }
        let node = result.node();
        self.scan_defs(&node);
        let prog = node.as_program_node().unwrap();
        let stmts = prog.statements().as_node();
        let slot = self.cache_alloc(2);
        if kind == ScopeKind::Top {
            self.cfold_scan(slot, &prog);
        }
        let spec = FnSpec {
            kind,
            lambda: false,
            label: format!("top_{}", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
            locals: names(&prog.locals()),
            analysis: vec![&stmts],
            cref_slot: slot,
            method: None,
            fast_arity: None,
            predeclared: None,
            bt_label: None,
            bmethod: false,
            ret_hops: None,
            entry_check: false,
        };
        Ok(self.build_fn(spec, |c, fx| c.expr(fx, &stmts)))
    }

    /// `require_relative "x"`: compile the file into its own top-level function (once).
    pub(crate) fn require_file(&mut self, path: PathBuf, at: usize) -> Option<(FuncId, usize)> {
        let path = path.canonicalize().unwrap_or(path);
        if let Some(&f) = self.loaded.get(&path) {
            return Some(f);
        }
        if self.loading.contains(&path) {
            return None;
        }
        let src = match std::fs::read(&path) {
            Ok(s) => s,
            Err(e) => {
                self.error_at(at, &format!("cannot load {}: {}", path.display(), e));
                return None;
            }
        };
        self.loading.insert(path.clone());
        let saved_file = std::mem::replace(&mut self.file, path.clone());
        let saved_src = std::mem::replace(&mut self.src, src.clone());
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_refine = self.refine_scope.take();
        let r = self.compile_program(&src, &path);
        self.refine_scope = saved_refine;
        self.scopes = saved_scopes;
        self.file = saved_file;
        self.src = saved_src;
        self.loading.remove(&path);
        let fid = r.ok()?;
        let flag = self.cache_alloc(1);
        self.src_hashes.insert(path.clone(), src_hash(&src));
        self.loaded.insert(path, (fid, flag));
        Some((fid, flag))
    }

    /// A fast entry's signature: self, n arguments, block.
    pub(crate) fn fast_sig(&mut self, n: usize) -> ir::Signature {
        let mut sig = self.module.make_signature();
        for _ in 0..n + 2 {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        sig
    }

    /// The `(self, argc, argv, blk)` entry of a method compiled as fast entry `fast` with
    /// `n` required parameters: checks the count, then calls `fast` with the arguments
    /// in registers. Its only frame in backtraces is an arity error's (at the `def` line).
    fn fast_wrapper(&mut self, fast: FuncId, n: usize, label: &str, def_line: u32) -> FuncId {
        let sig = self.method_sig();
        self.fn_counter += 1;
        let l: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let name = format!("__kn_{}_{}", self.fn_counter, l);
        let id = self.module.declare_function(&name, Linkage::Local, &sig).expect("declare fn");
        let mut func = ir::Function::with_name_signature(ir::UserFuncName::user(0, id.as_u32()), sig);
        let mut fctx = FunctionBuilderContext::new();
        let fsig = self.fast_sig(n);
        let arity = self.rt_id("kn_check_arity");
        {
            let mut b = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry).to_vec();
            let ok = b.ins().icmp_imm_s(IntCC::Equal, p[1], n as i64);
            let go = b.create_block();
            let fail = b.create_block();
            b.set_cold_block(fail);
            b.ins().brif(ok, go, &[], fail, &[]);
            b.switch_to_block(go);
            let mut args = vec![p[0]];
            for i in 0..n {
                args.push(b.ins().load(I64, mem(), p[2], (i * 8) as i32));
            }
            args.push(p[3]);
            let fref = self.module.declare_func_in_func(fast, b.func);
            let _ = fsig;
            let call = b.ins().call(fref, &args);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.switch_to_block(fail);
            b.set_srcloc(ir::SourceLoc::new(def_line));
            let fr = self.module.declare_func_in_func(arity, b.func);
            let k = b.ins().iconst(I64, n as i64);
            let call = b.ins().call(fr, &[p[1], k, k]);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.seal_all_blocks();
            b.finalize(self.module.isa().frontend_config());
        }
        let mut ctx = cranelift_codegen::Context::for_function(func);
        if let Err(e) = self.module.define_function(id, &mut ctx) {
            self.errors.push(format!("internal codegen error in {name}: {e:?}"));
        } else if let Some(code) = ctx.compiled_code() {
            let before = self.debug_fns.len();
            self.record_debug_info(id, label, code);
            if let Some(d) = self.debug_fns.get_mut(before) {
                d.exact = true;
            }
        }
        id
    }

    pub(crate) fn method_sig(&mut self) -> ir::Signature {
        let mut sig = self.module.make_signature();
        for _ in 0..4 {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        sig
    }
    fn block_sig(&mut self) -> ir::Signature {
        let mut sig = self.module.make_signature();
        for _ in 0..5 {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        sig
    }

    pub(crate) fn build_fn<'a, 'pr, F>(&mut self, spec: FnSpec<'a, 'pr>, body: F) -> FuncId
    where
        F: for<'b> FnOnce(&mut Compiler<M>, &mut Fx<'b>) -> Value,
    {
        let is_block = spec.kind == ScopeKind::Block;
        let sig = match spec.fast_arity {
            Some(n) => self.fast_sig(n),
            None if is_block => self.block_sig(),
            None => self.method_sig(),
        };
        // the frame's name in backtraces, as CRuby labels it
        let bt_label = match spec.kind {
            _ if spec.bt_label.is_some() => spec.bt_label.clone().unwrap(),
            ScopeKind::Top | ScopeKind::Class if self.eval_unit && !self.file_unit && self.fn_labels.is_empty() => "<eval>".to_string(),
            ScopeKind::Top if self.fn_labels.is_empty() && self.file == self.main_file => "<main>".to_string(),
            ScopeKind::Top => "<top (required)>".to_string(),
            ScopeKind::Block => {
                // (a block inside inlined code: that code's frame is its parent)
                let parent = self.dbg_frames.last().map(|f| f.label.clone()).or_else(|| self.fn_labels.last().cloned()).unwrap_or_else(|| "<main>".into());
                crate::statics::block_label(&parent)
            }
            _ => spec.label.clone(),
        };
        self.fn_labels.push(bt_label.clone());
        // (a function of its own: its call sites aren't inside the enclosing inlined code)
        let saved_dbg = std::mem::take(&mut self.dbg_frames);
        let saved_parent = self.block_parent.take();
        self.fn_counter += 1;
        let label: String = spec.label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let name = format!("__kn_{}_{}", self.fn_counter, label);
        let id = match spec.predeclared {
            Some(id) => id,
            None => self.module.declare_function(&name, Linkage::Local, &sig).expect("declare fn"),
        };
        let mut func = ir::Function::with_name_signature(ir::UserFuncName::user(0, id.as_u32()), sig);
        let mut fctx = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry).to_vec();
            let gv = |m: &mut M, b: &mut FunctionBuilder, id: DataId| {
                let g = m.declare_data_in_func(id, b.func);
                b.ins().symbol_value(I64, g)
            };
            let caches = gv(&mut self.module, &mut b, self.caches_id);
            let syms = gv(&mut self.module, &mut b, self.syms_id);
            let strs = gv(&mut self.module, &mut b, self.strs_id);
            let gcflag = gv(&mut self.module, &mut b, self.gc_flag_id);
            let mk = |b: &mut FunctionBuilder, v: Value| {
                let var = b.declare_var(I64);
                b.def_var(var, v);
                var
            };
            let (self_v, argc_v, argv_v, blk_v, proc_v) = if let Some(n) = spec.fast_arity {
                let c = b.ins().iconst(I64, n as i64);
                let z = b.ins().iconst(I64, 0);
                (mk(&mut b, p[0]), mk(&mut b, c), mk(&mut b, z), mk(&mut b, p[n + 1]), None)
            } else if is_block {
                (mk(&mut b, p[1]), mk(&mut b, p[2]), mk(&mut b, p[3]), mk(&mut b, p[4]), Some(mk(&mut b, p[0])))
            } else {
                (mk(&mut b, p[0]), mk(&mut b, p[1]), mk(&mut b, p[2]), mk(&mut b, p[3]), None)
            };
            let penv_v = proc_v.map(|pv| {
                let pr = b.use_var(pv);
                let e = b.ins().load(I64, mem(), pr, PROC_ENV_OFFSET);
                mk(&mut b, e)
            });
            let zero = b.ins().iconst(I64, 0);
            let tag_v = mk(&mut b, zero);

            // locals: captured ones go to the heap env, the rest to registers
            let caps = analysis::captures(&spec.analysis);
            let mut env_slots = HashMap::new();
            for n in &spec.locals {
                if (caps.captured.contains(n) || (caps.zsuper_in_block && spec.kind == ScopeKind::Method)) && !env_slots.contains_key(n) {
                    let i = env_slots.len();
                    env_slots.insert(n.clone(), i);
                }
            }
            // a bare `super` inside a block also passes the method's anonymous `*` / `**`
            // (`def render(*) = logger.silence { super }`), which aren't in the locals list
            // and so do `...` and anonymous `*` / `**` / `&` arguments used in a block
            if spec.kind == ScopeKind::Method {
                for n in [&b"*"[..], b"**", b"&"] {
                    let used = (caps.zsuper_in_block && n != b"&") || caps.captured.contains(n);
                    if used && !env_slots.contains_key(n) {
                        let i = env_slots.len();
                        env_slots.insert(n.to_vec(), i);
                    }
                }
            }
            let has_env = caps.has_block;
            let nil = b.ins().iconst(I64, QNIL);
            let mut locals = HashMap::new();
            for (n, &i) in env_slots.iter() {
                if !spec.locals.contains(n) {
                    locals.insert(n.clone(), Local::Env(i));
                }
            }
            for n in &spec.locals {
                if let Some(&i) = env_slots.get(n) {
                    locals.insert(n.clone(), Local::Env(i));
                } else if !locals.contains_key(n) {
                    let v = b.declare_var(I64);
                    b.def_var(v, nil);
                    locals.insert(n.clone(), Local::Var(v));
                }
            }
            // unboxed Float shadows for register locals assigned from arithmetic
            let (num_locals, flocals, int_sure, typed_info) = typed_setup(&mut b, &locals, &spec.analysis, self.refine_scope.is_none());
            let vlocals = self.vobj_plan(&mut b, &locals, &spec.analysis);
            // a method whose locals all stay in registers needs its env only for the blocks it
            // makes into closures (their way back to this frame): made when the first one is
            // (`lazy_env`), so a method whose blocks were all inlined never makes it
            let lazy_env = has_env && spec.kind == ScopeKind::Method && env_slots.is_empty();
            let env_v = if lazy_env {
                let z = b.ins().iconst(I64, 0);
                Some(mk(&mut b, z))
            } else if has_env {
                let parent = match penv_v {
                    Some(pe) => b.use_var(pe),
                    None => b.ins().iconst(I64, 0),
                };
                let blk = if spec.kind == ScopeKind::Method { b.use_var(blk_v) } else { nil };
                let n = b.ins().iconst(I64, env_slots.len() as i64);
                let id = self.rt_id("kn_env_new");
                let f = self.module.declare_func_in_func(id, b.func);
                let call = b.ins().call(f, &[parent, blk, n]);
                let e = b.inst_results(call)[0];
                Some(mk(&mut b, e))
            } else {
                None
            };
            let ret_here = is_block && (spec.lambda || spec.bmethod);
            if spec.kind == ScopeKind::Method || ret_here {
                let tag = match env_v {
                    Some(e) if !lazy_env => b.use_var(e),
                    _ => {
                        let ss = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
                        b.ins().stack_addr(I64, ss, 0)
                    }
                };
                b.def_var(tag_v, tag);
            }
            if spec.kind == ScopeKind::Class {
                let s = b.use_var(self_v);
                let p = b.ins().iadd_imm_s(caches, (spec.cref_slot * 8) as i64);
                b.ins().store(mem(), s, p, 0);
                // link to the enclosing lexical scope (`module A; class B` vs `class A::B`)
                if let Some(&parent) = self.cref_parents.get(&spec.cref_slot) {
                    let pp = b.ins().iadd_imm_s(caches, (parent * 8) as i64);
                    b.ins().store(mem(), pp, p, 8);
                }
            }
            let exit = b.create_block();
            self.scopes.push(ScopeInfo { env_slots });
            let mut fx = Fx {
                b,
                kind: spec.kind,
                lambda: spec.lambda,
                self_fid: if spec.kind == ScopeKind::Method { spec.predeclared } else { None },
                ret_here,
                ret_hops: spec.ret_hops,
                self_v,
                blk_v,
                argc_v,
                argv_v,
                env_v,
                penv_v,
                proc_v,
                tag_v,
                locals,
                unwind: vec![exit],
                ensure_depth: 0,
                rescue_prev: Vec::new(),
                loops: Vec::new(),
                retry: Vec::new(),
                caches,
                syms,
                strs,
                gcflag,
                cref_slot: spec.cref_slot,
                method: spec.method,
                funcs: HashMap::new(),
                scope: self.scopes.len() - 1,
                zsuper: Vec::new(),
                redo_block: None,
                inline_levels: Vec::new(),
                checked: std::collections::HashSet::new(),
                num_locals,
                int_locals: analysis::int_locals(&spec.analysis),
                int_vals: HashSet::new(),
                flocals,
                int_sure,
                sure_ints: HashSet::new(),
                typed_info,
                spec: None,
                spec_depth: 0,
                int_spec: HashSet::new(),
                sure_fix: HashSet::new(),
                resume: HashMap::new(),
                resume_req: HashMap::new(),
                resume_before: HashMap::new(),
                resume_before_req: HashMap::new(),
                spec_stmt: None,
                spec_stmt_used: false,
                hoisted: HashMap::new(),
                stmt_memo: HashMap::new(),
                stmt_at: usize::MAX,
                fast_args: spec.fast_arity.map(|n| p[1..=n].to_vec()),
                vlocals,
                vself: None,
                vself_next: None,
                licm: Vec::new(),
                licm_pending: None,
                licm_vals: Vec::new(),
                no_block_def: false,
                lazy_env,
                inl_self: Vec::new(),
                bools: HashMap::new(),
                owned: HashMap::new(),
            };
            if self.refine_scope.is_none() && std::env::var_os("KENNY_NO_OWNED").is_none() {
                for n in crate::owned::append_locals(&spec.analysis) {
                    let plain = matches!(fx.locals.get(&n), Some(Local::Var(_)))
                        && !fx.num_locals.contains_key(&n)
                        && !fx.flocals.contains_key(&n)
                        && !fx.int_sure.contains(&n)
                        && !fx.vlocals.contains_key(&n);
                    if plain {
                        let v = fx.b.declare_var(ir::types::I8);
                        let z = fx.b.ins().iconst(ir::types::I8, 0);
                        fx.b.def_var(v, z);
                        fx.owned.insert(n, v);
                    }
                }
            }
            if spec.entry_check {
                self.entry_check(&mut fx);
            }
            let v = body(self, &mut fx);
            fx.b.ins().return_(&[v]);
            // unwinding out of this function
            fx.b.switch_to_block(exit);
            if fx.kind == ScopeKind::Method || fx.ret_here {
                let tag = fx.b.use_var(fx.tag_v);
                let r = self.call(&mut fx, "kn_catch_return", &[tag]);
                fx.b.ins().return_(&[r]);
            } else {
                let u = fx.b.ins().iconst(I64, UNDEF);
                fx.b.ins().return_(&[u]);
            }
            fx.b.seal_all_blocks();
            let cfg = self.module.isa().frontend_config();
            fx.b.finalize(cfg);
            self.scopes.pop();
        }
        self.fn_labels.pop();
        self.dbg_frames = saved_dbg;
        self.block_parent = saved_parent;
        let mut ctx = cranelift_codegen::Context::for_function(func);
        if let Err(e) = self.module.define_function(id, &mut ctx) {
            self.errors.push(format!("internal codegen error in {name}: {e:?}"));
        } else if let Some(code) = ctx.compiled_code() {
            self.record_debug_info(id, &bt_label, code);
        }
        id
    }

    pub(crate) fn fn_addr(&mut self, fx: &mut Fx, id: FuncId) -> Value {
        let f = self.module.declare_func_in_func(id, fx.b.func);
        fx.b.ins().func_addr(I64, f)
    }
    pub(crate) fn call_fn(&mut self, fx: &mut Fx, id: FuncId, args: &[Value]) -> Value {
        let f = self.module.declare_func_in_func(id, fx.b.func);
        let inst = fx.b.ins().call(f, args);
        self.licm_kill(fx);
        fx.b.inst_results(inst)[0]
    }

    // ---------- locals ----------

    fn env_ptr_at_depth(&mut self, fx: &mut Fx, depth: u32) -> Value {
        let pe = fx.penv_v.expect("outer variable access outside a block");
        let mut p = fx.b.use_var(pe);
        for _ in 1..depth {
            p = fx.b.ins().load(I64, mem(), p, ENV_PARENT);
        }
        p
    }
    fn outer_slot(&self, fx: &Fx, name: &[u8], depth: u32) -> Option<usize> {
        let idx = fx.scope.checked_sub(depth as usize)?;
        self.scopes.get(idx)?.env_slots.get(name).copied()
    }
    pub(crate) fn local_get(&mut self, fx: &mut Fx, name: &[u8], depth: u32) -> Value {
        let n = fx.inline_levels.len() as u32;
        if depth < n {
            let level = &mut fx.inline_levels[(n - 1 - depth) as usize];
            let v = match level.get(name) {
                Some(v) => *v,
                None => {
                    let v = fx.b.declare_var(I64);
                    let nil = fx.b.ins().iconst(I64, QNIL);
                    fx.b.def_var(v, nil);
                    fx.inline_levels[(n - 1 - depth) as usize].insert(name.to_vec(), v);
                    v
                }
            };
            return fx.b.use_var(v);
        }
        let depth = depth - n;
        if depth == 0 {
            if fx.vlocals.contains_key(name) {
                self.vobj_materialize(fx, name);
            }
            match fx.locals.get(name) {
                Some(Local::Var(v)) => {
                    let v = *v;
                    if let Some(&(raw, cache)) = fx.flocals.get(name) {
                        return self.flocal_box(fx, raw, cache);
                    }
                    if let Some(&(raw, kind)) = fx.num_locals.get(name) {
                        return self.local_box(fx, v, raw, kind);
                    }
                    let r = fx.b.use_var(v);
                    // (the value may be kept: no longer appended to in place)
                    if let Some(&ov) = fx.owned.get(name) {
                        let z = fx.b.ins().iconst(ir::types::I8, 0);
                        fx.b.def_var(ov, z);
                    }
                    if !fx.licm.is_empty() {
                        self.licm_note(fx, name, r);
                    }
                    if fx.int_sure.contains(name) {
                        fx.sure_ints.insert(r);
                    }
                    if fx.int_spec.contains(name) {
                        fx.sure_ints.insert(r);
                        fx.sure_fix.insert(r);
                    }
                    return r;
                }
                Some(Local::Env(i)) => {
                    let i = *i;
                    let e = fx.b.use_var(fx.env_v.unwrap());
                    return fx.b.ins().load(I64, mem(), e, ENV_SLOTS + 8 * i as i32);
                }
                None => {
                    let v = fx.b.declare_var(I64);
                    let nil = self.nil(fx);
                    fx.b.def_var(v, nil);
                    fx.locals.insert(name.to_vec(), Local::Var(v));
                    return nil;
                }
            }
        }
        match self.outer_slot(fx, name, depth) {
            Some(slot) => {
                let p = self.env_ptr_at_depth(fx, depth);
                fx.b.ins().load(I64, mem(), p, ENV_SLOTS + 8 * slot as i32)
            }
            None => self.nil(fx),
        }
    }
    pub(crate) fn local_set(&mut self, fx: &mut Fx, name: &[u8], depth: u32, val: Value) {
        let n = fx.inline_levels.len() as u32;
        if depth < n {
            let idx = (n - 1 - depth) as usize;
            let v = match fx.inline_levels[idx].get(name) {
                Some(v) => *v,
                None => {
                    let v = fx.b.declare_var(I64);
                    fx.inline_levels[idx].insert(name.to_vec(), v);
                    v
                }
            };
            fx.b.def_var(v, val);
            return;
        }
        let depth = depth - n;
        if depth == 0 {
            if let Some(vl) = fx.vlocals.get(name) {
                let isv = vl.isv;
                let zero = fx.b.ins().iconst(I64, 0);
                fx.b.def_var(isv, zero);
            }
            match fx.locals.get(name) {
                Some(Local::Var(v)) => {
                    let v = *v;
                    if let Some(&(raw, cache)) = fx.flocals.get(name) {
                        let f = self.float_unbox(fx, val);
                        fx.b.def_var(raw, f);
                        if let Some(cache) = cache {
                            fx.b.def_var(cache, val);
                        }
                        return;
                    }
                    fx.b.def_var(v, val);
                    if let Some(&ov) = fx.owned.get(name) {
                        let z = fx.b.ins().iconst(ir::types::I8, 0);
                        fx.b.def_var(ov, z);
                    }
                    if let Some(&(_, kind)) = fx.num_locals.get(name) {
                        let k = fx.b.ins().iconst(ir::types::I8, 0);
                        fx.b.def_var(kind, k);
                    }
                }
                Some(Local::Env(i)) => {
                    let i = *i;
                    let e = fx.b.use_var(fx.env_v.unwrap());
                    fx.b.ins().store(mem(), val, e, ENV_SLOTS + 8 * i as i32);
                    self.env_write_barrier(fx, e);
                }
                None => {
                    let v = fx.b.declare_var(I64);
                    fx.b.def_var(v, val);
                    fx.locals.insert(name.to_vec(), Local::Var(v));
                }
            }
            return;
        }
        if let Some(slot) = self.outer_slot(fx, name, depth) {
            let p = self.env_ptr_at_depth(fx, depth);
            fx.b.ins().store(mem(), val, p, ENV_SLOTS + 8 * slot as i32);
            self.env_write_barrier(fx, p);
        }
    }
    /// Generational GC: storing into an old environment (its header's mark bit, 4 bytes
    /// before the environment) tells the runtime, so the next minor collection rescans it
    /// (once: kenny-runtime gc.rs `FL_REMEMBERED`, 1 << 21, set while it is recorded).
    fn env_write_barrier(&mut self, fx: &mut Fx, e: Value) {
        let flags = fx.b.ins().load(ir::types::I32, mem(), e, -4);
        let bits = fx.b.ins().band_imm_u(flags, 4 | (1 << 21));
        let old = fx.b.ins().icmp_imm_u(IntCC::Equal, bits, 4);
        let slow = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.set_cold_block(slow);
        fx.b.ins().brif(old, slow, &[], done, &[]);
        fx.b.switch_to_block(slow);
        self.call(fx, "kn_wb_env", &[e]);
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
    }

    /// A shadowed local's tagged value, boxing its raw Float the first time it is needed.
    fn local_box(&mut self, fx: &mut Fx, v: Variable, raw: Variable, kind: Variable) -> Value {
        let cur = fx.b.use_var(v);
        let k = fx.b.use_var(kind);
        let need = fx.b.ins().icmp_imm_s(IntCC::Equal, k, 1);
        let enc = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(need, enc, &[], done, &[cur.into()]);
        fx.b.switch_to_block(enc);
        let r = fx.b.use_var(raw);
        let lazy = crate::fastpath::Num::Lazy { boxed: cur, raw: r, kind: k, lit: false };
        let boxed = self.materialize(fx, lazy);
        fx.b.def_var(v, boxed);
        let k2 = fx.b.ins().iconst(ir::types::I8, 2);
        fx.b.def_var(kind, k2);
        fx.b.ins().jump(done, &[boxed.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// A local as a numeric operand (unboxed if it has a raw Float).
    pub(crate) fn local_get_num(&mut self, fx: &mut Fx, name: &[u8], depth: u32) -> crate::fastpath::Num {
        if depth == fx.inline_levels.len() as u32 {
            if let (Some(Local::Var(_)), Some(&(raw, _))) = (fx.locals.get(name), fx.flocals.get(name)) {
                return crate::fastpath::Num::F64(fx.b.use_var(raw));
            }
            if let (Some(Local::Var(v)), Some(&(raw, kind))) = (fx.locals.get(name), fx.num_locals.get(name)) {
                let v = *v;
                let boxed = fx.b.use_var(v);
                let raw = fx.b.use_var(raw);
                let kind = fx.b.use_var(kind);
                return crate::fastpath::Num::Lazy { boxed, raw, kind, lit: false };
            }
        }
        let v = self.local_get(fx, name, depth);
        if depth == fx.inline_levels.len() as u32 && fx.int_locals.contains(name) {
            fx.int_vals.insert(v);
        }
        crate::fastpath::Num::Boxed(v)
    }
    /// Assign a numeric result to a local without boxing it (when it has a shadow).
    pub(crate) fn local_set_num(&mut self, fx: &mut Fx, name: &[u8], depth: u32, n: crate::fastpath::Num) {
        if depth == fx.inline_levels.len() as u32 {
            if let (Some(Local::Var(_)), Some(&(rawv, cache))) = (fx.locals.get(name), fx.flocals.get(name)) {
                let (f, boxed) = self.num_f64_boxed(fx, n);
                fx.b.def_var(rawv, f);
                if let Some(cache) = cache {
                    fx.b.def_var(cache, boxed);
                }
                return;
            }
        }
        if let crate::fastpath::Num::Lazy { boxed, raw, kind: k, .. } = n {
            if depth == fx.inline_levels.len() as u32 {
                if let (Some(Local::Var(v)), Some(&(rawv, kind))) = (fx.locals.get(name), fx.num_locals.get(name)) {
                    let v = *v;
                    fx.b.def_var(v, boxed);
                    fx.b.def_var(rawv, raw);
                    fx.b.def_var(kind, k);
                    return;
                }
            }
        }
        let v = self.materialize(fx, n);
        self.local_set(fx, name, depth, v);
    }
    /// Does this local keep an unboxed shadow?
    pub(crate) fn has_num_shadow(&self, fx: &Fx, name: &[u8], depth: u32) -> bool {
        depth == fx.inline_levels.len() as u32 && (fx.num_locals.contains_key(name) || fx.flocals.contains_key(name)) && matches!(fx.locals.get(name), Some(Local::Var(_)))
    }

    /// The env of the enclosing method (for yield / block_given? / return inside blocks).
    /// This method's env (`Fx::lazy_env`), made now if it isn't yet; it is also the tag that
    /// a `return` in its blocks names.
    fn lazy_env(&mut self, fx: &mut Fx) -> Value {
        let ev = fx.env_v.unwrap();
        let e = fx.b.use_var(ev);
        let make = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(make);
        fx.b.ins().brif(e, done, &[e.into()], make, &[]);
        fx.b.switch_to_block(make);
        let zero = self.iconst(fx, 0);
        let blk = fx.b.use_var(fx.blk_v);
        let n = self.iconst(fx, 0);
        let ne = self.call(fx, "kn_env_new", &[zero, blk, n]);
        fx.b.def_var(ev, ne);
        fx.b.def_var(fx.tag_v, ne);
        fx.b.ins().jump(done, &[ne.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    pub(crate) fn method_env(&mut self, fx: &mut Fx) -> Option<Value> {
        let hops = fx.method.as_ref()?.hops?;
        let pe = fx.penv_v?;
        let mut p = fx.b.use_var(pe);
        for _ in 0..hops {
            p = fx.b.ins().load(I64, mem(), p, ENV_PARENT);
        }
        Some(p)
    }
    /// The block passed to the current method (works inside nested blocks too).
    pub(crate) fn method_block(&mut self, fx: &mut Fx) -> Value {
        match fx.kind {
            ScopeKind::Method => fx.b.use_var(fx.blk_v),
            ScopeKind::Block => match self.method_env(fx) {
                Some(e) => fx.b.ins().load(I64, mem(), e, ENV_BLK),
                None => self.nil(fx),
            },
            _ => self.nil(fx),
        }
    }

    // ---------- argument access ----------

    fn arg_at(&mut self, fx: &mut Fx, i: Value) -> Value {
        let argv = fx.b.use_var(fx.argv_v);
        let off = fx.b.ins().imul_imm_s(i, 8);
        let p = fx.b.ins().iadd(argv, off);
        fx.b.ins().load(I64, mem(), p, 0)
    }
    fn arg_const(&mut self, fx: &mut Fx, i: usize) -> Value {
        let argv = fx.b.use_var(fx.argv_v);
        fx.b.ins().load(I64, mem(), argv, (i * 8) as i32)
    }
    /// argv[i] if i < argc else nil
    fn arg_or_nil(&mut self, fx: &mut Fx, i: usize) -> Value {
        let argc = fx.b.use_var(fx.argc_v);
        let has = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, argc, i as i64);
        let yes = fx.b.create_block();
        let no = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(has, yes, &[], no, &[]);
        fx.b.switch_to_block(yes);
        let v = self.arg_const(fx, i);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(no);
        let n = self.nil(fx);
        fx.b.ins().jump(done, &[n.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }
    /// Assign the default value of an optional parameter unless an argument was passed.
    /// Optional parameter `idx` gets an argument only when there are more than `idx`
    /// plus the `posts` required after it (those are filled first).
    fn optional_param(&mut self, fx: &mut Fx, name: &[u8], idx: usize, posts: usize, default: &Node, from_ary: Option<Value>) {
        let argc = match from_ary {
            Some(a) => self.call(fx, "kn_ary_len", &[a]),
            None => fx.b.use_var(fx.argc_v),
        };
        let has = fx.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, argc, (idx + posts) as i64);
        let yes = fx.b.create_block();
        let no = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.ins().brif(has, yes, &[], no, &[]);
        fx.b.switch_to_block(yes);
        let v = match from_ary {
            Some(a) => {
                let i = self.iconst(fx, idx as i64);
                self.call(fx, "kn_ary_ref", &[a, i])
            }
            None => self.arg_const(fx, idx),
        };
        self.local_set(fx, name, 0, v);
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(no);
        let d = self.expr(fx, default);
        self.local_set(fx, name, 0, d);
        fx.b.ins().jump(done, &[]);
        fx.b.switch_to_block(done);
    }

    fn param_name(n: &Node) -> Option<Vec<u8>> {
        if let Some(r) = n.as_required_parameter_node() {
            return Some(r.name().as_slice().to_vec());
        }
        None
    }

    /// Bind a positional parameter value to a required param or a destructuring pattern.
    fn bind_param_target(&mut self, fx: &mut Fx, target: &Node, v: Value) {
        if let Some(n) = Self::param_name(target) {
            self.local_set(fx, &n, 0, v);
        } else if let Some(m) = target.as_multi_target_node() {
            let ary = self.callc(fx, "kn_masgn_ary", &[v]);
            self.destructure(fx, &m.lefts(), m.rest(), &m.rights(), ary);
        } else {
            self.assign_target(fx, target, v);
        }
    }

    // ---------- method parameters ----------

    /// Inline argument-count check; the runtime is only called to raise the ArgumentError.
    fn arity_check(&mut self, fx: &mut Fx, min: i64, max: i64) {
        let argc = fx.b.use_var(fx.argc_v);
        let ok = if max < 0 {
            fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, argc, min)
        } else if min == max {
            fx.b.ins().icmp_imm_s(IntCC::Equal, argc, min)
        } else {
            let lo = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, argc, min);
            let hi = fx.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, argc, max);
            fx.b.ins().band(lo, hi)
        };
        let fail = fx.b.create_block();
        let cont = fx.b.create_block();
        fx.b.ins().brif(ok, cont, &[], fail, &[]);
        fx.b.set_cold_block(fail);
        fx.b.switch_to_block(fail);
        let (lo, hi) = (self.iconst(fx, min), self.iconst(fx, max));
        let r = self.call(fx, "kn_check_arity", &[argc, lo, hi]);
        self.check(fx, r);
        fx.b.ins().jump(cont, &[]);
        fx.b.switch_to_block(cont);
    }

    pub(crate) fn bind_method_params(&mut self, fx: &mut Fx, params: Option<ParametersNode>) {
        // a fast entry: only required parameters (and maybe &blk), already counted
        if let Some(args) = fx.fast_args.clone() {
            if let Some(p) = params {
                let reqs: Vec<Node> = p.requireds().iter().collect();
                for r in reqs.iter() {
                    if let Some(n) = Self::param_name(r) {
                        fx.zsuper.push((n, 0));
                    }
                }
                for (i, r) in reqs.iter().enumerate() {
                    self.bind_param_target(fx, r, args[i]);
                }
                if let Some(bp) = p.block() {
                    if let Some(n) = bp.name() {
                        let b = fx.b.use_var(fx.blk_v);
                        self.local_set(fx, n.as_slice(), 0, b);
                    }
                }
            }
            return;
        }
        let Some(p) = params else {
            self.arity_check(fx, 0, 0);
            return;
        };
        let reqs: Vec<Node> = p.requireds().iter().collect();
        let opts: Vec<Node> = p.optionals().iter().collect();
        let posts: Vec<Node> = p.posts().iter().collect();
        let kws: Vec<Node> = p.keywords().iter().collect();
        let rest = p.rest();
        let kwrest = p.keyword_rest();
        // `def m(...)`: everything lands in the hidden locals `*` (args incl. keywords) and `&`
        let forwarding = kwrest.as_ref().is_some_and(|k| k.as_forwarding_parameter_node().is_some());
        let has_rest = rest.is_some() || forwarding;
        let has_kw = !forwarding && (!kws.is_empty() || kwrest.as_ref().is_some_and(|k| k.as_keyword_rest_parameter_node().is_some()));
        let (nreq, nopt, npost) = (reqs.len(), opts.len(), posts.len());

        let maxpos = if has_rest { -1 } else { (nreq + nopt + npost) as i64 };
        let consume = kwrest.as_ref().is_some_and(|k| k.as_keyword_rest_parameter_node().is_some());
        let kwh = if has_kw { Some(self.extract_kwhash(fx, nreq + npost, maxpos, nreq + npost, consume)) } else { None };

        self.arity_check(fx, (nreq + npost) as i64, if has_rest { -1 } else { (nreq + nopt + npost) as i64 });

        for r in reqs.iter() {
            if let Some(n) = Self::param_name(r) {
                fx.zsuper.push((n, 0));
            }
        }
        for o in opts.iter() {
            fx.zsuper.push((o.as_optional_parameter_node().unwrap().name().as_slice().to_vec(), 0));
        }
        if let Some(n) = rest.as_ref().and_then(|r| r.as_rest_parameter_node()).and_then(|r| r.name()) {
            fx.zsuper.push((n.as_slice().to_vec(), 1));
        }
        for pn in posts.iter() {
            if let Some(n) = Self::param_name(pn) {
                fx.zsuper.push((n, 0));
            }
        }
        for k in kws.iter() {
            let n = k
                .as_required_keyword_parameter_node()
                .map(|r| r.name().as_slice().to_vec())
                .or_else(|| k.as_optional_keyword_parameter_node().map(|o| o.name().as_slice().to_vec()));
            if let Some(n) = n {
                fx.zsuper.push((n.strip_suffix(b":").unwrap_or(&n).to_vec(), 2));
            }
        }
        if let Some(k) = kwrest.as_ref().and_then(|k| k.as_keyword_rest_parameter_node()) {
            // anonymous `**` is bound as the local `**`
            fx.zsuper.push((k.name().map(|n| n.as_slice().to_vec()).unwrap_or(b"**".to_vec()), 3));
        }
        for (i, r) in reqs.iter().enumerate() {
            let v = self.arg_const(fx, i);
            self.bind_param_target(fx, r, v);
        }
        for (j, o) in opts.iter().enumerate() {
            let o = o.as_optional_parameter_node().unwrap();
            self.optional_param(fx, o.name().as_slice(), nreq + j, npost, &o.value(), None);
        }
        let rest_name: Option<Vec<u8>> = match rest.as_ref().and_then(|r| r.as_rest_parameter_node()) {
            Some(rp) => Some(rp.name().map(|n| n.as_slice().to_vec()).unwrap_or(b"*".to_vec())),
            None if forwarding => Some(b"*".to_vec()),
            None => None,
        };
        if let Some(n) = rest_name {
            let argc = fx.b.use_var(fx.argc_v);
            let argv = fx.b.use_var(fx.argv_v);
            let st = self.iconst(fx, (nreq + nopt) as i64);
            let po = self.iconst(fx, npost as i64);
            let a = self.call(fx, "kn_args_rest", &[argc, argv, st, po]);
            self.local_set(fx, &n, 0, a);
            if forwarding || n == b"*" {
                fx.zsuper.push((n.clone(), 1));
            }
        }
        if forwarding {
            let b = fx.b.use_var(fx.blk_v);
            self.local_set(fx, b"&", 0, b);
        }
        for (k, pn) in posts.iter().enumerate() {
            let argc = fx.b.use_var(fx.argc_v);
            let i = fx.b.ins().iadd_imm_s(argc, -((npost - k) as i64));
            let v = self.arg_at(fx, i);
            self.bind_param_target(fx, pn, v);
        }
        if let Some(h) = kwh {
            self.bind_keywords(fx, h, &kws, kwrest.as_ref());
        }
        if let Some(bp) = p.block() {
            if let Some(n) = bp.name() {
                let b = fx.b.use_var(fx.blk_v);
                self.local_set(fx, n.as_slice(), 0, b);
            }
        }
    }


    /// Take the trailing keyword-hash argument (if any) off the positional arguments.
    /// `maxpos`: positional capacity (-1 with a rest param); `nreq2`: required positionals,
    /// for the trailing-Hash keyword rule applied to code from dependencies (see kn_take_kwhash2).
    /// `consume`: a copy whose keys `bind_keywords` removes (for a `**rest` parameter);
    /// otherwise the caller's hash itself, read only, or nil when none was passed.
    fn extract_kwhash(&mut self, fx: &mut Fx, npos_required: usize, maxpos: i64, nreq2: usize, consume: bool) -> Value {
        let argc = fx.b.use_var(fx.argc_v);
        let enough = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, argc, npos_required as i64);
        let check = fx.b.create_block();
        let take = fx.b.create_block();
        let none = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(enough, check, &[], none, &[]);
        fx.b.switch_to_block(check);
        let last_i = fx.b.ins().iadd_imm_s(argc, -1);
        let last = self.arg_at(fx, last_i);
        let is_kw = if self.in_dependency() || self.eval_unit {
            let argc = fx.b.use_var(fx.argc_v);
            let nr = self.iconst(fx, nreq2 as i64);
            self.call(fx, "kn_take_kwhash2", &[last, argc, nr])
        } else if maxpos >= 0 {
            let argc = fx.b.use_var(fx.argc_v);
            let mp = self.iconst(fx, maxpos);
            self.call(fx, "kn_take_kwhash", &[last, argc, mp])
        } else {
            self.call(fx, "kn_is_kwhash", &[last])
        };
        fx.b.ins().brif(is_kw, take, &[], none, &[]);
        fx.b.switch_to_block(take);
        let h = if consume { self.call(fx, "kn_kw_dup", &[last]) } else { last };
        let argc = fx.b.use_var(fx.argc_v);
        let argc2 = fx.b.ins().iadd_imm_s(argc, -1);
        fx.b.def_var(fx.argc_v, argc2);
        fx.b.ins().jump(done, &[h.into()]);
        fx.b.switch_to_block(none);
        let nil = self.nil(fx);
        let h = if consume { self.call(fx, "kn_kw_dup", &[nil]) } else { nil };
        fx.b.ins().jump(done, &[h.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    fn bind_keywords(&mut self, fx: &mut Fx, h: Value, kws: &[Node], kwrest: Option<&Node>) {
            if kwrest.and_then(|k| k.as_keyword_rest_parameter_node()).is_none() {
                return self.bind_keywords_read(fx, h, kws);
            }
            for k in kws {
                let (name, default) = if let Some(r) = k.as_required_keyword_parameter_node() {
                    (r.name().as_slice().to_vec(), None)
                } else if let Some(o) = k.as_optional_keyword_parameter_node() {
                    (o.name().as_slice().to_vec(), Some(o.value()))
                } else {
                    continue;
                };
                let key = name.strip_suffix(b":").unwrap_or(&name).to_vec();
                let sk = self.sym_val(fx, &key);
                let v = self.call(fx, "kn_kw_take", &[h, sk]);
                let missing = fx.b.ins().icmp_imm_s(IntCC::Equal, v, KW_MISSING);
                let yes = fx.b.create_block();
                let no = fx.b.create_block();
                let done = fx.b.create_block();
                fx.b.ins().brif(missing, yes, &[], no, &[]);
                fx.b.switch_to_block(yes);
                match default {
                    Some(d) => {
                        let dv = self.expr(fx, &d);
                        self.local_set(fx, &key, 0, dv);
                    }
                    None => {
                        let e = self.call(fx, "kn_kw_missing", &[sk]);
                        self.check(fx, e);
                    }
                }
                fx.b.ins().jump(done, &[]);
                fx.b.switch_to_block(no);
                self.local_set(fx, &key, 0, v);
                fx.b.ins().jump(done, &[]);
                fx.b.switch_to_block(done);
            }
            match kwrest.and_then(|k| k.as_keyword_rest_parameter_node()) {
                Some(kr) => {
                    let n = kr.name().map(|n| n.as_slice().to_vec()).unwrap_or(b"**".to_vec());
                    self.local_set(fx, &n, 0, h);
                }
                None => {
                    self.callc(fx, "kn_kw_assert_empty", &[h]);
                }
            }
    }

    /// Keyword parameters without `**rest`: each read from the caller's keyword hash `h`
    /// (nil when none was passed) without copying it; any key that none of them matched
    /// (the count found is less than the hash's size) is an unknown keyword.
    fn bind_keywords_read(&mut self, fx: &mut Fx, h: Value, kws: &[Node]) {
        let found = fx.b.declare_var(I64);
        let zero = self.iconst(fx, 0);
        fx.b.def_var(found, zero);
        let mut keys = Vec::new();
        for k in kws {
            let (name, default) = if let Some(r) = k.as_required_keyword_parameter_node() {
                (r.name().as_slice().to_vec(), None)
            } else if let Some(o) = k.as_optional_keyword_parameter_node() {
                (o.name().as_slice().to_vec(), Some(o.value()))
            } else {
                continue;
            };
            let key = name.strip_suffix(b":").unwrap_or(&name).to_vec();
            keys.push(key.clone());
            let sk = self.sym_val(fx, &key);
            let v = self.call(fx, "kn_kw_get", &[h, sk]);
            let missing = fx.b.ins().icmp_imm_s(IntCC::Equal, v, KW_MISSING);
            let yes = fx.b.create_block();
            let no = fx.b.create_block();
            let done = fx.b.create_block();
            fx.b.ins().brif(missing, yes, &[], no, &[]);
            fx.b.switch_to_block(yes);
            match default {
                Some(d) => {
                    let dv = self.expr(fx, &d);
                    self.local_set(fx, &key, 0, dv);
                }
                None => {
                    let e = self.call(fx, "kn_kw_missing", &[sk]);
                    self.check(fx, e);
                }
            }
            fx.b.ins().jump(done, &[]);
            fx.b.switch_to_block(no);
            self.local_set(fx, &key, 0, v);
            let n = fx.b.use_var(found);
            let n1 = fx.b.ins().iadd_imm_s(n, 1);
            fx.b.def_var(found, n1);
            fx.b.ins().jump(done, &[]);
            fx.b.switch_to_block(done);
        }
        let n = fx.b.use_var(found);
        let total = self.call(fx, "kn_kw_count", &[h]);
        let extra = fx.b.ins().icmp(IntCC::NotEqual, n, total);
        let bad = fx.b.create_block();
        let ok = fx.b.create_block();
        fx.b.set_cold_block(bad);
        fx.b.ins().brif(extra, bad, &[], ok, &[]);
        fx.b.switch_to_block(bad);
        let syms: Vec<Value> = keys.iter().map(|k| self.sym_val(fx, k)).collect();
        let buf = if syms.is_empty() { self.iconst(fx, 8) } else { self.args_buf(fx, &syms) };
        let nk = self.iconst(fx, syms.len() as i64);
        let e = self.call(fx, "kn_kw_unknown", &[h, buf, nk]);
        self.check(fx, e);
        fx.b.ins().jump(ok, &[]);
        fx.b.switch_to_block(ok);
    }

    // ---------- block parameters ----------

    pub(crate) fn bind_block_params(&mut self, fx: &mut Fx, params: Option<&Node>) -> BlockArity {
        let Some(pn) = params else {
            return BlockArity { min: 0, max: 0, autosplat: false };
        };
        if let Some(np) = pn.as_numbered_parameters_node() {
            let max = np.maximum() as usize;
            for i in 0..max {
                let v = self.arg_or_nil(fx, i);
                self.local_set(fx, format!("_{}", i + 1).as_bytes(), 0, v);
            }
            return BlockArity { min: max as i64, max: max as i64, autosplat: max > 1 };
        }
        if pn.as_it_parameters_node().is_some() {
            let v = self.arg_or_nil(fx, 0);
            self.local_set(fx, b"it", 0, v);
            return BlockArity { min: 1, max: 1, autosplat: false };
        }
        let Some(bp) = pn.as_block_parameters_node() else {
            return BlockArity { min: 0, max: -1, autosplat: false };
        };
        let Some(p) = bp.parameters() else {
            return BlockArity { min: 0, max: 0, autosplat: false };
        };
        let reqs: Vec<Node> = p.requireds().iter().collect();
        let opts: Vec<Node> = p.optionals().iter().collect();
        let posts: Vec<Node> = p.posts().iter().collect();
        let kws: Vec<Node> = p.keywords().iter().collect();
        let kwrest = p.keyword_rest();
        let rest = p.rest();
        let (nreq, nopt, npost) = (reqs.len(), opts.len(), posts.len());
        let has_kw = !kws.is_empty() || kwrest.as_ref().is_some_and(|k| k.as_keyword_rest_parameter_node().is_some());
        let consume = kwrest.as_ref().is_some_and(|k| k.as_keyword_rest_parameter_node().is_some());
        let kwh = if has_kw { Some(self.extract_kwhash(fx, 0, -1, nreq + npost, consume)) } else { None };
        let has_rest = rest.is_some();
        let implicit_rest = rest.as_ref().is_some_and(|r| r.as_implicit_rest_node().is_some());
        if !has_rest && npost == 0 {
            for (i, r) in reqs.iter().enumerate() {
                let v = self.arg_or_nil(fx, i);
                self.bind_param_target(fx, r, v);
            }
            for (j, o) in opts.iter().enumerate() {
                let o = o.as_optional_parameter_node().unwrap();
                self.optional_param(fx, o.name().as_slice(), nreq + j, 0, &o.value(), None);
            }
        } else {
            let argc = fx.b.use_var(fx.argc_v);
            let argv = fx.b.use_var(fx.argv_v);
            let ary = self.call(fx, "kn_ary_new", &[argc, argv]);
            for (i, r) in reqs.iter().enumerate() {
                let iv = self.iconst(fx, i as i64);
                let v = self.call(fx, "kn_ary_ref", &[ary, iv]);
                self.bind_param_target(fx, r, v);
            }
            for (j, o) in opts.iter().enumerate() {
                let o = o.as_optional_parameter_node().unwrap();
                self.optional_param(fx, o.name().as_slice(), nreq + j, npost, &o.value(), Some(ary));
            }
            if let Some(rp) = rest.as_ref().and_then(|r| r.as_rest_parameter_node()) {
                if let Some(n) = rp.name() {
                    let st = self.iconst(fx, (nreq + nopt) as i64);
                    let po = self.iconst(fx, npost as i64);
                    let a = self.call(fx, "kn_ary_slice_rest", &[ary, st, po]);
                    self.local_set(fx, n.as_slice(), 0, a);
                }
            }
            for (k, pn) in posts.iter().enumerate() {
                let rq = self.iconst(fx, nreq as i64);
                let op = self.iconst(fx, (nopt | if has_rest { 1 << 32 } else { 0 }) as i64);
                let po = self.iconst(fx, npost as i64);
                let kk = self.iconst(fx, k as i64);
                let v = self.call(fx, "kn_block_post_ref", &[ary, rq, op, po, kk]);
                self.bind_param_target(fx, pn, v);
            }
        }
        if let Some(h) = kwh {
            self.bind_keywords(fx, h, &kws, kwrest.as_ref());
        }
        if let Some(bpp) = p.block() {
            if let Some(n) = bpp.name() {
                let b = fx.b.use_var(fx.blk_v);
                self.local_set(fx, n.as_slice(), 0, b);
            }
        }
        let positional = nreq + nopt + npost;
        BlockArity {
            min: (nreq + npost) as i64,
            max: if has_rest && !implicit_rest { -1 } else { positional as i64 },
            autosplat: positional > 1 || has_rest,
        }
    }

    // ---------- definitions ----------

    pub(crate) fn def_node(&mut self, fx: &mut Fx, d: &DefNode) -> Value {
        let name = d.name().as_slice().to_vec();
        let (target, kind) = match d.receiver() {
            Some(r) => (self.expr(fx, &r), 3),
            None => {
                let s = self.self_val(fx);
                let k = match fx.kind {
                    ScopeKind::Class => 0,
                    ScopeKind::Top if !self.eval_unit || self.file_unit => 1,
                    _ => 2,
                };
                (s, k)
            }
        };
        let bt_label = self.method_label(fx, &name, d.receiver().as_ref());
        let parts = DefParts {
            name,
            target,
            kind,
            params: d.parameters(),
            body: d.body(),
            locals: names(&d.locals()),
            offset: d.location().start_offset(),
            singleton: d.receiver().is_some(),
            bt_label,
        };
        self.def_parts(fx, parts)
    }

    /// `define_method(:name) { |a| ... }` that can be a `def` (statics.rs `block_def`):
    /// compiled as one, defined by `kn_def` while `define_method` is the builtin (else the
    /// call as written).
    pub(crate) fn def_from_block(&mut self, fx: &mut Fx, c: &CallNode) -> Option<Value> {
        if !matches!(fx.kind, ScopeKind::Top | ScopeKind::Class) || !fx.inline_levels.is_empty() || !self.inline_ctx.is_empty() {
            return None;
        }
        let (name, params, blk) = crate::statics::block_def(c)?;
        let parent = self.fn_labels.last().cloned().unwrap_or_else(|| "<main>".into());
        let target = self.self_val(fx);
        let parts = DefParts {
            name,
            target,
            kind: 4,
            params,
            body: blk.body(),
            locals: names(&blk.locals()),
            offset: c.location().start_offset(),
            singleton: false,
            bt_label: Some(crate::statics::block_label(&parent)),
        };
        let r = self.def_parts(fx, parts);
        // 0: `define_method` isn't the builtin one here
        let generic = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(generic);
        fx.b.ins().brif(r, done, &[r.into()], generic, &[]);
        fx.b.switch_to_block(generic);
        fx.no_block_def = true;
        let v = self.call_node(fx, c);
        fx.no_block_def = false;
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(done);
        Some(fx.b.block_params(done)[0])
    }

    fn def_parts(&mut self, fx: &mut Fx, d: DefParts) -> Value {
        let DefParts { name, target, kind, params, body, locals, offset, singleton, bt_label } = d;
        let wrapper_label = bt_label.clone().unwrap_or_else(|| String::from_utf8_lossy(&name).into_owned());
        let owner_slot = self.cache_alloc(1);
        let params_node = params.as_ref().map(|p| p.as_node());
        let params_copy = params_node.as_ref().and_then(|p| p.as_parameters_node());
        let mut analysis = Vec::new();
        if let Some(p) = &params_node {
            analysis.push(p);
        }
        if let Some(b) = &body {
            analysis.push(b);
        }
        let spec = FnSpec {
            kind: ScopeKind::Method,
            lambda: false,
            label: String::from_utf8_lossy(&name).into_owned(),
            locals,
            analysis,
            // constants resolve lexically: the enclosing class body's scope
            cref_slot: fx.cref_slot,
            method: Some(MethodCtx {
                name: name.clone(),
                owner_slot,
                hops: None,
                params: Vec::new(),
                scope: self.scopes.len(),
                lex: self.def_lex.get(&(self.file.clone(), offset)).cloned(),
                singleton,
            }),
            fast_arity: None,
            predeclared: None,
            bt_label,
            bmethod: false,
            ret_hops: None,
            entry_check: false,
        };
        // only required parameters (and maybe &blk): a fast entry taking them in registers
        // (the app's own code: in gems the extra entry and call guards cost more code than
        // they save time)
        let fast_n = crate::statics::fast_arity(&params).filter(|_| !self.in_dependency());
        let mut spec = spec;
        spec.fast_arity = fast_n;
        if fast_n.is_some() && !singleton {
            spec.predeclared = self.static_fid_for_def(&name, offset);
        }
        // calls to itself: checked by a word (statics.rs `self_call`), which the def hands over
        let claim = match (spec.predeclared, fast_n, &body) {
            (Some(fid), Some(n), Some(b)) if crate::statics::calls_itself(&name, n, b) => {
                spec.entry_check = true;
                Some(self.claim_word_id(fid))
            }
            _ => None,
        };
        // a small integer method calling itself: an integer copy too (ispec.rs)
        let ispec = match (spec.predeclared, &claim, &body) {
            (Some(fid), Some(_), Some(b)) => crate::ispec::eligible(&name, &params, b).map(|ps| (fid, self.ispec_build(fid, &name, &ps, b))),
            _ => None,
        };
        let def_line = self.line_of(offset) as u32;
        // (code inlined into this body is compiled with no inline context of its own)
        let saved_ctx = std::mem::take(&mut self.inline_ctx);
        self.def_stack.push((self.file.clone(), offset));
        self.def_cref.insert((self.file.clone(), offset), fx.cref_slot);
        let fid = self.build_fn(spec, |c, fx| {
            c.bind_method_params(fx, params);
            if let Some((fid, copy)) = ispec {
                let args = fx.fast_args.clone().unwrap_or_default();
                c.ispec_enter(fx, fid, copy, &args);
            }
            match &body {
                Some(b) => c.expr(fx, b),
                None => c.nil(fx),
            }
        });
        self.def_stack.pop();
        self.inline_ctx = saved_ctx;
        let (fid, fast) = match fast_n {
            Some(n) => {
                let w = self.fast_wrapper(fid, n, &wrapper_label, def_line);
                let fa = self.fn_addr(fx, fid);
                let tag = fx.b.ins().iconst(I64, ((n as i64) + 1) << 56);
                let tagged = fx.b.ins().bor(fa, tag);
                (w, tagged)
            }
            None => (fid, self.iconst(fx, 0)),
        };
        let addr = self.fn_addr(fx, fid);
        let k = self.iconst(fx, kind);
        let s = self.sym_val(fx, &name);
        let slot = self.slot_ptr(fx, owner_slot);
        let mut desc = params_desc(params_copy).into_bytes();
        desc.push(0);
        // (a keyword def's calls may be inlined: statics.rs `kw_marker`)
        if !self.in_dependency() {
            desc.extend(self.kw_marker(offset).unwrap_or_default());
        }
        let (pd, _) = self.str_ptr(fx, &desc);
        match claim {
            Some(id) => {
                let gv = self.module.declare_data_in_func(id, fx.b.func);
                let w = fx.b.ins().symbol_value(I64, gv);
                self.callc(fx, "kn_def_claim", &[target, k, s, addr, slot, pd, fast, w])
            }
            None => self.callc(fx, "kn_def", &[target, k, s, addr, slot, pd, fast]),
        }
    }

    /// This frame's env for a closure made here.
    fn frame_env(&mut self, fx: &mut Fx) -> Value {
        match fx.env_v {
            Some(_) if fx.lazy_env => self.lazy_env(fx),
            Some(e) => fx.b.use_var(e),
            None => self.iconst(fx, 0),
        }
    }

    /// In inlined block bodies (`fx.inline_levels`): one environment per body, holding a
    /// copy of its variables, chained onto this frame's; their scopes are pushed for the
    /// closure being compiled (the caller pops them). Copies are enough: inline.rs `Check`
    /// lets a nested block use only variables the body never assigns.
    fn inline_envs(&mut self, fx: &mut Fx) -> Option<Value> {
        if fx.inline_levels.is_empty() {
            return None;
        }
        let mut env = self.frame_env(fx);
        let nil = self.nil(fx);
        for li in 0..fx.inline_levels.len() {
            let mut names: Vec<(Vec<u8>, Variable)> = fx.inline_levels[li].iter().map(|(n, v)| (n.clone(), *v)).collect();
            names.sort_by(|a, b| a.0.cmp(&b.0));
            let n = self.iconst(fx, names.len() as i64);
            env = self.call(fx, "kn_env_new", &[env, nil, n]);
            let mut env_slots = HashMap::new();
            for (i, (name, var)) in names.into_iter().enumerate() {
                let v = fx.b.use_var(var);
                fx.b.ins().store(mem(), v, env, ENV_SLOTS + 8 * i as i32);
                env_slots.insert(name, i);
            }
            self.scopes.push(ScopeInfo { env_slots });
        }
        Some(env)
    }

    /// Compile a block or lambda literal into a Proc value.
    pub(crate) fn block_proc(&mut self, fx: &mut Fx, params: Option<Node>, body: Option<Node>, locals: Vec<Vec<u8>>, lambda: bool) -> Value {
        // (only this literal: the blocks in its body are ordinary)
        let bmethod = std::mem::take(&mut self.bmethod_next);
        // made in an inlined block body (inline.rs `Check`): its variables copied into
        // environments of their own, between this frame's and the closure's
        let inl = self.inline_envs(fx);
        let k = fx.inline_levels.len() as u32;
        // what its `return` returns from: this frame when it is a lambda or a method body
        // block (its env, one hop up), else whatever this frame's own returns from
        let ret_hops = match fx.kind {
            ScopeKind::Block if fx.ret_here => Some(k),
            ScopeKind::Block => fx.ret_hops.map(|h| h + 1 + k),
            _ => None,
        };
        let method = fx.method.as_ref().map(|m| MethodCtx {
            hops: Some(match m.hops {
                None => k,
                Some(h) => h + 1 + k,
            }),
            params: if m.hops.is_none() { fx.zsuper.clone() } else { m.params.clone() },
            ..m.clone()
        });
        let mut analysis = Vec::new();
        if let Some(p) = &params {
            analysis.push(p);
        }
        if let Some(b) = &body {
            analysis.push(b);
        }
        let spec = FnSpec {
            kind: ScopeKind::Block,
            lambda,
            label: "block".into(),
            locals,
            analysis,
            cref_slot: fx.cref_slot,
            method,
            fast_arity: None,
            predeclared: None,
            bt_label: None,
            bmethod,
            ret_hops,
            entry_check: false,
        };
        let mut arity = BlockArity { min: 0, max: -1, autosplat: false };
        let fid = {
            let arity_ref = &mut arity;
            self.build_fn(spec, |c, fx| {
                *arity_ref = c.bind_block_params(fx, params.as_ref());
                let start = fx.b.create_block();
                fx.b.ins().jump(start, &[]);
                fx.b.switch_to_block(start);
                fx.redo_block = Some(start);
                match &body {
                    Some(b) => c.expr(fx, b),
                    None => c.nil(fx),
                }
            })
        };
        let desc = block_params_desc(params.as_ref());
        if !desc.is_empty() && !self.eval_unit {
            self.block_params.push((fid, desc));
        }
        for _ in 0..k {
            self.scopes.pop();
        }
        let addr = self.fn_addr(fx, fid);
        let env = match inl {
            Some(e) => e,
            None => self.frame_env(fx),
        };
        let s = self.self_val(fx);
        let min = self.iconst(fx, arity.min);
        // the real maximum for procs too: Proc#arity needs it, and define_method checks it
        // (calling a non-lambda proc itself stays lenient at runtime)
        let max = self.iconst(fx, arity.max);
        let takes_kw = params.as_ref().and_then(|pn| pn.as_block_parameters_node()).and_then(|bp| bp.parameters()).is_some_and(|p| {
            p.keywords().iter().next().is_some() || p.keyword_rest().is_some_and(|k| k.as_keyword_rest_parameter_node().is_some())
        });
        let flags = self.iconst(fx, (arity.autosplat as i64) | if lambda { 2 } else { 0 } | if takes_kw { 4 } else { 0 });
        self.call(fx, "kn_proc_new", &[addr, env, s, min, max, flags])
    }

    pub(crate) fn class_body(&mut self, fx: &mut Fx, target: Value, label: &str, locals: Vec<Vec<u8>>, body: Option<Node>) -> Value {
        let slot = self.cache_alloc(2);
        self.cref_parents.insert(slot, fx.cref_slot);
        let spec = FnSpec {
            kind: ScopeKind::Class,
            lambda: false,
            label: label.to_string(),
            locals,
            analysis: body.iter().collect(),
            cref_slot: slot,
            method: None,
            fast_arity: None,
            predeclared: None,
            bt_label: None,
            bmethod: false,
            ret_hops: None,
            entry_check: false,
        };
        let fid = self.build_fn(spec, |c, fx| match &body {
            Some(b) => c.expr(fx, b),
            None => c.nil(fx),
        });
        let zero = self.iconst(fx, 0);
        let nil = self.nil(fx);
        // `private` with no arguments lasts until the end of this class body
        self.call(fx, "kn_class_enter", &[target]);
        let r = self.call_fn(fx, fid, &[target, zero, zero, nil]);
        self.call(fx, "kn_class_leave", &[target]);
        self.check(fx, r);
        r
    }

    fn const_target(&mut self, fx: &mut Fx, path: &Node) -> (Value, Vec<u8>) {
        if let Some(cp) = path.as_constant_path_node() {
            let outer = match cp.parent() {
                Some(p) => self.expr(fx, &p),
                None => self.call(fx, "kn_object_class", &[]),
            };
            let name = cp.name().map(|n| n.as_slice().to_vec()).unwrap_or_default();
            return (outer, name);
        }
        let name = path.as_constant_read_node().map(|c| c.name().as_slice().to_vec()).unwrap_or_default();
        let cref = self.cref(fx);
        (cref, name)
    }

    pub(crate) fn class_node(&mut self, fx: &mut Fx, c: &ClassNode) -> Value {
        let (outer, name) = self.const_target(fx, &c.constant_path());
        let sup = match c.superclass() {
            Some(s) => self.expr(fx, &s),
            None => self.iconst(fx, UNDEF),
        };
        let s = self.sym_val(fx, &name);
        let cls = self.callc(fx, "kn_define_class", &[outer, s, sup]);
        let lex = self.lex_class_name(&c.constant_path());
        self.lex_classes.push(lex.map(|n| (n, false)));
        let r = self.class_body(fx, cls, &format!("<class:{}>", String::from_utf8_lossy(&name)), names(&c.locals()), c.body());
        self.lex_classes.pop();
        r
    }
    pub(crate) fn module_node(&mut self, fx: &mut Fx, m: &ModuleNode) -> Value {
        let (outer, name) = self.const_target(fx, &m.constant_path());
        let s = self.sym_val(fx, &name);
        let md = self.callc(fx, "kn_define_module", &[outer, s]);
        let lex = self.lex_class_name(&m.constant_path());
        self.lex_classes.push(lex.map(|n| (n, false)));
        let r = self.class_body(fx, md, &format!("<module:{}>", String::from_utf8_lossy(&name)), names(&m.locals()), m.body());
        self.lex_classes.pop();
        r
    }
    pub(crate) fn sclass_node(&mut self, fx: &mut Fx, s: &SingletonClassNode) -> Value {
        let target = self.expr(fx, &s.expression());
        let sc = self.callc(fx, "kn_singleton_class_for", &[target]);
        let cref = self.cref(fx);
        self.call(fx, "kn_set_lexical_parent", &[sc, cref]);
        // `class << self` in a class body: its methods are that class's singleton methods
        let lex = match self.lex_classes.last() {
            Some(Some((n, false))) if s.expression().as_self_node().is_some() && fx.kind == ScopeKind::Class => Some((n.clone(), true)),
            _ => None,
        };
        self.lex_classes.push(lex);
        let r = self.class_body(fx, sc, "singleton class", names(&s.locals()), s.body());
        self.lex_classes.pop();
        r
    }

    /// The name of the class a `class` / `module` statement opens, as CRuby would name it:
    /// `Foo::Bar` inside `module A` is `A::Foo::Bar`, `::Foo` is `Foo`.
    fn lex_class_name(&self, path: &Node) -> Option<String> {
        fn written(n: &Node) -> Option<(String, bool)> {
            if let Some(r) = n.as_constant_read_node() {
                return Some((String::from_utf8_lossy(r.name().as_slice()).into_owned(), false));
            }
            let cp = n.as_constant_path_node()?;
            let name = String::from_utf8_lossy(cp.name()?.as_slice()).into_owned();
            match cp.parent() {
                None => Some((name, true)),
                Some(p) => {
                    let (outer, abs) = written(&p)?;
                    Some((format!("{outer}::{name}"), abs))
                }
            }
        }
        let (w, absolute) = written(path)?;
        if absolute {
            return Some(w);
        }
        match self.lex_classes.last() {
            Some(Some((outer, false))) if outer != "Object" => Some(format!("{outer}::{w}")),
            Some(Some((_, false))) | None => Some(w),
            _ => None,
        }
    }

    /// CRuby 3.4's backtrace label of a method `def`d here: `C#m`, `C.m` (`def self.m`, or
    /// in `class << self`), `Object#m` at the top level; None (just `m`) where the class
    /// isn't known statically.
    fn method_label(&self, fx: &Fx, name: &[u8], receiver: Option<&Node>) -> Option<String> {
        let m = String::from_utf8_lossy(name);
        let top = fx.kind == ScopeKind::Top && (!self.eval_unit || self.file_unit) && self.lex_classes.is_empty();
        match (receiver, fx.kind) {
            (Some(r), ScopeKind::Class) if r.as_self_node().is_some() => match self.lex_classes.last() {
                Some(Some((c, false))) => Some(format!("{c}.{m}")),
                _ => None,
            },
            (None, ScopeKind::Class) => match self.lex_classes.last() {
                Some(Some((c, false))) => Some(format!("{c}#{m}")),
                Some(Some((c, true))) => Some(format!("{c}.{m}")),
                _ => None,
            },
            (None, ScopeKind::Top) if top => Some(format!("Object#{m}")),
            _ => None,
        }
    }
}

/// Register shadows for a scope's numeric locals: (unboxed Float shadows with a kind,
/// f64 registers of always-Float locals, always-Integer locals). See analysis.rs
/// `numeric_locals` / `typed_locals`.
#[allow(clippy::type_complexity)]
pub(crate) fn typed_setup(
    b: &mut FunctionBuilder,
    locals: &HashMap<Vec<u8>, Local>,
    nodes: &[&Node],
    typed: bool,
) -> (HashMap<Vec<u8>, (Variable, Variable)>, HashMap<Vec<u8>, (Variable, Option<Variable>)>, HashSet<Vec<u8>>, std::rc::Rc<analysis::Typed>) {
    let info = std::rc::Rc::new(if typed && std::env::var_os("KENNY_NO_TYPED").is_none() {
        analysis::typed_locals(nodes)
    } else {
        analysis::Typed { types: HashMap::new(), boxed: HashSet::new(), refs: HashMap::new(), bad: HashSet::new(), seed: HashMap::new(), float_arrays: HashSet::new() }
    });
    let (types, boxed) = (&info.types, &info.boxed);
    if std::env::var_os("KENNY_DEBUG_TYPED").is_some() {
        let mut v: Vec<String> = types.iter().map(|(n, t)| format!("{}:{:?}{}", String::from_utf8_lossy(n), t, if boxed.contains(n) { "+box" } else { "" })).collect();
        v.sort();
        eprintln!("typed: {}", v.join(" "));
    }
    let mut num_locals = HashMap::new();
    let mut flocals = HashMap::new();
    let mut int_sure = HashSet::new();
    for (n, t) in types {
        if !matches!(locals.get(n), Some(Local::Var(_))) {
            continue;
        }
        match t {
            analysis::Ty::Float => {
                let raw = b.declare_var(ir::types::F64);
                let z = b.ins().f64const(0.0);
                b.def_var(raw, z);
                let cache = boxed.contains(n).then(|| {
                    let cache = b.declare_var(I64);
                    let z = b.ins().iconst(I64, 0);
                    b.def_var(cache, z);
                    cache
                });
                flocals.insert(n.clone(), (raw, cache));
            }
            analysis::Ty::Int => {
                int_sure.insert(n.clone());
            }
        }
    }
    for n in analysis::numeric_locals(nodes) {
        if matches!(locals.get(&n), Some(Local::Var(_))) && !flocals.contains_key(&n) {
            let raw = b.declare_var(ir::types::F64);
            let z = b.ins().f64const(0.0);
            b.def_var(raw, z);
            let kind = b.declare_var(ir::types::I8);
            let k = b.ins().iconst(ir::types::I8, 0);
            b.def_var(kind, k);
            num_locals.insert(n, (raw, kind));
        }
    }
    if std::env::var_os("KENNY_DEBUG_TYPED").is_some() && !num_locals.is_empty() {
        let mut v: Vec<String> = num_locals.keys().map(|n| String::from_utf8_lossy(n).into_owned()).collect();
        v.sort();
        eprintln!("untyped numeric: {}", v.join(" "));
    }
    (num_locals, flocals, int_sure, info)
}
