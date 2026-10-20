//! `KMap`: the insertion-ordered table behind `Hash`.
//!
//! Like `SVec` it stays memory-safe when several OS threads use one hash at once (a race can
//! lose an update, never crash):
//!
//! * One allocation holds a table: entries in insertion order, then (above 8 entries) an
//!   open-addressing index. Its capacity never changes; growing builds a new table, publishes
//!   it with one atomic store and retires the old one (freed at the next GC unless a stack still
//!   points into it — always deferred, because a key's Ruby `eql?`/`hash` may run in the middle
//!   of a lookup and change the hash).
//! * Readers take no locks. Writers lock only while several OS threads run Ruby code, and never
//!   call Ruby while holding the lock: the key is hashed and looked up first, then the writer
//!   locks, checks that the table did not change meanwhile, and writes (or retries).
//! * Deleting marks the entry; tables are compacted when they grow.

use crate::core::{HKey, V, eql, hash_of};
use crate::mt;
use std::sync::atomic::{AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};

/// Key of a deleted entry (never a valid Ruby value).
const DELETED: V = 0x44;
/// Tables up to this many entries have no index (a linear scan is faster).
const SMALL: usize = 8;
const HDR: usize = 32;

#[repr(C)]
struct Table {
    /// Entries used (live and deleted); only grows within a table.
    n: AtomicUsize,
    cap: usize,
    /// Index slots - 1, or 0 when the table has no index.
    mask: usize,
    /// Every entry before this one is deleted (makes repeated `Hash#shift` cheap).
    head: AtomicUsize,
}

#[repr(C)]
struct Entry {
    hash: AtomicU64,
    key: AtomicU64,
    val: AtomicU64,
}

