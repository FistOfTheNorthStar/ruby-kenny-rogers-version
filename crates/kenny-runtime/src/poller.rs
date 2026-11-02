//! The event loop behind waits on sockets, pipes, terminals and child processes: a thread that
//! wants an fd readable or writable registers it here (one-shot: each wait arms it again) and
//! parks in the scheduler; whoever polls the event queue (kqueue on macOS and the BSDs, epoll
//! on Linux) makes it runnable again.
//!
//! Who polls: one P at a time that has nothing to run (it holds the *token*) waits in the
//! event queue instead of on its parker, so a ready fd wakes that P's OS thread directly and
//! the woken thread runs there (one OS wake-up, as a blocking read would cost). A P holding
//! the token hands it on when it gets work: to another idle P, else to a dedicated poller
//! thread, which polls while every P is busy (in the HTTP server every P runs its own event
//! loop). An idle P takes the token back from that thread.
//!
//! Closing an fd that threads wait on (`IO#close` in another thread) wakes them with
//! `closed`. After `fork` the child gets a new event queue (the parent's isn't inherited, or
//! is shared on Linux) and poller thread.
//!
//! `KENNY_NO_POLLER=1` turns it off (waits then poll the fds in 2 ms steps, as before).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// The waits on one fd: the keys (see `next_key`) of the threads waiting for each direction.
#[derive(Default)]
struct Waits {
    read: Vec<usize>,
    write: Vec<usize>,
}

#[derive(Default)]
struct State {
    fds: HashMap<i32, Waits>,
    /// child processes waited for (kqueue's EVFILT_PROC; Linux waits on a pidfd instead)
    procs: HashMap<i32, Vec<usize>>,
    /// keys whose fd was closed while they waited (taken by `deregister`)
    closed: HashSet<usize>,
}

struct Poller {
    /// the event queue
    fd: i32,
    /// Linux: the eventfd that interrupts the poller thread (kqueue uses an EVFILT_USER event)
    kick_fd: i32,
    state: Mutex<State>,
    /// the poller thread waits here while it doesn't hold the token
    d_wake: (Mutex<()>, Condvar),
}

static POLLER: AtomicPtr<Poller> = AtomicPtr::new(std::ptr::null_mut());
static INIT: AtomicBool = AtomicBool::new(false);
static OFF: AtomicBool = AtomicBool::new(false);
/// Registered (fd or process, key) waits: when 0, nobody needs to poll.
static NWAITS: AtomicUsize = AtomicUsize::new(0);
/// Who polls: FREE, P `p` as p + 1, or THREAD (the poller thread).
static TOKEN: AtomicUsize = AtomicUsize::new(FREE);
const FREE: usize = 0;
const THREAD: usize = usize::MAX;
/// Bumped in a forked child: the per-P queues of the parent are gone.
static GEN: AtomicUsize = AtomicUsize::new(1);

/// Per P: its own small event queue (its wake-up event, plus the shared queue while it holds
/// the token), and whether it is waiting in it now.
const MAXP: usize = 256;
static P_KQ: [AtomicI32; MAXP] = [const { AtomicI32::new(-1) }; MAXP];
static P_EFD: [AtomicI32; MAXP] = [const { AtomicI32::new(-1) }; MAXP];
static P_GEN: [AtomicUsize; MAXP] = [const { AtomicUsize::new(0) }; MAXP];
static P_POLLING: [AtomicBool; MAXP] = [const { AtomicBool::new(false) }; MAXP];

/// The event identifiers of the wake-up events (never an fd: fds are small).
const KICK: usize = usize::MAX - 1;
const SHARED: usize = usize::MAX - 2;

fn get() -> Option<&'static Poller> {
    let p = POLLER.load(Ordering::Acquire);
    if !p.is_null() {
        return Some(unsafe { &*p });
    }
    if OFF.load(Ordering::Relaxed) {
        return None;
    }
    init()
}

#[cold]
fn init() -> Option<&'static Poller> {
    while INIT.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
    let p = POLLER.load(Ordering::Acquire);
    if !p.is_null() || OFF.load(Ordering::Relaxed) {
        INIT.store(false, Ordering::Release);
        return if p.is_null() { None } else { Some(unsafe { &*p }) };
    }
    let r = create();
    if r.is_none() {
        OFF.store(true, Ordering::Relaxed);
    }
    INIT.store(false, Ordering::Release);
    r
}

