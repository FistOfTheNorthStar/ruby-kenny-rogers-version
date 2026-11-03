//! Stack maps: where a compiled function keeps its Ruby objects while it is in a call, so the
//! collector can find them precisely and update them when it moves objects
//! (kenny-runtime stackmap.rs).
//!
//! Before a function is finalized, `declare_gc_values` finds the values that are certainly
//! Ruby objects (the function's `self`, block and arguments, what Ruby methods and the
//! runtime functions returning a VALUE return, and block parameters fed only by such values)
//! and declares them to Cranelift, which then keeps each one that is live across a call in a
//! stack slot of its own (stored once after its definition, reloaded at every use) and lists
//! those slots for the call's return address. Anything not declared (unboxed numbers, raw
//! pointers, values loaded from memory) stays where register allocation puts it, and the
//! collector scans those words conservatively: such objects stay put.
//!
//! `record_stack_maps` keeps the maps of a compiled function; `emit_stack_maps` writes them
//! into the program (`kn_register_stackmaps`), and eval units hand theirs to the runtime.
//!
//! Table format (u32 words), per function: [sites, then per call site: return offset, frame
//! size (stack pointer to frame pointer), first slot index, slot count], followed by the
//! slots (offsets from the stack pointer at the call).

use crate::*;
use cranelift_codegen::ir::{instructions::CallInfo, BlockArg, ExternalName, InstructionData, Opcode, ValueDef};
use std::collections::HashSet;

/// Which parameters of a function are Ruby objects.
#[derive(Clone, Copy)]
pub(crate) enum Entry {
    /// (self, argc, argv, blk)
    Method,
    /// (proc, self, argc, argv, blk)
    Block,
    /// a fast entry: (self, args..., blk)
    Fast,
}

/// Runtime functions whose result is a VALUE (`-> V`). A function missing here only makes
/// its results conservative roots.
const RETURNS_VALUE: &[&str] = &[
    "kn_alias", "kn_args_rest", "kn_ary_at", "kn_ary_concat_splat", "kn_ary_new", "kn_ary_pop0", "kn_ary_post_ref", "kn_ary_push", "kn_ary_push1", "kn_ary_push_kw",
    "kn_ary_ref", "kn_ary_shift0", "kn_ary_slice_rest", "kn_ary_sum_vals", "kn_attr_get", "kn_attr_set", "kn_autoload", "kn_autosplat", "kn_backref", "kn_binding_new",
    "kn_binding_new_l", "kn_block_post_ref", "kn_catch_break", "kn_catch_return", "kn_check_arity", "kn_class_enter", "kn_class_leave", "kn_clear_pending",
    "kn_const_fold", "kn_const_get", "kn_const_get_c", "kn_const_get_l", "kn_const_get_under", "kn_const_set", "kn_cvar_get", "kn_cvar_get_soft", "kn_cvar_set",
    "kn_def", "kn_def_claim", "kn_define_class", "kn_define_module", "kn_dm_name", "kn_dm_name_for", "kn_entry_slow", "kn_eq", "kn_errinfo", "kn_eval_locals",
    "kn_eval_scope", "kn_exc_match", "kn_float_box", "kn_float_lit", "kn_gvar_alias", "kn_gvar_get", "kn_gvar_set", "kn_hash_merge_splat", "kn_hash_new",
    "kn_hash_new_syms", "kn_int_lit", "kn_is_a", "kn_is_a_fill", "kn_is_a_send", "kn_ispec_poll", "kn_ivar_get", "kn_ivar_set", "kn_kw_assert_empty", "kn_kw_dup",
    "kn_kw_get", "kn_kw_missing", "kn_kw_take", "kn_kw_unknown", "kn_last_match", "kn_main_obj", "kn_masgn_ary", "kn_match_named", "kn_match_save", "kn_new_fill",
    "kn_nth_ref", "kn_obj_alloc", "kn_object_class", "kn_pending_exc", "kn_pm_deconstruct", "kn_pm_deconstruct_keys", "kn_pm_hash_rest", "kn_pm_key", "kn_pm_nomatch",
    "kn_pm_slice", "kn_proc_new", "kn_proc_set_locals", "kn_raise_msg", "kn_range_first", "kn_range_last", "kn_range_new", "kn_rational_lit", "kn_refine_scope",
    "kn_regexp_lit", "kn_regexp_new", "kn_require_file", "kn_safepoint", "kn_send", "kn_send0", "kn_send1", "kn_send2", "kn_send_pub", "kn_send_refined",
    "kn_send_splat", "kn_set_lexical_parent", "kn_singleton_class_for", "kn_splat_to_ary", "kn_static_fill", "kn_str_append", "kn_str_append_own", "kn_str_append_raw",
    "kn_str_buf", "kn_str_chilled", "kn_str_chilled_eval", "kn_str_idx_eq", "kn_str_interp", "kn_str_interp_raw", "kn_str_lit", "kn_str_lit_eval", "kn_str_new",
    "kn_str_to_sym", "kn_super", "kn_super_blk", "kn_super_dyn", "kn_super_splat", "kn_throw_break", "kn_throw_return", "kn_to_proc", "kn_undef", "kn_using",
    "kn_vcall", "kn_vobj_fill", "kn_wb_env", "kn_yield", "kn_yield_splat",
];

