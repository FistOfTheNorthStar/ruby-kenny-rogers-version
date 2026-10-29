//! Values, heap objects, classes, symbols, method dispatch and the pending-throw state.
//!
//! Value tagging (64-bit, CRuby-compatible layout so a C-API shim stays possible):
//!   ...xxx1  Fixnum (62-bit signed, value << 1 | 1)
//!   0x00     false
//!   0x08     nil
//!   0x14     true
//!   0x34     UNDEF  (never visible to Ruby; a function returning it means "a throw is pending")
//!   id<<8|0x0c  Symbol
//!   ...xx10  Flonum: a Float stored in the value itself (CRuby's encoding: the double's bits
//!            rotated left by 3 with the low 2 bits replaced by 0b10). Covers doubles whose
//!            exponent is within about 2^-255..2^256, plus +0.0 (0x8000000000000002); -0.0,
//!            NaN, Infinity and very large/small values stay heap `RFloat`s. The compiler
//!            emits the same encode/decode inline, and include/ruby.h mirrors it.
//!   8-aligned pointer  heap object (starts with `Header`)
//!
//! v0 is single-threaded and never frees memory (no GC yet).

pub use crate::svec::SVec;
pub use crate::cmap::CMap;
use num_bigint::BigInt;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub type V = u64;

pub const QFALSE: V = 0x00;
pub const QNIL: V = 0x08;
pub const QTRUE: V = 0x14;
pub const UNDEF: V = 0x34;
pub const SYM_TAG: V = 0x0c;

pub const FIX_MAX: i64 = (1i64 << 62) - 1;
pub const FIX_MIN: i64 = -(1i64 << 62);

pub struct G<T>(UnsafeCell<T>);
unsafe impl<T> Sync for G<T> {}
impl<T> G<T> {
    pub const fn new(v: T) -> Self {
        G(UnsafeCell::new(v))
    }
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

// ---------- immediates ----------

#[inline(always)]
pub fn is_fix(v: V) -> bool {
    v & 1 == 1
}
#[inline(always)]
pub fn fix(v: V) -> i64 {
    (v as i64) >> 1
}
#[inline(always)]
pub fn mkfix(i: i64) -> V {
    ((i << 1) | 1) as u64
}
#[inline(always)]
pub fn truthy(v: V) -> bool {
    v & !QNIL != 0
}
#[inline(always)]
pub fn bool_v(b: bool) -> V {
    if b { QTRUE } else { QFALSE }
}
#[inline(always)]
pub fn is_sym(v: V) -> bool {
    v & 0xff == SYM_TAG
}
#[inline(always)]
pub fn sym_id(v: V) -> u32 {
    (v >> 8) as u32
}
#[inline(always)]
pub fn mksym(id: u32) -> V {
    ((id as u64) << 8) | SYM_TAG
}
#[inline(always)]
pub fn is_heap(v: V) -> bool {
    v & 7 == 0 && v != QFALSE && v != QNIL
}

// ---------- flonums (immediate Floats) ----------

pub const FLONUM_MASK: V = 3;
pub const FLONUM_FLAG: V = 2;
/// The flonum encoding of +0.0.
pub const FLONUM_ZERO: V = 0x8000_0000_0000_0002;

#[inline(always)]
pub fn is_flonum(v: V) -> bool {
    v & FLONUM_MASK == FLONUM_FLAG
}
#[inline(always)]
pub fn flonum_to_f64(v: V) -> f64 {
    if v == FLONUM_ZERO {
        return 0.0;
    }
    let b63 = v >> 63;
    f64::from_bits(((2 - b63) | (v & !FLONUM_MASK)).rotate_right(3))
}
/// The flonum for `d`, if it has one (else the Float must live on the heap).
#[inline(always)]
pub fn f64_to_flonum(d: f64) -> Option<V> {
    let t = d.to_bits();
    let bits = ((t >> 60) & 7) as i64;
    if t != 0x3000_0000_0000_0000 && (bits - 3) & !1 == 0 {
        Some((t.rotate_left(3) & !1) | FLONUM_FLAG)
    } else if t == 0 {
        Some(FLONUM_ZERO)
    } else {
        None
    }
}
/// The f64 of a Float value (flonum or heap); the caller has checked `is_float`.
#[inline(always)]
pub fn float_val(v: V) -> f64 {
    if is_flonum(v) { flonum_to_f64(v) } else { rfloat(v).f }
}

// ---------- heap objects ----------

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Object,
    String,
    Array,
    Hash,
    Float,
    Bignum,
    Proc,
    Class,
    Range,
    /// Opaque C-extension data (TypedData / Data_Wrap_Struct)
    Data,
    /// A closure environment (see `Env`): never a Ruby value. The header's `klass` field holds
    /// its slot count, and compiled code points at the words after the header.
    Env,
}

pub const FL_FROZEN: u32 = 1;
pub const FL_KWHASH: u32 = 2;
/// String flag: ASCII-8BIT (binary) encoding. (4 is gc::FL_MARK; 16 matches compat::FL_BINARY upstream.)
pub const FL_BINARY: u32 = 16;
/// Proc flag: the block declares keyword parameters (a trailing keyword hash is not positional).
pub const FL_PROC_KW: u32 = 256;
/// String flag: frozen and ASCII-only, known (string.rs `ascii_only`; a frozen String's
/// bytes never change, so this stays true).
pub const FL_STR_ASCII: u32 = 1 << 12;

#[repr(C)]
pub struct Header {
    pub klass: *mut Class,
    pub kind: Kind,
    pub flags: u32,
}
/// (compiled code reads the kind: kenny-compiler fastpath.rs `HDR_KIND`, `KIND_CLASS`)
const _: () = assert!(std::mem::offset_of!(Header, kind) == 8 && Kind::Class as u32 == 7);

#[repr(C)]
pub struct RObject {
    pub h: Header,
    pub shape: *mut Class,
    pub ivars: SVec<V>,
}
#[repr(C)]
pub struct RString {
    pub h: Header,
    pub b: SVec<u8>,
    pub iv: GenIv,
}
#[repr(C)]
pub struct RArray {
    pub h: Header,
    pub v: SVec<V>,
    pub iv: GenIv,
}
pub type FxBuild = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;
pub type HMap = crate::khash::KMap;

#[repr(C)]
pub struct RHash {
    pub h: Header,
    pub m: HMap,
    pub default: V,
    pub default_proc: V,
    pub iv: GenIv,
}

/// Instance variables of a String, Array or Hash (e.g. ActiveSupport::SafeBuffer's
/// `@html_safe`), created on first use.
#[derive(Default)]
pub struct GenIv(std::sync::atomic::AtomicPtr<CMap<u32, V>>);
impl GenIv {
    pub const fn new() -> Self {
        GenIv(std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()))
    }
    pub fn map(&self) -> Option<&'static mut CMap<u32, V>> {
        let p = self.0.load(std::sync::atomic::Ordering::Acquire);
        if p.is_null() { None } else { Some(unsafe { &mut *p }) }
    }
    pub fn map_or_create(&self) -> &'static mut CMap<u32, V> {
        if let Some(m) = self.map() {
            return m;
        }
        let n = Box::into_raw(Box::new(CMap::new()));
        match self.0.compare_exchange(std::ptr::null_mut(), n, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire) {
            Ok(_) => unsafe { &mut *n },
            Err(cur) => {
                drop(unsafe { Box::from_raw(n) });
                unsafe { &mut *cur }
            }
        }
    }
}
impl Drop for GenIv {
    fn drop(&mut self) {
        let p = *self.0.get_mut();
        if !p.is_null() {
            drop(unsafe { Box::from_raw(p) });
        }
    }
}
#[repr(C)]
pub struct RFloat {
    pub h: Header,
    pub f: f64,
}
#[repr(C)]
pub struct RBignum {
    pub h: Header,
    pub n: BigInt,
}
#[repr(C)]
pub struct RRange {
    pub h: Header,
    pub lo: V,
    pub hi: V,
    pub excl: bool,
}

pub type BlockFn = unsafe extern "C" fn(*mut RProc, V, u64, *const V, V) -> V;
pub type NativeBlock = Box<dyn Fn(V, &[V], V) -> V>;

/// Closure environment: captured locals live here. Layout is read by compiled code:
/// [parent, method_block, slot0, slot1, ...]
#[repr(C)]
pub struct Env {
    pub parent: *mut Env,
    pub blk: V,
    pub slots: [V; 0],
}

#[repr(C)]
pub struct RProc {
    pub h: Header,
    pub func: BlockFn,
    pub env: *mut Env,
    pub self_: V,
    pub min: i32,
    pub max: i32, // -1 = unlimited
    pub autosplat: bool,
    pub lambda: bool,
    pub native: *mut NativeBlock,
    /// Values captured by a native closure (kept alive by the GC).
    pub roots: Vec<V>,
}

#[repr(C)]
pub struct RData {
    pub h: Header,
    pub data: *mut std::ffi::c_void,
    pub dmark: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub dfree: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    pub roots: Vec<V>,
}

#[repr(C)]
pub struct Class {
    pub h: Header,
    pub name: String,
    pub superclass: *mut Class,
    pub methods: CMap<u32, *const Method>,
    pub includes: SVec<*mut Class>,
    pub consts: CMap<u32, V>,
    pub ivar_index: CMap<u32, u32>,
    pub ivar_names: SVec<u32>,
    pub civars: CMap<u32, V>,
    pub cvars: CMap<u32, V>,
    pub is_module: bool,
    pub is_singleton: bool,
    pub attached: V,
    pub lexical_parent: *mut Class,
    pub alloc: Kind,
    pub struct_members: SVec<u32>,
    pub module_function: bool,
    /// Modules prepended to this class/module (looked up before its own methods).
    pub prepends: SVec<*mut Class>,
    /// Visibility for methods defined next in a class body (`private` with no arguments).
    pub default_vis: u8,
    /// The superclass chain from the root down to this class (`superclass` never changes),
    /// so `is_a` for a class is one index: `k.supers[depth(c)] == c`. Empty for modules.
    /// (Never freed, like classes; compiled code reads both fields, see `CLASS_SUP_PTR`.)
    pub sup_ptr: *const *mut Class,
    pub sup_len: usize,
}
/// Offsets compiled code uses (kenny-compiler fastpath.rs).
pub const CLASS_SUP_PTR: usize = std::mem::offset_of!(Class, sup_ptr);
const _: () = assert!(CLASS_SUP_PTR == 272 && std::mem::offset_of!(Class, sup_len) == 280);
/// A class's own ivars (`civars`, a `CMap<u32, V>`): compiled code reads them inline
/// (kenny-compiler fastpath.rs `civar_fast`): the map's first word points to its slab
/// (len, cap, then (key, value) pairs of 16 bytes, key first).
pub const CLASS_CIVARS: usize = std::mem::offset_of!(Class, civars);
const _: () = assert!(
    CLASS_CIVARS == 144
        && std::mem::offset_of!((u32, V), 0) == 0
        && std::mem::offset_of!((u32, V), 1) == 8
        && std::mem::size_of::<(u32, V)>() == 16
);
impl Class {
    #[inline(always)]
    pub fn supers(&self) -> &[*mut Class] {
        if self.sup_len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(self.sup_ptr, self.sup_len) } }
    }
}

#[inline(always)]
pub fn hdr(v: V) -> &'static mut Header {
    unsafe { &mut *(v as *mut Header) }
}
#[inline(always)]
pub fn kind(v: V) -> Option<Kind> {
    // flonums report Kind::Float too, so `kind`-based dispatch treats both Float forms alike;
    // anything that touches the header must check `is_heap` (not `kind(..).is_some()`)
    if is_heap(v) {
        Some(hdr(v).kind)
    } else if is_flonum(v) {
        Some(Kind::Float)
    } else {
        None
    }
}
macro_rules! caster {
    ($name:ident, $t:ty) => {
        #[inline(always)]
        pub fn $name(v: V) -> &'static mut $t {
            unsafe { &mut *(v as *mut $t) }
        }
    };
}
caster!(obj, RObject);
caster!(rstr, RString);
caster!(rary, RArray);
caster!(rhash, RHash);
caster!(rfloat, RFloat);
caster!(rbig, RBignum);
caster!(rrange, RRange);
caster!(rproc, RProc);
caster!(rclass, Class);

pub fn is_kind(v: V, k: Kind) -> bool {
    kind(v) == Some(k)
}
pub fn is_str(v: V) -> bool {
    is_kind(v, Kind::String)
}
pub fn is_ary(v: V) -> bool {
    is_kind(v, Kind::Array)
}
pub fn is_hash(v: V) -> bool {
    is_kind(v, Kind::Hash)
}
pub fn is_float(v: V) -> bool {
    is_kind(v, Kind::Float)
}
pub fn is_big(v: V) -> bool {
    is_kind(v, Kind::Bignum)
}
pub fn is_int(v: V) -> bool {
    is_fix(v) || is_big(v)
}
pub fn is_class(v: V) -> bool {
    is_kind(v, Kind::Class)
}
pub fn is_proc(v: V) -> bool {
    is_kind(v, Kind::Proc)
}
pub fn is_range(v: V) -> bool {
    is_kind(v, Kind::Range)
}

