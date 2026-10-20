//! `SVec<T>`: the growable buffer behind `Array`, `String` and instance variables.
//!
//! It behaves like `Vec<T>` (same method names, derefs to a slice) but stays memory-safe
//! when several OS threads use the same object at once (LANGUAGE.md rule 4: a data race can
//! give wrong answers, never a crash):
//!
//! * A buffer never shrinks in place, and a reader clamps the length it sees to the capacity
//!   it saw first. Capacity is published *after* the pointer when a buffer grows, so any
//!   (capacity, pointer) pair a reader can observe is in bounds.
//! * A replaced buffer is not freed while other threads may still read it: it is *retired*
//!   (see `mt::retire`) and freed at the next GC unless a stack still points into it.
//! * Writers take a lock bit (the top bit of `cap`) only while more than one OS thread runs
//!   Ruby code, so single-threaded programs pay nothing. The lock is never held across a call
//!   into Ruby or a safepoint.
//! * Unused capacity of value buffers is zeroed, so a racing reader never sees garbage.
//!
//! Layout is `repr(C)` {ptr, len, cap} (compiled code may read ptr/len directly).

use crate::mt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::*};

const LOCK: usize = 1 << 63;
/// The owning object is old (generational GC): writes of new values must be remembered.
const OLD: usize = 1 << 62;
/// Already in the remembered set since the last collection.
const REM: usize = 1 << 61;
const FLAGS: usize = OLD | REM;
const CAP_MASK: usize = (1 << 56) - 1;

#[repr(C)]
pub struct SVec<T: Copy> {
    ptr: AtomicPtr<T>,
    len: AtomicUsize,
    cap: AtomicUsize,
    _t: PhantomData<T>,
}

unsafe impl<T: Copy> Sync for SVec<T> {}
unsafe impl<T: Copy> Send for SVec<T> {}

#[inline]
fn dangling<T>() -> *mut T {
    std::ptr::NonNull::<T>::dangling().as_ptr()
}

impl<T: Copy> SVec<T> {
    pub const fn new() -> Self {
        SVec { ptr: AtomicPtr::new(std::ptr::NonNull::<T>::dangling().as_ptr()), len: AtomicUsize::new(0), cap: AtomicUsize::new(0), _t: PhantomData }
    }
    /// Builder: set the length (≤ capacity) of a fresh buffer.
    pub fn with_len(self, n: usize) -> Self {
        self.len.store(n.min(self.capacity()), Relaxed);
        self
    }
    pub fn with_capacity(n: usize) -> Self {
        Self::from(Vec::with_capacity(n))
    }

    // ----- reading -----