fn create() -> Option<&'static Poller> {
    if std::env::var_os("KENNY_NO_POLLER").is_some() {
        return None;
    }
    static ATFORK: std::sync::Once = std::sync::Once::new();
    ATFORK.call_once(|| unsafe {
        libc::pthread_atfork(None, None, Some(after_fork_child));
    });
    let fd = sys::create();
    if fd < 0 {
        return None;
    }
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    let kick_fd = sys::add_kick(fd, KICK);
    if kick_fd == -2 {
        unsafe { libc::close(fd) };
        return None;
    }
    let p: &'static Poller = Box::leak(Box::new(Poller { fd, kick_fd, state: Mutex::new(State::default()), d_wake: (Mutex::new(()), Condvar::new()) }));
    let generation = GEN.load(Ordering::Relaxed);
    if std::thread::Builder::new().name("kenny-poller".into()).spawn(move || poller_thread(p, generation)).is_err() {
        return None;
    }
    POLLER.store(p as *const Poller as *mut Poller, Ordering::Release);
    Some(p)
}

/// In a forked child: the parent's event queues and poller thread don't carry over (a kqueue
/// isn't inherited; an epoll instance would be shared with the parent). Start afresh: the
/// next wait creates new ones. (Only the forking thread lives on, so nobody waits.)
extern "C" fn after_fork_child() {
    let p = POLLER.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !p.is_null() {
        // (the old Poller is leaked: its lock may have been held by a thread that is gone)
        let p = unsafe { &*p };
        unsafe { libc::close(p.fd) };
        if p.kick_fd >= 0 {
            unsafe { libc::close(p.kick_fd) };
        }
    }
    for i in 0..MAXP {
        for a in [&P_KQ[i], &P_EFD[i]] {
            let fd = a.swap(-1, Ordering::Relaxed);
            if fd >= 0 {
                unsafe { libc::close(fd) };
            }
        }
        P_POLLING[i].store(false, Ordering::Relaxed);
    }
    INIT.store(false, Ordering::Relaxed);
    NWAITS.store(0, Ordering::Relaxed);
    TOKEN.store(FREE, Ordering::Relaxed);
    GEN.fetch_add(1, Ordering::Relaxed);
}

/// The poller is off (KENNY_NO_POLLER) or can't work here.
pub fn off() -> bool {
    get().is_none()
}

/// A key for one wait: odd, so it never equals an object a thread can also block on.
pub fn next_key() -> usize {
    static N: AtomicUsize = AtomicUsize::new(1);
    (N.fetch_add(1, Ordering::Relaxed) << 4) | 1
}

fn lock(p: &Poller) -> std::sync::MutexGuard<'_, State> {
    p.state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wake `key` when one of `fds` (fd, wants writing) is ready. False: the poller is off or
/// can't watch one of them (nothing stays registered then).
pub fn register(fds: &[(i32, bool)], key: usize) -> bool {
    let Some(p) = get() else { return false };
    let mut w = lock(p);
    for (i, &(fd, write)) in fds.iter().enumerate() {
        let e = w.fds.entry(fd).or_default();
        if write { e.write.push(key) } else { e.read.push(key) }
        let (r, wr) = (!e.read.is_empty(), !e.write.is_empty());
        NWAITS.fetch_add(1, Ordering::SeqCst);
        if !sys::arm(p.fd, fd, write, r, wr) {
            drop_key(&mut w, &fds[..=i], key);
            return false;
        }
    }
    true
}

/// The wait `key` is over (woken, timed out or interrupted). True: one of its fds was
/// closed by another thread meanwhile.
pub fn deregister(fds: &[(i32, bool)], key: usize) -> bool {
    let Some(p) = get() else { return false };
    let mut w = lock(p);
    drop_key(&mut w, fds, key);
    w.closed.remove(&key)
}

fn drop_key(w: &mut State, fds: &[(i32, bool)], key: usize) {
    for &(fd, _) in fds {
        if let Some(e) = w.fds.get_mut(&fd) {
            let before = e.read.len() + e.write.len();
            e.read.retain(|&k| k != key);
            e.write.retain(|&k| k != key);
            NWAITS.fetch_sub(before - e.read.len() - e.write.len(), Ordering::SeqCst);
            if e.read.is_empty() && e.write.is_empty() {
                w.fds.remove(&fd);
            }
        }
    }
}