pub fn alloc<T>(t: T) -> V {
    debug_assert!(std::mem::align_of::<T>() <= 16);
    let p = crate::gc::alloc_slot(std::mem::size_of::<T>());
    unsafe { std::ptr::write(p as *mut T, t) };
    p as V
}
fn header(klass: *mut Class, kind: Kind) -> Header {
    Header { klass, kind, flags: 0 }
}

pub fn str_new(b: Vec<u8>) -> V {
    alloc(RString { h: header(rt().c.string, Kind::String), b: SVec::from(b), iv: GenIv::new() })
}
pub fn str_from(s: &str) -> V {
    str_new(s.as_bytes().to_vec())
}
pub fn ary_new(v: Vec<V>) -> V {
    alloc(RArray { h: header(rt().c.array, Kind::Array), v: SVec::from(v), iv: GenIv::new() })
}
pub fn hash_new() -> V {
    alloc(RHash { h: header(rt().c.hash, Kind::Hash), m: HMap::new(), default: QNIL, default_proc: QNIL, iv: GenIv::new() })
}
pub fn float_new(f: f64) -> V {
    match f64_to_flonum(f) {
        Some(v) => v,
        None => float_new_heap(f),
    }
}
/// A heap Float (Floats are always frozen, like CRuby's).
pub fn float_new_heap(f: f64) -> V {
    let v = alloc(RFloat { h: header(rt().c.float, Kind::Float), f });
    hdr(v).flags |= FL_FROZEN;
    v
}
pub fn range_new(lo: V, hi: V, excl: bool) -> V {
    let r = alloc(RRange { h: header(rt().c.range, Kind::Range), lo, hi, excl });
    hdr(r).flags |= FL_FROZEN;
    r
}
pub fn object_new(klass: *mut Class) -> V {
    // instances usually get the same ivars as earlier ones: make room for them up front,
    // as unset slots (UNDEF), so setting one is a plain store (also from compiled code)
    let n = if klass.is_null() { 0 } else { unsafe { (*klass).ivar_names.len() } };
    // up to 6 of them in the object's own slot (no buffer to allocate and free; read in the
    // same cache line), filling the slot size class: 64 bytes for 1-2, 96 for 3-6
    let base = std::mem::size_of::<RObject>();
    let room = (crate::gc::slot_size(base + 8 * n) - base) / 8;
    if n == 0 || base + 8 * n > crate::gc::MAX_SLOT {
        return alloc(RObject { h: header(klass, Kind::Object), shape: klass, ivars: SVec::from(vec![UNDEF; n]) });
    }
    let p = crate::gc::alloc_slot(base + 8 * room);
    unsafe {
        let ivars = SVec::embedded((p + base) as *mut V, n, room, UNDEF);
        std::ptr::write(p as *mut RObject, RObject { h: header(klass, Kind::Object), shape: klass, ivars });
    }
    p as V
}
/// Where an object's embedded instance variables start (`SVec::rebase` after a move).
pub const OBJ_EMBED_OFF: usize = std::mem::size_of::<RObject>();
const _: () = assert!(OBJ_EMBED_OFF == 48);
pub fn int_from_big(n: BigInt) -> V {
    use num_traits::ToPrimitive;
    if let Some(i) = n.to_i64() {
        if (FIX_MIN..=FIX_MAX).contains(&i) {
            return mkfix(i);
        }
    }
    let v = alloc(RBignum { h: header(rt().c.integer, Kind::Bignum), n });
    hdr(v).flags |= FL_FROZEN;
    v
}
pub fn int_from_i128(i: i128) -> V {
    if i >= FIX_MIN as i128 && i <= FIX_MAX as i128 { mkfix(i as i64) } else { int_from_big(BigInt::from(i)) }
}

pub fn proc_new_compiled(func: BlockFn, env: *mut Env, self_: V, min: i32, max: i32, autosplat: bool, lambda: bool) -> V {
    alloc(RProc {
        h: header(rt().c.proc_, Kind::Proc),
        func,
        env,
        self_,
        min,
        max,
        autosplat,
        lambda,
        native: std::ptr::null_mut(),
        roots: Vec::new(),
    })
}
pub fn proc_new_native(f: NativeBlock, min: i32, max: i32) -> V {
    proc_new_native_rooted(f, min, max, Vec::new())
}
/// A native proc whose closure captures Ruby values: list them in `roots`.
pub fn proc_new_native_rooted(f: NativeBlock, min: i32, max: i32, roots: Vec<V>) -> V {
    let p = alloc(RProc {
        h: header(rt().c.proc_, Kind::Proc),
        func: native_block_tramp,
        env: std::ptr::null_mut(),
        self_: QNIL,
        min,
        max,
        autosplat: max != 1 && min != 1,
        lambda: false,
        native: Box::into_raw(Box::new(f)),
        roots,
    });
    p
}
unsafe extern "C" fn native_block_tramp(p: *mut RProc, self_: V, argc: u64, argv: *const V, blk: V) -> V {
    let args = unsafe { args_slice(argc, argv) };
    call_native_block(unsafe { &**(*p).native }, self_, args, blk)
}

// Native (Rust) methods and blocks may keep object references where the GC can't see or
// update them (in Rust collections), so the GC moves objects only when no stack has such a
// frame (gc::compact). Every call into one goes through `call_native`/`call_native_block`
// (never inlined, never a tail call), so the return address into them marks a native frame
// on a stack; `native_return_addresses` finds those addresses once. C extension functions
// are counted instead (C_CALLS: rarer, and reached through a C dispatcher).
#[inline(never)]
pub fn call_native(f: NativeFn, recv: V, args: &[V], blk: V) -> V {
    let r = f(recv, args, blk);
    not_a_tail_call();
    r
}
#[inline(never)]
fn call_native_block(f: &dyn Fn(V, &[V], V) -> V, recv: V, args: &[V], blk: V) -> V {
    let r = f(recv, args, blk);
    not_a_tail_call();
    r
}
/// An empty instruction after the call keeps it from becoming a tail call (which would leave
/// no frame, and so no return address, behind).
#[inline(always)]
fn not_a_tail_call() {
    unsafe { std::arch::asm!("", options(nomem, nostack, preserves_flags)) };
}
/// C extension functions running on any thread (see call_native).
pub static C_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn call_native_c(f: *const std::ffi::c_void, arity: i32, recv: V, args: &[V]) -> V {
    C_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let r = crate::capi::call_cfunc(f, arity, recv, args);
    C_CALLS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    r
}
/// The return addresses inside `call_native` and `call_native_block`, found by calling each
/// with a probe that reads where it returns to (0 where that isn't possible).
pub fn native_return_addresses() -> [usize; 2] {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RET: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
    static DONE: std::sync::Once = std::sync::Once::new();
    #[inline(never)]
    fn not_a_leaf() {
        std::hint::black_box(());
    }
    /// The return address saved in the current function's frame record (it must have one:
    /// call `not_a_leaf` first).
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[inline(always)]
    fn own_return_address() -> usize {
        let fp: usize;
        #[cfg(target_arch = "aarch64")]
        unsafe {
            std::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack));
        }
        #[cfg(target_arch = "x86_64")]
        unsafe {
            std::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack));
        }
        (unsafe { *((fp + 8) as *const usize) }) & 0x0000_ffff_ffff_ffff
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    fn own_return_address() -> usize {
        0
    }
    fn probe0(_: V, _: &[V], _: V) -> V {
        not_a_leaf();
        RET[0].store(own_return_address(), Ordering::Relaxed);
        QNIL
    }
    DONE.call_once(|| {
        call_native(probe0, QNIL, &[], QNIL);
        call_native_block(
            &|_, _, _| {
                not_a_leaf();
                RET[1].store(own_return_address(), Ordering::Relaxed);
                QNIL
            },
            QNIL,
            &[],
            QNIL,
        );
    });
    [RET[0].load(Ordering::Relaxed), RET[1].load(Ordering::Relaxed)]
}
pub unsafe fn args_slice<'a>(argc: u64, argv: *const V) -> &'a [V] {
    if argc == 0 { &[] } else { unsafe { std::slice::from_raw_parts(argv, argc as usize) } }
}

pub fn is_frozen(v: V) -> bool {
    !is_heap(v) || hdr(v).flags & FL_FROZEN != 0
}
pub fn check_frozen(v: V) -> V {
    if is_frozen(v) {
        let msg = format!("can't modify frozen {}: {}", class_name(class_of(v)), crate::fmt::inspect_lossy(v));
        let e = exc_new(rt().c.frozen_error, &msg);
        ivar_set(e, sym("@__receiver"), v);
        return raise_exc(e);
    }
    QNIL
}

// ---------- symbols ----------

// Names live in chunks that never move (lock-free `sym_name`); lookup is a lock-free
// open-addressing table that only grows (a new table is published whole). Adding a symbol
// takes SYM_LOCK.
const SYM_CHUNK: usize = 4096;
struct SymName {
    ptr: std::sync::atomic::AtomicPtr<u8>,
    len: std::sync::atomic::AtomicUsize,
}
static SYM_CHUNKS: [std::sync::atomic::AtomicPtr<SymName>; 4096] = [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 4096];
static SYM_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SYM_TABLE: std::sync::atomic::AtomicPtr<SymTable> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static SYM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct SymTable {
    mask: usize,
    slots: Box<[std::sync::atomic::AtomicU32]>,
}

#[inline]
fn sym_hash(b: &[u8]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    h.write(b);
    h.write_usize(b.len());
    h.finish()
}
fn sym_probe(t: &SymTable, h: u64, name: &[u8]) -> Result<u32, usize> {
    use std::sync::atomic::Ordering;
    let mut i = (h.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & t.mask;
    loop {
        let e = t.slots[i].load(Ordering::Acquire);
        if e == 0 {
            return Err(i);
        }
        if sym_name(e - 1) == name {
            return Ok(e - 1);
        }
        i = (i + 1) & t.mask;
    }
}
pub fn intern(name: &[u8]) -> u32 {
    // The one-character names come first, so a symbol's id is its character, as CRuby's IDs
    // are: C extensions call operators by char (`rb_funcall(x, '+', 1, y)`).
    static ONE_CHAR: std::sync::Once = std::sync::Once::new();
    ONE_CHAR.call_once(|| {
        for c in 0..128u8 {
            intern_name(&[c]);
        }
    });
    intern_name(name)
}
fn intern_name(name: &[u8]) -> u32 {
    use std::sync::atomic::Ordering;
    let h = sym_hash(name);
    let t = SYM_TABLE.load(Ordering::Acquire);
    if !t.is_null() {
        if let Ok(id) = sym_probe(unsafe { &*t }, h, name) {
            return id;
        }
    }
    let _g = SYM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = SYM_TABLE.load(Ordering::Acquire);
    if !t.is_null() {
        if let Ok(id) = sym_probe(unsafe { &*t }, h, name) {
            return id;
        }
    }
    let id = SYM_COUNT.load(Ordering::Relaxed);
    // the name
    let ci = id as usize / SYM_CHUNK;
    let mut chunk = SYM_CHUNKS[ci].load(Ordering::Acquire);
    if chunk.is_null() {
        let v: Vec<SymName> = (0..SYM_CHUNK).map(|_| SymName { ptr: std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()), len: std::sync::atomic::AtomicUsize::new(0) }).collect();
        chunk = Box::leak(v.into_boxed_slice()).as_mut_ptr();
        SYM_CHUNKS[ci].store(chunk, Ordering::Release);
    }
    let leaked: &'static mut [u8] = Box::leak(name.to_vec().into_boxed_slice());
    let e = unsafe { &*chunk.add(id as usize % SYM_CHUNK) };
    e.len.store(leaked.len(), Ordering::Relaxed);
    e.ptr.store(if leaked.is_empty() { std::ptr::NonNull::dangling().as_ptr() } else { leaked.as_mut_ptr() }, Ordering::Release);
    SYM_COUNT.store(id + 1, Ordering::Release);
    // the lookup table (grown at half full; the old one stays readable until the next GC)
    let n = id as usize + 1;
    if t.is_null() || n * 2 > unsafe { (*t).mask } + 1 {
        let size = (n * 4).next_power_of_two().max(1024);
        let nt = Box::into_raw(Box::new(SymTable { mask: size - 1, slots: (0..size).map(|_| std::sync::atomic::AtomicU32::new(0)).collect() }));
        for i in 0..n as u32 {
            let nm = sym_name(i);
            if let Err(slot) = sym_probe(unsafe { &*nt }, sym_hash(nm), nm) {
                unsafe { (*nt).slots[slot].store(i + 1, Ordering::Release) };
            }
        }
        // the old table is leaked: lock-free readers may still be probing it, and tables
        // double in size, so all old ones together are smaller than the current one
        SYM_TABLE.store(nt, Ordering::Release);
        t = nt;
    } else if let Err(slot) = sym_probe(unsafe { &*t }, h, name) {
        unsafe { (*t).slots[slot].store(id + 1, Ordering::Release) };
    }
    let _ = t;
    id
}
pub fn sym(name: &str) -> u32 {
    intern(name.as_bytes())
}
pub fn sym_count() -> u32 {
    SYM_COUNT.load(std::sync::atomic::Ordering::Acquire)
}
#[inline]
pub fn sym_name(id: u32) -> &'static [u8] {
    use std::sync::atomic::Ordering;
    let chunk = SYM_CHUNKS[id as usize / SYM_CHUNK].load(Ordering::Acquire);
    if !chunk.is_null() {
        let e = unsafe { &*chunk.add(id as usize % SYM_CHUNK) };
        let p = e.ptr.load(Ordering::Acquire);
        if !p.is_null() {
            return unsafe { std::slice::from_raw_parts(p, e.len.load(Ordering::Relaxed)) };
        }
    }
    // an id handed over without synchronisation: wait for its name under the lock
    let _g = SYM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let chunk = SYM_CHUNKS[id as usize / SYM_CHUNK].load(Ordering::Acquire);
    assert!(!chunk.is_null(), "unknown symbol id {id}");
    let e = unsafe { &*chunk.add(id as usize % SYM_CHUNK) };
    unsafe { std::slice::from_raw_parts(e.ptr.load(Ordering::Acquire), e.len.load(Ordering::Relaxed)) }
}
pub fn sym_str(id: u32) -> String {
    String::from_utf8_lossy(sym_name(id)).into_owned()
}