    /// (pointer, length) of a consistent snapshot.
    #[inline(always)]
    fn snap(&self) -> (*mut T, usize) {
        // capacity first (acquire orders the pointer load after it); the element loads
        // depend on the pointer. A stale length only gives a stale (zeroed or old) element.
        let c = self.cap.load(Acquire) & CAP_MASK;
        let p = self.ptr.load(Relaxed);
        let n = self.len.load(Relaxed);
        (p, n.min(c))
    }
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        let (p, n) = self.snap();
        unsafe { std::slice::from_raw_parts(p, n) }
    }
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.wb();
        let (p, n) = self.snap();
        unsafe { std::slice::from_raw_parts_mut(p, n) }
    }
    pub fn capacity(&self) -> usize {
        self.cap.load(Relaxed) & CAP_MASK
    }

    // ----- generational GC -----

    /// Write barrier: a value buffer of an old object that gets new values joins the
    /// remembered set (once per collection cycle).
    #[inline(always)]
    fn wb(&self) {
        if std::mem::size_of::<T>() == 8 && self.cap.load(Relaxed) & FLAGS == OLD {
            self.remember();
        }
    }
    #[inline(never)]
    fn remember(&self) {
        let w = self.cap.fetch_or(REM, Relaxed);
        if w & REM == 0 {
            crate::gc::remember(crate::gc::REM_SVEC, self as *const SVec<T> as usize);
        }
    }
    /// For the GC: the owning object is now old.
    pub fn set_old(&self) {
        if self.cap.load(Relaxed) & OLD == 0 {
            self.cap.fetch_or(OLD, Relaxed);
        }
    }
    /// For the GC: processed the remembered entry.
    pub fn clear_remembered(&self) {
        self.cap.fetch_and(!REM, Relaxed);
    }
    /// A copy of the contents (shadows `Clone::clone`, so `x.v.clone()` keeps returning a Vec).
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> Vec<T> {
        self.as_slice().to_vec()
    }

    // ----- writer lock -----

    /// Lock for writing (only when several OS threads run Ruby code). Returns the capacity.
    #[inline(always)]
    fn lock(&self) -> usize {
        if !mt::multi() {
            return self.cap.load(Relaxed) & CAP_MASK;
        }
        self.lock_slow()
    }
    #[inline(never)]
    fn lock_slow(&self) -> usize {
        let mut spins = 0u32;
        loop {
            let c = self.cap.load(Relaxed);
            if c & LOCK == 0 && self.cap.compare_exchange_weak(c, c | LOCK, Acquire, Relaxed).is_ok() {
                return c & CAP_MASK;
            }
            spins += 1;
            if spins < 64 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
    }
    /// Unlock, publishing capacity `c` (also when unlocked: a plain store); the GC flags
    /// are kept.
    #[inline(always)]
    fn unlock(&self, c: usize) {
        let flags = self.cap.load(Relaxed) & FLAGS;
        self.cap.store(c | flags, Release);
    }

    /// Replace the buffer with one of capacity `newcap` holding the first `n` elements
    /// followed by `extra`. Called with the lock held; unlocks.
    fn grow_with(&self, oldcap: usize, n: usize, newcap: usize, extra: &[T]) {
        let old = self.ptr.load(Relaxed);
        let total = n + extra.len();
        let newcap = newcap.max(total);
        let mut v: Vec<T> = Vec::with_capacity(newcap);
        unsafe {
            std::ptr::copy_nonoverlapping(old, v.as_mut_ptr(), n);
            std::ptr::copy_nonoverlapping(extra.as_ptr(), v.as_mut_ptr().add(n), extra.len());
            v.set_len(total);
        }
        let mut v = std::mem::ManuallyDrop::new(v);
        let (np, ncap) = (v.as_mut_ptr(), v.capacity());
        zero_tail::<T>(np, total, ncap);
        self.ptr.store(np, Release);
        self.len.store(total, Release);
        self.unlock(ncap);
        retire_buf(old, oldcap);
    }

    // ----- writing -----

    #[inline]
    pub fn push(&mut self, x: T) {
        self.wb();
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if n < cap {
            unsafe { self.ptr.load(Relaxed).add(n).write(x) };
            self.len.store(n + 1, Release);
            self.unlock(cap);
        } else {
            self.grow_with(cap, n, (cap * 2).max(4), &[x]);
        }
    }
    pub fn pop(&mut self) -> Option<T> {
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        let r = if n == 0 {
            None
        } else {
            let x = unsafe { self.ptr.load(Relaxed).add(n - 1).read() };
            self.len.store(n - 1, Release);
            Some(x)
        };
        self.unlock(cap);
        r
    }
    pub fn truncate(&mut self, k: usize) {
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if k < n {
            self.len.store(k, Release);
        }
        self.unlock(cap);
    }
    pub fn clear(&mut self) {
        self.truncate(0);
    }
    pub fn insert(&mut self, i: usize, x: T) {
        self.wb();
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if i > n {
            self.unlock(cap);
            panic!("insertion index (is {i}) should be <= len (is {n})");
        }
        if n < cap {
            unsafe {
                let p = self.ptr.load(Relaxed);
                std::ptr::copy(p.add(i), p.add(i + 1), n - i);
                p.add(i).write(x);
            }
            self.len.store(n + 1, Release);
            self.unlock(cap);
        } else {
            let mut tail = Vec::with_capacity(n - i + 1);
            tail.push(x);
            tail.extend_from_slice(unsafe { std::slice::from_raw_parts(self.ptr.load(Relaxed).add(i), n - i) });
            self.grow_with(cap, i, (cap * 2).max(4), &tail);
        }
    }
    pub fn remove(&mut self, i: usize) -> T {
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if i >= n {
            self.unlock(cap);
            panic!("removal index (is {i}) should be < len (is {n})");
        }
        let x = unsafe {
            let p = self.ptr.load(Relaxed);
            let x = p.add(i).read();
            std::ptr::copy(p.add(i + 1), p.add(i), n - i - 1);
            x
        };
        self.len.store(n - 1, Release);
        self.unlock(cap);
        x
    }
    pub fn swap_remove(&mut self, i: usize) -> T {
        let n = self.len();
        let last = self.as_slice()[n - 1];
        let x = self.as_slice()[i];
        self.as_mut_slice()[i] = last; // (as_mut_slice applies the barrier)
        self.truncate(n - 1);
        x
    }
    pub fn extend_from_slice(&mut self, xs: &[T]) {
        if xs.is_empty() {
            return;
        }
        self.wb();
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if n + xs.len() <= cap {
            unsafe { std::ptr::copy(xs.as_ptr(), self.ptr.load(Relaxed).add(n), xs.len()) };
            self.len.store(n + xs.len(), Release);
            self.unlock(cap);
        } else {
            self.grow_with(cap, n, (cap * 2).max(n + xs.len()).max(8), xs);
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        let v: Vec<T> = it.into_iter().collect();
        self.extend_from_slice(&v);
    }
    pub fn append(&mut self, other: &mut Vec<T>) {
        self.extend_from_slice(other);
        other.clear();
    }
    pub fn reserve(&mut self, extra: usize) {
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if n + extra <= cap {
            self.unlock(cap);
        } else {
            self.grow_with(cap, n, (n + extra).max(cap * 2), &[]);
        }
    }
    pub fn resize(&mut self, k: usize, fill: T) {
        self.wb();
        let cap = self.lock();
        let n = self.len.load(Relaxed).min(cap);
        if k <= n {
            self.len.store(k, Release);
            self.unlock(cap);
        } else if k <= cap {
            let p = self.ptr.load(Relaxed);
            // usually one or two slots (an object's next ivar): plain stores, not a memset call
            for i in n..k {
                unsafe { std::ptr::write_volatile(p.add(i), fill) };
            }
            self.len.store(k, Release);
            self.unlock(cap);
        } else {
            let old = self.ptr.load(Relaxed);
            let mut v: Vec<T> = Vec::with_capacity(k.max(cap * 2).max(4));
            unsafe {
                std::ptr::copy_nonoverlapping(old, v.as_mut_ptr(), n);
                v.set_len(n);
            }
            v.resize(k, fill);
            let mut v = std::mem::ManuallyDrop::new(v);
            let (np, ncap) = (v.as_mut_ptr(), v.capacity());
            zero_tail::<T>(np, k, ncap);
            self.ptr.store(np, Release);
            self.len.store(k, Release);
            self.unlock(ncap);
            retire_buf(old, cap);
        }
    }
    /// Replace the whole contents.
    pub fn set(&mut self, v: Vec<T>) {
        self.wb();
        let cap = self.lock();
        if v.len() <= cap {
            unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), self.ptr.load(Relaxed), v.len()) };
            self.len.store(v.len(), Release);
            self.unlock(cap);
        } else {
            self.grow_with(cap, 0, v.len(), &v);
        }
    }
    pub fn set_from_slice(&mut self, v: &[T]) {
        self.wb();
        let cap = self.lock();
        if v.len() <= cap {
            unsafe { std::ptr::copy(v.as_ptr(), self.ptr.load(Relaxed), v.len()) };
            self.len.store(v.len(), Release);
            self.unlock(cap);
        } else {
            self.grow_with(cap, 0, v.len(), v);
        }
    }
    /// Take the contents out, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<T> {
        let v = self.clone();
        self.truncate(0);
        v
    }
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, f: F) {
        let mut v = self.clone();
        v.retain(f);
        self.set(v);
    }
    pub fn dedup(&mut self)
    where
        T: PartialEq,
    {
        let mut v = self.clone();
        v.dedup();
        self.set(v);
    }
    pub fn drain<R: std::ops::RangeBounds<usize>>(&mut self, r: R) -> std::vec::IntoIter<T> {
        let mut v = self.clone();
        let d: Vec<T> = v.drain(r).collect();
        self.set(v);
        d.into_iter()
    }
    pub fn splice<R: std::ops::RangeBounds<usize>, I: IntoIterator<Item = T>>(&mut self, r: R, with: I) -> std::vec::IntoIter<T> {
        let mut v = self.clone();
        let d: Vec<T> = v.splice(r, with).collect();
        self.set(v);
        d.into_iter()
    }
    pub fn split_off(&mut self, at: usize) -> Vec<T> {
        let v = self.as_slice()[at..].to_vec();
        self.truncate(at);
        v
    }
}