/// `fd` is about to be closed: the threads waiting on it wake up, and their wait reports it
/// (`deregister` returns true).
pub fn closing(fd: i32) {
    if NWAITS.load(Ordering::SeqCst) == 0 {
        return;
    }
    let Some(p) = get() else { return };
    let keys = {
        let mut w = lock(p);
        let Some(e) = w.fds.remove(&fd) else { return };
        let keys: Vec<usize> = e.read.into_iter().chain(e.write).collect();
        NWAITS.fetch_sub(keys.len(), Ordering::SeqCst);
        w.closed.extend(keys.iter().copied());
        sys::forget(p.fd, fd);
        keys
    };
    for k in keys {
        crate::thread::wake_io(k);
    }
}

/// Wake `key` when child process `pid` exits (kqueue). False: not supported here (Linux
/// waits on a pidfd through `register` instead) or no such process.
pub fn register_proc(pid: i32, key: usize) -> bool {
    let Some(p) = get() else { return false };
    if !sys::HAS_PROC {
        return false;
    }
    let mut w = lock(p);
    if !sys::arm_proc(p.fd, pid) {
        return false;
    }
    w.procs.entry(pid).or_default().push(key);
    NWAITS.fetch_add(1, Ordering::SeqCst);
    true
}

pub fn deregister_proc(pid: i32, key: usize) {
    let Some(p) = get() else { return };
    let mut w = lock(p);
    if let Some(v) = w.procs.get_mut(&pid) {
        let before = v.len();
        v.retain(|&k| k != key);
        NWAITS.fetch_sub(before - v.len(), Ordering::SeqCst);
        if v.is_empty() {
            w.procs.remove(&pid);
        }
    }
}

/// One event: (ident, readable, writable, a process exit)
type Ev = (usize, bool, bool, bool);

/// The keys to wake for `ready` (taken off their waits).
fn take_ready(p: &Poller, ready: &[Ev], keys: &mut Vec<usize>) {
    if ready.iter().all(|e| e.0 >= SHARED) {
        return;
    }
    let mut w = lock(p);
    for &(id, r, wr, proc_exit) in ready {
        if id >= SHARED {
            continue;
        }
        if proc_exit {
            if let Some(v) = w.procs.remove(&(id as i32)) {
                NWAITS.fetch_sub(v.len(), Ordering::SeqCst);
                keys.extend(v);
            }
            continue;
        }
        let fd = id as i32;
        let Some(e) = w.fds.get_mut(&fd) else { continue };
        let n = keys.len();
        if r {
            keys.append(&mut e.read);
        }
        if wr {
            keys.append(&mut e.write);
        }
        NWAITS.fetch_sub(keys.len() - n, Ordering::SeqCst);
        if e.read.is_empty() && e.write.is_empty() {
            w.fds.remove(&fd);
        } else {
            // (epoll disarmed the fd: arm it again for the waits left)
            sys::rearm(p.fd, fd, !e.read.is_empty(), !e.write.is_empty());
        }
    }
}

// ---------- the token ----------

/// Called by P `idx` with nothing to run: should it wait in the event queue (it now holds
/// the token)? (Only when someone waits; takes the token from the poller thread.)
pub fn want_poll(idx: usize) -> bool {
    let me = idx + 1;
    if NWAITS.load(Ordering::SeqCst) == 0 {
        let _ = TOKEN.compare_exchange(me, FREE, Ordering::SeqCst, Ordering::Relaxed);
        return false;
    }
    let Some(p) = get() else { return false };
    if idx >= MAXP {
        return false;
    }
    loop {
        let t = TOKEN.load(Ordering::SeqCst);
        if t == me {
            return true;
        }
        if t != FREE && t != THREAD {
            return false; // another P polls
        }
        if TOKEN.compare_exchange(t, me, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
            if t == THREAD {
                sys::kick(p.fd, p.kick_fd, KICK);
            }
            return true;
        }
    }
}

/// P `idx` is about to run a thread: if it holds the token, or nobody does while threads
/// wait, someone else must poll. Some(()) when the caller should find that someone
/// (`give_token`).
#[inline]
pub fn needs_poller(idx: usize) -> bool {
    let t = TOKEN.load(Ordering::Relaxed);
    t == idx + 1 || (t == FREE && NWAITS.load(Ordering::Relaxed) > 0)
}