// ---------- methods ----------

pub type CompiledFn = unsafe extern "C" fn(V, u64, *const V, V) -> V;
pub type NativeFn = fn(V, &[V], V) -> V;

pub enum Body {
    Compiled(CompiledFn),
    Native(NativeFn, i32, i32),
    AttrReader(u32),
    AttrWriter(u32),
    Proc(V),
    /// C extension function and its arity (-1 = argc/argv, 0..15 = fixed).
    CFunc(*const std::ffi::c_void, i32),
    Undefined,
}
#[repr(C)]
pub struct Method {
    /// A compiled method's entry taking its arguments in registers, `(self, a0.., blk)`,
    /// for `n` required parameters: `address | (n + 1) << 56` (0 = none). Compiled code
    /// calls it when a call passes exactly `n` arguments (kenny-compiler `fast_wrapper`).
    pub fast: usize,
    pub body: Body,
    pub owner: *mut Class,
    pub name: u32,
    /// Compiled methods: NUL-terminated parameter list "req:a,opt:b,rest:c,key:d,block:e".
    pub params: *const u8,
    /// 0 = public, 1 = private, 2 = protected
    pub vis: u8,
    /// Builtin core method with a specialised call path (`opt::OPT_*`, 0 = none); describes
    /// the body, so aliases and copies keep it.
    pub opt: u8,
    /// A native method audited to keep object references only on the stack (it blocks or
    /// loops, like `loop`, `sleep`, `Queue#pop`): it is called directly instead of through
    /// `call_native`, so a thread waiting in it doesn't stop the GC from moving objects.
    pub gc_transparent: bool,
    /// attr_reader/attr_writer: (receiver class << 16) | the ivar's slot index for the last
    /// two classes it ran on (0 = none), so a call needn't look the slot up by name.
    pub attr_slot: [std::sync::atomic::AtomicU64; 2],
}

/// (compiled code reads `Method::fast` at this offset)
const _: () = assert!(std::mem::offset_of!(Method, fast) == 0);

pub const VIS_PUBLIC: u8 = 0;
pub const VIS_PRIVATE: u8 = 1;
pub const VIS_PROTECTED: u8 = 2;

/// Methods Ruby always makes private.
pub fn always_private(mid: u32) -> bool {
    matches!(sym_name(mid), b"initialize" | b"initialize_copy" | b"initialize_dup" | b"initialize_clone" | b"respond_to_missing?")
}

/// A call site's inline cache (3 words reserved by the compiler). The lookup result lives in
/// an immutable entry behind one atomic pointer, so a thread never sees one site's class
/// paired with another's method. With one OS thread the entry is updated in place.
#[repr(C)]
pub struct CallCache {
    pub entry: std::sync::atomic::AtomicPtr<CacheEntry>,
    /// Misses after several threads started; past a limit the site stops caching (megamorphic).
    pub misses: u64,
    /// For compiled code's inline attribute access in loops: (receiver class << 16) | ivar
    /// index once this site has reached an attr_reader/attr_writer it may use, else 0.
    /// Cleared whenever the method serial changes (see `opt::kn_attr_get`).
    pub attr: std::sync::atomic::AtomicU64,
}
/// Up to `IC_WAYS` receiver classes and their methods (a polymorphic inline cache), valid
/// while the method serial is unchanged.
pub const IC_WAYS: usize = 4;
/// (`repr(C)`: compiled code reads `serial`, `klass` and `direct`; kenny-compiler
/// fastpath.rs `direct_call`.)
#[repr(C)]
pub struct CacheEntry {
    pub serial: u64,
    pub n: usize,
    pub klass: [*mut Class; IC_WAYS],
    pub method: [*const Method; IC_WAYS],
    /// For each way: the compiled function of a `Body::Compiled` method, with the low bit
    /// set for a private one; 0 for anything else. Compiled code calls it directly.
    pub direct: [usize; IC_WAYS],
    /// For each way: an attr_reader/attr_writer's ivar slot in the class's objects, as
    /// `slot << 8 | 4 | writer << 1 | private`; 0 for anything else (see `attr_of`).
    pub attr: [u64; IC_WAYS],
}
/// `CacheEntry::attr` for method `m` found for receivers of class `k`: for plain classes
/// only (an object's slot table is the class it was allocated from, which compiled code
/// checks is its class), and for a reader only once the class has the ivar.
pub fn attr_of(k: *mut Class, m: *const Method) -> u64 {
    if m.is_null() || k.is_null() || unsafe { (*k).is_singleton } {
        return 0;
    }
    let private = (unsafe { (*m).vis } == VIS_PRIVATE) as u64;
    let (slot, writer) = match unsafe { &(*m).body } {
        Body::AttrReader(iv) => match shape_index(unsafe { &*k }, *iv) {
            Some(i) => (i as u64, 0),
            None => return 0,
        },
        Body::AttrWriter(iv) => (ivar_index_for(k, *iv) as u64, 2),
        // builtin nil? / is_a? (kind_of?): answered by the direct-call stub
        Body::Native(..) => {
            return match unsafe { (*m).opt } {
                crate::opt::OPT_NIL_P => ATTR_NIL_P | private,
                crate::opt::OPT_IS_A => ATTR_IS_A | private,
                _ => 0,
            };
        }
        _ => return 0,
    };
    if slot >= 1 << 48 {
        return 0;
    }
    slot << 8 | 4 | writer | private
}
/// `CacheEntry::attr` tags of builtin methods compiled code answers itself.
pub const ATTR_NIL_P: u64 = 8;
pub const ATTR_IS_A: u64 = 16;
/// `CacheEntry::direct` for method `m`.
pub fn direct_of(m: *const Method) -> usize {
    if m.is_null() {
        return 0;
    }
    match unsafe { &(*m).body } {
        Body::Compiled(f) if *f as usize & 1 == 0 => *f as usize | (unsafe { (*m).vis } == VIS_PRIVATE) as usize,
        _ => 0,
    }
}

// ---------- pending throws (exceptions and non-local control flow) ----------

#[derive(Clone, Copy, Debug)]
pub enum Throw {
    None,
    Raise(V),
    Break { tag: u64, val: V },
    Return { tag: u64, val: V },
    Throw { tag: V, val: V },
    StopLoop(V),
    /// Thread#kill: unwinds the thread, running ensure clauses; never rescued.
    Kill,
}

// ---------- the runtime ----------

#[derive(Default, Clone, Copy)]
pub struct Core {
    pub basic_object: *mut Class,
    pub object: *mut Class,
    pub module: *mut Class,
    pub class: *mut Class,
    pub kernel: *mut Class,
    pub comparable: *mut Class,
    pub enumerable: *mut Class,
    pub nil: *mut Class,
    pub true_: *mut Class,
    pub false_: *mut Class,
    pub numeric: *mut Class,
    pub integer: *mut Class,
    pub float: *mut Class,
    pub string: *mut Class,
    pub symbol: *mut Class,
    pub array: *mut Class,
    pub hash: *mut Class,
    pub range: *mut Class,
    pub proc_: *mut Class,
    pub enumerator: *mut Class,
    pub exception: *mut Class,
    pub script_error: *mut Class,
    pub not_implemented_error: *mut Class,
    pub standard_error: *mut Class,
    pub runtime_error: *mut Class,
    pub frozen_error: *mut Class,
    pub argument_error: *mut Class,
    pub name_error: *mut Class,
    pub no_method_error: *mut Class,
    pub type_error: *mut Class,
    pub zero_div_error: *mut Class,
    pub index_error: *mut Class,
    pub key_error: *mut Class,
    pub stop_iteration: *mut Class,
    pub range_error: *mut Class,
    pub float_domain_error: *mut Class,
    pub local_jump_error: *mut Class,
    pub io_error: *mut Class,
    pub system_stack_error: *mut Class,
    pub uncaught_throw_error: *mut Class,
    pub struct_: *mut Class,
    pub time: *mut Class,
    pub io: *mut Class,
    pub mutex: *mut Class,
}

pub struct Runtime {
    pub c: Core,
    pub gvars: CMap<u32, V>,
    pub main_obj: V,
    /// Buffered $stdout output (shared by all threads, like the real stdout).
    pub out: crate::mt::L<Vec<u8>>,
    pub rng: u64,
    pub script_name: String,
}

/// Interpreter state of one OS thread running Ruby code (a "P"). Green threads running on it
/// swap the first few fields in and out (thread.rs).
pub struct PState {
    pub pending: Throw,
    pub errinfo: V,
    pub last_match: V,
    /// The caller's locals for the string about to be eval'd (`eval_src`): set by the
    /// compiled call site just before the call (see kernel::eval_string).
    pub eval_locals: V,
    pub eval_src: V,
    pub stack_limit: usize,
    /// `class`/`module` bodies and `class_eval` blocks being run (the default definee).
    pub definee: Vec<*mut Class>,
    /// Methods defined with define_method that are running (for `super` inside them).
    pub dm_stack: Vec<*const Method>,
    /// Blocks passed to running C-extension functions (rb_block_given_p / rb_yield).
    pub current_block: Vec<V>,
    pub mcache: HashMap<(usize, u32), *const Method, FxBuild>,
    pub mcache_serial: u64,
    /// Allocation, GC roots and stop-the-world state of this thread.
    pub heap: crate::gc::PHeap,
    /// This P's scheduler state.
    pub thr: crate::thread::PThr,
    /// Direct-mapped method cache in front of `mcache`.
    pub mc: Box<[McEntry; MC_SIZE]>,
}

impl PState {
    pub fn pending_is_set(&self) -> bool {
        !matches!(self.pending, Throw::None)
    }
}

static RT: G<*mut Runtime> = G::new(std::ptr::null_mut());

#[inline(always)]
pub fn rt() -> &'static mut Runtime {
    unsafe { &mut **RT.get() }
}

thread_local! {
    static PST: std::cell::Cell<*mut PState> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}
