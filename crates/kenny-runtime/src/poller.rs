//! The event loop behind waits on sockets, pipes and terminals: a thread that wants an fd
//! readable or writable registers it here and parks in the scheduler; one OS thread blocks in
//! kqueue (macOS, BSD) or epoll (Linux) and wakes the parked threads when their fds are ready.
//! Registrations are one-shot: each wait arms the fd again.
//!
//! `KENNY_NO_POLLER=1` turns it off (waits then poll the fds, as before).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// The waits on one fd: the keys (see `next_key`) of the threads waiting for each direction.
#[derive(Default)]
struct Waits {
    read: Vec<usize>,
    write: Vec<usize>,
}

struct Poller {
    fd: i32,
    waits: Mutex<HashMap<i32, Waits>>,
}

static POLLER: OnceLock<Option<Poller>> = OnceLock::new();

fn get() -> Option<&'static Poller> {
    POLLER
        .get_or_init(|| {
            if std::env::var_os("KENNY_NO_POLLER").is_some() {
                return None;
            }
            let fd = sys::create();
            if fd < 0 {
                return None;
            }
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            std::thread::Builder::new().name("kenny-poller".into()).spawn(move || run(fd)).ok()?;
            Some(Poller { fd, waits: Mutex::new(HashMap::new()) })
        })
        .as_ref()
}

/// A key for one wait: odd, so it never equals an object a thread can also block on.
pub fn next_key() -> usize {
    static N: AtomicUsize = AtomicUsize::new(1);
    (N.fetch_add(1, Ordering::Relaxed) << 4) | 1
}

/// Wake `key` when one of `fds` (fd, wants writing) is ready. False: the poller is off or
/// can't watch one of them (nothing stays registered then).
pub fn register(fds: &[(i32, bool)], key: usize) -> bool {
    let Some(p) = get() else { return false };
    let mut w = p.waits.lock().unwrap_or_else(|e| e.into_inner());
    for (i, &(fd, write)) in fds.iter().enumerate() {
        let e = w.entry(fd).or_default();
        if write { e.write.push(key) } else { e.read.push(key) }
        let (r, wr) = (!e.read.is_empty(), !e.write.is_empty());
        if !sys::arm(p.fd, fd, write, r, wr) {
            drop_key(&mut w, &fds[..=i], key);
            return false;
        }
    }
    true
}

/// The wait `key` is over (woken, timed out or interrupted).
pub fn deregister(fds: &[(i32, bool)], key: usize) {
    let Some(p) = get() else { return };
    let mut w = p.waits.lock().unwrap_or_else(|e| e.into_inner());
    drop_key(&mut w, fds, key);
}

fn drop_key(w: &mut HashMap<i32, Waits>, fds: &[(i32, bool)], key: usize) {
    for &(fd, _) in fds {
        if let Some(e) = w.get_mut(&fd) {
            e.read.retain(|&k| k != key);
            e.write.retain(|&k| k != key);
            if e.read.is_empty() && e.write.is_empty() {
                w.remove(&fd);
            }
        }
    }
}

/// The poller thread: wait for events, wake the threads waiting on them.
fn run(pfd: i32) {
    let mut ready: Vec<(i32, bool, bool)> = Vec::with_capacity(64);
    let mut wake: Vec<usize> = Vec::with_capacity(64);
    loop {
        ready.clear();
        sys::wait(pfd, &mut ready);
        let Some(p) = get() else { return };
        {
            let mut w = p.waits.lock().unwrap_or_else(|e| e.into_inner());
            for &(fd, r, wr) in &ready {
                let Some(e) = w.get_mut(&fd) else { continue };
                if r {
                    wake.append(&mut e.read);
                }
                if wr {
                    wake.append(&mut e.write);
                }
                if e.read.is_empty() && e.write.is_empty() {
                    w.remove(&fd);
                } else {
                    // (epoll disarmed the fd: arm it again for the waits left)
                    sys::rearm(pfd, fd, !e.read.is_empty(), !e.write.is_empty());
                }
            }
        }
        for k in wake.drain(..) {
            crate::thread::wake_io(k);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
mod sys {
    pub fn create() -> i32 {
        unsafe { libc::kqueue() }
    }
    fn change(fd: i32, write: bool) -> libc::kevent {
        let mut k: libc::kevent = unsafe { std::mem::zeroed() };
        k.ident = fd as libc::uintptr_t;
        k.filter = if write { libc::EVFILT_WRITE } else { libc::EVFILT_READ };
        k.flags = libc::EV_ADD | libc::EV_ONESHOT;
        k
    }
    /// Arm the filter for `write` (kqueue keeps one per direction).
    pub fn arm(kq: i32, fd: i32, write: bool, _read_any: bool, _write_any: bool) -> bool {
        let ch = change(fd, write);
        loop {
            let n = unsafe { libc::kevent(kq, &ch, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
            if n >= 0 {
                return true;
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                return false;
            }
        }
    }
    /// (a one-shot kqueue filter only disarms its own direction)
    pub fn rearm(_kq: i32, _fd: i32, _read: bool, _write: bool) {}
    pub fn wait(kq: i32, out: &mut Vec<(i32, bool, bool)>) {
        let mut evs: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
        let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, evs.as_mut_ptr(), 64, std::ptr::null()) };
        for e in evs.iter().take(n.max(0) as usize) {
            let fd = e.ident as i32;
            let err = e.flags & libc::EV_ERROR != 0;
            let write = e.filter == libc::EVFILT_WRITE;
            out.push((fd, !write || err, write || err));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    pub fn create() -> i32 {
        unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) }
    }
    fn set(ep: i32, fd: i32, read: bool, write: bool) -> bool {
        let mut ev = libc::epoll_event {
            events: (if read { (libc::EPOLLIN | libc::EPOLLRDHUP) as u32 } else { 0 })
                | (if write { libc::EPOLLOUT as u32 } else { 0 })
                | libc::EPOLLONESHOT as u32,
            u64: fd as u64,
        };
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
    /// epoll has one registration per fd: arm it for every direction someone waits for.
    pub fn arm(ep: i32, fd: i32, _write: bool, read_any: bool, write_any: bool) -> bool {
        set(ep, fd, read_any, write_any)
    }
    pub fn rearm(ep: i32, fd: i32, read: bool, write: bool) {
        set(ep, fd, read, write);
    }
    pub fn wait(ep: i32, out: &mut Vec<(i32, bool, bool)>) {
        let mut evs: [libc::epoll_event; 64] = unsafe { std::mem::zeroed() };
        let n = unsafe { libc::epoll_wait(ep, evs.as_mut_ptr(), 64, -1) };
        for e in evs.iter().take(n.max(0) as usize) {
            let (fd, m) = (e.u64 as i32, e.events);
            let bad = m & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0;
            let r = m & (libc::EPOLLIN | libc::EPOLLRDHUP) as u32 != 0 || bad;
            let w = m & libc::EPOLLOUT as u32 != 0 || bad;
            out.push((fd, r, w));
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
    pub fn create() -> i32 {
        -1
    }
    pub fn arm(_: i32, _: i32, _: bool, _: bool, _: bool) -> bool {
        false
    }
    pub fn rearm(_: i32, _: i32, _: bool, _: bool) {}
    pub fn wait(_: i32, _: &mut Vec<(i32, bool, bool)>) {}
}
