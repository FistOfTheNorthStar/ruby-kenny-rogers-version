//! Runtime compilation of Ruby source strings (`eval`, `class_eval`, `instance_eval`)
//! with the same code generator, emitting into executable memory.

use crate::*;
use cranelift_jit::{BranchProtection, JITBuilder, JITMemoryKind, JITMemoryProvider, JITModule};
use std::sync::{Mutex, OnceLock};

// ---------- shared JIT memory ----------
//
// Every eval unit used to get its own code, read-only and writable pages (at least 3 pages
// of 16 KB on Apple Silicon, ~48 KB per `class_eval` string). All units now bump-allocate
// from shared chunks. Other OS threads may be running older code in a chunk while a new
// unit is written into it, so a page holding finished code is never made non-executable:
// on Apple Silicon code chunks are MAP_JIT and only the compiling thread switches to write
// mode (pthread_jit_write_protect_np); elsewhere a new unit starts on a fresh page.
// Compilation itself is serialised by the caller. A unit's memory is returned with
// `free_unit` once the runtime knows nothing can run it any more (see the eval cache in
// kenny-runtime-jit); freed ranges are reused by later units (code ranges only where pages
// stay writable for the compiling thread, i.e. MAP_JIT on Apple Silicon).

const CHUNK: usize = 256 << 10;

struct Pool {
    base: usize,
    len: usize,
    pos: usize,
    /// (non-MAP_JIT code pools) everything below this is finished, read+execute code
    sealed: usize,
    /// Freed ranges (start, len), sorted by address and coalesced.
    free: Vec<(usize, usize)>,
}

const MAC_JIT: bool = cfg!(all(target_arch = "aarch64", target_os = "macos"));

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
unsafe extern "C" {
    fn pthread_jit_write_protect_np(enabled: libc::c_int);
}
fn jit_write_mode(on: bool) {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    unsafe {
        pthread_jit_write_protect_np(if on { 0 } else { 1 });
    }
    let _ = on;
}
fn map_code(len: usize) -> std::io::Result<usize> {
    if !MAC_JIT {
        return map_rw(len);
    }
    #[cfg(target_os = "macos")]
    let flags = libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_JIT;
    #[cfg(not(target_os = "macos"))]
    let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
    let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC, flags, -1, 0) };
    if p == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    Ok(p as usize)
}

struct SharedMemory {
    code: Pool,
    data: Pool,
    /// code written since the last finalize: (start, len)
    dirty: Vec<(usize, usize)>,
    /// everything allocated for the unit being compiled: (start, len, is_code)
    unit: Vec<(usize, usize, bool)>,
}

static SHARED: Mutex<Option<SharedMemory>> = Mutex::new(None);

fn page_size() -> usize {
    static P: OnceLock<usize> = OnceLock::new();
    *P.get_or_init(|| unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize })
}

fn map_rw(len: usize) -> std::io::Result<usize> {
    let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
    if p == libc::MAP_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    Ok(p as usize)
}