/// Every P's state (for the GC). A collection holds this lock from start to end, so a P
/// that is starting up waits instead of joining the list halfway through one.
pub static ALL_PS: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// This P's state. A Ruby thread can move to another P whenever it calls into Ruby or
/// blocks (thread.rs), so never keep the result across such a call; and the lookup must
/// not be merged with an earlier one in the same function (LLVM treats thread-local
/// addresses as constant within a function), hence the opaque call.
#[inline(always)]
pub fn pst() -> &'static mut PState {
    unsafe { &mut *pst_ptr() }
}
#[inline(always)]
fn pst_ptr() -> *mut PState {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        let key = PST_KEY.load(std::sync::atomic::Ordering::Relaxed);
        if key != 0 {
            // the thread's TSD array (TPIDRRO_EL0) indexed by our pthread key: what
            // pthread_getspecific reads, in two instructions; the asm is volatile, so the read
            // is never merged with one made before the thread moved to another OS thread
            let tsd: usize;
            unsafe { std::arch::asm!("mrs {}, tpidrro_el0", out(reg) tsd, options(nomem, nostack, preserves_flags)) };
            return unsafe { *((tsd & !7) as *const *mut PState).add(key) };
        }
    }
    pst_ptr_tls()
}
#[inline(never)]
fn pst_ptr_tls() -> *mut PState {
    std::hint::black_box(PST.with(|c| c.get()))
}
/// (Apple Silicon) The pthread key holding each P's state, once checked to work (0: not used).
static PST_KEY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
fn set_pst_key(p: *mut PState) {
    use std::sync::atomic::Ordering;
    static KEY: std::sync::OnceLock<Option<libc::pthread_key_t>> = std::sync::OnceLock::new();
    let Some(key) = *KEY.get_or_init(|| {
        let mut k: libc::pthread_key_t = 0;
        (unsafe { libc::pthread_key_create(&mut k, None) } == 0).then_some(k)
    }) else {
        return;
    };
    unsafe { libc::pthread_setspecific(key, p as *const std::ffi::c_void) };
    // use the direct read only if it finds what pthread_getspecific does
    let tsd: usize;
    unsafe { std::arch::asm!("mrs {}, tpidrro_el0", out(reg) tsd, options(nomem, nostack, preserves_flags)) };
    let direct = unsafe { *((tsd & !7) as *const *mut PState).add(key as usize) };
    if direct == p && key != 0 {
        PST_KEY.store(key as usize, Ordering::Relaxed);
    } else {
        PST_KEY.store(0, Ordering::Relaxed);
    }
}
#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
fn set_pst_key(_: *mut PState) {}
/// This OS thread's state, if it runs Ruby code.
pub fn pst_opt() -> Option<&'static mut PState> {
    let p = pst_ptr();
    if p.is_null() { None } else { Some(unsafe { &mut *p }) }
}
/// Set up interpreter state for the calling OS thread.
pub fn new_pstate() -> *mut PState {
    let p = Box::into_raw(Box::new(PState {
        pending: Throw::None,
        errinfo: QNIL,
        last_match: QNIL,
        eval_locals: QNIL,
        eval_src: QNIL,
        stack_limit: 0,
        definee: Vec::new(),
        dm_stack: Vec::new(),
        current_block: Vec::new(),
        mcache: HashMap::default(),
        mcache_serial: 0,
        heap: crate::gc::PHeap::new(),
        thr: crate::thread::PThr::default(),
        mc: new_method_cache(),
    }));
    PST.with(|c| c.set(p));
    set_pst_key(p);
    ALL_PS.lock().unwrap_or_else(|e| e.into_inner()).push(p as usize);
    p
}

