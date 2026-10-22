//! Kenny compiler: Ruby source -> Prism AST -> Cranelift IR -> native object file.
//!
//! Every Ruby scope (top level, method, block, class body) becomes one native function.
//! Values are tagged 64-bit words (see kenny-runtime/src/core.rs). Calls go through
//! runtime functions with per-call-site inline caches; Integer and Float arithmetic and
//! comparisons are inlined (fastpath.rs), and small `times`/`each`/`map`... blocks become
//! loops in the caller (inline.rs).

mod analysis;
mod calls;
mod control;
mod expr;
mod fastpath;
mod inline;
mod statics;
mod pattern;
mod scopes;
mod spec;
mod vobj;
#[cfg(feature = "jit")]
pub mod jit;

use cranelift_codegen::ir::{
    self, condcodes::IntCC, types::I32, types::I64, AbiParam, Block, FuncRef, InstBuilder, MemFlagsData, StackSlotData,
    StackSlotKind, Value,
};
use cranelift_codegen::{isa, settings, settings::Configurable};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const QFALSE: i64 = 0x00;
pub const QNIL: i64 = 0x08;
pub const QTRUE: i64 = 0x14;
pub const UNDEF: i64 = 0x34;
pub const KW_MISSING: i64 = 0x24;
/// Kinds of `owned_caches` (must match kenny_runtime::abi::free_owned_caches).
pub const OWNED_CALL: u8 = 1;
pub const OWNED_CONST: u8 = 2;

/// Offsets into runtime structs read by compiled code.
pub const PROC_ENV_OFFSET: i32 = 24;
pub const ENV_PARENT: i32 = 0;
pub const ENV_BLK: i32 = 8;
pub const ENV_SLOTS: i32 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScopeKind {
    Top,
    Method,
    Block,
    Class,
}

pub(crate) enum Local {
    Var(Variable),
    Env(usize),
}

pub(crate) struct ScopeInfo {
    pub env_slots: HashMap<Vec<u8>, usize>,
}

pub(crate) struct LoopCtx {
    pub next: Block,
    pub exit: Block,
    pub redo: Block,
    /// `next` passes its value to `next` (inlined `map`/`select` blocks).
    pub next_val: bool,
}

#[derive(Clone)]
pub(crate) struct MethodCtx {
    pub name: Vec<u8>,
    pub owner_slot: usize,
    /// For blocks: env hops from the block's parent env to the method's env.
    /// None when the current function is the method itself.
    pub hops: Option<u32>,
    /// The method's parameters (for implicit `super` inside blocks).
    pub params: Vec<(Vec<u8>, u8)>,
    /// Scope-stack index of the method.
    pub scope: usize,
    /// Enclosing class/module names of its `def` (statics.rs), when known.
    pub lex: Option<Vec<Vec<u8>>>,
    /// `def self.x` / `def Foo.x`: `self` is most likely a class or module
    pub singleton: bool,
}

pub(crate) struct Fx<'a> {
    pub b: FunctionBuilder<'a>,
    pub kind: ScopeKind,
    pub lambda: bool,
    pub self_v: Variable,
    pub blk_v: Variable,
    pub argc_v: Variable,
    pub argv_v: Variable,
    pub env_v: Option<Variable>,
    pub penv_v: Option<Variable>,
    pub proc_v: Option<Variable>,
    pub tag_v: Variable,
    pub locals: HashMap<Vec<u8>, Local>,
    pub unwind: Vec<Block>,
    pub ensure_depth: usize,
    pub loops: Vec<LoopCtx>,
    pub retry: Vec<Block>,
    pub caches: Value,
    pub syms: Value,
    pub strs: Value,
    pub gcflag: Value,
    pub cref_slot: usize,
    pub method: Option<MethodCtx>,
    pub funcs: HashMap<&'static str, FuncRef>,
    /// Scope stack index of this function.
    pub scope: usize,
    /// Method parameters for implicit `super`: (name, 0=positional 1=rest 2=keyword 3=kwrest).
    pub zsuper: Vec<(Vec<u8>, u8)>,
    /// Start of the block body (target of `redo` inside a block).
    pub redo_block: Option<Block>,
    /// Locals of blocks inlined into this function (innermost last): a local at Prism
    /// depth d < len belongs to inlined block len-1-d; deeper ones are this function's
    /// at depth d - len.
    pub inline_levels: Vec<HashMap<Vec<u8>, Variable>>,
    /// values known not to be UNDEF (their exception paths were checked already): `check`
    /// skips them
    pub checked: std::collections::HashSet<Value>,
    /// Unboxed shadows of register locals assigned from arithmetic: (raw f64, kind) with
    /// kind 0 = the tagged variable is current, 1 = only the raw f64 is, 2 = both.
    pub num_locals: HashMap<Vec<u8>, (Variable, Variable)>,
    /// Locals that only ever get Integers (analysis::int_locals), and values read from
    /// them or computed from such values: a hint that a Float path is the unlikely one.
    pub int_locals: HashSet<Vec<u8>>,
    pub int_vals: HashSet<Value>,
    /// Locals that always hold Floats (analysis::typed_locals): (f64 register, the boxed
    /// Float or 0 when not boxed yet). The box keeps a heap Float's identity (`x.equal?(x)`,
    /// a NaN Hash key) for as long as the local holds that value.
    /// (No box for a local only ever used unboxed: analysis::Typed::boxed.)
    pub flocals: HashMap<Vec<u8>, (Variable, Option<Variable>)>,
    /// Locals that always hold Integers (analysis::typed_locals), and values known to be
    /// Integers (Fixnum or Bignum): read from them or computed from such values.
    pub int_sure: HashSet<Vec<u8>>,
    pub sure_ints: HashSet<Value>,
    /// The scope's typed-locals analysis (references and exclusions, for spec.rs).
    pub typed_info: std::rc::Rc<analysis::Typed>,
    /// spec.rs: the speculative copy of a loop being compiled.
    pub spec: Option<spec::SpecCtx>,
    /// > 0 while compiling either copy of a speculated loop (no nested speculation).
    pub spec_depth: u32,
    /// Locals speculated to hold Fixnums (spec.rs), and values known to be Fixnums.
    pub int_spec: HashSet<Vec<u8>>,
    pub sure_fix: HashSet<Value>,
    /// spec.rs: in the generic copy of a speculated loop, the block to continue at after
    /// the local assignment at each source offset (entered from the speculative copy).
    pub resume: HashMap<usize, Block>,
    /// spec.rs: those places, as the fast copy asks for them
    pub resume_req: HashMap<usize, Block>,
    /// A method's fast entry (`FnSpec::fast_arity`): its argument values.
    pub fast_args: Option<Vec<Value>>,
    /// vobj.rs: locals that may hold a virtual object
    pub vlocals: HashMap<Vec<u8>, std::rc::Rc<vobj::VLocal>>,
    /// vobj.rs: compiling a virtual object's `initialize`: its instance variables
    pub vself: Option<HashMap<Vec<u8>, Variable>>,
    /// ... for the next inlined method body only
    pub vself_next: Option<HashMap<Vec<u8>, Variable>>,
}

