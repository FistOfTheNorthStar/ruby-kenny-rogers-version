//! Method calls, operators (with inline Integer fast paths), blocks, super and yield.

use crate::*;
use crate::fastpath::is_fast_op;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, InstBuilder};
use ruby_prism::*;

pub(crate) struct Args {
    pub vals: Vec<Value>,
    /// When any argument is a splat, all arguments are collected into this array instead.
    pub splat: Option<Value>,
}

const DROPPED_CALLS: &[(&str, &str)] = &[
    ("set_trace_func", "set_trace_func is not supported by Kenny (see docs/LANGUAGE.md)"),
    ("callcc", "callcc is not supported by Kenny"),
];

impl<M: Module> Compiler<M> {
    pub(crate) fn eval_args(&mut self, fx: &mut Fx, args: Option<ArgumentsNode>) -> Args {
        let items: Vec<Node> = match args {
            Some(a) => a.arguments().iter().collect(),
            None => vec![],
        };
        // `**h` alone passes nothing when h is empty (as in CRuby), so its argument count is
        // only known at runtime
        let splat_only_kw = |n: &Node| n.as_keyword_hash_node().is_some_and(|h| h.elements().iter().all(|e| e.as_assoc_splat_node().is_some()));
        let needs_ary = items.iter().any(|n| n.as_splat_node().is_some() || n.as_forwarding_arguments_node().is_some() || splat_only_kw(n));
        if needs_ary {
            let zero = self.iconst(fx, 0);
            let a = self.call(fx, "kn_ary_new", &[zero, zero]);
            for n in &items {
                if let Some(s) = n.as_splat_node() {
                    let v = match s.expression() {
                        Some(e) => self.expr(fx, &e),
                        None => self.forwarded_local(fx, b"*"),
                    };
                    self.callc(fx, "kn_ary_concat_splat", &[a, v]);
                } else if n.as_forwarding_arguments_node().is_some() {
                    let v = self.forwarded_local(fx, b"*");
                    self.callc(fx, "kn_ary_concat_splat", &[a, v]);
                } else if splat_only_kw(n) {
                    let v = self.expr(fx, n);
                    self.call(fx, "kn_ary_push_kw", &[a, v]);
                } else {
                    let v = self.expr(fx, n);
                    self.call(fx, "kn_ary_push", &[a, v]);
                }
            }
            return Args { vals: vec![], splat: Some(a) };
        }
        let vals = items.iter().map(|n| self.expr(fx, n)).collect();
        Args { vals, splat: None }
    }

    /// A hidden local of the enclosing method for anonymous forwarding (`*`, `**`, `&`, `...`).
    pub(crate) fn forwarded_local(&mut self, fx: &mut Fx, name: &[u8]) -> Value {
        let depth = match (fx.kind, &fx.method) {
            (ScopeKind::Block, Some(m)) => (fx.scope - m.scope) as u32,
            _ => 0,
        };
        let depth = depth + fx.inline_levels.len() as u32;
        self.local_get(fx, name, depth)
    }

    /// `blk` None: the call passes no block at all.
    pub(crate) fn send_raw(&mut self, fx: &mut Fx, recv: Value, name: &[u8], args: &Args, blk: Option<Value>, explicit: bool) -> Value {
        self.mark_loc(fx);
        let cache = self.owned_cache_ptr(fx, OWNED_CALL);
        let mid = self.sym_val(fx, name);
        if blk.is_none() && args.splat.is_none() && args.vals.len() <= 2 && self.refine_scope.is_none() {
            return self.send_simple(fx, cache, recv, name, mid, &args.vals, explicit);
        }
        let blk_given = blk;
        let blk = match blk {
            Some(b) => b,
            None => self.nil(fx),
        };
        if let Some(scope) = self.refine_scope {
            // call sites after `using`: refinement-aware lookup (other sites pay nothing)
            let sc = self.iconst(fx, scope as i64);
            let (n, buf) = match args.splat {
                Some(a) => {
                    let n = self.call(fx, "kn_ary_len", &[a]);
                    let p = self.call(fx, "kn_ary_ptr", &[a]);
                    (n, p)
                }
                None => (self.iconst(fx, args.vals.len() as i64), self.args_buf(fx, &args.vals)),
            };
            return self.call(fx, "kn_send_refined", &[cache, sc, recv, mid, n, buf, blk]);
        }
        match args.splat {
            Some(a) => self.call(fx, "kn_send_splat", &[cache, recv, mid, a, blk]),
            None => {
                let no_blk = blk_given.is_none();
                if no_blk {
                    // (3+ arguments; fewer go through `send_simple`)
                    let vals = args.vals.clone();
                    let stub = if explicit { crate::fastpath::STUB_SEND_PUB_N } else { crate::fastpath::STUB_SEND_N };
                    let dynamic = |c: &mut Self, fx: &mut Fx| {
                        let n = c.iconst(fx, vals.len() as i64);
                        let buf = c.args_buf(fx, &vals);
                        c.call_stub(fx, stub, &[cache, recv, mid, n, buf, blk])
                    };
                    if let Some(r) = self.inline_call(fx, recv, name, mid, &args.vals, explicit, dynamic) {
                        return r;
                    }
                }
                if !explicit && no_blk {
                    // (the argument buffer only on the way that uses it)
                    let vals = args.vals.clone();
                    let dynamic = |c: &mut Self, fx: &mut Fx| {
                        let n = c.iconst(fx, vals.len() as i64);
                        let buf = c.args_buf(fx, &vals);
                        c.call_stub(fx, crate::fastpath::STUB_SEND_N, &[cache, recv, mid, n, buf, blk])
                    };
                    if let Some(r) = self.static_call(fx, recv, name, mid, &args.vals, dynamic) {
                        return r;
                    }
                }
                let n = self.iconst(fx, args.vals.len() as i64);
                let buf = self.args_buf(fx, &args.vals);
                // `obj.foo` must not reach a private method; `foo` / `self.foo` may.
                // Through the direct-call stub when the name may be a Ruby method
                // (not core iterators taking a block, like `each`).
                let args6 = [cache, recv, mid, n, buf, blk];
                let core_iter = matches!(
                    name,
                    b"each" | b"map" | b"times" | b"each_with_index" | b"select" | b"filter" | b"reject" | b"each_with_object"
                        | b"inject" | b"reduce" | b"any?" | b"all?" | b"find" | b"detect" | b"sort_by" | b"each_pair"
                        | b"upto" | b"downto" | b"step" | b"loop" | b"tap" | b"then" | b"collect" | b"flat_map" | b"group_by"
                        | b"each_key" | b"each_value" | b"count" | b"sum" | b"min_by" | b"max_by" | b"each_slice" | b"zip"
                        | b"new" | b"define_method" | b"instance_eval" | b"class_eval" | b"fetch" | b"synchronize"
                );
                if crate::fastpath::may_be_ruby_method(name) && !core_iter {
                    self.call_stub(fx, if explicit { crate::fastpath::STUB_SEND_PUB_N } else { crate::fastpath::STUB_SEND_N }, &args6)
                } else {
                    self.call(fx, if explicit { "kn_send_pub" } else { "kn_send" }, &args6)
                }
            }
        }
    }
    /// Send with unwinding on exceptions.
    pub(crate) fn send(&mut self, fx: &mut Fx, recv: Value, name: &[u8], args: &[Value], blk: Option<Value>) -> Value {
        let a = Args { vals: args.to_vec(), splat: None };
        let r = self.send_raw(fx, recv, name, &a, blk, false);
        self.check(fx, r);
        r
    }