/// Method-table version: bumped whenever method lookup could change; call caches compare it.
pub static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What compiled code checks before calling a method directly (kenny-compiler fastpath.rs
/// `direct_call`): a copy of `SERIAL`, and the stack limit while only one OS thread runs
/// Ruby code (then it is this P's `stack_limit`; usize::MAX otherwise, which sends every
/// call through the runtime, where `pst()` gives the right limit).
#[repr(C)]
pub struct KnDirect {
    pub serial: std::sync::atomic::AtomicU64,
    pub call_limit: std::sync::atomic::AtomicUsize,
    /// Core methods compiled code may skip calling while they are the builtin ones:
    /// `DIRECT_SYM_EQ` (Symbol#==), `DIRECT_NIL_P` (nil? of every class of immediate
    /// values), `DIRECT_IS_A` (is_a? and kind_of? of those classes).
    pub flags: std::sync::atomic::AtomicU64,
    /// A copy of `CONST_SERIAL` (constant caches, abi.rs `ConstEntry`).
    pub const_serial: std::sync::atomic::AtomicU64,
    /// The builtin Module#=== (`*const Method`): `case v when SomeClass` is answered
    /// inline while a site's cache entry finds this method for the class.
    pub mod_eqq: std::sync::atomic::AtomicUsize,
    /// The Math module (compiled code computes Math functions inline while `DIRECT_MATH`).
    pub math_module: std::sync::atomic::AtomicUsize,
    /// The Array class (compiled code indexes plain Arrays inline while `DIRECT_ARY_AREF` /
    /// `DIRECT_ARY_ASET` are set).
    pub array_class: std::sync::atomic::AtomicUsize,
    /// The stack limit checked on entry to compiled methods that call themselves directly
    /// (kenny-compiler statics.rs `entry_check`): `call_limit` while only one OS thread runs
    /// Ruby code, else 0 (no such direct calls then).
    pub entry_limit: std::sync::atomic::AtomicUsize,
    /// The String class (compiled code runs getbyte/setbyte of plain Strings inline while
    /// `DIRECT_STR_BYTE` is set).
    pub string_class: std::sync::atomic::AtomicUsize,
}
#[unsafe(no_mangle)]
pub static KN_DIRECT: KnDirect = KnDirect {
    serial: std::sync::atomic::AtomicU64::new(1),
    call_limit: std::sync::atomic::AtomicUsize::new(usize::MAX),
    flags: std::sync::atomic::AtomicU64::new(0),
    const_serial: std::sync::atomic::AtomicU64::new(1),
    mod_eqq: std::sync::atomic::AtomicUsize::new(0),
    math_module: std::sync::atomic::AtomicUsize::new(0),
    array_class: std::sync::atomic::AtomicUsize::new(0),
    entry_limit: std::sync::atomic::AtomicUsize::new(0),
    string_class: std::sync::atomic::AtomicUsize::new(0),
};
const _: () = assert!(std::mem::offset_of!(KnDirect, array_class) == 48 && std::mem::offset_of!(KnDirect, math_module) == 40);
const _: () = assert!(std::mem::offset_of!(KnDirect, entry_limit) == 56 && std::mem::offset_of!(KnDirect, string_class) == 64);
/// What compiled code reads of a String (fastpath.rs `STR_*`): the buffer's ptr, len, cap.
const _: () = assert!(std::mem::offset_of!(RString, b) == 16);
/// What compiled code reads of an Array (fastpath.rs `ARY_*`): the buffer's ptr, len, cap.
const _: () = assert!(std::mem::offset_of!(RArray, v) == 16);
pub const DIRECT_SYM_EQ: u64 = 1;
pub const DIRECT_NIL_P: u64 = 2;
pub const DIRECT_IS_A: u64 = 4;
/// Array#[] / Array#[]= of the Array class itself are the builtin ones.
pub const DIRECT_ARY_AREF: u64 = 8;
pub const DIRECT_ARY_ASET: u64 = 16;
/// Every function of `opt::MATH_FNS` is Math's builtin one.
pub const DIRECT_MATH: u64 = 32;
/// Integer#to_f, Float#to_f, Integer#-@, Float#-@, Float#**, Integer#<< and Integer#>> are
/// the builtin ones.
pub const DIRECT_NUM: u64 = 64;
/// Every heap object's is_a? / kind_of? is the builtin one (`IS_A_TOUCHED` unset).
pub const DIRECT_IS_A_HEAP: u64 = 128;
/// Every class's and module's === is the builtin Module#=== (`EQQ_TOUCHED` unset).
pub const DIRECT_MOD_EQQ_ALL: u64 = 256;
/// String#getbyte and String#setbyte of the String class itself are the builtin ones.
pub const DIRECT_STR_BYTE: u64 = 512;
/// String#+ of the String class itself is the builtin one (string.rs `kn_str_append_own`).
pub const DIRECT_STR_PLUS: u64 = 1024;
/// String#[], String#==, NilClass#== and the #!= of both are the builtin ones (string.rs
/// `kn_str_idx_eq`).
pub const DIRECT_STR_IDX_EQ: u64 = 2048;
/// The Array methods compiled code runs as inline loops (kenny-compiler inline.rs) are
/// the public builtin ones of Array itself.
pub const DIRECT_ARY_ITER: u64 = 4096;
/// Integer#times, #upto and #downto are the public builtin ones.
pub const DIRECT_INT_ITER: u64 = 8192;
/// Every object's nil? is the builtin one, public (`NIL_P_TOUCHED` unset): `x.nil?` is
/// `x == nil` (kenny-compiler fastpath.rs).
pub const DIRECT_NIL_P_ALL: u64 = 16384;
/// Array#size, #length, #empty?, #first, #last, #push, #<<, #pop and #shift of Array itself
/// are the public builtin ones (compiled code runs them inline or calls `kn_ary_push1`,
/// `kn_ary_pop0`, `kn_ary_shift0` directly).
pub const DIRECT_ARY_BASIC: u64 = 32768;
/// Set once the builtin methods are tagged (opt.rs `init`).
pub static DIRECT_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `KN_DIRECT.flags` is 0 until recomputed (methods changed): on the next inline cache
/// miss (`abi::dispatch_miss`, a Ruby thread with a method cache, not inside a collection).
pub static DIRECT_FLAGS_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Methods changed: no core method may be skipped until `update_direct_flags` checks again.
pub fn invalidate_direct_flags() {
    use std::sync::atomic::Ordering::SeqCst;
    KN_DIRECT.flags.store(0, SeqCst);
    DIRECT_FLAGS_DIRTY.store(true, SeqCst);
}
#[inline(never)]
pub fn update_direct_flags() {
    use std::sync::atomic::Ordering::SeqCst;
    if !DIRECT_READY.load(SeqCst) {
        return;
    }
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    DIRECT_FLAGS_DIRTY.store(false, SeqCst);
    static IDS: std::sync::OnceLock<[u32; 4]> = std::sync::OnceLock::new();
    let [eq, nil_p, is_a, kind_of] = *IDS.get_or_init(|| [sym("=="), sym("nil?"), sym("is_a?"), sym("kind_of?")]);
    let c = rt().c;
    let is = |k: *mut Class, mid: u32, opt: u8| {
        let m = find_method(k, mid);
        !m.is_null() && unsafe { (*m).opt } == opt
    };
    let imm = [c.integer, c.float, c.symbol, c.nil, c.true_, c.false_];
    let mut flags = 0;
    if is(c.symbol, eq, crate::opt::OPT_IDENTITY_EQ) {
        flags |= DIRECT_SYM_EQ;
    }
    if imm.iter().all(|&k| is(k, nil_p, crate::opt::OPT_NIL_P)) {
        flags |= DIRECT_NIL_P;
    }
    if imm.iter().all(|&k| is(k, is_a, crate::opt::OPT_IS_A) && is(k, kind_of, crate::opt::OPT_IS_A)) {
        flags |= DIRECT_IS_A;
    }
    static ARY_IDS: std::sync::OnceLock<[u32; 2]> = std::sync::OnceLock::new();
    let [aref, aset] = *ARY_IDS.get_or_init(|| [sym("[]"), sym("[]=")]);
    if is(c.array, aref, crate::opt::OPT_ARY_AREF) {
        flags |= DIRECT_ARY_AREF;
    }
    if is(c.array, aset, crate::opt::OPT_ARY_ASET) {
        flags |= DIRECT_ARY_ASET;
    }
    static STR_IDS: std::sync::OnceLock<[u32; 2]> = std::sync::OnceLock::new();
    let [getbyte, setbyte] = *STR_IDS.get_or_init(|| [sym("getbyte"), sym("setbyte")]);
    if is(c.string, getbyte, crate::opt::OPT_STR_GETBYTE) && is(c.string, setbyte, crate::opt::OPT_STR_SETBYTE) {
        flags |= DIRECT_STR_BYTE;
    }
    if is(c.string, sym("+"), crate::opt::OPT_STR_PLUS) {
        flags |= DIRECT_STR_PLUS;
    }
    let public = |k: *mut Class, name: &str, opt: u8| {
        let m = find_method(k, sym(name));
        !m.is_null() && unsafe { (*m).opt == opt && (*m).vis != VIS_PRIVATE }
    };
    {
        use crate::opt::*;
        let ary = [
            ("each", OPT_ARY_EACH),
            ("each_with_index", OPT_ARY_EACH_WITH_INDEX),
            ("each_index", OPT_ARY_EACH_INDEX),
            ("map", OPT_ARY_MAP),
            ("collect", OPT_ARY_MAP),
            ("select", OPT_ARY_SELECT),
            ("filter", OPT_ARY_SELECT),
            ("reject", OPT_ARY_REJECT),
            ("any?", OPT_ARY_ANY),
            ("all?", OPT_ARY_ALL),
            ("count", OPT_ARY_COUNT),
            ("sum", OPT_ARY_SUM),
        ];
        if ary.iter().all(|&(n, o)| public(c.array, n, o)) {
            flags |= DIRECT_ARY_ITER;
        }
        let basic = [
            ("size", OPT_ARY_SIZE),
            ("length", OPT_ARY_SIZE),
            ("empty?", OPT_ARY_EMPTY),
            ("first", OPT_ARY_FIRST),
            ("last", OPT_ARY_LAST),
            ("push", OPT_ARY_PUSH),
            ("<<", OPT_ARY_PUSH),
            ("pop", OPT_ARY_POP),
            ("shift", OPT_ARY_SHIFT),
        ];
        if basic.iter().all(|&(n, o)| public(c.array, n, o)) {
            flags |= DIRECT_ARY_BASIC;
        }
        if public(c.integer, "times", OPT_INT_TIMES) && public(c.integer, "upto", OPT_INT_UPTO) && public(c.integer, "downto", OPT_INT_DOWNTO) {
            flags |= DIRECT_INT_ITER;
        }
    }
    let ne = sym("!=");
    if is(c.string, sym("[]"), crate::opt::OPT_STR_AREF)
        && is(c.string, eq, crate::opt::OPT_STR_EQ)
        && is(c.nil, eq, crate::opt::OPT_IDENTITY_EQ)
        && is(c.string, ne, crate::opt::OPT_NEQ)
        && is(c.nil, ne, crate::opt::OPT_NEQ)
    {
        flags |= DIRECT_STR_IDX_EQ;
    }
    static NUM_IDS: std::sync::OnceLock<[u32; 5]> = std::sync::OnceLock::new();
    let [to_f, neg, pow, shl, shr] = *NUM_IDS.get_or_init(|| [sym("to_f"), sym("-@"), sym("**"), sym("<<"), sym(">>")]);
    let num = crate::opt::OPT_NUM_INLINE;
    if is(c.integer, to_f, num)
        && is(c.float, to_f, num)
        && is(c.integer, neg, num)
        && is(c.float, neg, num)
        && is(c.float, pow, num)
        && is(c.integer, shl, num)
        && is(c.integer, shr, num)
    {
        flags |= DIRECT_NUM;
    }
    let math = KN_DIRECT.math_module.load(SeqCst);
    if math != 0 && is_kind(math as V, Kind::Class) {
        let sc = singleton_class(math as V);
        if crate::opt::MATH_FNS.iter().all(|n| is(sc, sym(n), crate::opt::OPT_MATH)) {
            flags |= DIRECT_MATH;
        }
    }
    if !IS_A_TOUCHED.load(SeqCst) {
        flags |= DIRECT_IS_A_HEAP;
    }
    if !NIL_P_TOUCHED.load(SeqCst) && flags & DIRECT_NIL_P != 0 && is(c.object, nil_p, crate::opt::OPT_NIL_P) {
        flags |= DIRECT_NIL_P_ALL;
    }
    if !EQQ_TOUCHED.load(SeqCst) && is(c.module, sym("==="), crate::opt::OPT_MOD_EQQ) {
        flags |= DIRECT_MOD_EQQ_ALL;
    }
    KN_DIRECT.flags.store(flags, SeqCst);
    // (set meanwhile)
    if IS_A_TOUCHED.load(SeqCst) {
        KN_DIRECT.flags.fetch_and(!DIRECT_IS_A_HEAP, SeqCst);
    }
    if NIL_P_TOUCHED.load(SeqCst) {
        KN_DIRECT.flags.fetch_and(!DIRECT_NIL_P_ALL, SeqCst);
    }
    if EQQ_TOUCHED.load(SeqCst) {
        KN_DIRECT.flags.fetch_and(!DIRECT_MOD_EQQ_ALL, SeqCst);
    }
    // changed again meanwhile: what we found may be stale
    if DIRECT_FLAGS_DIRTY.load(SeqCst) {
        KN_DIRECT.flags.store(0, SeqCst);
    }
}

/// Set the running stack's limit (`PState::stack_limit`), for direct calls too.
pub fn set_stack_limit(limit: usize) {
    pst().stack_limit = limit;
    let multi = crate::mt::multi();
    KN_DIRECT.call_limit.store(if multi { usize::MAX } else { limit }, std::sync::atomic::Ordering::SeqCst);
    KN_DIRECT.entry_limit.store(if multi { 0 } else { limit }, std::sync::atomic::Ordering::SeqCst);
}
#[inline(always)]
pub fn serial() -> u64 {
    SERIAL.load(std::sync::atomic::Ordering::Acquire)
}

pub fn init_runtime() {
    let x = 0u8;
    init_runtime_at(std::hint::black_box(&x) as *const u8 as usize + 256);
}
/// Boot the runtime on the calling thread; `stack_base` is the top of its stack (for the GC).
pub fn init_runtime_at(stack_base: usize) {
    let r = Box::new(Runtime {
        c: Core::default(),
        gvars: CMap::new(),
        main_obj: QNIL,
        out: crate::mt::L::new(Vec::with_capacity(1 << 16)),
        rng: 0x2545F4914F6CDD1D,
        script_name: String::new(),
    });
    *RT.get() = Box::into_raw(r);
    new_pstate();
    pst().heap.stack_base = stack_base;
    crate::mt::SPIN_HOOK.store(crate::gc::park_if_stopping as fn() as *mut (), std::sync::atomic::Ordering::Release);
    crate::boot::boot();
}

// ---------- classes ----------

pub fn new_class_raw(name: &str, superclass: *mut Class, is_module: bool) -> *mut Class {
    if !superclass.is_null() && DIRECT_READY.load(std::sync::atomic::Ordering::SeqCst) && superclass == rt().c.basic_object {
        // (its instances have no is_a? or nil?)
        is_a_touched();
        nil_p_touched();
    }
    let alloc_kind = if superclass.is_null() { Kind::Object } else { unsafe { (*superclass).alloc } };
    let c = Box::into_raw(Box::new(Class {
        h: header(std::ptr::null_mut(), Kind::Class),
        name: name.to_string(),
        superclass,
        methods: CMap::new(),
        includes: SVec::new(),
        consts: CMap::new(),
        ivar_index: CMap::new(),
        ivar_names: SVec::new(),
        civars: CMap::new(),
        cvars: CMap::new(),
        is_module,
        is_singleton: false,
        attached: QNIL,
        lexical_parent: std::ptr::null_mut(),
        alloc: alloc_kind,
        struct_members: SVec::new(),
        module_function: false,
        prepends: SVec::new(),
        default_vis: 0,
        sup_ptr: std::ptr::null(),
        sup_len: 0,
    }));
    if !is_module {
        let mut s: Vec<*mut Class> = if superclass.is_null() { Vec::new() } else { unsafe { (*superclass).supers().to_vec() } };
        s.push(c);
        let s = Box::leak(s.into_boxed_slice());
        unsafe {
            (*c).sup_ptr = s.as_ptr();
            (*c).sup_len = s.len();
        }
    }
    crate::gc::register_class(c);
    c
}

/// Create a class with its metaclass (class methods are inherited through metaclasses).
pub fn new_class(name: &str, superclass: *mut Class) -> *mut Class {
    let c = new_class_raw(name, superclass, false);
    let r = rt();
    if !r.c.class.is_null() {
        let meta_super = if superclass.is_null() { r.c.class } else { singleton_of_class(superclass) };
        let meta = new_class_raw("", meta_super, false);
        unsafe {
            (*meta).is_singleton = true;
            (*meta).attached = c as V;
            (*meta).h.klass = r.c.class;
            (*c).h.klass = meta;
        }
    }
    if !superclass.is_null() {
        unsafe {
            (*c).struct_members.set((*superclass).struct_members.clone());
        }
    }
    // no cache invalidation: method caches are keyed by class, and a new class has none
    c
}
pub fn new_module(name: &str) -> *mut Class {
    let m = new_class_raw(name, std::ptr::null_mut(), true);
    unsafe {
        (*m).h.klass = rt().c.module;
    }
    m
}
fn singleton_of_class(c: *mut Class) -> *mut Class {
    unsafe {
        let k = (*c).h.klass;
        if !k.is_null() && (*k).is_singleton { k } else { singleton_class(c as V) }
    }
}

pub fn singleton_class(v: V) -> *mut Class {
    if !is_heap(v) {
        // nil/true/false/numbers/symbols: methods go on the class itself
        return class_of(v);
    }
    let h = hdr(v);
    unsafe {
        if !h.klass.is_null() && (*h.klass).is_singleton && (*h.klass).attached == v {
            return h.klass;
        }
        let sup = if is_class(v) && !(*(v as *mut Class)).is_module {
            let s = (*(v as *mut Class)).superclass;
            if s.is_null() { rt().c.class } else { singleton_of_class(s) }
        } else {
            h.klass
        };
        let meta = new_class_raw("", sup, false);
        (*meta).is_singleton = true;
        (*meta).attached = v;
        // the singleton class of a plain object sees its class's class methods (CRuby: its
        // singleton class inherits from the class's): `class << obj; self; end.class_macro`
        (*meta).h.klass = if !is_class(v) && !(*sup).is_module && !(*sup).is_singleton { singleton_of_class(sup) } else { rt().c.class };
        (*meta).alloc = (*sup).alloc;
        h.klass = meta;

        // no global cache invalidation: the object's class pointer changed, so its call
        // sites miss their caches anyway (`extend` / `define_singleton_method` on objects
        // at request time must not flush every method cache)
        meta
    }
}

#[inline(always)]
pub fn class_of(v: V) -> *mut Class {
    let c = &rt().c;
    if is_fix(v) {
        c.integer
    } else if is_heap(v) {
        hdr(v).klass
    } else if v == QNIL {
        c.nil
    } else if v == QTRUE {
        c.true_
    } else if v == QFALSE {
        c.false_
    } else if is_sym(v) {
        c.symbol
    } else if is_flonum(v) {
        c.float
    } else {
        c.object
    }
}
/// The class ignoring singleton classes.
pub fn real_class(v: V) -> *mut Class {
    let mut k = class_of(v);
    unsafe {
        while !k.is_null() && (*k).is_singleton {
            k = (*k).superclass;
        }
    }
    k
}
pub fn class_name(c: *mut Class) -> String {
    unsafe {
        if c.is_null() {
            return "?".into();
        }
        if (*c).is_singleton {
            return format!("#<Class:{}>", crate::fmt::inspect_lossy((*c).attached));
        }
        if (*c).name.is_empty() { format!("#<Class:0x{:016x}>", c as usize) } else { (*c).name.clone() }
    }
}

/// How many method tables have an entry of each name: never too low (every new entry
/// counts; a freed table's entries are not taken off). For `claim`.
static METHOD_NAMES: std::sync::Mutex<Option<rustc_hash::FxHashMap<u32, u32>>> = std::sync::Mutex::new(None);
/// Set (for good) once a method named is_a? or kind_of? is defined, removed, undefined or
/// made private/protected after startup, or a class inherits from BasicObject directly:
/// until then every heap object's is_a?/kind_of? is the builtin one (abi.rs `kn_is_a_send`).
pub static IS_A_TOUCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Set (for good) once a method named === is defined, removed, undefined or made
/// private/protected after startup in a singleton class, a module (it may be `extend`ed) or
/// Module/Class: until then every class's and module's === is the builtin Module#===
/// (`DIRECT_MOD_EQQ_ALL`).
pub static EQQ_TOUCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Class `c`'s method table entry of name `mid` changed (see `IS_A_TOUCHED`, `EQQ_TOUCHED`).
pub fn touch_method_name(c: *mut Class, mid: u32) {
    use std::sync::atomic::Ordering::SeqCst;
    if !DIRECT_READY.load(SeqCst) {
        return;
    }
    static IDS: std::sync::OnceLock<[u32; 4]> = std::sync::OnceLock::new();
    let [is_a, kind_of, eqq, nil_p] = *IDS.get_or_init(|| [sym("is_a?"), sym("kind_of?"), sym("==="), sym("nil?")]);
    if mid == is_a || mid == kind_of {
        is_a_touched();
    } else if mid == nil_p {
        nil_p_touched();
    } else if mid == eqq && !c.is_null() {
        let k = unsafe { &*c };
        if k.is_singleton || k.is_module || c == rt().c.module || c == rt().c.class {
            EQQ_TOUCHED.store(true, SeqCst);
            KN_DIRECT.flags.fetch_and(!DIRECT_MOD_EQQ_ALL, SeqCst);
        }
    }
}
/// Module `m` was included in or prepended to `target`: its methods named === count as
/// defined in `target` (`EQQ_TOUCHED`).
pub fn touch_mixin(target: *mut Class, m: *mut Class) {
    if DIRECT_READY.load(std::sync::atomic::Ordering::SeqCst) && !find_method(m, sym("===")).is_null() {
        touch_method_name(target, sym("==="));
    }
}
/// Some class's nil? was defined, removed or changed, or a class without Kernel exists
/// (`DIRECT_NIL_P_ALL`; never unset).
pub static NIL_P_TOUCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn nil_p_touched() {
    use std::sync::atomic::Ordering::SeqCst;
    NIL_P_TOUCHED.store(true, SeqCst);
    KN_DIRECT.flags.fetch_and(!DIRECT_NIL_P_ALL, SeqCst);
}
pub fn is_a_touched() {
    use std::sync::atomic::Ordering::SeqCst;
    IS_A_TOUCHED.store(true, SeqCst);
    KN_DIRECT.flags.fetch_and(!DIRECT_IS_A_HEAP, SeqCst);
}
/// A method table got an entry of name `mid` (`added`) or lost one.
pub fn method_name_count(mid: u32, added: bool) {
    let mut g = METHOD_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.get_or_insert_with(Default::default).entry(mid).or_insert(0);
    if added {
        *c += 1;
    } else {
        *c = c.saturating_sub(1);
    }
}
pub fn methods_named(mid: u32) -> u32 {
    let g = METHOD_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(&mid).copied()).unwrap_or(0)
}
/// A compiled method's word for its calls to itself (kenny-compiler statics.rs
/// `self_call`): 0 while the method is the only one of its name in any method table, its
/// owner a class (a module's method may be bound to any object), and one OS thread runs
/// Ruby code; else 1. Set by its `def` (abi.rs `kn_def_claim`), redone on every method table
/// change (`bump_serial`).
struct Claim {
    word: usize,
    owner: usize,
    mid: u32,
    fast: usize,
}
static CLAIMS: std::sync::Mutex<Vec<Claim>> = std::sync::Mutex::new(Vec::new());
fn claim_holds(c: &Claim) -> bool {
    let k = c.owner as *mut Class;
    !crate::mt::multi()
        && methods_named(c.mid) == 1
        && unsafe { (*k).methods.get(&c.mid) }.is_some_and(|&m| unsafe { (*m).fast } == c.fast)
}
fn set_claim_word(c: &Claim) {
    let v = if claim_holds(c) { 0 } else { 1 };
    unsafe { (*(c.word as *const std::sync::atomic::AtomicU64)).store(v, std::sync::atomic::Ordering::SeqCst) };
}
/// `def` of a method that calls itself, with fast entry `fast`, in `owner`.
pub fn claim(word: *mut u64, owner: *mut Class, mid: u32, fast: usize) {
    let mut cs = CLAIMS.lock().unwrap_or_else(|e| e.into_inner());
    cs.retain(|x| x.word != word as usize);
    // (a singleton class may be freed: never kept here)
    let kept = unsafe { !(*owner).is_module && !(*owner).is_singleton };
    // (the method's fast entry is set after `add_method`: so is this)
    let holds = kept && !crate::mt::multi() && methods_named(mid) == 1;
    unsafe { (*(word as *const std::sync::atomic::AtomicU64)).store(if holds { 0 } else { 1 }, std::sync::atomic::Ordering::SeqCst) };
    if kept {
        cs.push(Claim { word: word as usize, owner: owner as usize, mid, fast });
    }
}
fn redo_claims() {
    for c in CLAIMS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        set_claim_word(c);
    }
}
/// Several OS threads may now run Ruby code: no more claims.
pub fn drop_claims() {
    redo_claims();
}