pub struct Compiler<M: Module> {
    pub(crate) module: M,
    pub(crate) rt: HashMap<&'static str, FuncId>,
    pub(crate) syms: indexmap::IndexMap<Vec<u8>, usize>,
    pub(crate) strs: Vec<u8>,
    pub(crate) str_offsets: HashMap<Vec<u8>, usize>,
    pub(crate) cache_words: usize,
    /// Cache slots whose first word points to a heap entry the runtime allocates (word
    /// offset, OWNED_CALL / OWNED_CONST): freed with an eval unit.
    pub(crate) owned_caches: Vec<(u32, u8)>,
    /// statics.rs: the program's `def`s with predeclared fast entries, by name
    pub(crate) static_defs: HashMap<Vec<u8>, Vec<statics::StaticDef>>,
    /// keyword `def`s of the program by name (statics.rs `KwDef`)
    pub(crate) kw_defs: HashMap<Vec<u8>, Vec<statics::KwDef>>,
    pub(crate) static_defined: HashSet<FuncId>,
    pub(crate) static_counter: usize,
    /// (file, offset) of every `def` scanned -> its enclosing class/module names
    pub(crate) def_lex: HashMap<(PathBuf, usize), Vec<Vec<u8>>>,
    /// statics.rs: methods that may be inlined, by (file, offset) of their `def`
    pub(crate) inline_defs: HashMap<(PathBuf, usize), std::rc::Rc<statics::InlineDef>>,
    /// statics.rs: the inlined methods being compiled, innermost last
    pub(crate) inline_ctx: Vec<statics::InlineFrame>,
    /// (file, offset) of the `def`s whose bodies are being compiled
    pub(crate) def_stack: Vec<(PathBuf, usize)>,
    /// (file, offset) of a compiled `def` -> the cache slot of its constant scope (cref)
    pub(crate) def_cref: HashMap<(PathBuf, usize), usize>,
    /// vobj.rs: the getters of the classes of the program, by class name
    pub(crate) class_getters: HashMap<Vec<u8>, vobj::Getters>,
    pub(crate) caches_id: DataId,
    pub(crate) syms_id: DataId,
    pub(crate) strs_id: DataId,
    pub(crate) gc_flag_id: DataId,
    /// kenny-runtime core.rs `KN_DIRECT` (method serial copy, direct-call stack limit).
    pub(crate) direct_id: DataId,
    /// This module's direct-call stubs (fastpath.rs `direct_stub`), by kind.
    pub(crate) direct_stubs: HashMap<u8, FuncId>,
    /// Source offset of the node being compiled (for call-site locations).
    pub(crate) cur_offset: usize,
    pub(crate) fn_counter: usize,
    pub(crate) scopes: Vec<ScopeInfo>,
    pub(crate) errors: Vec<String>,
    pub(crate) file: PathBuf,
    pub(crate) src: Vec<u8>,
    pub(crate) loaded: HashMap<PathBuf, (FuncId, usize)>,
    pub(crate) loading: HashSet<PathBuf>,
    /// The program evaluates strings at runtime: link the runtime compiler.
    pub needs_jit: bool,
    /// Compiling an eval'd string: `def` uses the dynamic definee.
    pub(crate) eval_unit: bool,
    /// (with eval_unit) a source file loaded at runtime (`load` of a changed or new file):
    /// top-level `def` and labels as in a required file.
    pub(crate) file_unit: bool,
    /// content hash of every compiled file (`load` recompiles a file whose disk copy differs)
    pub(crate) src_hashes: HashMap<PathBuf, u64>,
    pub(crate) load_paths: Vec<PathBuf>,
    /// Canonical load paths and dependency directories (code compiled with gem semantics).
    pub(crate) dep_roots: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// Lexical refinement scope active at this point of the current file (`using`).
    pub(crate) refine_scope: Option<u64>,
    pub(crate) refine_counter: u64,
    /// The program's entry file (its top level is `<main>` in backtraces).
    pub(crate) main_file: PathBuf,
    /// Enclosing scope's cref slot of each class body's cref slot.
    pub(crate) cref_parents: HashMap<usize, usize>,
    /// Backtrace labels of the functions being compiled (innermost last).
    pub(crate) fn_labels: Vec<String>,
    /// Per compiled function: return-address offsets of its calls and their source locations.
    pub(crate) debug_fns: Vec<DebugFn>,
    /// Start offset of every line of the current source (for `line_of`), keyed by the source.
    line_index: (PathBuf, usize, usize, Vec<usize>),
}

/// Backtrace information of one compiled function: its file and label (offsets into the
/// string table) and, for each call, the offset of its return address and its source line.
/// Consecutive calls on the same line share one entry.
pub(crate) struct DebugFn {
    pub id: FuncId,
    pub size: u32,
    pub label: u32,
    pub file: u32,
    pub calls: Vec<(u32, u32)>,
    /// Only `calls` are frames (kenny-runtime backtrace.rs `Func::exact`; written as the
    /// size's top bit).
    pub exact: bool,
}
pub(crate) const DEBUG_EXACT: u32 = 1 << 31;