fn protect(start: usize, len: usize, prot: i32) -> std::io::Result<()> {
    let ps = page_size();
    let lo = start & !(ps - 1);
    let hi = (start + len + ps - 1) & !(ps - 1);
    if unsafe { libc::mprotect(lo as *mut libc::c_void, hi - lo, prot) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn flush_icache(start: usize, len: usize) {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    unsafe {
        unsafe extern "C" {
            fn sys_icache_invalidate(start: *mut libc::c_void, len: usize);
        }
        sys_icache_invalidate(start as *mut libc::c_void, len);
    }
    #[cfg(all(target_arch = "aarch64", not(target_os = "macos")))]
    unsafe {
        unsafe extern "C" {
            fn __clear_cache(start: *mut libc::c_char, end: *mut libc::c_char);
        }
        __clear_cache(start as *mut libc::c_char, (start + len) as *mut libc::c_char);
    }
    let _ = (start, len);
}

impl Pool {
    const fn new() -> Pool {
        Pool { base: 0, len: 0, pos: 0, sealed: 0, free: Vec::new() }
    }
    /// First fit from the freed ranges.
    fn alloc_freed(&mut self, size: usize, align: usize) -> Option<usize> {
        for i in 0..self.free.len() {
            let (start, len) = self.free[i];
            let at = (start + align - 1) & !(align - 1);
            if at + size <= start + len {
                let (head, tail) = (at - start, start + len - (at + size));
                self.free.remove(i);
                if tail > 0 {
                    self.free.insert(i, (at + size, tail));
                }
                if head > 0 {
                    self.free.insert(i, (start, head));
                }
                return Some(at);
            }
        }
        None
    }
    fn release(&mut self, start: usize, len: usize) {
        let i = self.free.partition_point(|r| r.0 < start);
        self.free.insert(i, (start, len));
        // coalesce with the neighbours
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }
    fn alloc(&mut self, size: usize, align: usize, code: bool) -> std::io::Result<(usize, bool)> {
        if !code || MAC_JIT {
            if let Some(at) = self.alloc_freed(size, align) {
                return Ok((at, false));
            }
        }
        let mut at = (self.base + self.pos + align - 1) & !(align - 1);
        if code && !MAC_JIT {
            at = at.max(self.sealed);
        }
        let mut fresh = false;
        if self.base == 0 || at + size > self.base + self.len {
            let len = CHUNK.max((size + page_size() - 1) & !(page_size() - 1));
            self.base = if code { map_code(len)? } else { map_rw(len)? };
            if std::env::var_os("KENNY_DEBUG_EVAL").is_some() {
                eprintln!("[eval] new {} chunk of {} KB (free ranges: {}, {} KB)", if code { "code" } else { "data" }, len >> 10, self.free.len(), self.free.iter().map(|r| r.1).sum::<usize>() >> 10);
            }
            self.len = len;
            self.pos = 0;
            at = self.base;
            fresh = true;
        }
        self.pos = at + size - self.base;
        Ok((at, fresh))
    }
}

/// Handle given to each JITModule; all state lives in `SHARED`.
struct SharedJitMemory;

impl JITMemoryProvider for SharedJitMemory {
    fn allocate(&mut self, size: usize, align: u64, kind: JITMemoryKind) -> std::io::Result<*mut u8> {
        let mut g = SHARED.lock().unwrap();
        let s = g.get_or_insert_with(|| SharedMemory { code: Pool::new(), data: Pool::new(), dirty: Vec::new(), unit: Vec::new() });
        let align = (align as usize).max(16);
        // whole 16-byte granules, so freed neighbours coalesce (no padding left between them)
        let size = (size.max(1) + 15) & !15;
        match kind {
            JITMemoryKind::Executable => {
                let (p, _fresh) = s.code.alloc(size, align, true)?;
                if MAC_JIT {
                    // this thread may write MAP_JIT pages now; others keep executing them
                    jit_write_mode(true);
                }
                s.dirty.push((p, size));
                s.unit.push((p, size, true));
                Ok(p as *mut u8)
            }
            // read-only data stays writable: it is only ever written by the JIT itself
            JITMemoryKind::Writable | JITMemoryKind::ReadOnly => {
                let p = s.data.alloc(size, align, false)?.0;
                s.unit.push((p, size, false));
                Ok(p as *mut u8)
            }
        }
    }

    unsafe fn free_memory(&mut self) {}

    fn finalize(&mut self, _branch_protection: BranchProtection) -> cranelift_module::ModuleResult<()> {
        let mut g = SHARED.lock().unwrap();
        if let Some(s) = g.as_mut() {
            for (p, len) in std::mem::take(&mut s.dirty) {
                flush_icache(p, len);
                if !MAC_JIT {
                    protect(p, len, libc::PROT_READ | libc::PROT_EXEC).map_err(|e| cranelift_module::ModuleError::Backend(e.into()))?;
                    // never write to these pages again
                    let ps = page_size();
                    s.code.sealed = s.code.sealed.max((p + len + ps - 1) & !(ps - 1));
                }
            }
            if MAC_JIT {
                jit_write_mode(false);
            }
        }
        Ok(())
    }
}

/// Runtime symbols, resolved by name when a unit references them.
static SYMBOLS: OnceLock<Vec<(&'static str, usize)>> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EvalMode {
    /// Kernel#eval / instance_eval: top-level-like scope with the given self.
    Eval,
    /// Module#class_eval: a class body with self = the module.
    ClassEval,
    /// `load` of a source file at runtime: a top-level file body named `file`.
    File,
}

pub struct JitUnit {
    pub entry: *const u8,
    pub caches: *mut u8,
    pub cache_words: usize,
    pub syms_ptr: *mut u8,
    pub syms: Vec<Vec<u8>>,
    /// backtrace info: (function address, code size, label, file, [(return offset, line)]);
    /// label and file are offsets into `strs`
    pub debug: Vec<(usize, u32, u32, u32, Vec<(u32, u32)>)>,
    pub strs: *const u8,
    /// The unit's memory: (start, len, is_code); see `free_unit`.
    pub memory: Vec<(usize, usize, bool)>,
    /// Cache slots holding runtime-allocated entries: (word offset into `caches`, kind).
    pub owned_caches: Vec<(u32, u8)>,
}

/// Give a unit's memory back for reuse. Only when no thread can be running its code or
/// reach its data (the caller's job), and with compilation serialised like `compile`.
pub fn free_unit(memory: &[(usize, usize, bool)]) {
    let mut g = SHARED.lock().unwrap();
    let Some(s) = g.as_mut() else { return };
    for &(p, len, code) in memory {
        if code {
            // (outside MAP_JIT the pages are read+execute for good: never reused)
            if MAC_JIT {
                s.code.release(p, len);
            }
        } else {
            unsafe { std::ptr::write_bytes(p as *mut u8, 0, len) };
            s.data.release(p, len);
        }
    }
}

/// Compile `src` into a native function `fn(self, argc, argv, blk) -> V`.
/// `symbols` maps runtime function / data names to their addresses.
pub fn compile(src: &[u8], mode: EvalMode, file: &str, symbols: &[(&'static str, *const u8)]) -> Result<JitUnit, String> {
    let table = SYMBOLS.get_or_init(|| symbols.iter().map(|(n, p)| (*n, *p as usize)).collect());
    let mut b = JITBuilder::with_isa(host_isa(false)?, cranelift_module::default_libcall_names());
    b.symbol_lookup_fn(Box::new(move |name| table.iter().find(|(n, _)| *n == name).map(|(_, p)| *p as *const u8)));
    b.memory_provider(Box::new(SharedJitMemory));
    let module = JITModule::new(b);
    let path = Path::new(file);
    if let Some(s) = SHARED.lock().unwrap().as_mut() {
        s.unit.clear();
    }
    let mut c = Compiler::with_module(module, path, src)?;
    c.eval_unit = true;
    c.file_unit = mode == EvalMode::File;
    let kind = match mode {
        EvalMode::Eval | EvalMode::File => ScopeKind::Top,
        EvalMode::ClassEval => ScopeKind::Class,
    };
    let fid = c.compile_unit(src, path, kind);
    if !c.errors.is_empty() {
        return Err(c.errors.join("\n"));
    }
    let fid = fid?;
    let e = |x: cranelift_module::ModuleError| x.to_string();
    let mut d = DataDescription::new();
    d.define_zeroinit(c.cache_words.max(1) * 8);
    d.set_align(8);
    c.module.define_data(c.caches_id, &d).map_err(e)?;
    let mut d = DataDescription::new();
    d.define_zeroinit(c.syms.len().max(1) * 8);
    d.set_align(8);
    c.module.define_data(c.syms_id, &d).map_err(e)?;
    let mut strs = std::mem::take(&mut c.strs);
    strs.push(0);
    let mut d = DataDescription::new();
    d.define(strs.into_boxed_slice());
    c.module.define_data(c.strs_id, &d).map_err(e)?;
    c.module.finalize_definitions().map_err(e)?;
    let entry = c.module.get_finalized_function(fid);
    let (caches, _) = c.module.get_finalized_data(c.caches_id);
    let (syms_ptr, _) = c.module.get_finalized_data(c.syms_id);
    let (strs_ptr, _) = c.module.get_finalized_data(c.strs_id);
    let debug = std::mem::take(&mut c.debug_fns)
        .into_iter()
        .map(|f| (c.module.get_finalized_function(f.id) as usize, f.size, f.label, f.file, f.calls))
        .collect();
    let memory = SHARED.lock().unwrap().as_mut().map(|s| std::mem::take(&mut s.unit)).unwrap_or_default();
    let unit = JitUnit {
        owned_caches: std::mem::take(&mut c.owned_caches),
        memory,
        debug,
        strs: strs_ptr,
        entry,
        caches: caches as *mut u8,
        cache_words: c.cache_words.max(1),
        syms_ptr: syms_ptr as *mut u8,
        syms: c.syms.keys().cloned().collect(),
    };
    // Only the code and data (in the shared JIT memory, never freed) outlive this call:
    // the compiler state, declarations and symbol tables are dropped here.
    drop(c);
    Ok(unit)
}

/// Names of everything compiled code may reference (for building the symbol table).
pub fn required_symbols() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = RUNTIME.iter().map(|(n, _, _)| *n).filter(|n| !n.starts_with("kn_start")).collect();
    v.push("KN_GC_PENDING");
    v.push("KN_DIRECT");
    v
}