/// A constant that is never an object's address (an immediate, a small number).
fn harmless_const(func: &ir::Function, v: Value) -> bool {
    match func.dfg.value_def(v) {
        ValueDef::Result(inst, 0) => match func.dfg.insts[inst] {
            InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => {
                let x = imm.bits() as u64;
                x < 4096 || x & 15 != 0
            }
            _ => false,
        },
        _ => false,
    }
}

impl<M: Module> Compiler<M> {
    /// Declare the function's certain Ruby objects as needing stack maps (see the module docs).
    /// Call once every block is sealed, before `finalize`.
    pub(crate) fn declare_gc_values(&self, b: &mut FunctionBuilder, entry: Entry) {
        if !stack_maps_enabled() {
            return;
        }
        let func = &*b.func;
        let dfg = &func.dfg;
        let mut objs: HashSet<Value> = HashSet::new();
        let Some(first) = func.layout.entry_block() else { return };
        let params = dfg.block_params(first);
        let (seeds, argv): (Vec<usize>, Option<usize>) = match entry {
            Entry::Method => (vec![0, 3], Some(2)),
            Entry::Block => (vec![0, 1, 4], Some(3)),
            Entry::Fast => ((0..params.len()).collect(), None),
        };
        for i in seeds {
            if let Some(&p) = params.get(i) {
                objs.insert(p);
            }
        }
        let argv = argv.and_then(|i| params.get(i).copied());
        // which called functions return objects
        let value_fn = |fr: ir::FuncRef| -> bool {
            let ExternalName::User(r) = &dfg.ext_funcs[fr].name else { return false };
            let un = &func.params.user_named_funcs()[*r];
            if un.namespace != 0 {
                return false;
            }
            let id = FuncId::from_u32(un.index);
            let Some(name) = self.module.declarations().get_function_decl(id).name.as_deref() else { return false };
            if let Some(rest) = name.strip_prefix("__kn_") {
                // compiled Ruby code (not the Integer copies: they return raw integers)
                return !rest.starts_with("ispec_");
            }
            RETURNS_VALUE.contains(&name)
        };
        let mut cands: Vec<Value> = Vec::new();
        for block in func.layout.blocks() {
            if block != first {
                for &p in dfg.block_params(block) {
                    if dfg.value_type(p) == I64 {
                        cands.push(p);
                    }
                }
            }
            for inst in func.layout.block_insts(block) {
                let data = &dfg.insts[inst];
                let results = dfg.inst_results(inst);
                match data.analyze_call(&dfg.value_lists, &dfg.exception_tables) {
                    CallInfo::Direct(fr, _) | CallInfo::DirectWithSig(fr, _, _) => {
                        if results.len() == 1 && dfg.value_type(results[0]) == I64 && value_fn(fr) {
                            objs.insert(results[0]);
                        }
                        continue;
                    }
                    // (indirect calls are to compiled Ruby methods)
                    CallInfo::Indirect(_, _) => {
                        if results.len() == 1 && dfg.value_type(results[0]) == I64 {
                            objs.insert(results[0]);
                        }
                        continue;
                    }
                    CallInfo::NotACall => {}
                }
                // an argument read from argv
                if let (Some(av), InstructionData::Load { opcode: Opcode::Load, arg, .. }) = (argv, data) {
                    if *arg == av && results.len() == 1 && dfg.value_type(results[0]) == I64 {
                        objs.insert(results[0]);
                    }
                }
            }
        }
        // block parameters (and selects) fed only by objects and harmless constants: assume
        // all are, then drop those with any other input until nothing changes
        let mut maybe: HashSet<Value> = cands.iter().copied().collect();
        let mut selects: Vec<(Value, Value, Value)> = Vec::new();
        for block in func.layout.blocks() {
            for inst in func.layout.block_insts(block) {
                if let InstructionData::Ternary { opcode: Opcode::Select, args } = dfg.insts[inst] {
                    let r = dfg.first_result(inst);
                    if dfg.value_type(r) == I64 {
                        maybe.insert(r);
                        selects.push((r, args[1], args[2]));
                    }
                }
            }
        }
        let is_obj = |v: Value, maybe: &HashSet<Value>| {
            let v = dfg.resolve_aliases(v);
            objs.contains(&v) || maybe.contains(&v) || harmless_const(func, v)
        };
        loop {
            let mut changed = false;
            for block in func.layout.blocks() {
                let Some(inst) = func.layout.last_inst(block) else { continue };
                for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                    let dest = call.block(&dfg.value_lists);
                    let ps = dfg.block_params(dest);
                    for (i, a) in call.args(&dfg.value_lists).enumerate() {
                        let Some(&p) = ps.get(i) else { break };
                        if !maybe.contains(&p) {
                            continue;
                        }
                        let ok = match a {
                            BlockArg::Value(v) => is_obj(v, &maybe),
                            _ => false,
                        };
                        if !ok {
                            maybe.remove(&p);
                            changed = true;
                        }
                    }
                }
            }
            for &(r, x, y) in &selects {
                if maybe.contains(&r) && !(is_obj(x, &maybe) && is_obj(y, &maybe)) {
                    maybe.remove(&r);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // (a block parameter no branch passes anything to stays out: an entry from elsewhere)
        let mut fed: HashSet<Value> = HashSet::new();
        for block in func.layout.blocks() {
            let Some(inst) = func.layout.last_inst(block) else { continue };
            for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                let dest = call.block(&dfg.value_lists);
                let n = call.args(&dfg.value_lists).len();
                fed.extend(dfg.block_params(dest).iter().take(n).copied());
            }
        }
        for &(r, _, _) in &selects {
            fed.insert(r);
        }
        // (and one only constants reach holds no object: a loop's `iter = 50` stays a
        // constant the optimizer can fold, instead of a stack slot reloaded every iteration)
        let mut reached: HashSet<Value> = HashSet::new();
        loop {
            let mut changed = false;
            let real = |v: Value, reached: &HashSet<Value>| {
                let v = dfg.resolve_aliases(v);
                objs.contains(&v) || reached.contains(&v)
            };
            for block in func.layout.blocks() {
                let Some(inst) = func.layout.last_inst(block) else { continue };
                for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                    let dest = call.block(&dfg.value_lists);
                    let ps = dfg.block_params(dest);
                    for (i, a) in call.args(&dfg.value_lists).enumerate() {
                        let Some(&p) = ps.get(i) else { break };
                        if maybe.contains(&p) && !reached.contains(&p) && matches!(a, BlockArg::Value(v) if real(v, &reached)) {
                            reached.insert(p);
                            changed = true;
                        }
                    }
                }
            }
            for &(r, x, y) in &selects {
                if maybe.contains(&r) && !reached.contains(&r) && (real(x, &reached) || real(y, &reached)) {
                    reached.insert(r);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        objs.extend(maybe.into_iter().filter(|v| fed.contains(v) && reached.contains(v)));
        let mut objs: Vec<Value> = objs.into_iter().filter(|&v| !dfg.value_is_alias(v)).collect();
        objs.sort();
        for v in objs {
            b.declare_value_needs_stack_map(v);
        }
    }

    /// Keep a compiled function's stack maps (for `emit_stack_maps`, or an eval unit's).
    pub(crate) fn record_stack_maps(&mut self, id: FuncId, code: &cranelift_codegen::CompiledCode) {
        let maps = code.buffer.user_stack_maps();
        if maps.is_empty() {
            return;
        }
        // each site's slots as a bitmap of the frame's words (bit i: SP + 8 * i), from its
        // first word with a slot; sites with
        // the same slots share one (a long function keeps the same values across many calls)
        let mut words: Vec<u32> = vec![maps.len() as u32];
        let mut bitmaps: Vec<u32> = Vec::new();
        let mut seen: std::collections::HashMap<Vec<u32>, u32> = std::collections::HashMap::new();
        let base = 1 + 4 * maps.len();
        for (ret, span, map) in maps {
            let mut bm: Vec<u32> = Vec::new();
            // (a slot not on an 8-byte boundary is left out: scanned conservatively)
            for o in map.entries().filter(|(ty, o)| *ty == I64 && o % 8 == 0).map(|(_, o)| o / 8) {
                let (w, b) = ((o / 32) as usize, o % 32);
                if bm.len() <= w {
                    bm.resize(w + 1, 0);
                }
                bm[w] |= 1 << b;
            }
            // (leading zero words left out: count's high 16 bits say how many)
            let lead = bm.iter().take_while(|&&w| w == 0).count().min(0xFFFF);
            bm.drain(..lead);
            let n = bm.len() as u32 | (lead as u32) << 16;
            let at = match seen.get(&bm) {
                Some(&at) => at,
                None => {
                    let at = (base + bitmaps.len()) as u32;
                    bitmaps.extend_from_slice(&bm);
                    seen.insert(bm, at);
                    at
                }
            };
            words.extend([*ret, *span, at, n]);
        }
        words.extend(bitmaps);
        self.stack_maps.push((id, code.buffer.data().len() as u32, words));
    }
}

/// KENNY_STACK_MAPS=0 (at build time) compiles without stack maps (every frame is then
/// scanned conservatively, as before them).
pub(crate) fn stack_maps_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("KENNY_STACK_MAPS").map_or(true, |v| v != "0"))
}

impl Compiler<cranelift_object::ObjectModule> {
    /// Write the stack maps into the program and register them from `main` (before the
    /// runtime starts): `kn_register_stackmaps(functions, count, tables)`, each function
    /// entry [address: u64, code size: u32, table start: u32 (in words)].
    pub(crate) fn emit_stack_maps(&mut self, b: &mut FunctionBuilder) -> Result<(), String> {
        let maps = std::mem::take(&mut self.stack_maps);
        if maps.is_empty() {
            return Ok(());
        }
        let e = |x: cranelift_module::ModuleError| x.to_string();
        let fns_id = self.module.declare_data("__kn_smap_fns", Linkage::Local, false, false).map_err(e)?;
        let words_id = self.module.declare_data("__kn_smap_words", Linkage::Local, false, false).map_err(e)?;
        let mut fns = DataDescription::new();
        let mut fn_bytes = Vec::with_capacity(maps.len() * 16);
        let mut words: Vec<u8> = Vec::new();
        for (i, (id, size, table)) in maps.iter().enumerate() {
            let fr = self.module.declare_func_in_data(*id, &mut fns);
            fns.write_function_addr((i * 16) as u32, fr);
            fn_bytes.extend_from_slice(&0u64.to_le_bytes());
            fn_bytes.extend_from_slice(&size.to_le_bytes());
            fn_bytes.extend_from_slice(&((words.len() / 4) as u32).to_le_bytes());
            for w in table {
                words.extend_from_slice(&w.to_le_bytes());
            }
        }
        if std::env::var("KENNY_STACK_MAPS").is_ok_and(|v| v == "stats") {
            let sites: usize = maps.iter().map(|m| m.2[0] as usize).sum();
            let big = maps.iter().map(|m| (m.2.len(), m.2[0])).max().unwrap_or((0, 0));
            eprintln!("kenny: stack maps: {} functions, {} call sites, {} KB (largest function: {} KB, {} sites)", maps.len(), sites, words.len() / 1024, big.0 * 4 / 1024, big.1);
        }
        fns.define(fn_bytes.into_boxed_slice());
        fns.set_align(8);
        self.module.define_data(fns_id, &fns).map_err(e)?;
        let mut d = DataDescription::new();
        d.define(words.into_boxed_slice());
        d.set_align(8);
        self.module.define_data(words_id, &d).map_err(e)?;
        let reg = self.rt_id("kn_register_stackmaps");
        let reg = self.module.declare_func_in_func(reg, b.func);
        let fp = self.module.declare_data_in_func(fns_id, b.func);
        let fp = b.ins().symbol_value(I64, fp);
        let n = b.ins().iconst(I64, maps.len() as i64);
        let wp = self.module.declare_data_in_func(words_id, b.func);
        let wp = b.ins().symbol_value(I64, wp);
        b.ins().call(reg, &[fp, n, wp]);
        Ok(())
    }
}