/// (name, params, returns) of every runtime function compiled code may call.
pub const RUNTIME: &[(&str, &[ir::Type], &[ir::Type])] = &[
    ("kn_start", &[I32, I64, I64, I64, I64, I64, I64, I64, I64], &[I32]),
    ("kn_start_jit", &[I32, I64, I64, I64, I64, I64, I64, I64, I64], &[I32]),
    ("kn_safepoint", &[], &[I64]),
    ("kn_send", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_send_pub", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_class_enter", &[I64], &[I64]),
    ("kn_class_leave", &[I64], &[I64]),
    ("kn_cvar_defined", &[I64, I64], &[I64]),
    ("kn_vcall", &[I64, I64, I64], &[I64]),
    ("kn_send_splat", &[I64, I64, I64, I64, I64], &[I64]),
    ("kn_super", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_super_splat", &[I64, I64, I64, I64, I64], &[I64]),
    ("kn_super_defined", &[I64, I64, I64], &[I64]),
    ("kn_yield", &[I64, I64, I64], &[I64]),
    ("kn_yield_splat", &[I64, I64], &[I64]),
    ("kn_str_lit", &[I64, I64, I64], &[I64]),
    ("kn_str_new", &[I64, I64], &[I64]),
    ("kn_str_append_raw", &[I64, I64, I64], &[I64]),
    ("kn_str_append", &[I64, I64], &[I64]),
    ("kn_str_to_sym", &[I64], &[I64]),
    ("kn_float_lit", &[I64, I64], &[I64]),
    ("kn_int_lit", &[I64, I64, I64], &[I64]),
    ("kn_ary_new", &[I64, I64], &[I64]),
    ("kn_ary_push", &[I64, I64], &[I64]),
    ("kn_ary_sum_vals", &[I64], &[I64]),
    ("kn_ary_concat_splat", &[I64, I64], &[I64]),
    ("kn_ary_len", &[I64], &[I64]),
    ("kn_ary_ptr", &[I64], &[I64]),
    ("kn_hash_new", &[I64, I64, I64], &[I64]),
    ("kn_hash_new_syms", &[I64, I64, I64], &[I64]),
    ("kn_hash_merge_splat", &[I64, I64], &[I64]),
    ("kn_range_new", &[I64, I64, I64], &[I64]),
    ("kn_proc_new", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_env_new", &[I64, I64, I64], &[I64]),
    ("kn_to_proc", &[I64], &[I64]),
    ("kn_ivar_get", &[I64, I64, I64], &[I64]),
    ("kn_ivar_set", &[I64, I64, I64, I64], &[I64]),
    ("kn_gvar_get", &[I64], &[I64]),
    ("kn_gvar_set", &[I64, I64], &[I64]),
    ("kn_cvar_get", &[I64, I64], &[I64]),
    ("kn_cvar_get_soft", &[I64, I64], &[I64]),
    ("kn_cvar_set", &[I64, I64, I64], &[I64]),
    ("kn_const_get", &[I64, I64], &[I64]),
    ("kn_const_get_c", &[I64, I64, I64], &[I64]),
    ("kn_const_get_l", &[I64, I64, I64], &[I64]),
    ("kn_const_defined_l", &[I64, I64], &[I64]),
    ("kn_const_defined_under", &[I64, I64], &[I64]),
    ("kn_const_get_under", &[I64, I64], &[I64]),
    ("kn_const_set", &[I64, I64, I64], &[I64]),
    ("kn_const_defined", &[I64, I64], &[I64]),
    ("kn_object_class", &[], &[I64]),
    ("kn_main_obj", &[], &[I64]),
    ("kn_define_class", &[I64, I64, I64], &[I64]),
    ("kn_define_module", &[I64, I64], &[I64]),
    ("kn_singleton_class_for", &[I64], &[I64]),
    ("kn_def", &[I64, I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_static_fill", &[I64, I64, I64, I64], &[I64]),
    ("kn_new_fill", &[I64, I64, I64], &[I64]),
    ("kn_obj_alloc", &[I64], &[I64]),
    ("kn_math1", &[I64, ir::types::F64], &[ir::types::F64]),
    ("kn_pow", &[ir::types::F64, ir::types::F64], &[ir::types::F64]),
    ("kn_alias", &[I64, I64, I64], &[I64]),
    ("kn_undef", &[I64, I64], &[I64]),
    ("kn_check_arity", &[I64, I64, I64], &[I64]),
    ("kn_args_rest", &[I64, I64, I64, I64], &[I64]),
    ("kn_is_kwhash", &[I64], &[I64]),
    ("kn_ary_push_kw", &[I64, I64], &[I64]),
    ("kn_take_kwhash", &[I64, I64, I64], &[I64]),
    ("kn_take_kwhash2", &[I64, I64, I64], &[I64]),
    ("kn_kw_dup", &[I64], &[I64]),
    ("kn_kw_take", &[I64, I64], &[I64]),
    ("kn_kw_get", &[I64, I64], &[I64]),
    ("kn_kw_count", &[I64], &[I64]),
    ("kn_kw_unknown", &[I64, I64, I64], &[I64]),
    ("kn_kw_missing", &[I64], &[I64]),
    ("kn_kw_assert_empty", &[I64], &[I64]),
    ("kn_splat_to_ary", &[I64], &[I64]),
    ("kn_masgn_ary", &[I64], &[I64]),
    ("kn_ary_ref", &[I64, I64], &[I64]),
    ("kn_ary_slice_rest", &[I64, I64, I64], &[I64]),
    ("kn_ary_post_ref", &[I64, I64, I64, I64], &[I64]),
    ("kn_pending_exc", &[], &[I64]),
    ("kn_exc_match", &[I64, I64], &[I64]),
    ("kn_rescued", &[I64], &[]),
    ("kn_errinfo", &[], &[I64]),
    ("kn_errinfo_restore", &[I64], &[]),
    ("kn_save_pending", &[], &[I64]),
    ("kn_restore_pending", &[I64], &[I64]),
    ("kn_throw_break", &[I64, I64], &[I64]),
    ("kn_catch_break", &[I64], &[I64]),
    ("kn_throw_return", &[I64, I64], &[I64]),
    ("kn_catch_return", &[I64], &[I64]),
    ("kn_raise_msg", &[I64, I64, I64], &[I64]),
    ("kn_respond_to", &[I64, I64], &[I64]),
    ("kn_ivar_defined", &[I64, I64], &[I64]),
    ("kn_gvar_defined", &[I64], &[I64]),
    ("kn_gvar_alias", &[I64, I64], &[I64]),
    ("kn_eq", &[I64, I64], &[I64]),
    ("kn_regexp_lit", &[I64, I64, I64, I64], &[I64]),
    ("kn_regexp_new", &[I64, I64], &[I64]),
    ("kn_nth_ref", &[I64], &[I64]),
    ("kn_backref", &[I64], &[I64]),
    ("kn_match_named", &[I64], &[I64]),
    ("kn_last_match", &[], &[I64]),
    ("kn_pm_deconstruct", &[I64], &[I64]),
    ("kn_pm_deconstruct_keys", &[I64, I64], &[I64]),
    ("kn_pm_key", &[I64, I64], &[I64]),
    ("kn_pm_hash_rest", &[I64, I64], &[I64]),
    ("kn_pm_size", &[I64], &[I64]),
    ("kn_pm_slice", &[I64, I64, I64], &[I64]),
    ("kn_pm_nomatch", &[I64], &[I64]),
    ("kn_autoload", &[I64, I64, I64, I64], &[I64]),
    ("kn_require_file", &[I64, I64], &[I64]),
    ("kn_wb_env", &[I64], &[I64]),
    ("kn_using", &[I64, I64, I64], &[I64]),
    ("kn_super_dyn", &[I64, I64, I64], &[I64]),
    ("kn_super_blk", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_clear_pending", &[], &[I64]),
    ("kn_set_lexical_parent", &[I64, I64], &[I64]),
    ("kn_rational_lit", &[I64, I64], &[I64]),
    ("kn_binding_new", &[I64], &[I64]),
    ("kn_register_file", &[I64, I64, I64, I64, I64], &[]),
    ("kn_register_debug", &[I64, I64, I64, I64], &[]),
    ("kn_send_refined", &[I64, I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_send0", &[I64, I64, I64, I64], &[I64]),
    ("kn_send1", &[I64, I64, I64, I64, I64], &[I64]),
    ("kn_send2", &[I64, I64, I64, I64, I64, I64], &[I64]),
    ("kn_attr_get", &[I64, I64, I64, I64], &[I64]),
    ("kn_is_a", &[I64, I64], &[I64]),
    ("kn_is_a_fill", &[I64, I64, I64], &[I64]),
    ("kn_attr_set", &[I64, I64, I64, I64, I64], &[I64]),
    ("kn_opt_check", &[I64, I64, I64, I64], &[I64]),
    ("kn_ary_at", &[I64, I64], &[I64]),
    ("kn_autosplat", &[I64], &[I64]),
    ("kn_range_first", &[I64], &[I64]),
    ("kn_range_last", &[I64], &[I64]),
    ("kn_float_box", &[I64], &[I64]),
    ("kn_float_unbox", &[I64], &[ir::types::F64]),
    ("kn_is_float", &[I64], &[I64]),
    ("kn_vobj_fill", &[I64, I64, I64, I64, I64], &[I64]),
];

pub struct Output {
    pub object: Vec<u8>,
    pub needs_jit: bool,
    pub warnings: Vec<String>,
}

#[derive(Default, Clone)]
pub struct Options {
    /// Directories searched by `require "name"` at build time.
    pub load_paths: Vec<PathBuf>,
    /// Directories whose every .rb file is compiled in, for requires/autoloads whose path is
    /// only known at runtime (e.g. ActiveSupport::Autoload).
    pub embed_dirs: Vec<PathBuf>,
    /// Single files always compiled in (runtime-loaded support code such as `kenny/gem`).
    pub embed_files: Vec<PathBuf>,
    /// Further directories of dependency code that are not load paths (a Rails engine's
    /// `app/` and `config/`): compiled like gem code (see `in_dependency`).
    pub dep_dirs: Vec<PathBuf>,
}

pub fn compile_file(path: &Path) -> Result<Output, Vec<String>> {
    compile_file_with(path, &Options::default())
}

pub fn compile_file_with(path: &Path, opts: &Options) -> Result<Output, Vec<String>> {
    let src = std::fs::read(path).map_err(|e| vec![format!("{}: {}", path.display(), e)])?;
    compile_with(&src, path, opts)
}

pub fn compile(src: &[u8], path: &Path) -> Result<Output, Vec<String>> {
    compile_with(src, path, &Options::default())
}

pub fn compile_with(src: &[u8], path: &Path, opts: &Options) -> Result<Output, Vec<String>> {
    let mut c = Compiler::new_object(path, src).map_err(|e| vec![e])?;
    c.load_paths = opts.load_paths.clone();
    c.dep_roots = opts.load_paths.iter().chain(opts.dep_dirs.iter()).map(|p| p.canonicalize().unwrap_or(p.clone())).collect();
    let top = c.compile_program(src, path);
    c.embed(&opts.embed_dirs);
    for f in &opts.embed_files {
        c.require_file(f.clone(), 0);
    }
    if !c.errors.is_empty() {
        return Err(c.errors);
    }
    let top = top.map_err(|e| vec![e])?;
    c.finish(top).map_err(|e| vec![e])
}

pub(crate) fn mem() -> MemFlagsData {
    MemFlagsData::trusted()
}

pub(crate) fn host_isa(pic: bool) -> Result<std::sync::Arc<dyn isa::TargetIsa>, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    // backtraces walk the frame-pointer chain (kenny-runtime/src/backtrace.rs)
    flags.set("preserve_frame_pointers", "true").map_err(|e| e.to_string())?;
    flags.set("is_pic", if pic { "true" } else { "false" }).map_err(|e| e.to_string())?;
    let isa_builder = isa::lookup(target_lexicon::Triple::host()).map_err(|e| e.to_string())?;
    isa_builder.finish(settings::Flags::new(flags)).map_err(|e| e.to_string())
}

impl Compiler<ObjectModule> {
    fn new_object(path: &Path, src: &[u8]) -> Result<Self, String> {
        let ob = ObjectBuilder::new(host_isa(true)?, "kenny_program", cranelift_module::default_libcall_names()).map_err(|e| e.to_string())?;
        Self::with_module(ObjectModule::new(ob), path, src)
    }
}

impl<M: Module> Compiler<M> {
    pub(crate) fn with_module(mut module: M, path: &Path, src: &[u8]) -> Result<Self, String> {
        // runtime functions are declared on first use (see `rt_id`)
        let rt = HashMap::new();
        let caches_id = module.declare_data("__kn_caches", Linkage::Local, true, false).map_err(|e| e.to_string())?;
        let syms_id = module.declare_data("__kn_syms", Linkage::Local, true, false).map_err(|e| e.to_string())?;
        let strs_id = module.declare_data("__kn_strs", Linkage::Local, false, false).map_err(|e| e.to_string())?;
        let gc_flag_id = module.declare_data("KN_GC_PENDING", Linkage::Import, true, false).map_err(|e| e.to_string())?;
        let direct_id = module.declare_data("KN_DIRECT", Linkage::Import, true, false).map_err(|e| e.to_string())?;
        Ok(Compiler {
            module,
            rt,
            syms: indexmap::IndexMap::new(),
            strs: Vec::new(),
            str_offsets: HashMap::new(),
            cache_words: 0,
            owned_caches: Vec::new(),
            static_defs: HashMap::new(),
            kw_defs: HashMap::new(),
            static_defined: HashSet::new(),
            static_counter: 0,
            def_lex: HashMap::new(),
            inline_defs: HashMap::new(),
            inline_ctx: Vec::new(),
            def_stack: Vec::new(),
            def_cref: HashMap::new(),
            class_getters: HashMap::new(),
            caches_id,
            syms_id,
            strs_id,
            gc_flag_id,
            direct_id,
            direct_stubs: HashMap::new(),
            cur_offset: 0,
            fn_counter: 0,
            scopes: Vec::new(),
            errors: Vec::new(),
            file: path.to_path_buf(),
            src: src.to_vec(),
            loaded: HashMap::new(),
            loading: HashSet::new(),
            needs_jit: false,
            eval_unit: false,
            file_unit: false,
            src_hashes: HashMap::new(),
            load_paths: Vec::new(),
            dep_roots: Vec::new(),
            warnings: Vec::new(),
            refine_scope: None,
            refine_counter: 0,
            main_file: path.to_path_buf(),
            fn_labels: Vec::new(),
            cref_parents: HashMap::new(),
            debug_fns: Vec::new(),
            line_index: (PathBuf::new(), 0, 0, Vec::new()),
        })
    }
}

impl Compiler<ObjectModule> {
    fn finish(mut self, toplevel: FuncId) -> Result<Output, String> {
        let needs_jit = self.needs_jit;
        let warnings = std::mem::take(&mut self.warnings);
        let e = |x: cranelift_module::ModuleError| x.to_string();
        // main(argc, argv) -> kn_start(...)
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(I32));
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I32));
        let main_id = self.module.declare_function("main", Linkage::Export, &sig).map_err(e)?;
        let names_id = self.module.declare_data("__kn_symnames", Linkage::Local, false, false).map_err(e)?;
        let script_id = self.module.declare_data("__kn_script", Linkage::Local, false, false).map_err(e)?;
        {
            let mut func = ir::Function::with_name_signature(ir::UserFuncName::user(0, main_id.as_u32()), sig);
            let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
            let mut b = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let (argc, argv) = (b.block_params(entry)[0], b.block_params(entry)[1]);
            let gv = |m: &mut ObjectModule, b: &mut FunctionBuilder, id: DataId| {
                let g = m.declare_data_in_func(id, b.func);
                b.ins().symbol_value(I64, g)
            };
            let names = gv(&mut self.module, &mut b, names_id);
            let syms = gv(&mut self.module, &mut b, self.syms_id);
            let script = gv(&mut self.module, &mut b, script_id);
            let nsyms = b.ins().iconst(I64, self.syms.len() as i64);
            // file table for runtime `require` / autoload of precompiled files
            let reg_id = self.rt_id("kn_register_file");
            let reg = self.module.declare_func_in_func(reg_id, b.func);
            let caches_base = gv(&mut self.module, &mut b, self.caches_id);
            let strs_base = gv(&mut self.module, &mut b, self.strs_id);
            for (key, fid, flag, hash) in self.file_table() {
                let off = self.str_offset(key.as_bytes());
                let kp = b.ins().iadd_imm_s(strs_base, off as i64);
                let kl = b.ins().iconst(I64, key.len() as i64);
                let fr = self.module.declare_func_in_func(fid, b.func);
                let fa = b.ins().func_addr(I64, fr);
                let fl = b.ins().iadd_imm_s(caches_base, (flag * 8) as i64);
                let h = b.ins().iconst(I64, hash as i64);
                b.ins().call(reg, &[kp, kl, fa, fl, h]);
            }
            // backtrace tables: functions (address, size, label, calls) and their call sites
            {
                let fns_id = self.module.declare_data("__kn_dbg_fns", Linkage::Local, false, false).map_err(e)?;
                let calls_id = self.module.declare_data("__kn_dbg_calls", Linkage::Local, false, false).map_err(e)?;
                let mut fns = DataDescription::new();
                let mut fn_bytes = Vec::with_capacity(self.debug_fns.len() * 32);
                let mut call_bytes = Vec::new();
                let debug_fns = std::mem::take(&mut self.debug_fns);
                for (i, f) in debug_fns.iter().enumerate() {
                    let fr = self.module.declare_func_in_data(f.id, &mut fns);
                    fns.write_function_addr((i * 32) as u32, fr);
                    fn_bytes.extend_from_slice(&0u64.to_le_bytes());
                    fn_bytes.extend_from_slice(&(f.size | if f.exact { DEBUG_EXACT } else { 0 }).to_le_bytes());
                    fn_bytes.extend_from_slice(&f.label.to_le_bytes());
                    fn_bytes.extend_from_slice(&f.file.to_le_bytes());
                    fn_bytes.extend_from_slice(&0u32.to_le_bytes());
                    fn_bytes.extend_from_slice(&((call_bytes.len() / 8) as u32).to_le_bytes());
                    fn_bytes.extend_from_slice(&(f.calls.len() as u32).to_le_bytes());
                    for (ret, loc) in &f.calls {
                        call_bytes.extend_from_slice(&ret.to_le_bytes());
                        call_bytes.extend_from_slice(&loc.to_le_bytes());
                    }
                }
                fn_bytes.resize(fn_bytes.len().max(8), 0);
                call_bytes.resize(call_bytes.len().max(8), 0);
                fns.define(fn_bytes.into_boxed_slice());
                fns.set_align(8);
                self.module.define_data(fns_id, &fns).map_err(e)?;
                let mut calls = DataDescription::new();
                calls.define(call_bytes.into_boxed_slice());
                calls.set_align(8);
                self.module.define_data(calls_id, &calls).map_err(e)?;
                let reg_dbg = self.rt_id("kn_register_debug");
                let reg_dbg = self.module.declare_func_in_func(reg_dbg, b.func);
                let fp = gv(&mut self.module, &mut b, fns_id);
                let n = b.ins().iconst(I64, debug_fns.len() as i64);
                let cp = gv(&mut self.module, &mut b, calls_id);
                b.ins().call(reg_dbg, &[fp, n, cp, strs_base]);
            }
            let top_ref = self.module.declare_func_in_func(toplevel, b.func);
            let top = b.ins().func_addr(I64, top_ref);
            let start_name = if needs_jit { "kn_start_jit" } else { "kn_start" };
            let start_id = self.rt_id(if needs_jit { "kn_start_jit" } else { "kn_start" });
            let _ = start_name;
            let start = self.module.declare_func_in_func(start_id, b.func);
            let caches = gv(&mut self.module, &mut b, self.caches_id);
            let ncache = b.ins().iconst(I64, self.cache_words.max(1) as i64);
            let call = b.ins().call(start, &[argc, argv, names, nsyms, syms, top, script, caches, ncache]);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.seal_all_blocks();
            b.finalize(self.module.isa().frontend_config());
            let mut ctx = cranelift_codegen::Context::for_function(func);
            self.module.define_function(main_id, &mut ctx).map_err(|x| format!("main: {x:?}"))?;
        }
        let mut d = DataDescription::new();
        d.define_zeroinit(self.cache_words.max(1) * 8);
        d.set_align(8);
        self.module.define_data(self.caches_id, &d).map_err(e)?;
        let mut d = DataDescription::new();
        d.define_zeroinit(self.syms.len().max(1) * 8);
        d.set_align(8);
        self.module.define_data(self.syms_id, &d).map_err(e)?;
        // length-prefixed (u32 LE): symbol names may contain NUL bytes (e.g. `:"\0"`)
        let mut names = Vec::new();
        for k in self.syms.keys() {
            names.extend_from_slice(&(k.len() as u32).to_le_bytes());
            names.extend_from_slice(k);
        }
        let mut d = DataDescription::new();
        d.define(names.into_boxed_slice());
        self.module.define_data(names_id, &d).map_err(e)?;
        let mut strs = std::mem::take(&mut self.strs);
        strs.push(0);
        let mut d = DataDescription::new();
        d.define(strs.into_boxed_slice());
        self.module.define_data(self.strs_id, &d).map_err(e)?;
        let mut script = self.file.display().to_string().into_bytes();
        script.push(0);
        let mut d = DataDescription::new();
        d.define(script.into_boxed_slice());
        self.module.define_data(script_id, &d).map_err(e)?;
        let product = self.module.finish();
        let object = product.emit().map_err(|x| x.to_string())?;
        Ok(Output { object, needs_jit, warnings })
    }
}

impl<M: Module> Compiler<M> {

    // ---------- tables ----------

    pub(crate) fn sym_index(&mut self, name: &[u8]) -> usize {
        let n = self.syms.len();
        *self.syms.entry(name.to_vec()).or_insert(n)
    }
    pub(crate) fn cache_alloc(&mut self, words: usize) -> usize {
        let i = self.cache_words;
        self.cache_words += words;
        i
    }
    pub(crate) fn str_offset(&mut self, b: &[u8]) -> usize {
        if let Some(&o) = self.str_offsets.get(b) {
            return o;
        }
        let o = self.strs.len();
        self.strs.extend_from_slice(b);
        self.str_offsets.insert(b.to_vec(), o);
        o
    }

    pub(crate) fn line_of(&mut self, offset: usize) -> usize {
        let key = (self.src.as_ptr() as usize, self.src.len());
        if (self.line_index.1, self.line_index.2) != key || self.line_index.0 != self.file {
            let mut starts = vec![0];
            starts.extend(self.src.iter().enumerate().filter(|(_, c)| **c == b'\n').map(|(i, _)| i + 1));
            self.line_index = (self.file.clone(), key.0, key.1, starts);
        }
        let off = offset.min(self.src.len());
        self.line_index.3.partition_point(|&s| s <= off)
    }

    /// Tag the instructions emitted from here on with the current source line (for backtraces).
    pub(crate) fn set_srcloc(&mut self, fx: &mut Fx) {
        let loc = self.loc_bits();
        fx.b.set_srcloc(ir::SourceLoc::new(loc));
    }

    /// Keep the call sites' source locations of a compiled function (see `DebugFn`).
    pub(crate) fn record_debug_info(&mut self, id: FuncId, label: &str, code: &cranelift_codegen::CompiledCode) {
        let srclocs = code.buffer.get_srclocs_sorted();
        let mut calls: Vec<(u32, u32)> = Vec::new();
        for cs in code.buffer.call_sites() {
            let at = cs.ret_addr.saturating_sub(1);
            let i = srclocs.partition_point(|s| s.end <= at);
            let Some(s) = srclocs.get(i).filter(|s| s.start <= at && !s.loc.is_default()) else { continue };
            let loc = s.loc.bits();
            if calls.last().is_some_and(|l| l.1 == loc) {
                continue;
            }
            calls.push((cs.ret_addr, loc));
        }
        if calls.is_empty() {
            return;
        }
        let mut l = label.as_bytes().to_vec();
        l.push(0);
        let label = self.str_offset(&l) as u32;
        let mut f = self.file.display().to_string().into_bytes();
        f.push(0);
        let file = self.str_offset(&f) as u32;
        let size = code.buffer.data().len() as u32;
        self.debug_fns.push(DebugFn { id, size, label, file, calls, exact: false });
    }
    /// True when compiling a file found via a load path (a dependency, not the app itself).
    pub(crate) fn in_dependency(&self) -> bool {
        self.dep_roots.iter().any(|lp| self.file.starts_with(lp))
    }
    /// An unsupported or dropped feature: a compile error in the app's own code; in a
    /// dependency, a build warning plus a NotImplementedError if that code ever runs.
    pub(crate) fn unsupported(&mut self, fx: &mut Fx, offset: usize, msg: &str) -> Value {
        if self.in_dependency() {
            let line = self.line_of(offset);
            self.warnings.push(format!("{}:{}: {} (raises NotImplementedError if reached)", self.file.display(), line, msg));
            return self.raise_msg(fx, 2, msg);
        }
        self.error_at(offset, msg);
        self.nil(fx)
    }

    pub(crate) fn error_at(&mut self, offset: usize, msg: &str) {
        let line = self.line_of(offset);
        self.errors.push(format!("{}:{}: {}", self.file.display(), line, msg));
    }

    // ---------- emission helpers ----------

    /// The module-level id of runtime function `name`, declared the first time it is used
    /// (an `eval` unit typically needs a handful of the ~100 runtime functions).
    pub(crate) fn rt_id(&mut self, name: &'static str) -> FuncId {
        if let Some(&id) = self.rt.get(name) {
            return id;
        }
        let (_, params, rets) = RUNTIME.iter().find(|(n, _, _)| *n == name).unwrap_or_else(|| panic!("unknown runtime fn {name}"));
        let mut sig = self.module.make_signature();
        for p in *params {
            sig.params.push(AbiParam::new(*p));
        }
        for r in *rets {
            sig.returns.push(AbiParam::new(*r));
        }
        let id = self.module.declare_function(name, Linkage::Import, &sig).unwrap_or_else(|e| panic!("declaring {name}: {e}"));
        self.rt.insert(name, id);
        id
    }

    pub(crate) fn rt_ref(&mut self, fx: &mut Fx, name: &'static str) -> FuncRef {
        if let Some(&f) = fx.funcs.get(name) {
            return f;
        }
        let id = self.rt_id(name);
        let f = self.module.declare_func_in_func(id, fx.b.func);
        fx.funcs.insert(name, f);
        f
    }
    pub(crate) fn call(&mut self, fx: &mut Fx, name: &'static str, args: &[Value]) -> Value {
        let f = self.rt_ref(fx, name);
        let inst = fx.b.ins().call(f, args);
        match fx.b.inst_results(inst).first() {
            Some(&v) => v,
            None => fx.b.ins().iconst(I64, 0),
        }
    }
    /// Call + unwind if the result is UNDEF.
    pub(crate) fn callc(&mut self, fx: &mut Fx, name: &'static str, args: &[Value]) -> Value {
        let v = self.call(fx, name, args);
        self.check(fx, v);
        v
    }
    pub(crate) fn check(&mut self, fx: &mut Fx, v: Value) {
        if fx.checked.contains(&v) {
            return;
        }
        let is_undef = fx.b.ins().icmp_imm_s(IntCC::Equal, v, UNDEF);
        let cont = fx.b.create_block();
        let target = *fx.unwind.last().unwrap();
        fx.b.ins().brif(is_undef, target, &[], cont, &[]);
        fx.b.switch_to_block(cont);
    }
    pub(crate) fn jump_unwind(&mut self, fx: &mut Fx) {
        let target = *fx.unwind.last().unwrap();
        fx.b.ins().jump(target, &[]);
        self.dead_block(fx);
    }
    /// Start an unreachable block after a terminator so later statements have somewhere to go.
    pub(crate) fn dead_block(&mut self, fx: &mut Fx) {
        let b = fx.b.create_block();
        fx.b.switch_to_block(b);
    }
    pub(crate) fn iconst(&mut self, fx: &mut Fx, v: i64) -> Value {
        fx.b.ins().iconst(I64, v)
    }
    pub(crate) fn nil(&mut self, fx: &mut Fx) -> Value {
        fx.b.ins().iconst(I64, QNIL)
    }
    pub(crate) fn sym_val(&mut self, fx: &mut Fx, name: &[u8]) -> Value {
        let i = self.sym_index(name);
        fx.b.ins().load(I64, mem(), fx.syms, (i * 8) as i32)
    }
    pub(crate) fn cache_ptr(&mut self, fx: &mut Fx, words: usize) -> Value {
        let i = self.cache_alloc(words);
        fx.b.ins().iadd_imm_s(fx.caches, (i * 8) as i64)
    }
    /// A call-site (OWNED_CALL) or constant (OWNED_CONST) cache: 3 words, the first pointing to
    /// an entry the runtime allocates.
    pub(crate) fn owned_cache_ptr(&mut self, fx: &mut Fx, kind: u8) -> Value {
        let i = self.cache_alloc(3);
        self.owned_caches.push((i as u32, kind));
        fx.b.ins().iadd_imm_s(fx.caches, (i * 8) as i64)
    }
    pub(crate) fn slot_ptr(&mut self, fx: &mut Fx, idx: usize) -> Value {
        fx.b.ins().iadd_imm_s(fx.caches, (idx * 8) as i64)
    }
    pub(crate) fn str_ptr(&mut self, fx: &mut Fx, b: &[u8]) -> (Value, Value) {
        let off = self.str_offset(b);
        let p = fx.b.ins().iadd_imm_s(fx.strs, off as i64);
        let l = fx.b.ins().iconst(I64, b.len() as i64);
        (p, l)
    }
    /// Frozen, cached string literal.
    pub(crate) fn str_lit(&mut self, fx: &mut Fx, b: &[u8]) -> Value {
        let slot = self.cache_ptr(fx, 1);
        let (p, l) = self.str_ptr(fx, b);
        self.call(fx, "kn_str_lit", &[slot, p, l])
    }
    pub(crate) fn args_buf(&mut self, fx: &mut Fx, vals: &[Value]) -> Value {
        if vals.is_empty() {
            return fx.b.ins().iconst(I64, 0);
        }
        let ss = fx.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (vals.len() * 8) as u32, 3));
        for (i, v) in vals.iter().enumerate() {
            fx.b.ins().stack_store(I64, *v, ss, (i * 8) as i32);
        }
        fx.b.ins().stack_addr(I64, ss, 0)
    }
    /// i8 condition: Ruby truthiness (not nil, not false).
    pub(crate) fn truthy(&mut self, fx: &mut Fx, v: Value) -> Value {
        let m = fx.b.ins().band_imm_s(v, !QNIL);
        fx.b.ins().icmp_imm_s(IntCC::NotEqual, m, 0)
    }
    pub(crate) fn bool_v(&mut self, fx: &mut Fx, c: Value) -> Value {
        let t = fx.b.ins().iconst(I64, QTRUE);
        let f = fx.b.ins().iconst(I64, QFALSE);
        fx.b.ins().select(c, t, f)
    }
    pub(crate) fn cref(&mut self, fx: &mut Fx) -> Value {
        let p = self.slot_ptr(fx, fx.cref_slot);
        fx.b.ins().load(I64, mem(), p, 0)
    }
    pub(crate) fn self_val(&mut self, fx: &mut Fx) -> Value {
        fx.b.use_var(fx.self_v)
    }
    /// Tag the following instructions (a call) with the current source line, for error
    /// messages and backtraces. Costs nothing at runtime: it only goes into the backtrace
    /// tables (an earlier version stored "file:line" in a global before every call, which
    /// made cores running threads in parallel fight over that cache line).
    pub(crate) fn mark_loc(&mut self, fx: &mut Fx) {
        self.set_srcloc(fx);
    }

    /// Compile every .rb file under `dirs` (skipping files with syntax errors, with a warning).
    pub(crate) fn embed(&mut self, dirs: &[PathBuf]) {
        let mut files = Vec::new();
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|e| e == "rb") {
                    out.push(p);
                }
            }
        }
        for d in dirs {
            walk(d, &mut files);
        }
        for f in files {
            let saved = std::mem::take(&mut self.errors);
            self.require_file(f.clone(), 0);
            let errs = std::mem::replace(&mut self.errors, saved);
            // one line per skipped file (e.g. a generator's ERB template named *.rb)
            if let Some(first) = errs.first() {
                let more = if errs.len() > 1 { format!(" (+{} more errors)", errs.len() - 1) } else { String::new() };
                self.warnings.push(format!("{first}{more} (file skipped)"));
            }
        }
    }

    /// (require key, function, loaded-flag slot) for every compiled file, keyed by path relative
    /// to each load path (without .rb) and by absolute path.
    pub(crate) fn file_table(&self) -> Vec<(String, FuncId, usize, u64)> {
        let mut out = Vec::new();
        let lps: Vec<PathBuf> = self.load_paths.iter().map(|p| p.canonicalize().unwrap_or(p.clone())).collect();
        for (path, (fid, flag)) in &self.loaded {
            let hash = self.src_hashes.get(path).copied().unwrap_or(0);
            let abs = path.display().to_string();
            out.push((abs.trim_end_matches(".rb").to_string(), *fid, *flag, hash));
            for lp in &lps {
                if let Ok(rel) = path.strip_prefix(lp) {
                    let r = rel.display().to_string();
                    out.push((r.trim_end_matches(".rb").to_string(), *fid, *flag, hash));
                }
            }
        }
        out
    }

    /// GC safepoint for loop back-edges: a load and a rarely taken branch.
    pub(crate) fn safepoint(&mut self, fx: &mut Fx) {
        let flag = fx.b.ins().load(ir::types::I8, mem(), fx.gcflag, 0);
        let slow = fx.b.create_block();
        let cont = fx.b.create_block();
        fx.b.ins().brif(flag, slow, &[], cont, &[]);
        fx.b.set_cold_block(slow);
        fx.b.switch_to_block(slow);
        let r = self.call(fx, "kn_safepoint", &[]);
        self.check(fx, r);
        fx.b.ins().jump(cont, &[]);
        fx.b.switch_to_block(cont);
    }

    /// Raise a runtime error with a fixed message (used for unsupported-at-runtime paths).
    pub(crate) fn raise_msg(&mut self, fx: &mut Fx, kind: i64, msg: &str) -> Value {
        let k = self.iconst(fx, kind);
        let (p, l) = self.str_ptr(fx, msg.as_bytes());
        let v = self.call(fx, "kn_raise_msg", &[k, p, l]);
        self.check(fx, v);
        self.nil(fx)
    }
}

/// Content hash of a source file (FNV-1a, 64 bit); the runtime's `load` computes the same over
/// the file on disk to see whether it changed since the build (`kenny_runtime::compat::src_hash`).
pub fn src_hash(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
