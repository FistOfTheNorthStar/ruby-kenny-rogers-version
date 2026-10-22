//! `CMap<K, V>`: the small maps inside a class (methods, constants, class-level ivars and
//! cvars, the ivar layout of instances).
//!
//! A sorted array of (key, value) in one allocation, looked up by binary search. Reads take
//! no locks. While one OS thread runs Ruby code it is updated in place; while several do,
//! adding or removing a key builds a new array and publishes it with one atomic store (so a
//! concurrent lookup never sees a half-shifted array), and the old one is retired until the
//! next GC. Changing the value of an existing key is a single atomic word store either way.

use crate::mt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::*};

/// Writers to class tables take this while several OS threads run (class changes are rare
/// after boot, so one lock for all of them is enough).
static WRITE: Mutex<()> = Mutex::new(());

/// The class-table write lock (None while only one OS thread runs Ruby code). Not reentrant:
/// don't call Ruby or another `CMap` writer while holding it.
pub fn write_lock() -> Option<std::sync::MutexGuard<'static, ()>> {
    if mt::multi() { Some(WRITE.lock().unwrap_or_else(|e| e.into_inner())) } else { None }
}

#[repr(C)]
struct Slab<K, V> {
    len: AtomicUsize,
    cap: usize,
    items: [(K, V); 0],
}

/// (`p` first: compiled code reads a class's ivar map inline, see core.rs `CLASS_CIVARS`.)
#[repr(C)]
pub struct CMap<K: Copy + Ord, V: Copy> {
    p: AtomicPtr<Slab<K, V>>,
    /// Generational GC: written since the last collection (and recorded).
    gc_rem: std::sync::atomic::AtomicU8,
}

unsafe impl<K: Copy + Ord, V: Copy> Sync for CMap<K, V> {}
unsafe impl<K: Copy + Ord, V: Copy> Send for CMap<K, V> {}

fn layout<K, V>(cap: usize) -> std::alloc::Layout {
    let size = std::mem::size_of::<Slab<K, V>>() + cap * std::mem::size_of::<(K, V)>();
    std::alloc::Layout::from_size_align(size, std::mem::align_of::<Slab<K, V>>().max(8)).unwrap()
}

impl<K: Copy + Ord, V: Copy> Default for CMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Ord, V: Copy> CMap<K, V> {
    pub const fn new() -> Self {
        CMap { p: AtomicPtr::new(std::ptr::null_mut()), gc_rem: std::sync::atomic::AtomicU8::new(0) }
    }
    fn slab_alloc(cap: usize) -> *mut Slab<K, V> {
        let l = layout::<K, V>(cap);
        let p = unsafe { std::alloc::alloc(l) } as *mut Slab<K, V>;
        if p.is_null() {
            std::alloc::handle_alloc_error(l);
        }
        unsafe {
            std::ptr::write(p, Slab { len: AtomicUsize::new(0), cap, items: [] });
        }
        p
    }
    fn retire(p: *mut Slab<K, V>) {
        if !p.is_null() {
            // deferred even with one thread: code iterating a class table may call Ruby,
            // which may change the table
            let l = layout::<K, V>(unsafe { (*p).cap });
            mt::retire_deferred(p as usize, l.size(), l.align());
        }
    }
    #[inline]
    fn items(&self) -> &[(K, V)] {
        let p = self.p.load(Acquire);
        if p.is_null() {
            return &[];
        }
        unsafe {
            let n = (*p).len.load(Acquire).min((*p).cap);
            std::slice::from_raw_parts((*p).items.as_ptr(), n)
        }
    }
    #[inline]
    fn find(&self, k: &K) -> Result<usize, usize> {
        self.items().binary_search_by(|e| e.0.cmp(k))
    }

    pub fn len(&self) -> usize {
        self.items().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    pub fn get(&self, k: &K) -> Option<&V> {
        let it = self.items();
        it.binary_search_by(|e| e.0.cmp(k)).ok().map(|i| &it[i].1)
    }
    /// Position of `k` (a hint for `get_at`, valid until keys are added or removed).
    pub fn index_of(&self, k: &K) -> Option<usize> {
        self.find(k).ok()
    }
    /// The value at position `i` if the key there is `k`.
    #[inline]
    pub fn get_at(&self, i: usize, k: &K) -> Option<V> {
        self.items().get(i).filter(|e| e.0 == *k).map(|e| e.1)
    }
    pub fn contains_key(&self, k: &K) -> bool {
        self.find(k).is_ok()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.items().iter().map(|(k, v)| (k, v))
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.items().iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.items().iter().map(|(_, v)| v)
    }
    /// A copy (shadows `Clone::clone`).
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> CMap<K, V> {
        let m = CMap::new();
        let it = self.items();
        if !it.is_empty() {
            let p = Self::slab_alloc(it.len());
            unsafe {
                std::ptr::copy_nonoverlapping(it.as_ptr(), (*p).items.as_mut_ptr(), it.len());
                (*p).len.store(it.len(), Release);
            }
            m.p.store(p, Release);
        }
        m
    }

    fn write_lock() -> Option<std::sync::MutexGuard<'static, ()>> {
        write_lock()
    }