pub fn bump_serial() {
    let s = SERIAL.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    KN_DIRECT.serial.store(s, std::sync::atomic::Ordering::SeqCst);
    redo_claims();
    crate::opt::clear_attr_sites();
    invalidate_direct_flags();
    bump_const_serial(); // include/prepend change constant lookup too
}

/// Bumped whenever a constant lookup could change (constant set/removed, autoload
/// registered, ancestry changed); per-site constant caches compare against it.
pub static CONST_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
#[inline(always)]
pub fn const_serial() -> u64 {
    CONST_SERIAL.load(std::sync::atomic::Ordering::Acquire)
}
pub fn bump_const_serial() {
    let s = CONST_SERIAL.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
    KN_DIRECT.const_serial.store(s, std::sync::atomic::Ordering::Release);
}

/// Ancestors in method-resolution order (a copy; see `ancestors_rc`).
pub fn ancestors(c: *mut Class) -> Vec<*mut Class> {
    ancestors_rc(c).to_vec()
}

struct AncestorCache {
    serial: u64,
    map: rustc_hash::FxHashMap<usize, std::sync::Arc<[*mut Class]>>,
    supers: rustc_hash::FxHashMap<(usize, usize, u32), *const Method>,
}

#[inline(never)]
fn ancestor_cache() -> &'static mut AncestorCache {
    // per OS thread (no locking on the hot path). The values are Arcs: a caller may keep one
    // across a call into Ruby, and its thread may then continue on another OS thread.
    thread_local!(static ANC: std::cell::Cell<*mut AncestorCache> = const { std::cell::Cell::new(std::ptr::null_mut()) });
    let serial = serial();
    let p = std::hint::black_box(ANC.with(|a| {
        if a.get().is_null() {
            a.set(Box::into_raw(Box::new(AncestorCache { serial, map: Default::default(), supers: Default::default() })));
        }
        a.get()
    }));
    let c = unsafe { &mut *p };
    // the method serial changes with every include/prepend (and method definition)
    if c.serial != serial {
        c.map.clear();
        c.supers.clear();
        c.serial = serial;
    }
    c
}

/// Ancestors in method-resolution order, cached per class until the hierarchy changes.
pub fn ancestors_rc(c: *mut Class) -> std::sync::Arc<[*mut Class]> {
    let cache = ancestor_cache();
    if let Some(a) = cache.map.get(&(c as usize)) {
        return a.clone();
    }
    let a: std::sync::Arc<[*mut Class]> = ancestors_uncached(c).into();
    cache.map.insert(c as usize, a.clone());
    a
}

/// Ruby's order: each `include` (in the order they happened) inserts the module and the
/// modules it includes right after the class, skipping any module already in the chain
/// below that point (so a module that a superclass already includes, directly or through
/// another module, is not inserted again: concurrent-ruby's SerialExecutorService brings
/// ExecutorService, which RubyThreadPoolExecutor's superclass already has).
fn ancestors_uncached(c: *mut Class) -> Vec<*mut Class> {
    unsafe {
        let rest: std::sync::Arc<[*mut Class]> = if (*c).superclass.is_null() { std::sync::Arc::from(Vec::new()) } else { ancestors_rc((*c).superclass) };
        let (pre, part) = own_chain(c, &rest, 0);
        let mut out = pre;
        out.push(c);
        out.extend(part.into_iter().map(|e| e.0));
        out.extend(rest.iter().copied());
        out
    }
}
/// When each runtime `include` happened: (class, module) -> sequence number. Boot-time
/// includes have none (0: always there).
static INCLUDE_SEQ: std::sync::Mutex<Option<rustc_hash::FxHashMap<(usize, usize), u64>>> = std::sync::Mutex::new(None);
static INCLUDE_CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Record that `m` was just included into `k`.
pub fn note_include(k: *mut Class, m: *mut Class) {
    let t = INCLUDE_CLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    INCLUDE_SEQ.lock().unwrap().get_or_insert_with(Default::default).insert((k as usize, m as usize), t);
}
fn include_seq(k: *mut Class, m: *mut Class) -> u64 {
    INCLUDE_SEQ.lock().unwrap().as_ref().and_then(|t| t.get(&(k as usize, m as usize)).copied()).unwrap_or(0)
}

/// When module `x` joined the chain `below` (a superclass's ancestors): the earliest runtime
/// include of it there, or 0 when it came at boot or through another module.
fn joined_below(below: &[*mut Class], x: *mut Class) -> u64 {
    let mut first: Option<u64> = None;
    for &c in below {
        if unsafe { (*c).includes.contains(&x) } {
            let t = include_seq(c, x);
            first = Some(first.map_or(t, |f| f.min(t)));
        }
    }
    first.unwrap_or(0)
}

/// (prepended chain, included chain) of `k`, leaving out anything already in `below`. Each
/// included entry carries when it joined `k`'s chain: a module that two includes bring sits
/// where it arrived first. (CRuby keeps both copies and finds the first: Rails 8 includes
/// its to_json module into Hash, then into Enumerable, which Hash already includes.)
fn own_chain(k: *mut Class, below: &[*mut Class], depth: u32) -> (Vec<*mut Class>, Vec<(*mut Class, u64)>) {
    let mut part: Vec<(*mut Class, u64)> = Vec::new();
    let mut pre: Vec<*mut Class> = Vec::new();
    if depth > 64 {
        return (pre, part);
    }
    unsafe {
        for &m in (*k).includes.iter() {
            if m == k {
                continue;
            }
            let seq = include_seq(k, m);
            let mut new: Vec<(*mut Class, u64)> = Vec::new();
            for (x, t) in module_chain(m, depth + 1) {
                let t = t.max(seq);
                // in the superclass chain already when this include happened: skipped
                if x == k || new.iter().any(|e| e.0 == x) || (below.contains(&x) && joined_below(below, x) <= t) {
                    continue;
                }
                match part.iter().position(|e| e.0 == x) {
                    Some(i) if part[i].1 > t => {
                        part.remove(i);
                    }
                    Some(_) => continue,
                    None => {}
                }
                new.push((x, t));
            }
            part.splice(0..0, new);
        }
        for &m in (*k).prepends.iter() {
            if m == k {
                continue;
            }
            // unlike include, prepend doesn't skip a module the superclass chain already has
            // (ActiveSupport prepends its to_json module to Object and to String)
            let chain = module_chain(m, depth + 1);
            let new: Vec<*mut Class> =
                chain.into_iter().map(|e| e.0).filter(|x| *x != k && !pre.contains(x) && !part.iter().any(|e| e.0 == *x)).collect();
            pre.splice(0..0, new);
        }
    }
    (pre, part)
}
/// A module with what it includes and prepends, in method-resolution order, each with when
/// it joined the module's chain.
fn module_chain(m: *mut Class, depth: u32) -> Vec<(*mut Class, u64)> {
    let (pre, part) = own_chain(m, &[], depth);
    let mut out: Vec<(*mut Class, u64)> = pre.into_iter().map(|x| (x, 0)).collect();
    out.push((m, 0));
    out.extend(part);
    out
}

/// First level of the global method cache: direct-mapped, so a call site whose inline cache
/// misses (a polymorphic site) usually costs one probe instead of a hash-map lookup.
#[derive(Clone, Copy)]
pub struct McEntry {
    klass: usize,
    mid: u32,
    serial: u64,
    m: *const Method,
}
const MC_SIZE: usize = 4096;
/// A fresh per-thread method cache (each P has its own: entries are several words).
pub fn new_method_cache() -> Box<[McEntry; MC_SIZE]> {
    Box::new([McEntry { klass: 0, mid: 0, serial: 0, m: std::ptr::null() }; MC_SIZE])
}

#[inline(always)]
pub fn find_method(c: *mut Class, mid: u32) -> *const Method {
    let slot = ((c as usize >> 4) ^ (mid as usize).wrapping_mul(0x9E37_79B9)) & (MC_SIZE - 1);
    let e = &pst().mc[slot];
    if e.klass == c as usize && e.mid == mid && e.serial == serial() {
        return e.m;
    }
    find_method_slow(c, mid, slot)
}
#[inline(never)]
fn find_method_slow(c: *mut Class, mid: u32, slot: usize) -> *const Method {
    let r = pst();
    let serial = serial();
    if r.mcache_serial != serial {
        r.mcache.clear();
        r.mcache_serial = serial;
    }
    let m = match r.mcache.get(&(c as usize, mid)) {
        Some(&m) => m,
        None => {
            let m = find_method_uncached(c, mid);
            r.mcache.insert((c as usize, mid), m);
            m
        }
    };
    r.mc[slot] = McEntry { klass: c as usize, mid, serial, m };
    m
}
/// The first definition along the ancestors (in Ruby's order, see `ancestors_uncached`).
fn find_method_uncached(c: *mut Class, mid: u32) -> *const Method {
    for &k in ancestors_rc(c).iter() {
        if let Some(&m) = unsafe { (*k).methods.get(&mid) } {
            return if matches!(unsafe { &(*m).body }, Body::Undefined) { std::ptr::null() } else { m };
        }
    }
    std::ptr::null()
}
pub fn find_super_method(self_: V, owner: *mut Class, mid: u32) -> *const Method {
    let k = class_of(self_);
    let key = (k as usize, owner as usize, mid);
    if let Some(&m) = ancestor_cache().supers.get(&key) {
        return m;
    }
    let m = find_super_method_uncached(k, owner, mid);
    ancestor_cache().supers.insert(key, m);
    m
}
fn find_super_method_uncached(klass: *mut Class, owner: *mut Class, mid: u32) -> *const Method {
    let anc = ancestors_rc(klass);
    if !anc.contains(&owner) {
        // `super` from a refinement method: the refined class's own method
        return find_method(klass, mid);
    }
    let mut found = false;
    for &k in anc.iter() {
        if found {
            if let Some(&m) = unsafe { (*k).methods.get(&mid) } {
                return m;
            }
        } else if k == owner {
            found = true;
        }
    }
    std::ptr::null()
}