/// Hand the token from P `idx` (or from nobody) to idle P `to`, or to the poller thread
/// when `to` is None (or no one, if nobody waits).
pub fn give_token(idx: usize, to: Option<usize>) {
    let me = idx + 1;
    let Some(p) = get() else { return };
    let waits = NWAITS.load(Ordering::SeqCst) > 0;
    let next = match to {
        _ if !waits => FREE,
        Some(v) => v + 1,
        None => THREAD,
    };
    let ok = TOKEN.compare_exchange(me, next, Ordering::SeqCst, Ordering::Relaxed).is_ok()
        || TOKEN.compare_exchange(FREE, next, Ordering::SeqCst, Ordering::Relaxed).is_ok();
    if ok && next == THREAD {
        let _g = p.d_wake.0.lock().unwrap_or_else(|e| e.into_inner());
        p.d_wake.1.notify_one();
    }
}

/// Make P `idx`'s event queue (before it says it polls: `kick_p` needs it). False: can't.
pub fn ensure_p(idx: usize) -> bool {
    if get().is_none() {
        return false;
    }
    let generation = GEN.load(Ordering::Relaxed);
    if P_GEN[idx].load(Ordering::Relaxed) != generation || P_KQ[idx].load(Ordering::Relaxed) < 0 {
        let kq = sys::create();
        if kq < 0 {
            return false;
        }
        unsafe { libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC) };
        let efd = sys::add_kick(kq, KICK);
        if efd == -2 {
            unsafe { libc::close(kq) };
            return false;
        }
        let old = P_KQ[idx].swap(kq, Ordering::SeqCst);
        if old >= 0 {
            unsafe { libc::close(old) };
        }
        let old = P_EFD[idx].swap(efd, Ordering::SeqCst);
        if old >= 0 {
            unsafe { libc::close(old) };
        }
        P_GEN[idx].store(generation, Ordering::Relaxed);
    }
    true
}

/// P `idx` holds the token and waits for events (or `kick_p`, or the timeout); the keys of
/// the waits that are ready go to `keys`. Called while counted as idle for the GC, after
/// `ensure_p`.
pub fn poll_p(idx: usize, timeout: Option<Duration>, keys: &mut Vec<usize>) {
    let Some(p) = get() else { return };
    let (kq, efd) = (P_KQ[idx].load(Ordering::Relaxed), P_EFD[idx].load(Ordering::Relaxed));
    if kq < 0 {
        return;
    }
    // (watch the shared queue from this one, armed once per wait)
    sys::nest(kq, p.fd, SHARED, false);
    let mut ready: Vec<Ev> = Vec::with_capacity(8);
    sys::wait(kq, timeout, &mut ready);
    if ready.iter().any(|e| e.0 == KICK) {
        sys::drain_kick(efd);
    }
    if ready.iter().any(|e| e.0 == SHARED) {
        let mut evs: Vec<Ev> = Vec::with_capacity(64);
        sys::wait(p.fd, Some(Duration::ZERO), &mut evs);
        take_ready(p, &evs, keys);
    }
}

/// Mark P `idx` as waiting in its event queue (true) or not; see `kick_p`.
pub fn set_polling(idx: usize, on: bool) {
    P_POLLING[idx].store(on, Ordering::SeqCst);
}
pub fn is_polling(idx: usize) -> bool {
    P_POLLING[idx].load(Ordering::SeqCst)
}

/// Interrupt P `idx`'s wait in its event queue (it was unparked).
pub fn kick_p(idx: usize) {
    let kq = P_KQ[idx].load(Ordering::SeqCst);
    if kq >= 0 {
        sys::kick(kq, P_EFD[idx].load(Ordering::SeqCst), KICK);
    }
}

