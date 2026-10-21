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
}

pub(crate) struct BlockArity {
    pub min: i64,
    pub max: i64,
    pub autosplat: bool,
}

/// Method#parameters descriptor: "req:a,opt:b,rest:c,keyreq:d,key:e,keyrest:f,block:g".
fn params_desc(p: Option<ParametersNode>) -> String {
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
        let prog = node.as_program_node().unwrap();
        let stmts = prog.statements().as_node();
        let slot = self.cache_alloc(2);
        let spec = FnSpec {
            kind,
            lambda: false,
            label: format!("top_{}", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()),
            locals: names(&prog.locals()),
            analysis: vec![&stmts],
            cref_slot: slot,
            method: None,
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
        let sig = if is_block { self.block_sig() } else { self.method_sig() };
        // the frame's name in backtraces, as CRuby labels it
        let bt_label = match spec.kind {
            ScopeKind::Top | ScopeKind::Class if self.eval_unit && !self.file_unit && self.fn_labels.is_empty() => "<eval>".to_string(),
            ScopeKind::Top if self.fn_labels.is_empty() && self.file == self.main_file => "<main>".to_string(),
            ScopeKind::Top => "<top (required)>".to_string(),
            ScopeKind::Block => {
                let parent = self.fn_labels.last().map(|l| l.trim_start_matches("block in ").to_string()).unwrap_or_else(|| "<main>".into());
                format!("block in {parent}")
            }
            _ => spec.label.clone(),
        };
        self.fn_labels.push(bt_label.clone());
        self.fn_counter += 1;
        let label: String = spec.label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let name = format!("__kn_{}_{}", self.fn_counter, label);
        let id = self.module.declare_function(&name, Linkage::Local, &sig).expect("declare fn");
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
            let (self_v, argc_v, argv_v, blk_v, proc_v) = if is_block {
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
            let mut num_locals = HashMap::new();
            for n in analysis::numeric_locals(&spec.analysis) {
                if matches!(locals.get(&n), Some(Local::Var(_))) {
                    let raw = b.declare_var(ir::types::F64);
                    let z = b.ins().f64const(0.0);
                    b.def_var(raw, z);
                    let kind = b.declare_var(ir::types::I8);
                    let k = b.ins().iconst(ir::types::I8, 0);
                    b.def_var(kind, k);
                    num_locals.insert(n, (raw, kind));
                }
            }
            let env_v = if has_env {
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
            if spec.kind == ScopeKind::Method {
                let tag = match env_v {
                    Some(e) => b.use_var(e),
                    None => {
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
                num_locals,
            };
            let v = body(self, &mut fx);
            fx.b.ins().return_(&[v]);
            // unwinding out of this function
            fx.b.switch_to_block(exit);
            if fx.kind == ScopeKind::Method {
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
            match fx.locals.get(name) {
                Some(Local::Var(v)) => {
                    let v = *v;
                    if let Some(&(raw, kind)) = fx.num_locals.get(name) {
                        return self.local_box(fx, v, raw, kind);
                    }
                    return fx.b.use_var(v);
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
            match fx.locals.get(name) {
                Some(Local::Var(v)) => {
                    let v = *v;
                    fx.b.def_var(v, val);
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
    /// before the environment) tells the runtime, so the next minor collection rescans it.
    fn env_write_barrier(&mut self, fx: &mut Fx, e: Value) {
        let flags = fx.b.ins().load(ir::types::I32, mem(), e, -4);
        let old = fx.b.ins().band_imm_u(flags, 4);
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
            if let (Some(Local::Var(v)), Some(&(raw, kind))) = (fx.locals.get(name), fx.num_locals.get(name)) {
                let v = *v;
                let boxed = fx.b.use_var(v);
                let raw = fx.b.use_var(raw);
                let kind = fx.b.use_var(kind);
                return crate::fastpath::Num::Lazy { boxed, raw, kind, lit: false };
            }
        }
        crate::fastpath::Num::Boxed(self.local_get(fx, name, depth))
    }
    /// Assign a numeric result to a local without boxing it (when it has a shadow).
    pub(crate) fn local_set_num(&mut self, fx: &mut Fx, name: &[u8], depth: u32, n: crate::fastpath::Num) {
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
        depth == fx.inline_levels.len() as u32 && fx.num_locals.contains_key(name) && matches!(fx.locals.get(name), Some(Local::Var(_)))
    }

    /// The env of the enclosing method (for yield / block_given? / return inside blocks).
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
    fn optional_param(&mut self, fx: &mut Fx, name: &[u8], idx: usize, default: &Node, from_ary: Option<Value>) {
        let argc = fx.b.use_var(fx.argc_v);
        let has = fx.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, argc, idx as i64);
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
        let kwh = if has_kw { Some(self.extract_kwhash(fx, nreq + npost, maxpos, nreq + npost)) } else { None };

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
            self.optional_param(fx, o.name().as_slice(), nreq + j, &o.value(), None);
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
    /// for the Ruby 2 rule applied to code from dependencies (see kn_take_kwhash2).
    fn extract_kwhash(&mut self, fx: &mut Fx, npos_required: usize, maxpos: i64, nreq2: usize) -> Value {
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
        let h = self.call(fx, "kn_kw_dup", &[last]);
        let argc = fx.b.use_var(fx.argc_v);
        let argc2 = fx.b.ins().iadd_imm_s(argc, -1);
        fx.b.def_var(fx.argc_v, argc2);
        fx.b.ins().jump(done, &[h.into()]);
        fx.b.switch_to_block(none);
        let nil = self.nil(fx);
        let h = self.call(fx, "kn_kw_dup", &[nil]);
        fx.b.ins().jump(done, &[h.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    fn bind_keywords(&mut self, fx: &mut Fx, h: Value, kws: &[Node], kwrest: Option<&Node>) {
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
        let kwh = if has_kw { Some(self.extract_kwhash(fx, 0, -1, nreq + npost)) } else { None };
        let has_rest = rest.is_some();
        let implicit_rest = rest.as_ref().is_some_and(|r| r.as_implicit_rest_node().is_some());
        if !has_rest && npost == 0 {
            for (i, r) in reqs.iter().enumerate() {
                let v = self.arg_or_nil(fx, i);
                self.bind_param_target(fx, r, v);
            }
            for (j, o) in opts.iter().enumerate() {
                let o = o.as_optional_parameter_node().unwrap();
                self.optional_param(fx, o.name().as_slice(), nreq + j, &o.value(), None);
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
                self.optional_param(fx, o.name().as_slice(), nreq + j, &o.value(), Some(ary));
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
                let pre = self.iconst(fx, (nreq + nopt) as i64);
                let po = self.iconst(fx, npost as i64);
                let kk = self.iconst(fx, k as i64);
                let v = self.call(fx, "kn_ary_post_ref", &[ary, pre, po, kk]);
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
        let owner_slot = self.cache_alloc(1);
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
        let params_node = d.parameters().map(|p| p.as_node());
        let body = d.body();
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
            locals: names(&d.locals()),
            analysis,
            // constants resolve lexically: the enclosing class body's scope
            cref_slot: fx.cref_slot,
            method: Some(MethodCtx { name: name.clone(), owner_slot, hops: None, params: Vec::new(), scope: self.scopes.len() }),
        };
        let params = d.parameters();
        let fid = self.build_fn(spec, |c, fx| {
            c.bind_method_params(fx, params);
            match &body {
                Some(b) => c.expr(fx, b),
                None => c.nil(fx),
            }
        });
        let addr = self.fn_addr(fx, fid);
        let k = self.iconst(fx, kind);
        let s = self.sym_val(fx, &name);
        let slot = self.slot_ptr(fx, owner_slot);
        let mut desc = params_desc(d.parameters()).into_bytes();
        desc.push(0);
        let (pd, _) = self.str_ptr(fx, &desc);
        self.callc(fx, "kn_def", &[target, k, s, addr, slot, pd])
    }

    /// Compile a block or lambda literal into a Proc value.
    pub(crate) fn block_proc(&mut self, fx: &mut Fx, params: Option<Node>, body: Option<Node>, locals: Vec<Vec<u8>>, lambda: bool) -> Value {
        let method = fx.method.as_ref().map(|m| MethodCtx {
            hops: Some(match m.hops {
                None => 0,
                Some(h) => h + 1,
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
        let addr = self.fn_addr(fx, fid);
        let env = match fx.env_v {
            Some(e) => fx.b.use_var(e),
            None => self.iconst(fx, 0),
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
        self.class_body(fx, cls, &format!("<class:{}>", String::from_utf8_lossy(&name)), names(&c.locals()), c.body())
    }
    pub(crate) fn module_node(&mut self, fx: &mut Fx, m: &ModuleNode) -> Value {
        let (outer, name) = self.const_target(fx, &m.constant_path());
        let s = self.sym_val(fx, &name);
        let md = self.callc(fx, "kn_define_module", &[outer, s]);
        self.class_body(fx, md, &format!("<module:{}>", String::from_utf8_lossy(&name)), names(&m.locals()), m.body())
    }
    pub(crate) fn sclass_node(&mut self, fx: &mut Fx, s: &SingletonClassNode) -> Value {
        let target = self.expr(fx, &s.expression());
        let sc = self.callc(fx, "kn_singleton_class_for", &[target]);
        let cref = self.cref(fx);
        self.call(fx, "kn_set_lexical_parent", &[sc, cref]);
        self.class_body(fx, sc, "singleton class", names(&s.locals()), s.body())
    }
}