pub fn add_method(c: *mut Class, mid: u32, body: Body) -> *const Method {
    let m = Box::into_raw(Box::new(Method { fast: 0, body, owner: c, name: mid, params: std::ptr::null(), vis: if always_private(mid) { VIS_PRIVATE } else { unsafe { (*c).default_vis } }, opt: 0, gc_transparent: false, attr_slot: Default::default() })) as *const Method;
    if unsafe { (*c).methods.insert(mid, m) }.is_none() {
        method_name_count(mid, true);
    }
    touch_method_name(c, mid);
    if BOOTING.load(std::sync::atomic::Ordering::Relaxed) {
        // (no compiled code has run yet: no claims, attr sites or direct flags to reset; the
        // method caches still see the new serial)
        let s = SERIAL.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        KN_DIRECT.serial.store(s, std::sync::atomic::Ordering::SeqCst);
    } else {
        bump_serial();
    }
    m
}
/// Set while `boot` defines the built-in classes and methods.
pub static BOOTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Mark native methods as GC-transparent (see Method::gc_transparent).
pub fn gc_transparent(c: *mut Class, names: &[&str]) {
    for n in names {
        if let Some(&m) = unsafe { (*c).methods.get(&sym(n)) } {
            unsafe { (*(m as *mut Method)).gc_transparent = true };
        }
    }
}
pub fn defn(c: *mut Class, name: &str, min: i32, max: i32, f: NativeFn) {
    add_method(c, sym(name), Body::Native(f, min, max));
}
pub fn defs(c: *mut Class, name: &str, min: i32, max: i32, f: NativeFn) {
    let meta = singleton_class(c as V);
    add_method(meta, sym(name), Body::Native(f, min, max));
}
pub fn respond_to(v: V, mid: u32) -> bool {
    !find_method(class_of(v), mid).is_null()
}

// ---------- calling ----------

#[inline(always)]
pub(crate) fn approx_sp() -> usize {
    let x = 0u8;
    &x as *const u8 as usize
}

pub fn invoke(m: *const Method, recv: V, args: &[V], blk: V) -> V {
    if crate::gc::safepoint() {
        return UNDEF;
    }
    let m = unsafe { &*m };
    match &m.body {
        Body::Compiled(f) => {
            if approx_sp() < pst().stack_limit {
                return raise(rt().c.system_stack_error, "stack level too deep");
            }
            unsafe { f(recv, args.len() as u64, args.as_ptr(), blk) }
        }
        Body::Native(f, min, max) => {
            let n = args.len() as i32;
            if n < *min || (*max >= 0 && n > *max) {
                return arity_error(n, *min, *max);
            }
            if m.gc_transparent {
                return f(recv, args, blk);
            }
            call_native(*f, recv, args, blk)
        }
        Body::AttrReader(iv) => {
            if !args.is_empty() {
                return arity_error(args.len() as i32, 0, 0);
            }
            ivar_get(recv, *iv)
        }
        Body::AttrWriter(iv) => {
            if args.len() != 1 {
                return arity_error(args.len() as i32, 1, 1);
            }
            ivar_set(recv, *iv, args[0])
        }
        Body::Proc(p) => {
            pst().dm_stack.push(m as *const Method);
            let r = call_proc_with_self(*p, recv, args, blk, true);
            pst().dm_stack.pop();
            r
        }
        Body::CFunc(f, arity) => {
            pst().current_block.push(blk);
            crate::capi::enter_frame((*m).name, recv);
            let r = call_native_c(*f, *arity, recv, args);
            crate::capi::leave_frame();
            pst().current_block.pop();
            r
        }
        Body::Undefined => QNIL,
    }
}


pub fn arity_error(given: i32, min: i32, max: i32) -> V {
    let expected = if max < 0 {
        format!("{min}+")
    } else if min == max {
        format!("{min}")
    } else {
        format!("{min}..{max}")
    };
    raise(rt().c.argument_error, &format!("wrong number of arguments (given {given}, expected {expected})"))
}

pub fn send(recv: V, mid: u32, args: &[V], blk: V) -> V {
    let m = find_method(class_of(recv), mid);
    if m.is_null() {
        return method_missing(recv, mid, args, blk, false);
    }
    invoke(m, recv, args, blk)
}
pub fn send0(recv: V, name: &str) -> V {
    send(recv, sym(name), &[], QNIL)
}
pub fn send1(recv: V, name: &str, a: V) -> V {
    send(recv, sym(name), &[a], QNIL)
}

pub fn method_missing(recv: V, mid: u32, args: &[V], blk: V, vcall: bool) -> V {
    let mm = sym("method_missing");
    let m = find_method(class_of(recv), mm);
    if !m.is_null() && !matches!(unsafe { &(*m).body }, Body::Native(..)) {
        let mut a = Vec::with_capacity(args.len() + 1);
        a.push(mksym(mid));
        a.extend_from_slice(args);
        return invoke(m, recv, &a, blk);
    }
    no_method_error(recv, mid, vcall)
}
pub fn no_method_error(recv: V, mid: u32, vcall: bool) -> V {
    let name = sym_str(mid);
    let e = if vcall {
        exc_new(rt().c.name_error, &format!("undefined local variable or method '{}' for {}", name, describe_recv(recv)))
    } else {
        exc_new(rt().c.no_method_error, &format!("undefined method '{}' for {}", name, describe_recv(recv)))
    };
    // what NameError#name / #receiver / NoMethodError#private_call? report (DidYouMean uses them)
    ivar_set(e, sym("@name"), mksym(mid));
    ivar_set(e, sym("@receiver"), recv);
    if vcall {
        ivar_set(e, sym("@__private_call"), QTRUE);
    }
    raise_exc(e)
}
pub fn describe_recv(v: V) -> String {
    if v == QNIL {
        "nil".into()
    } else if v == QTRUE {
        "true".into()
    } else if v == QFALSE {
        "false".into()
    } else if v == rt().main_obj {
        "main".into()
    } else if is_class(v) {
        let c = rclass(v);
        if c.is_module { format!("module {}", c.name) } else { format!("class {}", class_name(v as *mut Class)) }
    } else {
        format!("an instance of {}", class_name(real_class(v)))
    }
}

/// Call a block/proc with Ruby block-argument semantics.
pub fn call_block(blk: V, args: &[V]) -> V {
    if !is_proc(blk) {
        return raise(rt().c.local_jump_error, "no block given (yield)");
    }
    let p = rproc(blk);
    call_proc_with_self(blk, p.self_, args, QNIL, false)
}
pub fn call_block_with_blk(blk: V, args: &[V], blkarg: V) -> V {
    if !is_proc(blk) {
        return raise(rt().c.local_jump_error, "no block given (yield)");
    }
    let p = rproc(blk);
    call_proc_with_self(blk, p.self_, args, blkarg, false)
}
pub fn call_proc_with_self(pv: V, self_: V, args: &[V], blk: V, as_method: bool) -> V {
    if crate::gc::safepoint() {
        return UNDEF;
    }
    let p = rproc(pv);
    if p.lambda || as_method {
        let mut n = args.len() as i32;
        if hdr(pv).flags & FL_PROC_KW != 0 && args.last().is_some_and(|&h| is_hash(h) && hdr(h).flags & FL_KWHASH != 0) {
            n -= 1;
        }
        if n < p.min || (p.max >= 0 && n > p.max) {
            return arity_error(n, p.min, p.max);
        }
    } else if p.autosplat && args.len() == 1 && is_ary(args[0]) {
        let v = rary(args[0]).v.clone();
        return unsafe { (p.func)(pv as *mut RProc, self_, v.len() as u64, v.as_ptr(), blk) };
    }
    unsafe { (p.func)(pv as *mut RProc, self_, args.len() as u64, args.as_ptr(), blk) }
}

// ---------- instance variables ----------

pub fn ivar_get(v: V, id: u32) -> V {
    match kind(v) {
        Some(Kind::Object) => {
            let o = obj(v);
            let shape = unsafe { &*o.shape };
            match shape_index(shape, id) {
                // UNDEF marks an ivar removed by remove_instance_variable
                Some(i) => o.ivars.get(i as usize).copied().filter(|x| *x != UNDEF).unwrap_or(QNIL),
                None => QNIL,
            }
        }
        Some(Kind::Class) => rclass(v).civars.get(&id).copied().unwrap_or(QNIL),
        _ => gen_ivar_get(v, id).unwrap_or(QNIL),
    }
}
pub fn ivar_defined(v: V, id: u32) -> bool {
    match kind(v) {
        Some(Kind::Object) => {
            let o = obj(v);
            let shape = unsafe { &*o.shape };
            matches!(shape_index(shape, id), Some(i) if (i as usize) < o.ivars.len() && o.ivars[i as usize] != UNDEF)
        }
        Some(Kind::Class) => rclass(v).civars.contains_key(&id),
        _ => gen_ivar_get(v, id).is_some(),
    }
}
/// Slot of ivar `id` in objects of `shape`, if it has one. Small shapes (the common case)
/// are searched linearly, which beats hashing.
#[inline]
pub fn shape_index(shape: &Class, id: u32) -> Option<u32> {
    if shape.ivar_names.len() <= 16 {
        return shape.ivar_names.iter().position(|&n| n == id).map(|i| i as u32);
    }
    shape.ivar_index.get(&id).copied()
}
pub fn ivar_index_for(shape: *mut Class, id: u32) -> u32 {
    let s = unsafe { &mut *shape };
    if let Some(i) = shape_index(s, id) {
        return i;
    }
    // two threads adding different ivars to one class must not get the same slot
    let g = crate::cmap::write_lock();
    if let Some(&i) = s.ivar_index.get(&id) {
        return i;
    }
    let i = s.ivar_names.len() as u32;
    s.ivar_names.push(id);
    s.ivar_index.insert_locked(id, i);
    drop(g);
    i
}
pub fn ivar_set(v: V, id: u32, val: V) -> V {
    if is_frozen(v) {
        let r = check_frozen(v);
        if r == UNDEF {
            return UNDEF;
        }
    }
    match kind(v) {
        Some(Kind::Object) => {
            let o = obj(v);
            let i = ivar_index_for(o.shape, id) as usize;
            if o.ivars.len() <= i {
                o.ivars.resize(i + 1, QNIL);
            }
            o.ivars[i] = val;
        }
        Some(Kind::Class) => {
            rclass(v).civars.insert(id, val);
        }
        _ => gen_ivar_set(v, id, val),
    }
    val
}
pub fn ivar_list(v: V) -> Vec<(u32, V)> {
    match kind(v) {
        Some(Kind::Object) => {
            let o = obj(v);
            let shape = unsafe { &*o.shape };
            o.ivars
                .iter()
                .enumerate()
                .filter(|(i, _)| *i < shape.ivar_names.len())
                .map(|(i, &x)| (shape.ivar_names[i], x))
                .filter(|(n, x)| *x != UNDEF && !sym_name(*n).starts_with(b"@__"))
                .collect()
        }
        Some(Kind::Class) => rclass(v).civars.iter().map(|(&k, &v)| (k, v)).filter(|(n, _)| !sym_name(*n).starts_with(b"@__")).collect(),
        _ => gen_ivar_list(v),
    }
}

// Instance variables of objects other than plain Objects and classes: on the object for
// String/Array/Hash, otherwise in a global table keyed by address.
/// (Holders do plain table work only, never anything that could stop the thread: gc.rs
/// `lazy_sweep` waits for it without stopping.)
pub static GENERIC_IVARS: crate::mt::L<Option<HashMap<usize, CMap<u32, V>, FxBuild>>> = crate::mt::L::new(None);
fn gen_slot(v: V) -> Option<&'static GenIv> {
    match kind(v) {
        Some(Kind::String) => Some(&rstr(v).iv),
        Some(Kind::Array) => Some(&rary(v).iv),
        Some(Kind::Hash) => Some(&rhash(v).iv),
        _ => None,
    }
}
pub fn gen_ivar_get(v: V, id: u32) -> Option<V> {
    match gen_slot(v) {
        Some(g) => g.map().and_then(|m| m.get(&id).copied()),
        None => GENERIC_IVARS.get().as_ref().and_then(|t| t.get(&(v as usize))).and_then(|m| m.get(&id).copied()),
    }
}
pub fn gen_ivar_set(v: V, id: u32, val: V) {
    crate::gc::wb_obj(v);
    match gen_slot(v) {
        // (wb_obj above is the barrier for these tables)
        Some(g) => {
            g.map_or_create().insert_untracked(id, val);
        }
        None => {
            GENERIC_IVARS.get().get_or_insert_with(HashMap::default).entry(v as usize).or_default().insert_untracked(id, val);
        }
    }
}
pub fn gen_ivar_remove(v: V, id: u32) -> Option<V> {
    match gen_slot(v) {
        Some(g) => g.map().and_then(|m| m.remove(&id)),
        None => GENERIC_IVARS.get().as_mut().and_then(|t| t.get_mut(&(v as usize))).and_then(|m| m.remove(&id)),
    }
}
pub fn gen_ivar_list(v: V) -> Vec<(u32, V)> {
    match gen_slot(v) {
        Some(g) => g.map().map(|m| m.iter().map(|(&k, &v)| (k, v)).collect()).unwrap_or_default(),
        None => GENERIC_IVARS.get().as_ref().and_then(|t| t.get(&(v as usize))).map(|m| m.iter().map(|(&k, &v)| (k, v)).collect()).unwrap_or_default(),
    }
}
/// Copy `from`'s generic ivars to `to` (dup/clone).
pub fn gen_ivar_copy(from: V, to: V) {
    for (k, x) in gen_ivar_list(from) {
        gen_ivar_set(to, k, x);
    }
}
/// For the GC: every value held as a generic ivar of `v`.
pub fn gen_ivar_values(v: V, out: &mut Vec<V>) {
    match gen_slot(v) {
        Some(g) => {
            if let Some(m) = g.map() {
                out.extend(m.values().copied());
            }
        }
        None => {
            if let Some(m) = GENERIC_IVARS.raw().as_ref().and_then(|t| t.get(&(v as usize))) {
                out.extend(m.values().copied());
            }
        }
    }
}