/// Zero `[from, cap)` of a value buffer, so racing readers never see uninitialised words.
#[inline]
fn zero_tail<T>(p: *mut T, from: usize, cap: usize) {
    if std::mem::size_of::<T>() > 1 && cap > from {
        unsafe { std::ptr::write_bytes(p.add(from), 0, cap - from) };
    }
}

fn retire_buf<T>(p: *mut T, cap: usize) {
    if cap == 0 || std::mem::size_of::<T>() == 0 {
        return;
    }
    let bytes = cap * std::mem::size_of::<T>();
    let align = std::mem::align_of::<T>();
    mt::retire(p as usize, bytes, align);
}

impl<T: Copy> From<Vec<T>> for SVec<T> {
    fn from(v: Vec<T>) -> Self {
        if v.capacity() == 0 {
            return SVec::new();
        }
        let mut v = std::mem::ManuallyDrop::new(v);
        let (p, n, c) = (v.as_mut_ptr(), v.len(), v.capacity());
        zero_tail::<T>(p, n, c);
        SVec { ptr: AtomicPtr::new(p), len: AtomicUsize::new(n), cap: AtomicUsize::new(c), _t: PhantomData }
    }
}
impl<T: Copy> From<&[T]> for SVec<T> {
    fn from(v: &[T]) -> Self {
        Self::from(v.to_vec())
    }
}
impl<T: Copy> Default for SVec<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Copy> Drop for SVec<T> {
    fn drop(&mut self) {
        let cap = self.cap.load(Relaxed) & CAP_MASK;
        let p = self.ptr.load(Relaxed);
        if cap != 0 && p != dangling::<T>() {
            retire_buf(p, cap);
        }
    }
}
impl<T: Copy> std::ops::Deref for SVec<T> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}
impl<T: Copy> std::ops::DerefMut for SVec<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}
impl<'a, T: Copy> IntoIterator for &'a SVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
impl<'a, T: Copy> IntoIterator for &'a mut SVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}
impl<T: Copy + PartialEq> PartialEq for SVec<T> {
    fn eq(&self, o: &Self) -> bool {
        self.as_slice() == o.as_slice()
    }
}
impl<T: Copy + PartialEq> PartialEq<Vec<T>> for SVec<T> {
    fn eq(&self, o: &Vec<T>) -> bool {
        self.as_slice() == o.as_slice()
    }
}
impl<T: Copy + PartialEq> PartialEq<[T]> for SVec<T> {
    fn eq(&self, o: &[T]) -> bool {
        self.as_slice() == o
    }
}
impl<T: Copy + PartialEq, const N: usize> PartialEq<[T; N]> for SVec<T> {
    fn eq(&self, o: &[T; N]) -> bool {
        self.as_slice() == o.as_slice()
    }
}
impl<T: Copy + PartialEq> PartialEq<&[T]> for SVec<T> {
    fn eq(&self, o: &&[T]) -> bool {
        self.as_slice() == *o
    }
}
impl<T: Copy + PartialEq, const N: usize> PartialEq<&[T; N]> for SVec<T> {
    fn eq(&self, o: &&[T; N]) -> bool {
        self.as_slice() == o.as_slice()
    }
}
impl<T: Copy + PartialEq> PartialEq<SVec<T>> for Vec<T> {
    fn eq(&self, o: &SVec<T>) -> bool {
        self.as_slice() == o.as_slice()
    }
}
impl<T: Copy + PartialEq> PartialEq<SVec<T>> for [T] {
    fn eq(&self, o: &SVec<T>) -> bool {
        self == o.as_slice()
    }
}
impl<T: Copy + Eq> Eq for SVec<T> {}
impl<T: Copy + std::hash::Hash> std::hash::Hash for SVec<T> {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.as_slice().hash(h)
    }
}
impl<T: Copy + std::fmt::Debug> std::fmt::Debug for SVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl<T: Copy> std::borrow::Borrow<[T]> for SVec<T> {
    fn borrow(&self) -> &[T] {
        self.as_slice()
    }
}
impl<T: Copy> AsRef<[T]> for SVec<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}
impl std::io::Write for SVec<u8> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
