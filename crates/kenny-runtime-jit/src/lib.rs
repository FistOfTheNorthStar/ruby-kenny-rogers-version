//! The runtime with the JIT compiler linked in: used by programs that `eval` strings
//! (e.g. `class_eval <<~RUBY ... RUBY`). Exposes `kn_start_jit`, which installs the
//! eval hook and then starts the program like `kn_start`.

use kenny_compiler::jit::{self, EvalMode, JitUnit};
use kenny_runtime::core::*;
use std::collections::HashMap;
use std::ffi::c_char;

fn symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        ("kn_safepoint", kenny_runtime::gc::kn_safepoint as *const u8),
        ("kn_send", kenny_runtime::abi::kn_send as *const u8),
        ("kn_send_pub", kenny_runtime::abi::kn_send_pub as *const u8),
        ("kn_class_enter", kenny_runtime::abi::kn_class_enter as *const u8),
        ("kn_class_leave", kenny_runtime::abi::kn_class_leave as *const u8),
        ("kn_cvar_defined", kenny_runtime::abi::kn_cvar_defined as *const u8),
        ("kn_vcall", kenny_runtime::abi::kn_vcall as *const u8),
        ("kn_send_splat", kenny_runtime::abi::kn_send_splat as *const u8),
        ("kn_super", kenny_runtime::abi::kn_super as *const u8),
        ("kn_super_splat", kenny_runtime::abi::kn_super_splat as *const u8),
        ("kn_super_defined", kenny_runtime::abi::kn_super_defined as *const u8),
        ("kn_yield", kenny_runtime::abi::kn_yield as *const u8),
        ("kn_yield_splat", kenny_runtime::abi::kn_yield_splat as *const u8),
        ("kn_str_lit", kenny_runtime::abi::kn_str_lit as *const u8),
        ("kn_str_new", kenny_runtime::abi::kn_str_new as *const u8),
        ("kn_str_append_raw", kenny_runtime::abi::kn_str_append_raw as *const u8),
        ("kn_str_append", kenny_runtime::abi::kn_str_append as *const u8),
        ("kn_str_buf", kenny_runtime::abi::kn_str_buf as *const u8),
        ("kn_str_interp_raw", kenny_runtime::abi::kn_str_interp_raw as *const u8),
        ("kn_str_interp", kenny_runtime::abi::kn_str_interp as *const u8),
        ("kn_str_to_sym", kenny_runtime::abi::kn_str_to_sym as *const u8),
        ("kn_float_lit", kenny_runtime::abi::kn_float_lit as *const u8),
        ("kn_int_lit", kenny_runtime::abi::kn_int_lit as *const u8),
        ("kn_ary_new", kenny_runtime::abi::kn_ary_new as *const u8),
        ("kn_ary_push", kenny_runtime::abi::kn_ary_push as *const u8),
        ("kn_ary_sum_vals", kenny_runtime::array::kn_ary_sum_vals as *const u8),
        ("kn_ary_concat_splat", kenny_runtime::abi::kn_ary_concat_splat as *const u8),
        ("kn_ary_len", kenny_runtime::abi::kn_ary_len as *const u8),
        ("kn_ary_ptr", kenny_runtime::abi::kn_ary_ptr as *const u8),
        ("kn_hash_new", kenny_runtime::abi::kn_hash_new as *const u8),
        ("kn_hash_new_syms", kenny_runtime::abi::kn_hash_new_syms as *const u8),
        ("kn_hash_merge_splat", kenny_runtime::abi::kn_hash_merge_splat as *const u8),
        ("kn_range_new", kenny_runtime::abi::kn_range_new as *const u8),
        ("kn_proc_new", kenny_runtime::abi::kn_proc_new as *const u8),
        ("kn_env_new", kenny_runtime::abi::kn_env_new as *const u8),
        ("kn_to_proc", kenny_runtime::abi::kn_to_proc as *const u8),
        ("kn_ivar_get", kenny_runtime::abi::kn_ivar_get as *const u8),
        ("kn_ivar_set", kenny_runtime::abi::kn_ivar_set as *const u8),
        ("kn_gvar_get", kenny_runtime::abi::kn_gvar_get as *const u8),
        ("kn_gvar_set", kenny_runtime::abi::kn_gvar_set as *const u8),
        ("kn_cvar_get", kenny_runtime::abi::kn_cvar_get as *const u8),
        ("kn_cvar_get_soft", kenny_runtime::abi::kn_cvar_get_soft as *const u8),
        ("kn_cvar_set", kenny_runtime::abi::kn_cvar_set as *const u8),
        ("kn_const_get", kenny_runtime::abi::kn_const_get as *const u8),
        ("kn_const_get_c", kenny_runtime::abi::kn_const_get_c as *const u8),
        ("kn_const_get_l", kenny_runtime::abi::kn_const_get_l as *const u8),
        ("kn_const_defined_l", kenny_runtime::abi::kn_const_defined_l as *const u8),
        ("kn_const_defined_under", kenny_runtime::abi::kn_const_defined_under as *const u8),
        ("kn_const_get_under", kenny_runtime::abi::kn_const_get_under as *const u8),
        ("kn_const_set", kenny_runtime::abi::kn_const_set as *const u8),
        ("kn_const_defined", kenny_runtime::abi::kn_const_defined as *const u8),
        ("kn_object_class", kenny_runtime::abi::kn_object_class as *const u8),
        ("kn_main_obj", kenny_runtime::abi::kn_main_obj as *const u8),
        ("kn_define_class", kenny_runtime::abi::kn_define_class as *const u8),
        ("kn_define_module", kenny_runtime::abi::kn_define_module as *const u8),
        ("kn_singleton_class_for", kenny_runtime::abi::kn_singleton_class_for as *const u8),
        ("kn_def", kenny_runtime::abi::kn_def as *const u8),
        ("kn_def_claim", kenny_runtime::abi::kn_def_claim as *const u8),
        ("kn_static_fill", kenny_runtime::abi::kn_static_fill as *const u8),
        ("kn_entry_slow", kenny_runtime::abi::kn_entry_slow as *const u8),
        ("kn_new_fill", kenny_runtime::abi::kn_new_fill as *const u8),
        ("kn_obj_alloc", kenny_runtime::abi::kn_obj_alloc as *const u8),
        ("kn_math1", kenny_runtime::abi::kn_math1 as *const u8),
        ("kn_pow", kenny_runtime::abi::kn_pow as *const u8),
        ("kn_alias", kenny_runtime::abi::kn_alias as *const u8),
        ("kn_undef", kenny_runtime::abi::kn_undef as *const u8),
        ("kn_check_arity", kenny_runtime::abi::kn_check_arity as *const u8),
        ("kn_args_rest", kenny_runtime::abi::kn_args_rest as *const u8),
        ("kn_is_kwhash", kenny_runtime::abi::kn_is_kwhash as *const u8),
        ("kn_ary_push_kw", kenny_runtime::abi::kn_ary_push_kw as *const u8),
        ("kn_take_kwhash", kenny_runtime::abi::kn_take_kwhash as *const u8),
        ("kn_take_kwhash2", kenny_runtime::abi::kn_take_kwhash2 as *const u8),
        ("kn_kw_dup", kenny_runtime::abi::kn_kw_dup as *const u8),
        ("kn_kw_take", kenny_runtime::abi::kn_kw_take as *const u8),
        ("kn_kw_get", kenny_runtime::abi::kn_kw_get as *const u8),
        ("kn_kw_count", kenny_runtime::abi::kn_kw_count as *const u8),
        ("kn_kw_unknown", kenny_runtime::abi::kn_kw_unknown as *const u8),
        ("kn_kw_missing", kenny_runtime::abi::kn_kw_missing as *const u8),
        ("kn_kw_assert_empty", kenny_runtime::abi::kn_kw_assert_empty as *const u8),
        ("kn_splat_to_ary", kenny_runtime::abi::kn_splat_to_ary as *const u8),
        ("kn_masgn_ary", kenny_runtime::abi::kn_masgn_ary as *const u8),
        ("kn_ary_ref", kenny_runtime::abi::kn_ary_ref as *const u8),
        ("kn_ary_slice_rest", kenny_runtime::abi::kn_ary_slice_rest as *const u8),
        ("kn_ary_post_ref", kenny_runtime::abi::kn_ary_post_ref as *const u8),
        ("kn_pending_exc", kenny_runtime::abi::kn_pending_exc as *const u8),
        ("kn_exc_match", kenny_runtime::abi::kn_exc_match as *const u8),
        ("kn_rescued", kenny_runtime::abi::kn_rescued as *const u8),
        ("kn_errinfo", kenny_runtime::abi::kn_errinfo as *const u8),
        ("kn_errinfo_restore", kenny_runtime::abi::kn_errinfo_restore as *const u8),
        ("kn_save_pending", kenny_runtime::abi::kn_save_pending as *const u8),
        ("kn_restore_pending", kenny_runtime::abi::kn_restore_pending as *const u8),
        ("kn_throw_break", kenny_runtime::abi::kn_throw_break as *const u8),
        ("kn_catch_break", kenny_runtime::abi::kn_catch_break as *const u8),
        ("kn_throw_return", kenny_runtime::abi::kn_throw_return as *const u8),
        ("kn_catch_return", kenny_runtime::abi::kn_catch_return as *const u8),
        ("kn_raise_msg", kenny_runtime::abi::kn_raise_msg as *const u8),
        ("kn_respond_to", kenny_runtime::abi::kn_respond_to as *const u8),
        ("kn_ivar_defined", kenny_runtime::abi::kn_ivar_defined as *const u8),
        ("kn_gvar_defined", kenny_runtime::abi::kn_gvar_defined as *const u8),
        ("kn_gvar_alias", kenny_runtime::abi::kn_gvar_alias as *const u8),
        ("kn_eq", kenny_runtime::abi::kn_eq as *const u8),
        ("kn_regexp_lit", kenny_runtime::regex::kn_regexp_lit as *const u8),
        ("kn_regexp_new", kenny_runtime::regex::kn_regexp_new as *const u8),
        ("kn_nth_ref", kenny_runtime::regex::kn_nth_ref as *const u8),
        ("kn_backref", kenny_runtime::regex::kn_backref as *const u8),
        ("kn_match_named", kenny_runtime::regex::kn_match_named as *const u8),
        ("kn_last_match", kenny_runtime::regex::kn_last_match as *const u8),
        ("kn_pm_deconstruct", kenny_runtime::abi::kn_pm_deconstruct as *const u8),
        ("kn_pm_deconstruct_keys", kenny_runtime::abi::kn_pm_deconstruct_keys as *const u8),
        ("kn_pm_key", kenny_runtime::abi::kn_pm_key as *const u8),
        ("kn_pm_hash_rest", kenny_runtime::abi::kn_pm_hash_rest as *const u8),
        ("kn_pm_size", kenny_runtime::abi::kn_pm_size as *const u8),
        ("kn_pm_slice", kenny_runtime::abi::kn_pm_slice as *const u8),
        ("kn_pm_nomatch", kenny_runtime::abi::kn_pm_nomatch as *const u8),
        ("kn_autoload", kenny_runtime::abi::kn_autoload as *const u8),
        ("kn_require_file", kenny_runtime::compat::kn_require_file as *const u8),
        ("kn_wb_env", kenny_runtime::abi::kn_wb_env as *const u8),
        ("kn_using", kenny_runtime::abi::kn_using as *const u8),
        ("kn_super_dyn", kenny_runtime::abi::kn_super_dyn as *const u8),
        ("kn_super_blk", kenny_runtime::abi::kn_super_blk as *const u8),
        ("kn_clear_pending", kenny_runtime::compat::kn_clear_pending as *const u8),
        ("kn_set_lexical_parent", kenny_runtime::compat::kn_set_lexical_parent as *const u8),
        ("kn_rational_lit", kenny_runtime::rational::kn_rational_lit as *const u8),
        ("kn_binding_new", kenny_runtime::compat::kn_binding_new as *const u8),
        ("kn_register_file", kenny_runtime::compat::kn_register_file as *const u8),
        ("kn_register_debug", kenny_runtime::backtrace::kn_register_debug as *const u8),
        ("kn_send_refined", kenny_runtime::abi::kn_send_refined as *const u8),
        ("kn_send0", kenny_runtime::opt::kn_send0 as *const u8),
        ("kn_send1", kenny_runtime::opt::kn_send1 as *const u8),
        ("kn_is_a_send", kenny_runtime::abi::kn_is_a_send as *const u8),
        ("kn_str_append_own", kenny_runtime::string::kn_str_append_own as *const u8),
        ("kn_str_idx_eq", kenny_runtime::string::kn_str_idx_eq as *const u8),
        ("kn_send2", kenny_runtime::opt::kn_send2 as *const u8),
        ("kn_attr_get", kenny_runtime::opt::kn_attr_get as *const u8),
        ("kn_is_a", kenny_runtime::abi::kn_is_a as *const u8),
        ("kn_is_a_fill", kenny_runtime::abi::kn_is_a_fill as *const u8),
        ("kn_attr_set", kenny_runtime::opt::kn_attr_set as *const u8),
        ("kn_opt_check", kenny_runtime::opt::kn_opt_check as *const u8),
        ("kn_ary_at", kenny_runtime::opt::kn_ary_at as *const u8),
        ("kn_autosplat", kenny_runtime::opt::kn_autosplat as *const u8),
        ("kn_range_first", kenny_runtime::opt::kn_range_first as *const u8),
        ("kn_range_last", kenny_runtime::opt::kn_range_last as *const u8),
        ("kn_float_box", kenny_runtime::opt::kn_float_box as *const u8),
        ("kn_float_unbox", kenny_runtime::opt::kn_float_unbox as *const u8),
        ("kn_is_float", kenny_runtime::opt::kn_is_float as *const u8),
        ("kn_vobj_fill", kenny_runtime::abi::kn_vobj_fill as *const u8),
        ("KN_GC_PENDING", &kenny_runtime::gc::KN_GC_PENDING as *const _ as *const u8),
        ("KN_DIRECT", &kenny_runtime::core::KN_DIRECT as *const _ as *const u8),
    ]
}