// ---------- constants ----------

pub fn const_lookup(cref: *mut Class, id: u32) -> Option<V> {
    unsafe {
        // lexical scopes first, but not the top level: Ruby checks the ancestors of
        // the innermost class before Object (so an included `Nodes::Symbol` beats `::Symbol`)
        let object = rt().c.object;
        let mut k = cref;
        while !k.is_null() && k != object {
            if let Some(&v) = (*k).consts.get(&id) {
                crate::abi::settle_autoload(k, id);
                return Some(v);
            }
            // a pending autoload here wins over outer/top-level constants (caller triggers it)
            if crate::abi::has_autoload(k, id) {
                return None;
            }
            k = (*k).lexical_parent;
        }
        if !cref.is_null() {
            for &a in ancestors_rc(cref).iter() {
                if let Some(&v) = (*a).consts.get(&id) {
                    crate::abi::settle_autoload(a, id);
                    return Some(v);
                }
                // an autoload in an earlier ancestor wins over a constant in a later one
                if crate::abi::has_autoload(a, id) {
                    return None;
                }
            }
        }
        let o = rt().c.object;
        let v = (*o).consts.get(&id).copied();
        if v.is_some() {
            crate::abi::settle_autoload(o, id);
        }
        v
    }
}
pub fn const_get_under(c: *mut Class, id: u32) -> Option<V> {
    for &a in ancestors_rc(c).iter() {
        if let Some(&v) = unsafe { (*a).consts.get(&id) } {
            crate::abi::settle_autoload(a, id);
            return Some(v);
        }
        // `Engine::Configuration` with `autoload :Configuration` in Engine must load it rather
        // than find the superclass's `Railtie::Configuration` (the caller triggers the autoload)
        if crate::abi::has_autoload(a, id) {
            return None;
        }
    }
    None
}
pub fn const_set(c: *mut Class, id: u32, v: V) {
    unsafe {
        if is_class(v) {
            let k = rclass(v);
            if k.name.is_empty() && !k.is_singleton {
                k.name = if c == rt().c.object || c.is_null() {
                    sym_str(id)
                } else {
                    format!("{}::{}", (*c).name, sym_str(id))
                };
            }
        }
        (*c).consts.insert(id, v);
    }
    crate::abi::cfold_invalidate(id);
    bump_const_serial();
}
pub fn set_global_const(name: &str, v: V) {
    const_set(rt().c.object, sym(name), v);
}

// ---------- raising ----------

/// "file:line" of the innermost Ruby call on the current stack (see pcmap.rs).
pub fn current_loc() -> Option<String> {
    let a = crate::backtrace::capture();
    let (file, line, _) = crate::backtrace::symbolize(*a.first()?)?;
    Some(format!("{file}:{line}"))
}

pub fn exc_new(cls: *mut Class, msg: &str) -> V {
    let e = object_new(cls);
    ivar_set(e, sym("@__message"), str_from(msg));
    e
}
pub fn raise(cls: *mut Class, msg: &str) -> V {
    raise_exc(exc_new(cls, msg))
}
pub fn raise_exc(e: V) -> V {
    // KENNY_DEBUG_RAISE=1: log every raise (class, message, last call site) to stderr,
    // which helps find the original error behind a rescue/ensure chain.
    static DEBUG_RAISE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEBUG_RAISE.get_or_init(|| std::env::var_os("KENNY_DEBUG_RAISE").is_some()) && is_heap(e) {
        let m = ivar_get(e, sym("@__message"));
        let msg = if is_str(m) { String::from_utf8_lossy(&rstr(m).b).into_owned() } else { String::new() };
        eprintln!("[raise] {}: {} (at {})", class_name(real_class(e)), msg, current_loc().unwrap_or_default());
    }
    // the exception being handled (`$!`) becomes the cause of one raised meanwhile, once
    if is_heap(e) && ivar_get(e, sym("@__cause_set")) != QTRUE {
        let cur = pst().errinfo;
        if cur != e && is_heap(cur) {
            ivar_set(e, sym("@__cause"), cur);
        }
        ivar_set(e, sym("@__cause_set"), QTRUE);
    }
    if is_heap(e) && !ivar_defined(e, sym("@__backtrace")) && !ivar_defined(e, sym("@__bt_raw")) {
        // the return addresses now; "file:line:in 'label'" strings only if `backtrace` is asked
        let addrs = crate::backtrace::capture();
        if !addrs.is_empty() {
            ivar_set(e, sym("@__bt_raw"), crate::backtrace::pack(&addrs));
        } else if let Some(loc) = current_loc() {
            ivar_set(e, sym("@__backtrace"), ary_new(vec![str_from(&loc)]));
        }
    }
    pst().pending = Throw::Raise(e);
    UNDEF
}
/// Integers, Floats and Symbols can't have singleton methods (TypeError, as in CRuby).
pub fn check_singleton_ok(v: V) -> V {
    if is_int(v) || is_float(v) || is_sym(v) {
        return type_error("can't define singleton");
    }
    QNIL
}
/// The builtin `v.is_a?(c)`.
pub fn is_a_v(v: V, c: V) -> V {
    if !is_class(c) {
        return type_error("class or module required");
    }
    bool_v(is_a(v, c as *mut Class))
}
pub fn type_error(msg: &str) -> V {
    raise(rt().c.type_error, msg)
}
pub fn arg_error(msg: &str) -> V {
    raise(rt().c.argument_error, msg)
}
/// A time interval in seconds (sleep, Mutex#sleep, IO.select ...) as CRuby checks it.
pub fn time_interval(secs: f64) -> Result<std::time::Duration, V> {
    if secs.is_nan() {
        Err(raise(rt().c.range_error, "NaN out of Time range"))
    } else if secs.is_infinite() {
        Err(raise(rt().c.range_error, if secs > 0.0 { "Inf out of Time range" } else { "-Inf out of Time range" }))
    } else if secs < 0.0 {
        Err(arg_error("time interval must not be negative"))
    } else {
        Ok(std::time::Duration::from_secs_f64(secs))
    }
}
/// A timeout that may be infinite (Thread#join, Queue#pop): None waits forever.
pub fn timeout_secs(secs: f64) -> Option<std::time::Duration> {
    if secs.is_infinite() && secs > 0.0 {
        None
    } else {
        Some(std::time::Duration::from_secs_f64(if secs.is_finite() { secs.max(0.0) } else { 0.0 }))
    }
}

#[macro_export]
macro_rules! tri {
    ($e:expr) => {{
        let v = $e;
        if v == $crate::core::UNDEF {
            return $crate::core::UNDEF;
        }
        v
    }};
}

pub fn is_a(v: V, c: *mut Class) -> bool {
    // the same answer as `ancestors(class_of(v)).contains(&c)`, without building the list
    fn in_mods(list: &[*mut Class], c: *mut Class) -> bool {
        list.iter().any(|&m| m == c || in_mods(unsafe { &(*m).includes }, c))
    }
    let is_mod = unsafe { (*c).is_module };
    let mut k = class_of(v);
    if !is_mod {
        let (ks, cs) = unsafe { ((*k).supers(), (*c).supers()) };
        if !cs.is_empty() {
            return ks.get(cs.len() - 1) == Some(&c);
        }
    }
    while !k.is_null() {
        if k == c {
            return true;
        }
        if is_mod && unsafe { in_mods(&(*k).prepends, c) || in_mods(&(*k).includes, c) } {
            return true;
        }
        k = unsafe { (*k).superclass };
    }
    false
}

// ---------- hash keys ----------

#[repr(transparent)]
pub struct HKey(pub V);

/// `hash` / `eql?` defined by the object's class (Ruby or native, e.g. Struct, Date),
/// as opposed to the identity defaults from Kernel/Object.
fn own_method(v: V, name: &str) -> Option<*const Method> {
    let m = find_method(class_of(v), sym(name));
    if m.is_null() {
        return None;
    }
    let owner = unsafe { (*m).owner };
    let c = &rt().c;
    if owner == c.kernel || owner == c.object || owner == c.basic_object {
        return None;
    }
    Some(m)
}

pub fn hash_of(v: V) -> u64 {
    hash_impl(v, true)
}
/// The default `hash` of `v` itself (Kernel#hash and the core classes' own), without asking
/// `v`'s `hash` method: a `def hash = ... super` would come back here forever.
pub fn hash_self(v: V) -> u64 {
    hash_impl(v, false)
}
fn hash_impl(v: V, dispatch: bool) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    match kind(v) {
        None => v.hash(&mut h),
        Some(Kind::String) => rstr(v).b.hash(&mut h),
        // same hash for 0.0 and -0.0, which are eql?
        Some(Kind::Float) => {
            let f = float_val(v);
            (if f == 0.0 { 0.0f64 } else { f }).to_bits().hash(&mut h)
        }
        Some(Kind::Bignum) => rbig(v).n.hash(&mut h),
        Some(Kind::Array) => {
            // By index, reading the Array again each time: an element's `hash` may run Ruby
            // code that changes it, or collects (when nothing else refers to `v`, a
            // reference into its buffer wouldn't keep it alive).
            let mut i = 0;
            while let Some(&x) = rary(v).v.as_slice().get(i) {
                hash_of(x).hash(&mut h);
                i += 1;
            }
        }
        Some(Kind::Range) => {
            hash_of(rrange(v).lo).hash(&mut h);
            hash_of(rrange(v).hi).hash(&mut h);
        }
        _ => {
            if let Some(m) = own_method(v, "hash").filter(|_| dispatch) {
                let r = invoke(m, v, &[], QNIL);
                if is_fix(r) {
                    return r;
                }
                if r == UNDEF {
                    pst().pending = Throw::None;
                }
            }
            if is_heap(v) {
                // hashed by address (a Hash key, say): the object must never move
                hdr(v).flags |= crate::gc::FL_ID;
            }
            v.hash(&mut h)
        }
    }
    h.finish()
}
pub fn eql(a: V, b: V) -> bool {
    if a == b {
        return true;
    }
    match (kind(a), kind(b)) {
        (Some(Kind::String), Some(Kind::String)) => rstr(a).b == rstr(b).b,
        (Some(Kind::Float), Some(Kind::Float)) => float_val(a) == float_val(b),
        (Some(Kind::Bignum), Some(Kind::Bignum)) => rbig(a).n == rbig(b).n,
        (Some(Kind::Array), Some(Kind::Array)) => {
            // by index, as in `hash_of`: an element's `eql?` may change or collect them
            let mut i = 0;
            loop {
                let (x, y) = (rary(a).v.as_slice(), rary(b).v.as_slice());
                if x.len() != y.len() {
                    return false;
                }
                let Some((&p, &q)) = x.get(i).zip(y.get(i)) else { return true };
                if !eql(p, q) {
                    return false;
                }
                i += 1;
            }
        }
        (Some(Kind::Range), Some(Kind::Range)) => {
            let (x, y) = (rrange(a), rrange(b));
            x.excl == y.excl && eql(x.lo, y.lo) && eql(x.hi, y.hi)
        }
        (Some(ka), Some(kb)) if ka == kb && !matches!(ka, Kind::String | Kind::Float | Kind::Bignum | Kind::Array | Kind::Range) => {
            match own_method(a, "eql?") {
                Some(m) => {
                    let r = invoke(m, a, &[b], QNIL);
                    if r == UNDEF {
                        pst().pending = Throw::None;
                        return false;
                    }
                    truthy(r)
                }
                None => false,
            }
        }
        _ => false,
    }
}
impl Hash for HKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(hash_of(self.0));
    }
}
impl PartialEq for HKey {
    fn eq(&self, o: &Self) -> bool {
        eql(self.0, o.0)
    }
}
impl Eq for HKey {}

/// Ruby `==` with fast paths for core types (CRuby's rb_equal: the same object is always
/// equal, even a NaN; `nan == nan` itself goes to Float#==).
pub fn equal(a: V, b: V) -> V {
    if a == b {
        return QTRUE;
    }
    match (kind(a), kind(b)) {
        (Some(Kind::Float), Some(Kind::Float)) => bool_v(float_val(a) == float_val(b)),
        (None, None) => QFALSE,
        (Some(Kind::String), Some(Kind::String)) => bool_v(rstr(a).b == rstr(b).b),
        _ => send(a, sym("=="), &[b], QNIL),
    }
}