    /// Write barrier: tables of values (constants, class ivars/cvars, generic ivars) and
    /// method tables are rescanned by the next minor collection after a write, so old
    /// classes need not be traced every time.
    #[inline]
    fn note_write(&self)
    where
        V: 'static,
    {
        let kind = if std::any::TypeId::of::<V>() == std::any::TypeId::of::<u64>() {
            crate::gc::REM_CMAP_VALUES
        } else if std::any::TypeId::of::<V>() == std::any::TypeId::of::<*const crate::core::Method>() {
            crate::gc::REM_CMAP_METHODS
        } else {
            return;
        };
        if self.gc_rem.load(Relaxed) == 0 && self.gc_rem.swap(1, Relaxed) == 0 {
            crate::gc::remember(kind, self as *const Self as usize);
        }
    }
    /// For the GC: processed the remembered entry.
    pub fn clear_remembered(&self) {
        self.gc_rem.store(0, Relaxed);
    }
    pub fn insert(&mut self, k: K, v: V) -> Option<V>
    where
        V: 'static,
    {
        self.note_write();
        // changing the value of an existing key needs no lock (one word-sized store)
        let it = self.items();
        if let Ok(i) = it.binary_search_by(|e| e.0.cmp(&k)) {
            let old = it[i].1;
            let base = it.as_ptr() as *mut (K, V);
            unsafe { std::ptr::addr_of_mut!((*base.add(i)).1).write_volatile(v) };
            return Some(old);
        }
        let _g = Self::write_lock();
        self.insert_locked(k, v)
    }
    /// `insert` for a caller already holding `write_lock()`.
    /// `insert` without the class-table barrier (for tables whose owner has its own barrier).
    pub fn insert_untracked(&mut self, k: K, v: V) -> Option<V> {
        let it = self.items();
        if let Ok(i) = it.binary_search_by(|e| e.0.cmp(&k)) {
            let old = it[i].1;
            let base = it.as_ptr() as *mut (K, V);
            unsafe { std::ptr::addr_of_mut!((*base.add(i)).1).write_volatile(v) };
            return Some(old);
        }
        let _g = Self::write_lock();
        self.insert_locked_inner(k, v)
    }
    pub fn insert_locked(&mut self, k: K, v: V) -> Option<V>
    where
        V: 'static,
    {
        self.note_write();
        self.insert_locked_inner(k, v)
    }
    fn insert_locked_inner(&mut self, k: K, v: V) -> Option<V> {
        let p = self.p.load(Relaxed);
        let (n, cap) = if p.is_null() { (0, 0) } else { unsafe { ((*p).len.load(Relaxed), (*p).cap) } };
        // (never null: a null pointer, even with length 0, would be UB in from_raw_parts)
        let items = if p.is_null() { std::ptr::NonNull::<(K, V)>::dangling().as_ptr() } else { unsafe { (*p).items.as_mut_ptr() } };
        let pos = unsafe { std::slice::from_raw_parts(items as *const (K, V), n) }.binary_search_by(|e| e.0.cmp(&k));
        match pos {
            Ok(i) => unsafe {
                // replace the value in place: one word-sized store
                let slot = &mut (*items.add(i)).1;
                let old = *slot;
                std::ptr::write_volatile(slot, v);
                Some(old)
            },
            Err(i) => {
                if !mt::multi() && n < cap {
                    unsafe {
                        std::ptr::copy(items.add(i), items.add(i + 1), n - i);
                        items.add(i).write((k, v));
                        (*p).len.store(n + 1, Release);
                    }
                } else {
                    let ncap = if n < cap { cap } else { (cap * 2).max(4) };
                    let np = Self::slab_alloc(ncap);
                    unsafe {
                        let dst = (*np).items.as_mut_ptr();
                        std::ptr::copy_nonoverlapping(items, dst, i);
                        dst.add(i).write((k, v));
                        std::ptr::copy_nonoverlapping(items.add(i), dst.add(i + 1), n - i);
                        (*np).len.store(n + 1, Release);
                    }
                    self.p.store(np, Release);
                    Self::retire(p);
                }
                None
            }
        }
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let _g = Self::write_lock();
        let p = self.p.load(Relaxed);
        if p.is_null() {
            return None;
        }
        let (n, cap) = unsafe { ((*p).len.load(Relaxed), (*p).cap) };
        let items = unsafe { (*p).items.as_mut_ptr() };
        let i = unsafe { std::slice::from_raw_parts(items as *const (K, V), n) }.binary_search_by(|e| e.0.cmp(k)).ok()?;
        let old = unsafe { (*items.add(i)).1 };
        if !mt::multi() {
            unsafe {
                std::ptr::copy(items.add(i + 1), items.add(i), n - i - 1);
                (*p).len.store(n - 1, Release);
            }
        } else {
            let np = Self::slab_alloc(cap);
            unsafe {
                let dst = (*np).items.as_mut_ptr();
                std::ptr::copy_nonoverlapping(items, dst, i);
                std::ptr::copy_nonoverlapping(items.add(i + 1), dst.add(i), n - i - 1);
                (*np).len.store(n - 1, Release);
            }
            self.p.store(np, Release);
            Self::retire(p);
        }
        Some(old)
    }
    pub fn clear(&mut self) {
        let _g = Self::write_lock();
        let p = self.p.swap(std::ptr::null_mut(), AcqRel);
        Self::retire(p);
    }
}

impl<K: Copy + Ord, V: Copy> Drop for CMap<K, V> {
    fn drop(&mut self) {
        Self::retire(*self.p.get_mut());
    }
}

impl<K: Copy + Ord + std::fmt::Debug, V: Copy + std::fmt::Debug> std::fmt::Debug for CMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.items().iter().map(|(k, v)| (k, v))).finish()
    }
}