impl Table {
    fn size_for(cap: usize) -> (usize, usize) {
        let mask = if cap <= SMALL { 0 } else { (cap * 2).next_power_of_two() - 1 };
        let idx = if mask == 0 { 0 } else { mask + 1 };
        (mask, HDR + cap * std::mem::size_of::<Entry>() + idx * 4)
    }
    fn alloc(cap: usize) -> *mut Table {
        let (mask, bytes) = Self::size_for(cap);
        let p = unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align_unchecked(bytes, 16)) } as *mut Table;
        if p.is_null() {
            std::alloc::handle_alloc_error(std::alloc::Layout::from_size_align(bytes, 16).unwrap());
        }
        unsafe {
            (*p).cap = cap;
            (*p).mask = mask;
        }
        p
    }
    fn bytes(&self) -> usize {
        Self::size_for(self.cap).1
    }
    #[inline]
    fn entries(&self) -> *const Entry {
        unsafe { (self as *const Table as *const u8).add(HDR) as *const Entry }
    }
    #[inline]
    fn entry(&self, i: usize) -> &Entry {
        debug_assert!(i < self.cap);
        unsafe { &*self.entries().add(i) }
    }
    #[inline]
    fn index(&self) -> *const AtomicU32 {
        unsafe { (self.entries().add(self.cap)) as *const AtomicU32 }
    }
    #[inline]
    fn used(&self) -> usize {
        self.n.load(Acquire).min(self.cap)
    }
    #[inline]
    fn slot0(&self, h: u64) -> usize {
        let x = (h ^ (h >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (x >> 32) as usize & self.mask
    }
    /// Entry index of `key`, or None. May call Ruby (`eql?`).
    fn find(&self, h: u64, key: V) -> Option<usize> {
        let n = self.used();
        if self.mask == 0 {
            for i in 0..n {
                let e = self.entry(i);
                if e.hash.load(Relaxed) == h {
                    let k = e.key.load(Relaxed);
                    if k != DELETED && (k == key || eql(k, key)) {
                        return Some(i);
                    }
                }
            }
            return None;
        }
        let mut s = self.slot0(h);
        for _ in 0..=self.mask {
            let ix = unsafe { (*self.index().add(s)).load(Acquire) } as usize;
            if ix == 0 {
                return None;
            }
            let i = ix - 1;
            if i < self.cap {
                let e = self.entry(i);
                if e.hash.load(Relaxed) == h {
                    let k = e.key.load(Relaxed);
                    if k != DELETED && (k == key || eql(k, key)) {
                        return Some(i);
                    }
                }
            }
            s = (s + 1) & self.mask;
        }
        None
    }
    /// Append an entry (caller checked there is room). Writer only.
    fn push(&self, h: u64, key: V, val: V) {
        let i = self.n.load(Relaxed);
        let e = self.entry(i);
        e.hash.store(h, Relaxed);
        e.key.store(key, Relaxed);
        e.val.store(val, Relaxed);
        if self.mask != 0 {
            let mut s = self.slot0(h);
            loop {
                let slot = unsafe { &*self.index().add(s) };
                if slot.load(Relaxed) == 0 {
                    slot.store(i as u32 + 1, Release);
                    break;
                }
                s = (s + 1) & self.mask;
            }
        }
        self.n.store(i + 1, Release);
    }
}

#[repr(C)]
pub struct KMap {
    tab: AtomicPtr<Table>,
    live: AtomicUsize,
    lock: AtomicU8,
    /// Generational GC: OLD (the owning hash is old), REM (in the remembered set).
    gc_gen: AtomicU8,
}
const G_OLD: u8 = 1;
const G_REM: u8 = 2;

unsafe impl Sync for KMap {}
unsafe impl Send for KMap {}

fn retire_table(t: *mut Table) {
    if !t.is_null() {
        let bytes = unsafe { (*t).bytes() };
        mt::retire_deferred(t as usize, bytes, 16);
    }
}

impl Default for KMap {
    fn default() -> Self {
        Self::new()
    }
}

impl KMap {
    pub const fn new() -> Self {
        KMap { tab: AtomicPtr::new(std::ptr::null_mut()), live: AtomicUsize::new(0), lock: AtomicU8::new(0), gc_gen: AtomicU8::new(0) }
    }
    #[inline]
    fn table(&self) -> Option<&Table> {
        let t = self.tab.load(Acquire);
        if t.is_null() { None } else { Some(unsafe { &*t }) }
    }
    fn lock(&self) {
        if !mt::multi() {
            return;
        }
        let mut spins = 0u32;
        while self.lock.compare_exchange_weak(0, 1, Acquire, Relaxed).is_err() {
            spins += 1;
            if spins < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
    }
    fn unlock(&self) {
        if self.lock.load(Relaxed) != 0 {
            self.lock.store(0, Release);
        }
    }

    // ----- generational GC -----

    /// Write barrier: an old hash that gets new entries joins the remembered set.
    #[inline(always)]
    fn wb(&self) {
        if self.gc_gen.load(Relaxed) == G_OLD {
            self.remember();
        }
    }
    #[inline(never)]
    fn remember(&self) {
        if self.gc_gen.fetch_or(G_REM, Relaxed) & G_REM == 0 {
            crate::gc::remember(crate::gc::REM_KMAP, self as *const KMap as usize);
        }
    }
    /// (GC, every other thread stopped) Rewrite every key and value with `f` (objects the GC
    /// moved). Hashes stay valid: a key hashed by identity is never moved (gc::FL_ID).
    pub fn remap(&self, f: impl Fn(V) -> V) {
        let Some(t) = self.table() else { return };
        for i in 0..t.used() {
            let e = t.entry(i);
            let k = e.key.load(Relaxed);
            if k != DELETED {
                e.key.store(f(k), Relaxed);
                e.val.store(f(e.val.load(Relaxed)), Relaxed);
            }
        }
    }
    /// (debugging) the generational bits: 1 = old, 2 = remembered
    pub fn gen_bits(&self) -> u8 {
        self.gc_gen.load(Relaxed)
    }
    pub fn set_old(&self) {
        if self.gc_gen.load(Relaxed) & G_OLD == 0 {
            self.gc_gen.fetch_or(G_OLD, Relaxed);
        }
    }
    pub fn clear_remembered(&self) {
        self.gc_gen.fetch_and(!G_REM, Relaxed);
    }

    // ----- reading -----

    pub fn len(&self) -> usize {
        self.live.load(Relaxed)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, k: &HKey) -> Option<&V> {
        let t = self.table()?;
        let i = t.find(hash_of(k.0), k.0)?;
        Some(unsafe { &*(t.entry(i).val.as_ptr() as *const V) })
    }
    pub fn get_key_value(&self, k: &HKey) -> Option<(&HKey, &V)> {
        let t = self.table()?;
        let i = t.find(hash_of(k.0), k.0)?;
        let e = t.entry(i);
        Some(unsafe { (&*(e.key.as_ptr() as *const HKey), &*(e.val.as_ptr() as *const V)) })
    }
    pub fn contains_key(&self, k: &HKey) -> bool {
        self.get(k).is_some()
    }
    /// The i-th live entry in insertion order.
    pub fn get_index(&self, i: usize) -> Option<(&HKey, &V)> {
        let t = self.table()?;
        let n = t.used();
        if self.len() == n {
            if i >= n {
                return None;
            }
            let e = t.entry(i);
            let k = e.key.load(Relaxed);
            if k != DELETED {
                return Some(unsafe { (&*(e.key.as_ptr() as *const HKey), &*(e.val.as_ptr() as *const V)) });
            }
        }
        self.iter().nth(i)
    }
    pub fn iter(&self) -> Iter<'_> {
        let t = self.tab.load(Acquire);
        let (i, n) = if t.is_null() { (0, 0) } else { unsafe { ((*t).head.load(Relaxed), (*t).used()) } };
        Iter { t, i, n, _m: std::marker::PhantomData }
    }
    pub fn keys(&self) -> impl Iterator<Item = &HKey> {
        self.iter().map(|(k, _)| k)
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
    /// A copy (shadows `Clone::clone` like the old IndexMap field did).
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> KMap {
        let m = KMap::new();
        let pairs: Vec<(u64, V, V)> = match self.table() {
            Some(t) => (0..t.used())
                .filter_map(|i| {
                    let e = t.entry(i);
                    let k = e.key.load(Relaxed);
                    (k != DELETED).then(|| (e.hash.load(Relaxed), k, e.val.load(Relaxed)))
                })
                .collect(),
            None => Vec::new(),
        };
        if !pairs.is_empty() {
            let nt = Table::alloc(pairs.len().max(4));
            for &(h, k, v) in &pairs {
                unsafe { (*nt).push(h, k, v) };
            }
            m.tab.store(nt, Release);
            m.live.store(pairs.len(), Relaxed);
        }
        m
    }

    // ----- writing -----

    /// Build a new table holding the live entries plus room to grow, publish it, retire the
    /// old one. Caller holds the lock.
    fn rebuild(&self, old: *mut Table, extra: Option<(u64, V, V)>) -> *mut Table {
        let mut pairs: Vec<(u64, V, V)> = Vec::new();
        if !old.is_null() {
            let t = unsafe { &*old };
            for i in 0..t.used() {
                let e = t.entry(i);
                let k = e.key.load(Relaxed);
                if k != DELETED {
                    pairs.push((e.hash.load(Relaxed), k, e.val.load(Relaxed)));
                }
            }
        }
        if let Some(x) = extra {
            pairs.push(x);
        }
        let cap = (pairs.len() * 2).max(4);
        let nt = Table::alloc(cap);
        for &(h, k, v) in &pairs {
            unsafe { (*nt).push(h, k, v) };
        }
        self.live.store(pairs.len(), Relaxed);
        self.tab.store(nt, Release);
        retire_table(old);
        nt
    }

    pub fn insert(&mut self, k: HKey, v: V) -> Option<V> {
        self.wb();
        let key = k.0;
        let h = hash_of(key);
        loop {
            let t = self.tab.load(Acquire);
            let n0 = if t.is_null() { 0 } else { unsafe { (*t).n.load(Acquire) } };
            let found = if t.is_null() { None } else { unsafe { (*t).find(h, key) } };
            self.lock();
            if self.tab.load(Relaxed) != t || (!t.is_null() && unsafe { (*t).n.load(Relaxed) } != n0) {
                self.unlock();
                continue;
            }
            match found {
                Some(i) => {
                    let e = unsafe { (*t).entry(i) };
                    if e.key.load(Relaxed) == DELETED {
                        self.unlock();
                        continue;
                    }
                    let old = e.val.swap(v, AcqRel);
                    self.unlock();
                    return Some(old);
                }
                None => {
                    if !t.is_null() && n0 < unsafe { (*t).cap } {
                        unsafe { (*t).push(h, key, v) };
                        self.live.fetch_add(1, Relaxed);
                    } else {
                        self.rebuild(t, Some((h, key, v)));
                    }
                    self.unlock();
                    return None;
                }
            }
        }
    }

    pub fn shift_remove(&mut self, k: &HKey) -> Option<V> {
        let key = k.0;
        let h = hash_of(key);
        loop {
            let t = self.tab.load(Acquire);
            if t.is_null() {
                return None;
            }
            let i = unsafe { (*t).find(h, key) }?;
            self.lock();
            if self.tab.load(Relaxed) != t {
                self.unlock();
                continue;
            }
            let r = self.remove_at(unsafe { &*t }, i);
            self.unlock();
            match r {
                Some((_, v)) => return Some(v),
                None => continue,
            }
        }
    }
    /// Mark entry i deleted (lock held). None if it already was.
    fn remove_at(&self, t: &Table, i: usize) -> Option<(V, V)> {
        let e = t.entry(i);
        let k = e.key.load(Relaxed);
        if k == DELETED {
            return None;
        }
        let v = e.val.load(Relaxed);
        e.key.store(DELETED, Release);
        self.live.fetch_sub(1, Relaxed);
        let mut h = t.head.load(Relaxed);
        if h == i {
            let n = t.used();
            while h < n && t.entry(h).key.load(Relaxed) == DELETED {
                h += 1;
            }
            t.head.store(h, Relaxed);
        }
        Some((k, v))
    }
    /// Remove the i-th live entry.
    pub fn shift_remove_index(&mut self, idx: usize) -> Option<(HKey, V)> {
        self.lock();
        let r = self.table().and_then(|t| {
            let mut seen = 0;
            for i in t.head.load(Relaxed)..t.used() {
                if t.entry(i).key.load(Relaxed) != DELETED {
                    if seen == idx {
                        return self.remove_at(t, i);
                    }
                    seen += 1;
                }
            }
            None
        });
        self.unlock();
        r.map(|(k, v)| (HKey(k), v))
    }
    pub fn clear(&mut self) {
        self.lock();
        let old = self.tab.swap(std::ptr::null_mut(), AcqRel);
        self.live.store(0, Relaxed);
        retire_table(old);
        self.unlock();
    }
    /// Keep the entries `f` returns true for (`f` must not call Ruby).
    pub fn retain<F: FnMut(&HKey, &mut V) -> bool>(&mut self, mut f: F) {
        self.wb();
        self.lock();
        if let Some(t) = self.table() {
            for i in 0..t.used() {
                let e = t.entry(i);
                let k = e.key.load(Relaxed);
                if k == DELETED {
                    continue;
                }
                let mut v = e.val.load(Relaxed);
                if f(&HKey(k), &mut v) {
                    e.val.store(v, Relaxed);
                } else {
                    e.key.store(DELETED, Release);
                    self.live.fetch_sub(1, Relaxed);
                }
            }
        }
        self.unlock();
    }
    /// Replace the contents with `other`'s (taking its table).
    pub fn assign(&mut self, other: KMap) {
        self.wb();
        let nt = other.tab.swap(std::ptr::null_mut(), AcqRel);
        let live = other.live.load(Relaxed);
        self.lock();
        let old = self.tab.swap(nt, AcqRel);
        self.live.store(live, Relaxed);
        retire_table(old);
        self.unlock();
    }
    pub fn entry(&mut self, k: HKey) -> KEntry<'_> {
        KEntry { m: self, k }
    }
    /// Pointer to the value slot of `k` (which must be present).
    fn slot_of(&self, k: V) -> Option<&mut V> {
        self.wb();
        let t = self.table()?;
        let i = t.find(hash_of(k), k)?;
        Some(unsafe { &mut *(t.entry(i).val.as_ptr() as *mut V) })
    }
}

impl Drop for KMap {
    fn drop(&mut self) {
        let t = *self.tab.get_mut();
        if !t.is_null() {
            // a dead hash (GC sweep) or a temporary map: nobody else can be reading it
            let bytes = unsafe { (*t).bytes() };
            mt::free_buf(t as usize, bytes, 16);
        }
    }
}

pub struct KEntry<'a> {
    m: &'a mut KMap,
    k: HKey,
}
impl<'a> KEntry<'a> {
    pub fn or_insert(self, v: V) -> &'a mut V {
        self.or_insert_with(|| v)
    }
    pub fn or_insert_with<F: FnOnce() -> V>(self, f: F) -> &'a mut V {
        let k = self.k.0;
        if self.m.get(&HKey(k)).is_none() {
            let v = f();
            self.m.insert(HKey(k), v);
        }
        let m: &'a KMap = self.m;
        m.slot_of(k).expect("entry just inserted")
    }
}

pub struct Iter<'a> {
    t: *mut Table,
    i: usize,
    n: usize,
    _m: std::marker::PhantomData<&'a KMap>,
}
impl<'a> Iterator for Iter<'a> {
    type Item = (&'a HKey, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        while self.i < self.n {
            let e = unsafe { (*self.t).entry(self.i) };
            self.i += 1;
            if e.key.load(Relaxed) != DELETED {
                return Some(unsafe { (&*(e.key.as_ptr() as *const HKey), &*(e.val.as_ptr() as *const V)) });
            }
        }
        None
    }
}