    /// `recv.name(arg) { literal block }` as a normal call (the fallback of an inlined block).
    pub(crate) fn send_with_block(&mut self, fx: &mut Fx, recv: Value, name: &[u8], arg: Option<Value>, p: Value) -> Value {
        let args = Args { vals: arg.into_iter().collect(), splat: None };
        let r = self.send_raw(fx, recv, name, &args, Some(p), true);
        self.catch_break(fx, r, p)
    }

    /// After calling with a literal block: a `break` inside that block lands here.
    fn catch_break(&mut self, fx: &mut Fx, r: Value, proc_v: Value) -> Value {
        let is_undef = fx.b.ins().icmp_imm_s(IntCC::Equal, r, UNDEF);
        let handler = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.ins().brif(is_undef, handler, &[], done, &[r.into()]);
        fx.b.switch_to_block(handler);
        let r2 = self.call(fx, "kn_catch_break", &[proc_v]);
        let still = fx.b.ins().icmp_imm_s(IntCC::Equal, r2, UNDEF);
        let target = *fx.unwind.last().unwrap();
        fx.b.ins().brif(still, target, &[], done, &[r2.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    /// The block argument of a call: (value, literal proc for break handling).
    fn block_arg(&mut self, fx: &mut Fx, block: Option<Node>, lambda: bool) -> (Value, Option<Value>) {
        match block {
            None => (self.nil(fx), None),
            Some(b) => {
                if let Some(ba) = b.as_block_argument_node() {
                    let v = match ba.expression() {
                        Some(e) => {
                            let v = self.expr(fx, &e);
                            self.callc(fx, "kn_to_proc", &[v])
                        }
                        None => self.method_block(fx),
                    };
                    (v, None)
                } else if let Some(bn) = b.as_block_node() {
                    let locals = bn.locals().iter().map(|c| c.as_slice().to_vec()).collect();
                    let p = self.block_proc(fx, bn.parameters(), bn.body(), locals, lambda, bn.location().start_offset());
                    (p, Some(p))
                } else {
                    (self.nil(fx), None)
                }
            }
        }
    }

    pub(crate) fn call_node(&mut self, fx: &mut Fx, c: &CallNode) -> Value {
        // `Module.nesting`: the lexical scopes, innermost first (the cref slots of the
        // enclosing class bodies, without the top level's)
        if c.name().as_slice() == b"nesting"
            && c.arguments().is_none()
            && c.block().is_none()
            && c.receiver().is_some_and(|r| r.as_constant_read_node().is_some_and(|k| k.name().as_slice() == b"Module"))
        {
            let mut vals = Vec::new();
            let mut slot = fx.cref_slot;
            while let Some(&parent) = self.cref_parents.get(&slot) {
                let p = self.slot_ptr(fx, slot);
                vals.push(fx.b.ins().load(I64, mem(), p, 0));
                slot = parent;
            }
            let n = self.iconst(fx, vals.len() as i64);
            let buf = self.args_buf(fx, &vals);
            return self.call(fx, "kn_ary_new", &[n, buf]);
        }
        // `refine X do ... end`: inside the block the module's refinements are active (CRuby
        // activates them in refine blocks), so its calls are refined call sites
        if c.receiver().is_none()
            && c.name().as_slice() == b"refine"
            && c.arguments().is_some_and(|a| a.arguments().iter().count() == 1)
            && c.block().is_some_and(|b| b.as_block_node().is_some())
        {
            self.refine_counter += 1;
            let id = (self.fn_counter as u64) << 20 | self.refine_counter;
            let parent = self.refine_scope.unwrap_or(0);
            let idv = self.iconst(fx, id as i64);
            let pv = self.iconst(fx, parent as i64);
            let m = self.self_val(fx);
            self.callc(fx, "kn_refine_scope", &[idv, m, pv]);
            let saved = self.refine_scope.replace(id);
            let r = self.call_node_inner(fx, c);
            self.refine_scope = saved;
            return r;
        }
        self.call_node_inner(fx, c)
    }

    fn call_node_inner(&mut self, fx: &mut Fx, c: &CallNode) -> Value {
        let name = c.name().as_slice().to_vec();
        let at = c.location().start_offset();
        let at_loc = c.message_loc().map(|l| l.start_offset()).unwrap_or(at);
        let recv_node = c.receiver();
        let arg_nodes: Vec<Node> = match c.arguments() {
            Some(a) => a.arguments().iter().collect(),
            None => vec![],
        };
        let block = c.block();

        // a getter of a local that may hold a virtual object (vobj.rs)
        if arg_nodes.is_empty() && block.is_none() && !c.is_safe_navigation() && fx.inline_levels.is_empty() && !fx.vlocals.is_empty() {
            if let Some(l) = recv_node.as_ref().and_then(|r| r.as_local_variable_read_node()).filter(|l| l.depth() == 0) {
                self.cur_offset = at_loc;
                if let Some(v) = self.vobj_get(fx, l.name().as_slice(), &name) {
                    return v;
                }
            }
        }
        // `%w[date time].each { |f| require f }` (or `require "x/#{f}"`): the files it names are
        // compiled in, so the runtime `require` finds them (the code itself is unchanged)
        if matches!(name.as_slice(), b"each" | b"each_with_index") && arg_nodes.is_empty() {
            if let (Some(r), Some(b)) = (recv_node.as_ref(), block.as_ref().and_then(|b| b.as_block_node())) {
                self.require_each_files(r, &b, at);
            }
        }
        // `define_method(:name) { |a| ... }` as a `def` (statics.rs `block_def`)
        if name == b"define_method" && !fx.no_block_def {
            self.cur_offset = at_loc;
            if let Some(v) = self.def_from_block(fx, c) {
                return v;
            }
        }
        // a read remembered across a loop's iterations (licm.rs)
        if arg_nodes.is_empty() && block.is_none() && !c.is_safe_navigation() && !fx.licm.is_empty() {
            if let Some(v) = self.licm_read(fx, c) {
                return v;
            }
        }

        // `Kernel.block_given?` / `::Kernel.block_given?` (BasicObject subclasses such as
        // Jbuilder write it so): like the bare call, it asks about the current method's block
        // (`Kernel.__method__` / `__callee__` too: sorbet-runtime)
        if matches!(name.as_slice(), b"block_given?" | b"iterator?" | b"__method__" | b"__callee__") && arg_nodes.is_empty() && block.is_none() {
            let kernel_recv = recv_node.as_ref().is_some_and(|r| {
                r.as_constant_read_node().is_some_and(|k| k.name().as_slice() == b"Kernel")
                    || r.as_constant_path_node().is_some_and(|k| k.parent().is_none() && k.name().is_some_and(|n| n.as_slice() == b"Kernel"))
            });
            if kernel_recv && matches!(name.as_slice(), b"__method__" | b"__callee__") {
                return self.method_name_val(fx, name.as_slice() == b"__callee__");
            }
            if kernel_recv {
                let b = self.method_block(fx);
                let c = fx.b.ins().icmp_imm_s(IntCC::NotEqual, b, QNIL);
                return self.bool_v(fx, c);
            }
        }

        if recv_node.is_none() {
            let sname = String::from_utf8_lossy(&name).into_owned();
            if let Some((_, msg)) = DROPPED_CALLS.iter().find(|(n, _)| *n == sname) {
                if arg_nodes.is_empty() || sname != "binding" {
                    return self.unsupported(fx, at, msg);
                }
            }
            match name.as_slice() {
                b"block_given?" | b"iterator?" if arg_nodes.is_empty() && block.is_none() => {
                    let b = self.method_block(fx);
                    let c = fx.b.ins().icmp_imm_s(IntCC::NotEqual, b, QNIL);
                    return self.bool_v(fx, c);
                }
                // `__callee__` (the alias a method was called by) as `__method__`: Rails uses
                // it to name the method in an error message
                b"__method__" | b"__callee__" if arg_nodes.is_empty() => {
                    return self.method_name_val(fx, name.as_slice() == b"__callee__");
                }
                b"__dir__" if arg_nodes.is_empty() => {
                    let abs = self.file.canonicalize().unwrap_or(self.file.clone());
                    let d = abs.parent().map(|p| p.display().to_string()).unwrap_or_default();
                    return self.str_lit(fx, d.as_bytes());
                }
                b"require_relative" if arg_nodes.len() == 1 && arg_nodes[0].as_string_node().is_some() => {
                    return self.require_relative(fx, &arg_nodes[0], at);
                }
                // a computed path (`require_relative "versions/#{v}"`): resolved at run time
                // against this file's directory; the files of gem directories are compiled in
                b"require_relative" if arg_nodes.len() == 1 && block.is_none() => {
                    let base = self.file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    let dir = base.canonicalize().unwrap_or(base);
                    // the program's own files aren't all compiled in: one it names this way is
                    // compiled when it is required (gems' files are all in the binary)
                    if !self.in_dependency() {
                        self.needs_jit = true;
                    }
                    let v = self.expr(fx, &arg_nodes[0]);
                    let d = self.str_lit(fx, dir.display().to_string().as_bytes());
                    let s = self.self_val(fx);
                    return self.send(fx, s, b"__kenny_require_relative", &[v, d], None);
                }
                b"using" if arg_nodes.len() == 1 => {
                    let m = self.expr(fx, &arg_nodes[0]);
                    self.refine_counter += 1;
                    let id = (self.fn_counter as u64) << 20 | self.refine_counter;
                    let parent = self.refine_scope.unwrap_or(0);
                    let idv = self.iconst(fx, id as i64);
                    let pv = self.iconst(fx, parent as i64);
                    self.callc(fx, "kn_using", &[idv, m, pv]);
                    self.refine_scope = Some(id);
                    return self.self_val(fx);
                }
                b"autoload"
                    if arg_nodes.len() == 2 && arg_nodes[0].as_symbol_node().is_some() && arg_nodes[1].as_string_node().is_some() =>
                {
                    let name = String::from_utf8_lossy(arg_nodes[1].as_string_node().unwrap().unescaped()).into_owned();
                    let mut dirs = self.load_paths.clone();
                    if let Some(d) = self.file.parent() {
                        dirs.push(d.to_path_buf());
                    }
                    let found = dirs.iter().map(|d| rb_file(d.join(&name))).find(|p| p.is_file());
                    if let Some(p) = found {
                        if let Some((fid, flag)) = self.require_file(p, at) {
                            let s = self.self_val(fx);
                            let sym_name = arg_nodes[0].as_symbol_node().unwrap().unescaped().to_vec();
                            let id = self.sym_val(fx, &sym_name);
                            let f = self.fn_addr(fx, fid);
                            let slot = self.slot_ptr(fx, flag);
                            return self.callc(fx, "kn_autoload", &[s, id, f, slot]);
                        }
                    }
                }
                b"load" if arg_nodes.len() == 1 && arg_nodes[0].as_string_node().is_some() => {
                    let name = String::from_utf8_lossy(arg_nodes[0].as_string_node().unwrap().unescaped()).into_owned();
                    let mut dirs: Vec<PathBuf> = self.file.parent().map(|d| vec![d.to_path_buf()]).unwrap_or_default();
                    dirs.extend(self.load_paths.clone());
                    if let Some(p) = dirs.iter().map(|d| d.join(&name)).find(|p| p.is_file()) {
                        return self.require_path(fx, p, at);
                    }
                }
                b"require" if arg_nodes.len() == 1 && arg_nodes[0].as_string_node().is_some() => {
                    let name = String::from_utf8_lossy(arg_nodes[0].as_string_node().unwrap().unescaped()).into_owned();
                    // the coverage library: the program is instrumented (lib.rs `compile_with`)
                    if name == "coverage" {
                        self.uses_coverage = true;
                    }
                    // only Ruby sources are compiled in (.so/.bundle/.jar stay runtime requires)
                    let found = if name.ends_with(".so") || name.ends_with(".bundle") || name.ends_with(".jar") || name.ends_with(".dll") {
                        // a Ruby file standing in for a C extension (Kenny's overlay), never a real binary
                        self.load_paths.iter().map(|d| d.join(&name)).find(|p| {
                            p.is_file()
                                && std::fs::read(p).is_ok_and(|b| {
                                    let head = &b[..b.len().min(4)];
                                    !(head == [0xcf, 0xfa, 0xed, 0xfe] || head == [0x7f, b'E', b'L', b'F'] || head == [0xca, 0xfe, 0xba, 0xbe] || head.starts_with(b"PK"))
                                })
                        })
                    } else {
                        self.load_paths.iter().map(|d| d.join(format!("{name}.rb"))).chain(
                            self.load_paths.iter().map(|d| d.join(&name)).filter(|p| p.extension().is_some_and(|e| e == "rb")),
                        ).find(|p| p.is_file())
                    };
                    if let Some(p) = found {
                        return self.require_path(fx, p, at);
                    }
                }
                b"binding" if arg_nodes.is_empty() && block.is_none() => {
                    // self and a snapshot of the locals; eval through it needs the runtime
                    // compiler
                    self.needs_jit = true;
                    let s = self.self_val(fx);
                    let l = self.locals_snapshot(fx);
                    return self.call(fx, "kn_binding_new_l", &[s, l]);
                }
                b"local_variables" if arg_nodes.is_empty() && block.is_none() => {
                    return self.local_variables(fx);
                }
                // in an eval'd string: the caller's locals (the runtime prefixes the source
                // with `a, b, = __kenny_evl;` and passes their values as the argument)
                b"__kenny_evl" if self.eval_unit && arg_nodes.is_empty() && block.is_none() => {
                    let argc = fx.b.use_var(fx.argc_v);
                    let argv = fx.b.use_var(fx.argv_v);
                    let has = fx.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, argc, 0);
                    let ok = fx.b.create_block();
                    let done = fx.b.create_block();
                    fx.b.append_block_param(done, I64);
                    let nil = self.nil(fx);
                    fx.b.ins().brif(has, ok, &[], done, &[nil.into()]);
                    fx.b.switch_to_block(ok);
                    let v = fx.b.ins().load(I64, mem(), argv, 0);
                    fx.b.ins().jump(done, &[v.into()]);
                    fx.b.switch_to_block(done);
                    return fx.b.block_params(done)[0];
                }
                b"lambda" if arg_nodes.is_empty() && block.as_ref().is_some_and(|b| b.as_block_node().is_some()) => {
                    let (p, _) = self.block_arg(fx, block, true);
                    return p;
                }
                _ => {}
            }
        }

        // `!x` / `not x`
        if name == b"!" && arg_nodes.is_empty() && block.is_none() {
            if let Some(r) = &recv_node {
                let v = self.expr(fx, r);
                let t = self.truthy(fx, v);
                let tr = self.iconst(fx, QTRUE);
                let fa = self.iconst(fx, QFALSE);
                let r = fx.b.ins().select(t, fa, tr);
                // (an `if` on it branches on the inverted condition)
                let nt = fx.b.ins().icmp_imm_s(IntCC::Equal, t, 0);
                fx.bools.insert(r, nt);
                return r;
            }
        }

        if matches!(name.as_slice(), b"eval" | b"class_eval" | b"module_eval" | b"instance_eval") && !arg_nodes.is_empty() {
            self.needs_jit = true;
        }
        // `load(path)` at runtime recompiles a file that changed since the build (or isn't
        // compiled in), so it needs the runtime compiler too
        if name.as_slice() == b"load" && !arg_nodes.is_empty() {
            self.needs_jit = true;
        }
        // so does a `require` or `autoload` of a computed path in the program's own code (it
        // may name a file that isn't compiled in; gems' files all are)
        let path_arg = match name.as_slice() {
            b"require" => arg_nodes.first(),
            b"autoload" => arg_nodes.get(1),
            _ => None,
        };
        if path_arg.is_some_and(|a| a.as_string_node().is_none()) && !self.in_dependency() {
            self.needs_jit = true;
        }

        let recv = match &recv_node {
            // (`Ripper.lex_state_name(..)`: not Ripper's parser)
            Some(r) if crate::ripper::is_data_method(&name) && crate::ripper::is_ripper(r) => self.ripper_quietly(|c| c.expr(fx, r)),
            Some(r) => self.expr(fx, r),
            None => self.self_val(fx),
        };

        // safe navigation: recv&.foo
        let safe = c.is_safe_navigation();
        let (safe_done, safe_cont) = if safe {
            let done = fx.b.create_block();
            fx.b.append_block_param(done, I64);
            let cont = fx.b.create_block();
            let is_nil = fx.b.ins().icmp_imm_s(IntCC::Equal, recv, QNIL);
            let nil = self.nil(fx);
            if self.cov_has_branch(&c.as_node()) {
                // (branch coverage: the nil case counted on its way out)
                let nil_b = fx.b.create_block();
                fx.b.ins().brif(is_nil, nil_b, &[], cont, &[]);
                fx.b.switch_to_block(nil_b);
                self.cov_branch(fx, &c.as_node(), 1);
                fx.b.ins().jump(done, &[nil.into()]);
            } else {
                fx.b.ins().brif(is_nil, done, &[nil.into()], cont, &[]);
            }
            fx.b.switch_to_block(cont);
            self.cov_branch(fx, &c.as_node(), 0);
            (Some(done), Some(cont))
        } else {
            (None, None)
        };
        let _ = safe_cont;

        self.cur_offset = at_loc;
        let result = if recv_node.is_some()
            && arg_nodes.len() == 1
            && block.is_none()
            && is_fast_op(&name)
            && arg_nodes[0].as_splat_node().is_none()
            && arg_nodes[0].as_keyword_hash_node().is_none()
            && arg_nodes[0].as_forwarding_arguments_node().is_none()
        {
            let a = self.expr(fx, &arg_nodes[0]);
            self.binop(fx, &name, recv, a)
        } else if c.is_variable_call() && self.refine_scope.is_some() {
            // (a self call sees refinements too)
            let r = self.send_raw(fx, recv, &name, &Args { vals: vec![], splat: None }, None, false);
            self.check(fx, r);
            r
        } else if c.is_variable_call() {
            self.mark_loc(fx);
            let cache = self.owned_cache_ptr(fx, OWNED_CALL);
            let mid = self.sym_val(fx, &name);
            let (inline, stub) = crate::fastpath::direct_plan(&name, !fx.loops.is_empty());
            let plain = move |c: &mut Self, fx: &mut Fx, rt: &'static str, args: &[Value]| -> Value {
                if inline && rt == "kn_vcall" {
                    c.direct_call(fx, cache, recv, &[], false, |c, fx| c.call(fx, rt, args))
                } else if (inline || stub) && rt == "kn_attr_get" {
                    c.call_stub(fx, crate::fastpath::STUB_ATTR_GET, args)
                } else if stub && rt == "kn_vcall" {
                    c.call_stub(fx, crate::fastpath::STUB_VCALL, args)
                } else {
                    c.call(fx, rt, args)
                }
            };
            let attr = !fx.loops.is_empty() && crate::fastpath::attr_name(&name);
            let dynamic = |c: &mut Self, fx: &mut Fx| -> Value {
                if attr {
                    let mode = c.iconst(fx, 2);
                    c.attr_fast(fx, cache, recv, None, |c, fx| plain(c, fx, "kn_attr_get", &[cache, recv, mid, mode]))
                } else {
                    plain(c, fx, "kn_vcall", &[cache, recv, mid])
                }
            };
            let r = match self.inline_call(fx, recv, &name, mid, &[], false, |c, fx| dynamic(c, fx)) {
                Some(r) => r,
                None => dynamic(self, fx),
            };
            self.check(fx, r);
            r
        } else if let Some(v) = match &recv_node {
            Some(r) if r.as_self_node().is_none() => self.inline_block_call(fx, c, recv),
            _ => None,
        } {
            v
        } else if let Some(r) = self.kw_inline_site(fx, c, recv, &name, &arg_nodes, block.is_none(), recv_node.as_ref().is_some_and(|r| r.as_self_node().is_none()), at_loc) {
            r
        } else if let Some(r) = self.yield_inline_site(fx, c, recv, &name, &arg_nodes, at_loc) {
            r
        } else {
            // `s << "lit"` (chilled.rs, `append_lit`)
            if name == b"<<" && arg_nodes.len() == 1 && recv_node.is_some() {
                if let Some(sn) = arg_nodes[0].as_string_node().filter(|s| !s.is_frozen() && !s.is_mutable()) {
                    self.append_recv = Some((crate::chilled::key(&sn), recv));
                }
            }
            let args = self.eval_args(fx, c.arguments());
            self.append_recv = None;
            // an eval'd string sees the caller's locals: a snapshot, handed to the runtime with
            // the source string it is for
            if crate::analysis::sees_locals(c) && args.splat.is_none() && !args.vals.is_empty() {
                let l = self.locals_snapshot(fx);
                let src = args.vals[0];
                self.call(fx, "kn_eval_locals", &[l, src]);
                // a string class_eval'd on an anonymous module sees the caller's constants
                if matches!(name.as_slice(), b"class_eval" | b"module_eval") {
                    let cref = self.cref(fx);
                    self.call(fx, "kn_eval_scope", &[recv, cref]);
                }
            }
            let forwards = arg_nodes.iter().any(|n| n.as_forwarding_arguments_node().is_some());
            let no_block = block.is_none() && !forwards;
            let (blk, literal) = if forwards && block.is_none() {
                (self.forwarded_local(fx, b"&"), None)
            } else {
                // a method's body: `return` returns from it
                self.bmethod_next = matches!(name.as_slice(), b"define_method" | b"define_singleton_method") && block.as_ref().is_some_and(|b| b.as_block_node().is_some());
                let r = self.block_arg(fx, block, false);
                self.bmethod_next = false;
                r
            };
            self.cur_offset = at_loc;
            let explicit = recv_node.as_ref().is_some_and(|r| r.as_self_node().is_none());
            // `C.new(args)`: allocation and `initialize` inline (statics.rs)
            let const_name = recv_node.as_ref().and_then(|r| match r.as_constant_read_node() {
                Some(k) => Some(k.name().as_slice().to_vec()),
                None => r.as_constant_path_node().and_then(|k| k.name().map(|n| n.as_slice().to_vec())),
            });
            if let (b"new", true, None, Some(cn)) = (name.as_slice(), no_block, &args.splat, &const_name) {
                let vals = args.vals.clone();
                let r = self.inline_new(fx, recv, cn, &vals, |c, fx| {
                    let r = c.send_raw(fx, recv, b"new", &Args { vals: vals.clone(), splat: None }, None, true);
                    c.check(fx, r);
                    r
                });
                if let Some(r) = r {
                    return match safe_done {
                        Some(done) => {
                            fx.b.ins().jump(done, &[r.into()]);
                            fx.b.switch_to_block(done);
                            fx.b.block_params(done)[0]
                        }
                        None => r,
                    };
                }
            }
            let r = self.send_raw(fx, recv, &name, &args, if no_block { None } else { Some(blk) }, explicit);
            let r = match literal {
                Some(p) => self.catch_break(fx, r, p),
                None => {
                    self.check(fx, r);
                    r
                }
            };
            if c.is_attribute_write() && args.splat.is_none() {
                *args.vals.last().unwrap_or(&r)
            } else {
                r
            }
        };
        match safe_done {
            Some(done) => {
                fx.b.ins().jump(done, &[result.into()]);
                fx.b.switch_to_block(done);
                fx.b.block_params(done)[0]
            }
            None => result,
        }
    }

    /// `m(args) { |a| ... }` to a small method of this program that yields (statics.rs
    /// `inline_target`): its body inline, the block's body at each `yield` (`inline_yield`).
    /// None (nothing compiled): not such a call.
    fn yield_inline_site(&mut self, fx: &mut Fx, c: &CallNode, recv: Value, name: &[u8], arg_nodes: &[Node], at_loc: usize) -> Option<Value> {
        if fx.spec.is_some() || c.is_safe_navigation() || c.is_attribute_write() || self.cov.is_some() || std::env::var_os("KENNY_NO_INLINE").is_some() {
            return None;
        }
        let bn = c.block()?.as_block_node()?;
        if arg_nodes.iter().any(|n| n.as_splat_node().is_some() || n.as_keyword_hash_node().is_some() || n.as_block_argument_node().is_some() || n.as_forwarding_arguments_node().is_some()) {
            return None;
        }
        let explicit = c.receiver().is_some_and(|r| r.as_self_node().is_none());
        self.inline_target(fx, name, arg_nodes.len(), explicit, true)?;
        let params = crate::inline::simple_params(bn.parameters())?;
        // `return` leaves this frame's method (or block, as the closure would)
        if !crate::inline::yield_inlinable(&bn, matches!(fx.kind, ScopeKind::Method | ScopeKind::Block)) {
            return None;
        }
        let vals: Vec<Value> = arg_nodes.iter().map(|n| self.expr(fx, n)).collect();
        self.cur_offset = at_loc;
        self.mark_loc(fx);
        let mid = self.sym_val(fx, name);
        let locals: Vec<Vec<u8>> = bn.locals().iter().map(|l| l.as_slice().to_vec()).collect();
        // (the tree outlives the compilation of this call)
        let blk: BlockNode<'static> = unsafe { std::mem::transmute::<BlockNode<'_>, BlockNode<'static>>(c.block()?.as_block_node()?) };
        let yb = crate::statics::YieldBlock { blk, params, locals };
        let (nerr, nwarn) = (self.errors.len(), self.warnings.len());
        let nm = name.to_vec();
        let vs = vals.clone();
        let r = self.inline_call_with(fx, recv, name, mid, &vals, explicit, Some(yb), |c2, fx| {
            // the call as written, with the block as a closure
            let (blk, literal) = c2.block_arg(fx, c.block(), false);
            c2.cur_offset = at_loc;
            let r = c2.send_raw(fx, recv, &nm, &Args { vals: vs, splat: None }, Some(blk), explicit);
            match literal {
                Some(p) => c2.catch_break(fx, r, p),
                None => r,
            }
        });
        // (the block's body was compiled more than once: report things once)
        let e: Vec<String> = self.errors.drain(nerr..).collect();
        for x in e {
            if !self.errors[nerr..].contains(&x) {
                self.errors.push(x);
            }
        }
        let w: Vec<_> = self.warnings.drain(nwarn..).collect();
        for x in w {
            if !self.warnings[nwarn..].contains(&x) {
                self.warnings.push(x);
            }
        }
        r
    }

    /// `name(a, k: x)` with literal keywords to a keyword `def` that can be inlined
    /// (statics.rs `kw_target`). None (nothing compiled): not such a call.
    #[allow(clippy::too_many_arguments)]
    fn kw_inline_site(&mut self, fx: &mut Fx, c: &CallNode, recv: Value, name: &[u8], arg_nodes: &[Node], no_block: bool, explicit: bool, at_loc: usize) -> Option<Value> {
        if !no_block || c.is_safe_navigation() || c.is_attribute_write() {
            return None;
        }
        let (last, pos) = arg_nodes.split_last()?;
        let h = last.as_keyword_hash_node()?;
        let mut keys: Vec<Vec<u8>> = Vec::new();
        let mut vals: Vec<Node> = Vec::new();
        for e in h.elements().iter() {
            let a = e.as_assoc_node()?;
            let k = a.key();
            let s = k.as_symbol_node()?;
            let kb = s.unescaped().to_vec();
            if keys.contains(&kb) {
                return None;
            }
            keys.push(kb);
            vals.push(a.value());
        }
        if pos.iter().any(|n| {
            n.as_splat_node().is_some() || n.as_forwarding_arguments_node().is_some() || n.as_keyword_hash_node().is_some() || n.as_block_argument_node().is_some()
        }) {
            return None;
        }
        let target = self.kw_target(fx, name, pos.len(), &keys, explicit)?;
        let pv: Vec<Value> = pos.iter().map(|n| self.expr(fx, n)).collect();
        let kv: Vec<(Vec<u8>, Value)> = keys.iter().cloned().zip(vals.iter().map(|n| self.expr(fx, n))).collect();
        self.cur_offset = at_loc;
        self.mark_loc(fx);
        let mid = self.sym_val(fx, name);
        let name = name.to_vec();
        let r = self.inline_kw_call(fx, target, recv, mid, &pv, &kv, explicit, |c, fx| {
            // the call as written: the keywords as a keyword hash
            let mut hv = Vec::new();
            for (k, v) in &kv {
                hv.push(c.sym_val(fx, k));
                hv.push(*v);
            }
            let n = c.iconst(fx, hv.len() as i64);
            let buf = c.args_buf(fx, &hv);
            let one = c.iconst(fx, 1);
            let h = c.call(fx, "kn_hash_new_syms", &[n, buf, one]);
            let mut vals = pv.clone();
            vals.push(h);
            c.cur_offset = at_loc;
            let r = c.send_raw(fx, recv, &name, &Args { vals, splat: None }, None, explicit);
            c.check(fx, r);
            r
        });
        Some(r)
    }

    /// The files `["a", "b"].each { |f| require f }` requires (see call_node_inner).
    fn require_each_files(&mut self, recv: &Node, block: &BlockNode, at: usize) {
        let Some(arr) = recv.as_array_node() else { return };
        let names: Vec<Vec<u8>> = arr.elements().iter().filter_map(|e| e.as_string_node().map(|s| s.unescaped().to_vec())).collect();
        if names.is_empty() || names.len() != arr.elements().iter().count() {
            return;
        }
        // the block's first parameter
        let Some(param) = block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
            .and_then(|p| p.parameters())
            .and_then(|p| p.requireds().iter().next())
            .and_then(|r| r.as_required_parameter_node().map(|r| r.name().as_slice().to_vec()))
        else {
            return;
        };
        let Some(body) = block.body().and_then(|b| b.as_statements_node()) else { return };
        // the require's argument as a template: literal parts and the parameter (None)
        let mut template: Option<Vec<Option<Vec<u8>>>> = None;
        for st in body.body().iter() {
            let Some(call) = st.as_call_node() else { continue };
            if call.receiver().is_some() || call.name().as_slice() != b"require" {
                continue;
            }
            let Some(arg) = call.arguments().and_then(|a| a.arguments().iter().next()) else { continue };
            let is_param = |n: &Node| n.as_local_variable_read_node().is_some_and(|l| l.depth() == 0 && l.name().as_slice() == param.as_slice());
            if is_param(&arg) {
                template = Some(vec![None]);
            } else if let Some(i) = arg.as_interpolated_string_node() {
                let mut t = Vec::new();
                for part in i.parts().iter() {
                    if let Some(s) = part.as_string_node() {
                        t.push(Some(s.unescaped().to_vec()));
                    } else if let Some(e) = part.as_embedded_statements_node() {
                        let one = e.statements().map(|s| s.body().iter().collect::<Vec<_>>()).unwrap_or_default();
                        if one.len() == 1 && is_param(&one[0]) {
                            t.push(None);
                        } else {
                            t.clear();
                            break;
                        }
                    }
                }
                if !t.is_empty() {
                    template = Some(t);
                }
            }
            break;
        }
        let Some(t) = template else { return };
        for n in names {
            let feature: Vec<u8> = t.iter().flat_map(|p| p.clone().unwrap_or_else(|| n.clone())).collect();
            let feature = String::from_utf8_lossy(&feature).into_owned();
            if feature.ends_with(".so") || feature.ends_with(".bundle") {
                continue;
            }
            let found = self.load_paths.iter().map(|d| d.join(format!("{feature}.rb"))).find(|p| p.is_file());
            if let Some(p) = found {
                self.require_file(p, at);
            }
        }
    }

    fn require_relative(&mut self, fx: &mut Fx, arg: &Node, at: usize) -> Value {
        let rel = String::from_utf8_lossy(arg.as_string_node().unwrap().unescaped()).into_owned();
        let base = self.file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let full = rb_file(base.join(&rel));
        if self.file_unit {
            // a file compiled at runtime: the program's own copy of the file (compiled in, or
            // loaded from disk by the runtime), not a second one inside this unit
            let abs = full.canonicalize().unwrap_or(full);
            let s = self.self_val(fx);
            let p = self.str_lit(fx, abs.display().to_string().as_bytes());
            return self.send(fx, s, b"require", &[p], None);
        }
        self.require_path(fx, full, at)
    }

    fn require_path(&mut self, fx: &mut Fx, full: PathBuf, at: usize) -> Value {
        let Some((fid, flag)) = self.require_file(full, at) else {
            return self.iconst(fx, QFALSE);
        };
        // flag: 0 = not loaded, 1 = loaded, otherwise the thread loading it right now
        // (kn_require_file waits for that thread, like CRuby's require)
        let flagp = self.slot_ptr(fx, flag);
        let st = fx.b.ins().load(I64, mem(), flagp, 0);
        let loaded = fx.b.ins().icmp_imm_s(IntCC::Equal, st, 1);
        let run = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let f = self.iconst(fx, QFALSE);
        fx.b.ins().brif(loaded, done, &[f.into()], run, &[]);
        fx.b.switch_to_block(run);
        let fa = self.fn_addr(fx, fid);
        let r = self.call(fx, "kn_require_file", &[flagp, fa]);
        self.check(fx, r);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        fx.b.block_params(done)[0]
    }

    // ---------- compound assignment on calls ----------

    fn recv_or_self(&mut self, fx: &mut Fx, r: Option<Node>) -> Value {
        match r {
            Some(n) => self.expr(fx, &n),
            None => self.self_val(fx),
        }
    }
    pub(crate) fn call_op_write(&mut self, fx: &mut Fx, n: &CallOperatorWriteNode) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let cur = self.send(fx, recv, n.read_name().as_slice(), &[], None);
        let rhs = self.num_expr(fx, &n.value(), true);
        let v = self.binop_rhs(fx, n.binary_operator().as_slice(), cur, rhs);
        self.send(fx, recv, n.write_name().as_slice(), &[v], None);
        v
    }
    pub(crate) fn call_or_write(&mut self, fx: &mut Fx, n: &CallOrWriteNode, _or: bool) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let cur = self.send(fx, recv, n.read_name().as_slice(), &[], None);
        let value = n.value();
        let wn = n.write_name().as_slice().to_vec();
        self.or_assign(fx, cur, |c, fx| {
            let v = c.expr(fx, &value);
            c.send(fx, recv, &wn, &[v], None);
            v
        })
    }
    pub(crate) fn call_and_write(&mut self, fx: &mut Fx, n: &CallAndWriteNode) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let cur = self.send(fx, recv, n.read_name().as_slice(), &[], None);
        let value = n.value();
        let wn = n.write_name().as_slice().to_vec();
        self.and_assign(fx, cur, |c, fx| {
            let v = c.expr(fx, &value);
            c.send(fx, recv, &wn, &[v], None);
            v
        })
    }
    fn index_args(&mut self, fx: &mut Fx, a: Option<ArgumentsNode>) -> Vec<Value> {
        match a {
            Some(a) => a.arguments().iter().map(|n| self.expr(fx, &n)).collect(),
            None => vec![],
        }
    }
    pub(crate) fn index_op_write(&mut self, fx: &mut Fx, n: &IndexOperatorWriteNode) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let mut args = self.index_args(fx, n.arguments());
        // `a[i] += k` with k a literal or a local: nothing runs between the read and the
        // write, so one guard covers both (fastpath.rs `ary_add_fast`)
        let op = n.binary_operator().as_slice().to_vec();
        let value = n.value();
        let simple_rhs = value.as_integer_node().is_some() || value.as_local_variable_read_node().is_some();
        if args.len() == 1
            && n.block().is_none()
            && simple_rhs
            && matches!(op.as_slice(), b"+" | b"-")
            && self.refine_scope.is_none()
            && (!fx.loops.is_empty() || !self.in_dependency())
        {
            let rhs = self.expr(fx, &value);
            let idx = args[0];
            return self.ary_add_fast(fx, recv, idx, rhs, op == b"-", |c, fx, cur| {
                // (the local again: a `[]` of another class may have changed it)
                let rhs = c.expr(fx, &value);
                let v = c.binop_rhs(fx, &op, cur, crate::fastpath::Num::Boxed(rhs));
                c.send(fx, recv, b"[]=", &[idx, v], None);
                v
            });
        }
        // one index: the inline Array read and write (each a real send when it doesn't apply)
        if args.len() == 1 && n.block().is_none() && self.refine_scope.is_none() && (!fx.loops.is_empty() || !self.in_dependency()) {
            let idx = args[0];
            let cur = self.ary_index_fast(fx, recv, idx, None, |c, fx| c.send(fx, recv, b"[]", &[idx], None));
            let rhs = self.num_expr(fx, &value, true);
            let v = self.binop_rhs(fx, &op, cur, rhs);
            self.ary_index_fast(fx, recv, idx, Some(v), |c, fx| c.send(fx, recv, b"[]=", &[idx, v], None));
            return v;
        }
        let cur = self.send(fx, recv, b"[]", &args, None);
        let rhs = self.num_expr(fx, &n.value(), true);
        let v = self.binop_rhs(fx, n.binary_operator().as_slice(), cur, rhs);
        args.push(v);
        self.send(fx, recv, b"[]=", &args, None);
        v
    }
    pub(crate) fn index_or_write(&mut self, fx: &mut Fx, n: &IndexOrWriteNode) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let args = self.index_args(fx, n.arguments());
        let cur = self.send(fx, recv, b"[]", &args, None);
        let value = n.value();
        self.or_assign(fx, cur, |c, fx| {
            let v = c.expr(fx, &value);
            let mut a = args.clone();
            a.push(v);
            c.send(fx, recv, b"[]=", &a, None);
            v
        })
    }
    pub(crate) fn index_and_write(&mut self, fx: &mut Fx, n: &IndexAndWriteNode) -> Value {
        let recv = self.recv_or_self(fx, n.receiver());
        let args = self.index_args(fx, n.arguments());
        let cur = self.send(fx, recv, b"[]", &args, None);
        let value = n.value();
        self.and_assign(fx, cur, |c, fx| {
            let v = c.expr(fx, &value);
            let mut a = args.clone();
            a.push(v);
            c.send(fx, recv, b"[]=", &a, None);
            v
        })
    }

    // ---------- super / yield ----------

    fn do_super(&mut self, fx: &mut Fx, args: Args, blk: Value, at: usize) -> Value {
        let _ = at;
        let Some(m) = fx.method.clone() else {
            // e.g. inside define_method { super(...) }: resolved at runtime
            let s = self.self_val(fx);
            let ary = match args.splat {
                Some(a) => a,
                None => {
                    let n = self.iconst(fx, args.vals.len() as i64);
                    let buf = self.args_buf(fx, &args.vals);
                    self.call(fx, "kn_ary_new", &[n, buf])
                }
            };
            return self.callc(fx, "kn_super_dyn", &[s, ary, blk]);
        };
        let slot = self.slot_ptr(fx, m.owner_slot);
        let s = self.self_val(fx);
        let mid = self.sym_val(fx, &m.name);
        if let Some(pv) = fx.proc_v {
            // `super` in a block inside a method: if that block has become a method body
            // (`define_method(:x) { super(v) }` in a method), it means the new method's super
            let ary = match args.splat {
                Some(a) => a,
                None => {
                    let n = self.iconst(fx, args.vals.len() as i64);
                    let buf = self.args_buf(fx, &args.vals);
                    self.call(fx, "kn_ary_new", &[n, buf])
                }
            };
            let p = fx.b.use_var(pv);
            return self.callc(fx, "kn_super_blk", &[p, slot, s, mid, ary, blk]);
        }
        match args.splat {
            Some(a) => self.callc(fx, "kn_super_splat", &[slot, s, mid, a, blk]),
            None => {
                let n = self.iconst(fx, args.vals.len() as i64);
                let buf = self.args_buf(fx, &args.vals);
                self.callc(fx, "kn_super", &[slot, s, mid, n, buf, blk])
            }
        }
    }
    pub(crate) fn super_node(&mut self, fx: &mut Fx, s: &SuperNode) -> Value {
        let at = s.location().start_offset();
        let args = self.eval_args(fx, s.arguments());
        let blk = match s.block() {
            Some(b) => self.block_arg(fx, Some(b), false).0,
            None => self.method_block(fx),
        };
        self.do_super(fx, args, blk, at)
    }
    pub(crate) fn zsuper_node(&mut self, fx: &mut Fx, s: &ForwardingSuperNode) -> Value {
        let at = s.location().start_offset();
        let (params, depth) = if fx.kind == ScopeKind::Method {
            (fx.zsuper.clone(), 0u32)
        } else {
            match fx.method.clone() {
                Some(m) => (m.params, (fx.scope - m.scope) as u32),
                None => return self.unsupported(fx, at, "super called outside of a method"),
            }
        };
        let zero = self.iconst(fx, 0);
        let ary = self.call(fx, "kn_ary_new", &[zero, zero]);
        let mut kw: Option<Value> = None;
        for (name, kind) in &params {
            let v = self.local_get(fx, name, depth);
            match kind {
                0 => {
                    self.call(fx, "kn_ary_push", &[ary, v]);
                }
                1 => {
                    self.callc(fx, "kn_ary_concat_splat", &[ary, v]);
                }
                2 => {
                    let h = match kw {
                        Some(h) => h,
                        None => {
                            let one = self.iconst(fx, 1);
                            let h = self.call(fx, "kn_hash_new", &[zero, zero, one]);
                            kw = Some(h);
                            h
                        }
                    };
                    let key = self.sym_val(fx, name);
                    let two = self.iconst(fx, 2);
                    let buf = self.args_buf(fx, &[key, v]);
                    let pair = self.call(fx, "kn_hash_new", &[two, buf, zero]);
                    self.callc(fx, "kn_hash_merge_splat", &[h, pair]);
                }
                _ => {
                    let h = match kw {
                        Some(h) => h,
                        None => {
                            let one = self.iconst(fx, 1);
                            let h = self.call(fx, "kn_hash_new", &[zero, zero, one]);
                            kw = Some(h);
                            h
                        }
                    };
                    self.callc(fx, "kn_hash_merge_splat", &[h, v]);
                }
            }
        }
        if let Some(h) = kw {
            // (an empty `**kw` passes nothing)
            self.call(fx, "kn_ary_push_kw", &[ary, h]);
        }
        let blk = match s.block() {
            Some(b) => self.block_arg(fx, Some(b.as_node()), false).0,
            None => self.method_block(fx),
        };
        self.do_super(fx, Args { vals: vec![], splat: Some(ary) }, blk, at)
    }

    pub(crate) fn yield_node(&mut self, fx: &mut Fx, y: &YieldNode) -> Value {
        // in a method inlined with a literal block: that block, here
        if !self.yield_targets.is_empty() {
            if let Some(v) = self.inline_yield(fx, y) {
                return v;
            }
        }
        let blk = self.method_block(fx);
        let args = self.eval_args(fx, y.arguments());
        match args.splat {
            Some(a) => self.callc(fx, "kn_yield_splat", &[blk, a]),
            None => {
                let n = self.iconst(fx, args.vals.len() as i64);
                let buf = self.args_buf(fx, &args.vals);
                self.callc(fx, "kn_yield", &[blk, n, buf])
            }
        }
    }
}

/// The file `require` loads for `path`: `path.rb` unless `path` ends in `.rb` (or names a
/// C extension) or only `path` itself exists, as in CRuby (`require_relative "parser.tab"`
/// loads parser.tab.rb).
pub(crate) fn rb_file(path: PathBuf) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rb" | "so" | "bundle" | "o" | "dll") => return path,
        _ => {}
    }
    let mut with = path.clone().into_os_string();
    with.push(".rb");
    let with = PathBuf::from(with);
    if with.exists() || !path.is_file() { with } else { path }
}