/// Compiled eval units by (mode, source). The least recently used ones are evicted past
/// KENNY_EVAL_CACHE entries (default 2000); an evicted unit's memory is reused once the GC
/// finds nothing can run it (methods it defined, blocks, frames: see gc::orphan_code).
struct Cached {
    f: CompiledFn,
    unit: u64,
    used: u64,
}
static CACHE: G<Option<HashMap<(u8, Vec<u8>), Cached>>> = G::new(None);
/// What freeing a unit needs: its memory, functions (for backtraces) and inline caches.
struct UnitMem {
    memory: Vec<(usize, usize, bool)>,
    funcs: Vec<usize>,
    caches: usize,
    cache_words: usize,
    owned_caches: Vec<(u32, u8)>,
}
static UNITS: G<Option<HashMap<u64, UnitMem>>> = G::new(None);
static CLOCK: G<u64> = G::new(0);
static SYMS: G<Vec<(&'static str, *const u8)>> = G::new(Vec::new());

fn cache_limit() -> usize {
    static L: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *L.get_or_init(|| std::env::var("KENNY_EVAL_CACHE").ok().and_then(|v| v.parse().ok()).unwrap_or(2000).max(1))
}

fn install(unit: &JitUnit) {
    kenny_runtime::backtrace::register_jit(&unit.debug, unit.strs);
    for (i, name) in unit.syms.iter().enumerate() {
        unsafe { *(unit.syms_ptr as *mut V).add(i) = mksym(intern(name)) };
    }
    kenny_runtime::gc::register_region(unit.caches as usize, unit.cache_words);
}

/// (compile lock held) Evict the least recently used tenth of the cache when it is full.
fn evict(cache: &mut HashMap<(u8, Vec<u8>), Cached>) {
    let limit = cache_limit();
    if cache.len() < limit {
        return;
    }
    let mut ages: Vec<u64> = cache.values().map(|c| c.used).collect();
    ages.sort_unstable();
    let cut = ages[(limit / 10).min(ages.len() - 1)];
    let units = UNITS.get().get_or_insert_with(HashMap::new);
    cache.retain(|_, c| {
        if c.used > cut {
            return true;
        }
        if let Some(u) = units.get(&c.unit) {
            let code = u.memory.iter().filter(|m| m.2).map(|m| (m.0, m.1)).collect();
            kenny_runtime::gc::orphan_code(c.unit, code);
        }
        false
    });
}

/// (compile lock held) Give back the memory of evicted units the GC found unreachable.
fn free_dead_units() {
    let dead = kenny_runtime::gc::take_dead_units();
    if dead.is_empty() {
        return;
    }
    let units = UNITS.get().get_or_insert_with(HashMap::new);
    for id in dead {
        if let Some(u) = units.remove(&id) {
            kenny_runtime::backtrace::unregister_jit(&u.funcs);
            kenny_runtime::abi::forget_defs(&u.funcs);
            kenny_runtime::gc::unpin_region(u.caches, u.cache_words);
            kenny_runtime::abi::free_owned_caches(u.caches, &u.owned_caches);
            kenny_runtime::gc::unregister_region(u.caches);
            jit::free_unit(&u.memory);
        }
    }
}

/// One compilation at a time (the cache and the JIT memory are shared by all threads).
/// Compiling never calls Ruby, so holding a plain mutex here can't block a collection for
/// longer than the compile itself.
static COMPILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The compiled unit for `key` (compiling `src` as `mode`, named `file`, on a miss), or the
/// SyntaxError raised.
fn compiled(key: (u8, Vec<u8>), src: &[u8], mode: EvalMode, file: &str) -> Result<CompiledFn, V> {
    let guard = COMPILE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = CACHE.get().get_or_insert_with(HashMap::new);
    let now = {
        let c = CLOCK.get();
        *c += 1;
        *c
    };
    if let Some(c) = cache.get_mut(&key) {
        c.used = now;
        return Ok(c.f);
    }
    free_dead_units();
    evict(cache);
    if std::env::var_os("KENNY_DEBUG_EVAL").is_some() {
        eprintln!("[eval] compiling {} bytes of {file} at {}: {}", src.len(), current_loc().unwrap_or_default(), String::from_utf8_lossy(&src[..src.len().min(80)]).replace('\n', " "));
    }
    match jit::compile(src, mode, file, SYMS.get()) {
        Ok(unit) => {
            install(&unit);
            let f: CompiledFn = unsafe { std::mem::transmute(unit.entry) };
            let id = now;
            let funcs = unit.debug.iter().map(|d| d.0).collect();
            UNITS.get().get_or_insert_with(HashMap::new).insert(id, UnitMem { memory: unit.memory, funcs, caches: unit.caches as usize, cache_words: unit.cache_words, owned_caches: unit.owned_caches });
            cache.insert(key, Cached { f, unit: id, used: now });
            Ok(f)
        }
        Err(e) => {
            drop(guard);
            let c = const_lookup(rt().c.object, sym("SyntaxError")).map(|c| c as *mut Class).unwrap_or(rt().c.script_error);
            Err(raise(c, &e))
        }
    }
}

fn eval_hook(src: &[u8], mode: u8, self_: V) -> V {
    let m = if mode == 1 { EvalMode::ClassEval } else { EvalMode::Eval };
    match compiled((mode, src.to_vec()), src, m, "(eval)") {
        Ok(f) => unsafe { f(self_, 0, std::ptr::null(), QNIL) },
        Err(e) => e,
    }
}

/// `load` / `require` of a source file at runtime (changed since the build, or not compiled
/// in): Rails code reloading in development.
fn load_hook(src: &[u8], path: &str) -> V {
    let mut k = path.as_bytes().to_vec();
    k.push(0);
    k.extend_from_slice(src);
    match compiled((2, k), src, EvalMode::File, path) {
        Ok(f) => {
            let r = unsafe { f(rt().main_obj, 0, std::ptr::null(), QNIL) };
            if r == UNDEF { UNDEF } else { QTRUE }
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_start_jit(
    argc: i32,
    argv: *const *const c_char,
    symnames: *const u8,
    nsyms: u64,
    symtab: *mut V,
    toplevel: CompiledFn,
    script: *const c_char,
    caches: *mut V,
    cache_words: u64,
) -> i32 {
    *SYMS.get() = symbols();
    *kenny_runtime::kernel::EVAL_HOOK.get() = Some(eval_hook);
    *kenny_runtime::compat::LOAD_HOOK.get() = Some(load_hook);
    unsafe { kenny_runtime::abi::kn_start(argc, argv, symnames, nsyms, symtab, toplevel, script, caches, cache_words) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn symbol_table_is_complete() {
        let have: Vec<&str> = super::symbols().iter().map(|(n, _)| *n).collect();
        for n in kenny_compiler::jit::required_symbols() {
            assert!(have.contains(&n), "missing runtime symbol {n}");
        }
    }
}