/// The poller thread: polls while it holds the token (no P is idle to do it).
fn poller_thread(p: &'static Poller, generation: usize) {
    let mut ready: Vec<Ev> = Vec::with_capacity(64);
    let mut keys: Vec<usize> = Vec::with_capacity(64);
    loop {
        if GEN.load(Ordering::Relaxed) != generation {
            return;
        }
        if TOKEN.load(Ordering::SeqCst) != THREAD {
            let g = p.d_wake.0.lock().unwrap_or_else(|e| e.into_inner());
            if TOKEN.load(Ordering::SeqCst) != THREAD {
                // (a timeout as a safety net: a token handed over just as the P quits polling)
                let _ = p.d_wake.1.wait_timeout(g, Duration::from_secs(1));
            }
            if TOKEN.load(Ordering::SeqCst) == FREE && NWAITS.load(Ordering::SeqCst) > 0 {
                let _ = TOKEN.compare_exchange(FREE, THREAD, Ordering::SeqCst, Ordering::Relaxed);
            }
            continue;
        }
        ready.clear();
        sys::wait(p.fd, None, &mut ready);
        if ready.iter().any(|e| e.0 == KICK) {
            sys::drain_kick(p.kick_fd);
        }
        take_ready(p, &ready, &mut keys);
        for k in keys.drain(..) {
            crate::thread::wake_io(k);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
mod sys {
    use super::Ev;
    use std::time::Duration;

    pub const HAS_PROC: bool = true;

    pub fn create() -> i32 {
        unsafe { libc::kqueue() }
    }
    fn kev(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
        let mut k: libc::kevent = unsafe { std::mem::zeroed() };
        k.ident = ident as libc::uintptr_t;
        k.filter = filter as _;
        k.flags = flags as _;
        k.fflags = fflags;
        k
    }
    fn apply(kq: i32, ch: &libc::kevent) -> bool {
        loop {
            let n = unsafe { libc::kevent(kq, ch, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
            if n >= 0 {
                return true;
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return false;
            }
        }
    }
    /// Arm the filter for `write` (kqueue keeps one per direction).
    pub fn arm(kq: i32, fd: i32, write: bool, _read_any: bool, _write_any: bool) -> bool {
        let f = if write { libc::EVFILT_WRITE } else { libc::EVFILT_READ };
        apply(kq, &kev(fd as usize, f, libc::EV_ADD | libc::EV_ONESHOT, 0))
    }
    /// (a one-shot kqueue filter only disarms its own direction)
    pub fn rearm(_kq: i32, _fd: i32, _read: bool, _write: bool) {}
    /// (closing an fd removes its filters)
    pub fn forget(_kq: i32, _fd: i32) {}
    pub fn arm_proc(kq: i32, pid: i32) -> bool {
        apply(kq, &kev(pid as usize, libc::EVFILT_PROC, libc::EV_ADD | libc::EV_ONESHOT, libc::NOTE_EXIT))
    }
    /// Add the wake-up event `id` (returns the fd to kick it through: none, -1).
    pub fn add_kick(kq: i32, id: usize) -> i32 {
        if apply(kq, &kev(id, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0)) { -1 } else { -2 }
    }
    pub fn kick(kq: i32, _efd: i32, id: usize) {
        apply(kq, &kev(id, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER));
    }
    pub fn drain_kick(_efd: i32) {}
    /// Watch queue `inner` from queue `kq` (once: armed again by every call).
    pub fn nest(kq: i32, inner: i32, id: usize, _first: bool) {
        let mut k = kev(inner as usize, libc::EVFILT_READ, libc::EV_ADD | libc::EV_ONESHOT, 0);
        k.udata = id as *mut libc::c_void;
        apply(kq, &k);
    }
    pub fn wait(kq: i32, timeout: Option<Duration>, out: &mut Vec<Ev>) {
        let mut evs: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
        let ts = timeout.map(|t| libc::timespec { tv_sec: t.as_secs().min(1 << 30) as _, tv_nsec: t.subsec_nanos() as _ });
        let tp = ts.as_ref().map_or(std::ptr::null(), |t| t as *const libc::timespec);
        let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, evs.as_mut_ptr(), 64, tp) };
        for e in evs.iter().take(n.max(0) as usize) {
            if e.filter == libc::EVFILT_USER {
                out.push((e.ident as usize, false, false, false));
                continue;
            }
            if e.filter == libc::EVFILT_PROC {
                out.push((e.ident as usize, false, false, true));
                continue;
            }
            if !e.udata.is_null() {
                out.push((e.udata as usize, true, false, false)); // the nested shared queue
                continue;
            }
            let err = e.flags & libc::EV_ERROR != 0;
            let write = e.filter == libc::EVFILT_WRITE;
            out.push((e.ident as usize, !write || err, write || err, false));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    use super::Ev;
    use std::time::Duration;

    pub const HAS_PROC: bool = false;

    pub fn create() -> i32 {
        unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) }
    }
    fn ctl(ep: i32, fd: i32, events: u32, data: u64) -> bool {
        let mut ev = libc::epoll_event { events, u64: data };
        unsafe {
            if libc::epoll_ctl(ep, libc::EPOLL_CTL_MOD, fd, &mut ev) == 0 {
                return true;
            }
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                if libc::epoll_ctl(ep, libc::EPOLL_CTL_ADD, fd, &mut ev) == 0 {
                    return true;
                }
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
                    return libc::epoll_ctl(ep, libc::EPOLL_CTL_MOD, fd, &mut ev) == 0;
                }
            }
            false
        }
    }
    fn set(ep: i32, fd: i32, read: bool, write: bool) -> bool {
        let events = (if read { (libc::EPOLLIN | libc::EPOLLRDHUP) as u32 } else { 0 })
            | (if write { libc::EPOLLOUT as u32 } else { 0 })
            | libc::EPOLLONESHOT as u32;
        ctl(ep, fd, events, fd as u64)
    }
    /// epoll has one registration per fd: arm it for every direction someone waits for.
    pub fn arm(ep: i32, fd: i32, _write: bool, read_any: bool, write_any: bool) -> bool {
        set(ep, fd, read_any, write_any)
    }
    pub fn rearm(ep: i32, fd: i32, read: bool, write: bool) {
        set(ep, fd, read, write);
    }
    /// (another process may share the file: closing our fd wouldn't remove it)
    pub fn forget(ep: i32, fd: i32) {
        let mut ev = libc::epoll_event { events: 0, u64: 0 };
        unsafe { libc::epoll_ctl(ep, libc::EPOLL_CTL_DEL, fd, &mut ev) };
    }
    pub fn arm_proc(_ep: i32, _pid: i32) -> bool {
        false
    }
    /// Add a wake-up eventfd as event `id`; returns it (-2: failed).
    pub fn add_kick(ep: i32, id: usize) -> i32 {
        let efd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if efd < 0 {
            return -2;
        }
        if !ctl(ep, efd, libc::EPOLLIN as u32, id as u64) {
            unsafe { libc::close(efd) };
            return -2;
        }
        efd
    }
    pub fn kick(_ep: i32, efd: i32, _id: usize) {
        let one: u64 = 1;
        unsafe { libc::write(efd, &one as *const u64 as *const libc::c_void, 8) };
    }
    pub fn drain_kick(efd: i32) {
        let mut v: u64 = 0;
        unsafe { libc::read(efd, &mut v as *mut u64 as *mut libc::c_void, 8) };
    }
    /// Watch queue `inner` from queue `ep` (once: armed again by every call).
    pub fn nest(ep: i32, inner: i32, id: usize, _first: bool) {
        ctl(ep, inner, (libc::EPOLLIN | libc::EPOLLONESHOT) as u32, id as u64);
    }
    pub fn wait(ep: i32, timeout: Option<Duration>, out: &mut Vec<Ev>) {
        let mut evs: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
        let ms = timeout.map_or(-1, |t| {
            let ms = t.as_millis().min(i32::MAX as u128) as i32;
            if ms == 0 && !t.is_zero() { 1 } else { ms }
        });
        let n = unsafe { libc::epoll_wait(ep, evs.as_mut_ptr(), 64, ms) };
        for e in evs.iter().take(n.max(0) as usize) {
            let (id, m) = (e.u64 as usize, e.events);
            if id >= super::SHARED {
                out.push((id, true, false, false));
                continue;
            }
            let bad = m & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0;
            let r = m & (libc::EPOLLIN | libc::EPOLLRDHUP) as u32 != 0 || bad;
            let w = m & libc::EPOLLOUT as u32 != 0 || bad;
            out.push((id, r, w, false));
        }
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly",
    target_os = "linux",
    target_os = "android"
)))]
mod sys {
    use super::Ev;
    use std::time::Duration;
    pub const HAS_PROC: bool = false;
    pub fn create() -> i32 {
        -1
    }
    pub fn arm(_: i32, _: i32, _: bool, _: bool, _: bool) -> bool {
        false
    }
    pub fn rearm(_: i32, _: i32, _: bool, _: bool) {}
    pub fn forget(_: i32, _: i32) {}
    pub fn arm_proc(_: i32, _: i32) -> bool {
        false
    }
    pub fn add_kick(_: i32, _: usize) -> i32 {
        -2
    }
    pub fn kick(_: i32, _: i32, _: usize) {}
    pub fn drain_kick(_: i32) {}
    pub fn nest(_: i32, _: i32, _: usize, _: bool) {}
    pub fn wait(_: i32, _: Option<Duration>, _: &mut Vec<Ev>) {}
}
