//! Lightweight threads (goroutine-style) on several OS threads, and Fibers.
//!
//! Every Ruby `Thread` runs on its own small stack (a stackful coroutine). Threads are spread
//! over up to KENNY_THREADS OS threads ("Ps", default: one per core), each with its own run
//! queue. A P switches between its threads when one blocks (sleep, join, Queue#pop,
//! Mutex#lock, CV#wait) and, for fairness, every 10ms at a safepoint. A P with nothing to
//! run takes a waiting thread from a busy P's queue (work stealing), and a P about to block
//! in a system call (file IO, SQLite) hands its waiting threads to an idle or spare P first.
//! A thread can therefore resume on a different OS thread than it suspended on: runtime code
//! never keeps a pointer to P state (`pst()`) across a call into Ruby, and per-thread state
//! (pending throw, `$!`, definees, saved throws, the Fiber stack) moves with the thread. There is no
//! GVL: threads on different Ps run Ruby code at the same time (see LANGUAGE.md rule 4 for
//! what keeps that memory-safe). A program that never starts a thread runs on one P and pays
//! nothing for any of this.
//!
//! The existing Ruby APIs are the interface: Thread, Queue, SizedQueue, Mutex,
//! ConditionVariable, Monitor, Fiber. No new syntax or API.
//!
//! Locking: all shared scheduler state is in `SCHED` (a runtime lock). It is held only for
//! short sections that never call Ruby, allocate nothing that could collect, and never switch
//! coroutines.

use crate::core::*;
use crate::gc::RVec;
use crate::kernel::new_str_v;
use crate::mt::L;
use crate::tri;
use corosensei::stack::{DefaultStack, Stack};
use corosensei::{Coroutine, CoroutineResult, Yielder};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const THREAD_STACK: usize = 16 << 20;
const MAIN_STACK: usize = 1 << 30;
/// How much of it Ruby code may use before SystemStackError: several hundred thousand frames
/// (CRuby stops near ten thousand), but not so deep that runaway recursion takes minutes to
/// fail (every GC scans the whole stack).
const MAIN_USABLE: usize = 64 << 20;
const FIBER_STACK: usize = 8 << 20;
const STACK_MARGIN: usize = 1 << 20;
const P_STACK: usize = 8 << 20;
pub const PREEMPT_BIT: u8 = 2;

type ThreadCo = Coroutine<(), (), V, DefaultStack>;
type FiberCo = Coroutine<V, V, V, DefaultStack>;

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Runnable,
    Running,
    Blocked,
    Done,
}

/// A stack the GC must scan (thread or fiber).
struct Ctx {
    base: usize,
    limit: usize,
    saved_sp: usize,
    live: bool,
    /// Executing on some P right now (the GC scans it through that P instead).
    running: bool,
}

struct GThread {
    obj: V,
    co: Option<ThreadCo>,
    yielder: *const Yielder<(), ()>,
    state: St,
    /// The P this thread runs on.
    p: usize,
    wake_at: Option<Instant>,
    timed_out: bool,
    value: V,
    exc: V,
    ctx: usize,
    /// Stays on its P (the main thread, the HTTP server's loops).
    pinned: bool,
    // per-thread runtime state, swapped on switch (only touched by the thread's own P)
    pending: Throw,
    errinfo: V,
    last_match: V,
    definee: Vec<*mut Class>,
    dm_stack: Vec<*const Method>,
    super_at: Vec<(V, u32, usize)>,
    current_block: Vec<V>,
    saved_throws: crate::gc::SavedThrows,
    fiber_stack: Vec<usize>,
    // set by other threads (under the lock)
    interrupt: Option<Throw>,
    has_interrupt: AtomicBool,
    waiting_on: usize,
}

struct GFiber {
    co: Option<FiberCo>,
    yielder: *const Yielder<V, V>,
    ctx: usize,
    started: bool,
    done: bool,
    resumer_ctx: usize,
    /// the Fiber object (for Fiber.current)
    obj: V,
}

/// Wakes an idle P.
struct Parker {
    m: Mutex<bool>,
    cv: Condvar,
    /// Set while the P's loop waits in its own poller instead (the HTTP server's event
    /// loop): wakes that poller when the P gets work (a new thread, a thread woken up).
    hook: Mutex<Option<WakeHook>>,
    has_hook: AtomicBool,
}
type WakeHook = Box<dyn Fn() + Send + Sync>;
impl Parker {
    fn new() -> Self {
        Parker { m: Mutex::new(false), cv: Condvar::new(), hook: Mutex::new(None), has_hook: AtomicBool::new(false) }
    }
    fn unpark(&self) {
        *self.m.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_one();
        if let Some(h) = &*self.hook.lock().unwrap_or_else(|e| e.into_inner()) {
            h();
        }
    }
    /// Wait until unparked (or the timeout); counts as idle for the GC.
    fn park(&self, timeout: Option<Duration>) {
        crate::gc::blocking(|| {
            let mut g = self.m.lock().unwrap_or_else(|e| e.into_inner());
            if !*g {
                g = match timeout {
                    Some(t) => self.cv.wait_timeout(g, t).unwrap_or_else(|e| e.into_inner()).0,
                    None => self.cv.wait(g).unwrap_or_else(|e| e.into_inner()),
                };
            }
            *g = false;
        });
    }
}

struct PSlot {
    parker: Arc<Parker>,
    started: bool,
    runq: VecDeque<usize>,
    /// Threads of this P blocked with a timeout (checked by its loop).
    sleepers: Vec<usize>,
    nthreads: usize,
    /// Parked in its loop with nothing to run (a candidate to steal work).
    idle: bool,
}

struct Sched {
    threads: Vec<Box<GThread>>,
    ctxs: Vec<Ctx>,
    waiters: HashMap<usize, VecDeque<usize>>,
    fibers: Vec<Box<GFiber>>,
    /// `max_ps` Ps for running threads, then as many spares: a spare starts when a P is about
    /// to block in a system call while threads wait in its queue and no P is idle.
    ps: Vec<PSlot>,
    max_ps: usize,
    next_p: usize,
    timer_started: bool,
    thread_class: *mut Class,
    fiber_class: *mut Class,
    fatal: *mut Class,
    thread_error: *mut Class,
    fiber_error: *mut Class,
}

static SCHED: L<Option<Sched>> = L::new(None);
/// Live threads per P, readable without the lock (the HTTP server asks on every loop).
static P_NTHREADS: [std::sync::atomic::AtomicUsize; 256] = [const { std::sync::atomic::AtomicUsize::new(0) }; 256];
/// KENNY_SCHED_STATS=1: threads taken by idle Ps, handed off before blocking calls, spare Ps
/// started (printed at exit).
static STEALS: AtomicU64 = AtomicU64::new(0);
static HANDOFFS: AtomicU64 = AtomicU64::new(0);
static SPARES: AtomicU64 = AtomicU64::new(0);
extern "C" fn print_sched_stats() {
    eprintln!(
        "sched: {} steals, {} handoffs before blocking calls, {} spare Ps started",
        STEALS.load(Ordering::Relaxed),
        HANDOFFS.load(Ordering::Relaxed),
        SPARES.load(Ordering::Relaxed)
    );
}
unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}
/// Length of each P's run queue, readable without the lock (checked before blocking calls).
static P_RUNQ: [std::sync::atomic::AtomicUsize; 256] = [const { std::sync::atomic::AtomicUsize::new(0) }; 256];

/// This P's view of the scheduler (part of PState).
pub struct PThr {
    pub idx: usize,
    current: Option<usize>,
    cur: *mut GThread,
    running_ctx: Option<usize>,
    fiber_stack: Vec<usize>,
    /// Top of the P's own OS-thread stack (where its loop runs).
    os_base: usize,
    seen_tick: u64,
}
impl Default for PThr {
    fn default() -> Self {
        PThr { idx: 0, current: None, cur: std::ptr::null_mut(), running_ctx: None, fiber_stack: Vec::new(), os_base: 0, seen_tick: 0 }
    }
}
fn pt() -> &'static mut PThr {
    &mut pst().thr
}

macro_rules! locked {
    ($s:ident => $body:expr) => {{
        let mut __g = SCHED.get();
        let $s: &mut Sched = __g.as_mut().expect("scheduler not initialised");
        $body
    }};
}

/// Unlocked access: for the GC (other threads stopped) and for single-P setup.
fn sraw() -> Option<&'static mut Sched> {
    SCHED.raw().as_mut()
}

pub fn active() -> bool {
    pt().current.is_some()
}

#[inline(always)]
fn approx_sp() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

fn enter_ctx(c: usize) {
    let (base, limit) = locked!(s => {
        let ctx = &mut s.ctxs[c];
        ctx.running = true;
        (ctx.base, ctx.limit)
    });
    pt().running_ctx = Some(c);
    pst().heap.stack_base = base;
    crate::core::set_stack_limit(limit + STACK_MARGIN);
}
/// Mark ctx `c` as suspended at the current stack position.
fn leave_ctx(c: usize) {
    let sp = approx_sp();
    locked!(s => {
        let ctx = &mut s.ctxs[c];
        ctx.saved_sp = sp;
        ctx.running = false;
    });
}

/// Stacks of suspended threads and fibers, for the GC (every other P is stopped).
fn suspended_stacks() -> Vec<(usize, usize)> {
    let Some(s) = sraw() else { return vec![] };
    s.ctxs.iter().filter(|c| c.live && !c.running && c.saved_sp != 0).map(|c| (c.saved_sp, c.base)).collect()
}

pub fn static_roots(out: &mut Vec<V>) {
    let Some(s) = sraw() else { return };
    out.extend(s.fibers.iter().filter(|f| !f.done).map(|f| f.obj));
    for t in &s.threads {
        out.extend([t.obj, t.value, t.exc, t.errinfo, t.last_match]);
        out.extend(t.current_block.iter().copied());
        crate::gc::definee_values(&t.definee, &t.dm_stack, out);
        for th in [Some(t.pending), t.interrupt].into_iter().flatten() {
            crate::gc::throw_values(th, out);
        }
        for &th in t.saved_throws.map.values() {
            crate::gc::throw_values(th, out);
        }
    }
}

/// (lowest, highest) address of the stack Ruby code is running on (for backtraces).
pub fn current_stack() -> Option<(usize, usize)> {
    let hi = pst().heap.stack_base;
    if hi == 0 { None } else { Some((approx_sp(), hi)) }
}

fn new_ctx(s: &mut Sched, base: usize, limit: usize) -> usize {
    s.ctxs.push(Ctx { base, limit, saved_sp: 0, live: true, running: false });
    s.ctxs.len() - 1
}

// ---------- per-thread state ----------

fn save_state(t: &mut GThread) {
    let p = pst();
    t.pending = std::mem::replace(&mut p.pending, Throw::None);
    t.errinfo = p.errinfo;
    t.last_match = p.last_match;
    t.definee = std::mem::take(&mut p.definee);
    t.dm_stack = std::mem::take(&mut p.dm_stack);
    t.super_at = std::mem::take(&mut p.super_at);
    t.current_block = std::mem::take(&mut p.current_block);
    t.saved_throws = std::mem::take(&mut p.heap.saved_throws);
    t.fiber_stack = std::mem::take(&mut p.thr.fiber_stack);
}
fn restore_state(t: &mut GThread) {
    let p = pst();
    p.pending = std::mem::replace(&mut t.pending, Throw::None);
    p.errinfo = t.errinfo;
    p.last_match = t.last_match;
    p.definee = std::mem::take(&mut t.definee);
    p.dm_stack = std::mem::take(&mut t.dm_stack);
    p.super_at = std::mem::take(&mut t.super_at);
    p.current_block = std::mem::take(&mut t.current_block);
    p.heap.saved_throws = std::mem::take(&mut t.saved_throws);
    p.thr.fiber_stack = std::mem::take(&mut t.fiber_stack);
}

/// Give control back to this P's loop. The caller has set the thread's state (under the lock).
fn suspend_current() {
    let p = pt();
    if p.current.is_none() {
        return;
    }
    let t = unsafe { &mut *p.cur };
    leave_ctx(t.ctx);
    save_state(t);
    let y = t.yielder;
    unsafe { (*y).suspend(()) };
    // resumed: the loop restored our state
}

/// Run `main` as the main thread; returns when it finishes. Called by kn_start.
pub fn run_main(main: Box<dyn FnOnce() -> V>) -> V {
    let r = rt();
    let thread_class = const_lookup(r.c.object, sym("Thread")).unwrap() as *mut Class;
    let fiber_class = const_lookup(r.c.object, sym("Fiber")).unwrap() as *mut Class;
    let fatal = new_class("fatal", r.c.exception);
    let thread_error = const_lookup(r.c.object, sym("ThreadError")).unwrap() as *mut Class;
    let fiber_error = const_lookup(r.c.object, sym("FiberError")).unwrap() as *mut Class;
    let max_ps = std::env::var("KENNY_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            // Rails' development mode (autoloading and code reloading) counts on a GVL: two
            // requests autoloading the same module at once each create their own. One P
            // gives the same one-at-a-time interleaving; KENNY_THREADS overrides it.
            let env = std::env::var("RAILS_ENV").or_else(|_| std::env::var("RACK_ENV")).unwrap_or_default();
            if env == "development" {
                return 1;
            }
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
        })
        .min(256);
    *SCHED.get() = Some(Sched {
        threads: Vec::new(),
        ctxs: Vec::new(),
        waiters: HashMap::new(),
        fibers: Vec::new(),
        ps: (0..(max_ps * 2).min(256))
            .map(|i| PSlot { parker: Arc::new(Parker::new()), started: i == 0, runq: VecDeque::new(), sleepers: Vec::new(), nthreads: 0, idle: false })
            .collect(),
        max_ps,
        // new threads go to the other Ps first (P 0 runs the main thread)
        next_p: 1,
        timer_started: false,
        thread_class,
        fiber_class,
        fatal,
        thread_error,
        fiber_error,
    });
    if std::env::var_os("KENNY_SCHED_STATS").is_some() {
        unsafe { atexit(print_sched_stats) };
    }
    crate::gc::heap().stack_provider = Some(suspended_stacks);
    crate::gc::heap().extra_roots.push(static_roots);
    let x = 0u8;
    pt().idx = 0;
    pt().os_base = std::hint::black_box(&x) as *const u8 as usize + 256;
    spawn_raw(Box::new(move |_| main()), MAIN_STACK, Some(0));
    p_loop(0);
    let main = locked!(s => {
        let m = &mut s.threads[0];
        (std::mem::replace(&mut m.pending, Throw::None), m.value)
    });
    pst().pending = main.0;
    pt().current = None;
    pt().running_ctx = None;
    main.1
}

fn thread_obj(tid: usize) -> V {
    let o = object_new(sraw().unwrap().thread_class);
    ivar_set(o, sym("@__tid"), mkfix(tid as i64));
    ivar_set(o, sym("@__locals"), hash_new());
    ivar_set(o, sym("@__vars"), hash_new());
    o
}

/// Create a thread running `f`, on P `on` (or the next P in turn). Returns its id.
fn spawn_raw(f: Box<dyn FnOnce(usize) -> V>, stack_size: usize, on: Option<usize>) -> usize {
    let stack = DefaultStack::new(stack_size).expect("thread stack");
    let (base, limit) = (stack.base().get(), stack.limit().get());
    let limit = if stack_size == MAIN_STACK { limit.max(base.saturating_sub(MAIN_USABLE)) } else { limit };
    // pick the P; starting a second P switches the runtime into multi-threaded mode first
    let p = on.unwrap_or_else(|| locked!(s => {
        let p = s.next_p % s.max_ps;
        s.next_p = s.next_p.wrapping_add(1);
        p
    }));
    let start_p = locked!(s => !s.ps[p].started);
    if start_p {
        crate::mt::set_multi();
    }
    let (tid, parker) = locked!(s => {
        let tid = s.threads.len();
        // (allocating never collects, so this is fine under the lock)
        let obj = thread_obj(tid);
        let ctx = new_ctx(s, base, limit);
        let co: ThreadCo = Coroutine::with_stack(stack, move |y: &Yielder<(), ()>, _| {
            let me = locked!(s => &mut *s.threads[tid] as *mut GThread);
            unsafe { (*me).yielder = y as *const _ };
            f(tid)
        });
        s.threads.push(Box::new(GThread {
            obj,
            co: Some(co),
            yielder: std::ptr::null(),
            state: St::Runnable,
            p,
            wake_at: None,
            timed_out: false,
            value: QNIL,
            exc: QNIL,
            ctx,
            pending: Throw::None,
            errinfo: QNIL,
            last_match: QNIL,
            definee: Vec::new(),
            dm_stack: Vec::new(),
            super_at: Vec::new(),
            current_block: Vec::new(),
            interrupt: None,
            has_interrupt: AtomicBool::new(false),
            waiting_on: 0,
            pinned: on.is_some(),
            saved_throws: Default::default(),
            fiber_stack: Vec::new(),
        }));
        s.ps[p].nthreads += 1;
        P_NTHREADS[p].fetch_add(1, Ordering::Relaxed);
        let kick = push_runq(s, p, tid);
        (tid, kick)
    });
    if start_p {
        let started = locked!(s => std::mem::replace(&mut s.ps[p].started, true));
        if !started {
            start_os_thread(p);
        }
    }
    // push_runq claimed a P to run or steal the thread (this one, or an idle one it kicked):
    // it must be woken even when this call also started P `p`, or it stays claimed but asleep
    if let Some(pk) = parker {
        pk.unpark();
    }
    if tid == 1 {
        start_timer();
    }
    tid
}

/// Start P `idx`: an OS thread with its own interpreter state running its loop.
fn start_os_thread(idx: usize) {
    std::thread::Builder::new()
        .name(format!("kenny-p{idx}"))
        .stack_size(P_STACK)
        .spawn(move || {
            let x = 0u8;
            new_pstate();
            let base = std::hint::black_box(&x) as *const u8 as usize + 256;
            pst().heap.stack_base = base;
            pt().idx = idx;
            pt().os_base = base;
            p_loop(idx);
        })
        .expect("failed to start an OS thread for Ruby threads");
}

/// Ticks of the 10ms fairness timer.
static TICK: AtomicU64 = AtomicU64::new(0);

fn start_timer() {
    let first = locked!(s => !std::mem::replace(&mut s.timer_started, true));
    if !first {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(10));
        TICK.fetch_add(1, Ordering::Relaxed);
        crate::gc::KN_GC_PENDING.fetch_or(PREEMPT_BIT, Ordering::Relaxed);
        if crate::mt::multi() {
            // every P gets a chance to see it; then stop sending them to the slow path
            std::thread::sleep(Duration::from_millis(1));
            crate::gc::KN_GC_PENDING.fetch_and(!PREEMPT_BIT, Ordering::Relaxed);
        }
    });
}

enum Pick {
    Run(usize),
    Wait(Option<Duration>),
    Exit,
}

/// A P's scheduler loop. P 0 returns when the main thread is done; others run forever.
fn p_loop(idx: usize) {
    loop {
        let pick = locked!(s => {
            if idx == 0 && s.threads[0].state == St::Done {
                Pick::Exit
            } else {
                let now = Instant::now();
                s.ps[idx].idle = false;
                wake_due_sleepers(s, idx, now);
                let mut pick = None;
                while let Some(tid) = pop_runq(s, idx) {
                    if s.threads[tid].state == St::Runnable && s.threads[tid].p == idx {
                        pick = Some(tid);
                        break;
                    }
                }
                if pick.is_none() && idx < s.max_ps {
                    pick = steal(s, idx);
                }
                if pick.is_none() {
                    s.ps[idx].idle = true;
                }
                match pick {
                    Some(t) => Pick::Run(t),
                    None => {
                        let earliest = s.ps[idx].sleepers.iter().filter_map(|&t| s.threads[t].wake_at).min();
                        match earliest {
                            Some(w) => Pick::Wait(Some(w.saturating_duration_since(now))),
                            None if idx == 0 && deadlocked(s) => {
                                // every thread is blocked forever: deadlock
                                let e = exc_new(s.fatal, "No live threads left. Deadlock?");
                                let main = &mut s.threads[0];
                                main.interrupt = Some(Throw::Raise(e));
                                main.has_interrupt.store(true, Ordering::Release);
                                main.state = St::Runnable;
                                s.ps[idx].idle = false;
                                Pick::Run(0)
                            }
                            // P 0 checks for deadlock now and then
                            None if idx == 0 => Pick::Wait(Some(Duration::from_millis(50))),
                            None => Pick::Wait(None),
                        }
                    }
                }
            }
        });
        match pick {
            Pick::Exit => return,
            Pick::Run(i) => {
                resume_thread(i);
                if idx >= sraw().map(|s| s.max_ps).unwrap_or(usize::MAX) {
                    give_back(i);
                }
            }
            Pick::Wait(t) => {
                crate::kernel::flush_out();
                let parker = locked!(s => s.ps[idx].parker.clone());
                parker.park(t);
            }
        }
    }
}

/// Make this P's sleepers whose time has come runnable (lock held).
fn wake_due_sleepers(s: &mut Sched, idx: usize, now: Instant) {
    let mut i = 0;
    while i < s.ps[idx].sleepers.len() {
        let tid = s.ps[idx].sleepers[i];
        let t = &mut s.threads[tid];
        if t.state != St::Blocked || t.wake_at.is_none() || t.p != idx {
            s.ps[idx].sleepers.swap_remove(i);
            continue;
        }
        if t.wake_at.is_some_and(|w| w <= now) {
            t.state = St::Runnable;
            t.timed_out = true;
            t.wake_at = None;
            s.ps[idx].sleepers.swap_remove(i);
            s.ps[idx].runq.push_back(tid);
            P_RUNQ[idx].store(s.ps[idx].runq.len(), Ordering::Relaxed);
            continue;
        }
        i += 1;
    }
}

/// No thread can ever run again (all blocked without a timeout, none running).
fn deadlocked(s: &Sched) -> bool {
    s.threads.iter().all(|t| t.state == St::Done || (t.state == St::Blocked && t.wake_at.is_none() && !t.has_interrupt.load(Ordering::Relaxed)))
        && s.threads[0].state == St::Blocked
}

fn resume_thread(i: usize) {
    let (tp, ctx, co) = locked!(s => {
        let t = &mut s.threads[i];
        t.state = St::Running;
        (&mut **t as *mut GThread, t.ctx, t.co.take())
    });
    let Some(mut co) = co else { return };
    let p = pt();
    p.current = Some(i);
    p.cur = tp;
    let t = unsafe { &mut *tp };
    restore_state(t);
    // (a pending Thread#raise is picked up where the thread resumes: finish_block, pass, or
    // its next safepoint)
    enter_ctx(ctx);
    let res = co.resume(());
    // back on the P's own stack
    let p = pt();
    p.running_ctx = None;
    p.current = None;
    p.cur = std::ptr::null_mut();
    pst().heap.stack_base = p.os_base;
    match res {
        CoroutineResult::Yield(()) => {
            let idx = p.idx;
            let pk = locked!(s => {
                let t = &mut s.threads[i];
                t.co = Some(co);
                s.ctxs[ctx].running = false;
                // it waits in a queue (preempted, or woken while it was suspending): now
                // that it is off this stack, an idle P may take it
                if s.threads[i].state == St::Runnable && stealable(&s.threads[i]) { kick_idle(s, idx) } else { None }
            });
            if let Some(pk) = pk {
                pk.unpark();
            }
        }
        CoroutineResult::Return(v) => {
            // the coroutine finished: its stack can go
            drop(co);
            let pending = std::mem::replace(&mut pst().pending, Throw::None);
            let mut report = None;
            let mut wake = Vec::new();
            locked!(s => {
                s.ctxs[ctx].live = false;
                s.ctxs[ctx].running = false;
                let t = &mut s.threads[i];
                t.value = if v == UNDEF { QNIL } else { v };
                if v == UNDEF {
                    if let Throw::Raise(e) = pending {
                        t.exc = e;
                        if i != 0 {
                            report = Some((t.obj, e));
                        }
                    }
                    if i == 0 {
                        t.pending = pending;
                    }
                } else {
                    t.pending = pending;
                }
                t.state = St::Done;
                let pi = t.p;
                let obj = t.obj;
                s.ps[pi].nthreads -= 1;
                P_NTHREADS[pi].fetch_sub(1, Ordering::Relaxed);
                // abort_on_exception (the thread's, or Thread.abort_on_exception): the exception
                // is raised in the main thread, in the same step (a main thread blocked in a
                // wait would otherwise be found deadlocked)
                if let Some((_, e)) = report {
                    let abort = truthy(ivar_get(obj, sym("@__abort"))) || truthy(crate::extra::thread_abort_default());
                    if abort && class_name(real_class(e)) != "SystemExit" && s.threads[0].state != St::Done {
                        let m = &mut s.threads[0];
                        m.interrupt = Some(Throw::Raise(e));
                        m.has_interrupt.store(true, Ordering::Release);
                        let mp = m.p;
                        match s.threads[0].state {
                            St::Blocked => wake.extend(make_runnable(s, 0)),
                            _ => wake.push(s.ps[mp].parker.clone()),
                        }
                        crate::gc::KN_GC_PENDING.fetch_or(PREEMPT_BIT, Ordering::Relaxed);
                    }
                }
                // wake the joiners in the same step: between "done" and "joiners woken", P 0's
                // deadlock check would see every thread finished or blocked
                loop {
                    match wake_one_locked(s, obj) {
                        (true, pk) => wake.extend(pk),
                        (false, _) => break,
                    }
                }
            });
            for pk in wake {
                pk.unpark();
            }
            if let Some((obj, e)) = report {
                // (a thread ending by `exit` isn't reported, as in CRuby)
                if truthy(ivar_get(obj, sym("@__report"))) && class_name(real_class(e)) != "SystemExit" {
                    report_thread_exception(obj, e);
                    pst().pending = Throw::None;
                }
            }
        }
    }
}

fn report_thread_exception(obj: V, e: V) {
    let m = send0(e, "message");
    let m = if is_str(m) { String::from_utf8_lossy(&rstr(m).b).into_owned() } else { String::new() };
    crate::kernel::err_out(
        format!(
            "#<Thread:0x{:016x} run> terminated with exception (report_on_exception is true):\n{} ({})\n",
            obj,
            m,
            class_name(real_class(e))
        )
        .as_bytes(),
    );
}

// ---------- blocking ----------

enum Wake {
    Woken,
    Timeout,
    Interrupted,
}

/// With the lock held: mark the current thread blocked on `on` (until woken or the timeout).
/// Returns a pending Thread#raise/kill instead if there is one (then don't suspend).
fn prepare_block(s: &mut Sched, on: V, timeout: Option<Duration>) -> Option<Throw> {
    let cur = pt().current.expect("blocking outside a thread");
    let t = &mut s.threads[cur];
    if let Some(i) = t.interrupt.take() {
        t.has_interrupt.store(false, Ordering::Release);
        return Some(i);
    }
    t.state = St::Blocked;
    t.wake_at = timeout.map(|d| Instant::now() + d);
    t.timed_out = false;
    t.waiting_on = on as usize;
    let p = t.p;
    if timeout.is_some() {
        s.ps[p].sleepers.push(cur);
    }
    if on != QNIL {
        s.waiters.entry(on as usize).or_default().push_back(cur);
    }
    None
}
/// After being resumed from a block: clean up and say why we woke.
fn finish_block(on: V) -> Wake {
    let cur = pt().current.unwrap();
    let (intr, timed_out) = locked!(s => {
        if on != QNIL {
            if let Some(q) = s.waiters.get_mut(&(on as usize)) {
                q.retain(|x| *x != cur);
                if q.is_empty() {
                    s.waiters.remove(&(on as usize));
                }
            }
        }
        let t = &mut s.threads[cur];
        t.waiting_on = 0;
        let i = t.interrupt.take();
        t.has_interrupt.store(false, Ordering::Release);
        (i, std::mem::replace(&mut t.timed_out, false))
    });
    if let Some(i) = intr {
        pst().pending = i;
        return Wake::Interrupted;
    }
    if timed_out {
        return Wake::Timeout;
    }
    Wake::Woken
}

/// Block the current thread on `on` until woken (or timeout). Must be called inside a thread.
fn block_on(on: V, timeout: Option<Duration>) -> Wake {
    if !active() {
        return Wake::Timeout;
    }
    if let Some(i) = locked!(s => prepare_block(s, on, timeout)) {
        pst().pending = i;
        return Wake::Interrupted;
    }
    suspend_current();
    finish_block(on)
}

/// How `wait_io` ended.
pub enum IoWake {
    /// woken by the poller (the fds may be ready: poll them to see)
    Ready,
    Timeout,
    /// a Thread#raise / kill is pending
    Interrupted,
    /// can't park here (no scheduler, inside a Fiber) or `register` failed: poll instead
    Unsupported,
}

/// Park the current thread until `wake_io(key)` (from the poller) or the timeout. `register`
/// arms the poller for `key`; it runs once the thread counts as blocked, so a wake that comes
/// before the thread is off its stack isn't lost.
pub fn wait_io(key: usize, timeout: Option<Duration>, register: impl FnOnce() -> bool) -> IoWake {
    if !active() || !pt().fiber_stack.is_empty() {
        return IoWake::Unsupported;
    }
    let on = key as V;
    if let Some(i) = locked!(s => prepare_block(s, on, timeout)) {
        pst().pending = i;
        return IoWake::Interrupted;
    }
    let ok = register();
    if !ok {
        // (undo the block: runnable again, back as soon as it suspends)
        wake_one(on);
    }
    suspend_current();
    match finish_block(on) {
        Wake::Interrupted => IoWake::Interrupted,
        _ if !ok => IoWake::Unsupported,
        Wake::Timeout => IoWake::Timeout,
        Wake::Woken => IoWake::Ready,
    }
}

/// Wake the thread parked in `wait_io(key)` (any OS thread may call it).
pub fn wake_io(key: usize) {
    wake_all(key as V);
}

/// Make thread `tid` runnable (lock held); returns the parker to poke if it is on another P.
fn make_runnable(s: &mut Sched, tid: usize) -> Option<Arc<Parker>> {
    let t = &mut s.threads[tid];
    if t.state != St::Blocked {
        return None;
    }
    t.state = St::Runnable;
    t.wake_at = None;
    let p = t.p;
    push_runq(s, p, tid)
}

/// Queue runnable thread `tid` on P `p` (lock held). Returns a parker to poke: P `p` if it
/// waits for work, else an idle P that can take the thread over.
fn push_runq(s: &mut Sched, p: usize, tid: usize) -> Option<Arc<Parker>> {
    s.ps[p].runq.push_back(tid);
    P_RUNQ[p].store(s.ps[p].runq.len(), Ordering::Relaxed);
    let me = pst_idx();
    if s.ps[p].idle {
        s.ps[p].idle = false;
        return if Some(p) != me { Some(s.ps[p].parker.clone()) } else { None };
    }
    // a P whose loop waits in its own poller (the HTTP server) isn't idle but may be blocked
    // there with no timeout, and in a server every P is such a loop: wake it to run the thread
    if Some(p) != me && s.ps[p].parker.has_hook.load(Ordering::Acquire) {
        return Some(s.ps[p].parker.clone());
    }
    if s.threads[tid].pinned { None } else { kick_idle(s, p) }
}
fn pop_runq(s: &mut Sched, p: usize) -> Option<usize> {
    let t = s.ps[p].runq.pop_front();
    P_RUNQ[p].store(s.ps[p].runq.len(), Ordering::Relaxed);
    t
}
/// The index of the P this OS thread is, if it is one.
fn pst_idx() -> Option<usize> {
    pst_opt().map(|p| p.thr.idx)
}
/// An idle P other than `busy` to wake (it will steal), claimed so the next kick picks another.
fn kick_idle(s: &mut Sched, busy: usize) -> Option<Arc<Parker>> {
    let n = s.max_ps;
    (0..n).map(|k| (busy + 1 + k) % n).find(|&v| v != busy && s.ps[v].started && s.ps[v].idle).map(|v| {
        s.ps[v].idle = false;
        s.ps[v].parker.clone()
    })
}
/// A thread waiting in some queue that may run on another P: not pinned, not running or about
/// to suspend (its coroutine is back), and not inside a Fiber (its Fiber stacks belong here).
fn stealable(t: &GThread) -> bool {
    static NO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *NO.get_or_init(|| std::env::var_os("KENNY_NO_STEAL").is_some()) {
        return false;
    }
    t.state == St::Runnable && !t.pinned && t.co.is_some() && t.fiber_stack.is_empty()
}
/// Move thread `tid` to P `to` (lock held): counts, and its timed wait if it has one.
fn move_thread(s: &mut Sched, tid: usize, to: usize) {
    let from = s.threads[tid].p;
    if from == to {
        return;
    }
    s.threads[tid].p = to;
    s.ps[from].nthreads -= 1;
    P_NTHREADS[from].fetch_sub(1, Ordering::Relaxed);
    s.ps[to].nthreads += 1;
    P_NTHREADS[to].fetch_add(1, Ordering::Relaxed);
    if let Some(i) = s.ps[from].sleepers.iter().position(|&t| t == tid) {
        s.ps[from].sleepers.swap_remove(i);
        s.ps[to].sleepers.push(tid);
    }
}
/// Take a waiting thread from another P's queue for idle P `idx` (lock held): the one that
/// has waited longest on the P with the most waiting.
fn steal(s: &mut Sched, idx: usize) -> Option<usize> {
    let victim = (0..s.ps.len())
        .filter(|&v| v != idx && s.ps[v].started && !s.ps[v].runq.is_empty())
        .max_by_key(|&v| s.ps[v].runq.len())?;
    let pos = s.ps[victim].runq.iter().position(|&t| stealable(&s.threads[t]))?;
    let tid = s.ps[victim].runq.remove(pos)?;
    P_RUNQ[victim].store(s.ps[victim].runq.len(), Ordering::Relaxed);
    move_thread(s, tid, idx);
    STEALS.fetch_add(1, Ordering::Relaxed);
    Some(tid)
}
/// A spare P ran thread `tid` until it suspended: hand it back to the least loaded regular P.
fn give_back(tid: usize) {
    let pk = locked!(s => {
        let t = &s.threads[tid];
        if t.state == St::Done || t.pinned || !t.fiber_stack.is_empty() {
            return;
        }
        let me = t.p;
        let to = (0..s.max_ps).filter(|&v| s.ps[v].started).min_by_key(|&v| s.ps[v].nthreads).unwrap_or(0);
        let queued = s.ps[me].runq.iter().position(|&x| x == tid);
        if let Some(i) = queued {
            s.ps[me].runq.remove(i);
            P_RUNQ[me].store(s.ps[me].runq.len(), Ordering::Relaxed);
        }
        move_thread(s, tid, to);
        if queued.is_some() {
            push_runq(s, to, tid)
        } else if s.ps[to].parker.has_hook.load(Ordering::Acquire) {
            // a timed wait moved to a P blocked in its own poller: it must recompute its timeout
            Some(s.ps[to].parker.clone())
        } else {
            None
        }
    });
    if let Some(pk) = pk {
        pk.unpark();
    }
}

/// About to block in a system call (gc::blocking): if threads wait in this P's queue, let an
/// idle P take them, or start a spare P when every P is busy.
pub fn before_blocking() {
    let Some(p) = pst_opt() else { return };
    let idx = p.thr.idx;
    if p.thr.current.is_none() || P_RUNQ[idx].load(Ordering::Relaxed) == 0 {
        return;
    }
    let (pk, start) = locked!(s => {
        if !s.ps[idx].runq.iter().any(|&t| stealable(&s.threads[t])) {
            (None, None)
        } else if let Some(pk) = kick_idle(s, idx) {
            HANDOFFS.fetch_add(1, Ordering::Relaxed);
            (Some(pk), None)
        } else if let Some(v) = (s.max_ps..s.ps.len()).find(|&v| s.ps[v].idle || !s.ps[v].started) {
            // a spare: give it one waiting thread now (it doesn't steal on its own)
            let pos = s.ps[idx].runq.iter().position(|&t| stealable(&s.threads[t])).unwrap();
            let tid = s.ps[idx].runq.remove(pos).unwrap();
            P_RUNQ[idx].store(s.ps[idx].runq.len(), Ordering::Relaxed);
            move_thread(s, tid, v);
            HANDOFFS.fetch_add(1, Ordering::Relaxed);
            s.ps[v].runq.push_back(tid);
            P_RUNQ[v].store(s.ps[v].runq.len(), Ordering::Relaxed);
            if s.ps[v].started {
                s.ps[v].idle = false;
                (Some(s.ps[v].parker.clone()), None)
            } else {
                s.ps[v].started = true;
                SPARES.fetch_add(1, Ordering::Relaxed);
                (None, Some(v))
            }
        } else {
            (None, None)
        }
    });
    if let Some(pk) = pk {
        pk.unpark();
    }
    if let Some(v) = start {
        start_os_thread(v);
    }
}

fn wake_one_locked(s: &mut Sched, on: V) -> (bool, Option<Arc<Parker>>) {
    if let Some(q) = s.waiters.get_mut(&(on as usize)) {
        while let Some(tid) = q.pop_front() {
            let t = &s.threads[tid];
            if t.state == St::Blocked && t.waiting_on == on as usize {
                let pk = make_runnable(s, tid);
                return (true, pk);
            }
        }
    }
    (false, None)
}
fn wake_one(on: V) -> bool {
    let (w, pk) = locked!(s => wake_one_locked(s, on));
    if let Some(pk) = pk {
        pk.unpark();
    }
    w
}
fn wake_all(on: V) {
    while wake_one(on) {}
}

/// Let other threads of this P run (Thread.pass / preemption).
pub fn pass() {
    if !active() {
        return;
    }
    let p = pt();
    if !p.fiber_stack.is_empty() {
        return; // no thread switches while a Fiber runs on top of this thread
    }
    let cur = p.current.unwrap();
    let idx = p.idx;
    let switched = locked!(s => {
        if !s.ps[idx].sleepers.is_empty() {
            wake_due_sleepers(s, idx, Instant::now());
        }
        if s.ps[idx].runq.iter().any(|&t| s.threads[t].state == St::Runnable) {
            s.threads[cur].state = St::Runnable;
            let pk = push_runq(s, idx, cur);
            (true, pk)
        } else {
            (false, None)
        }
    });
    let (switched, pk) = switched;
    if let Some(pk) = pk {
        pk.unpark();
    }
    if switched {
        suspend_current();
    }
    deliver_interrupt();
}

fn deliver_interrupt() {
    let p = pt();
    if p.current.is_none() {
        return;
    }
    let t = unsafe { &*p.cur };
    if !t.has_interrupt.load(Ordering::Acquire) {
        return;
    }
    let cur = p.current.unwrap();
    let i = locked!(s => {
        let t = &mut s.threads[cur];
        t.has_interrupt.store(false, Ordering::Release);
        t.interrupt.take()
    });
    if let Some(i) = i {
        pst().pending = i;
    }
}

/// A token identifying the running thread (never 0 or 1; used by require's loading flags).
pub fn current_token() -> u64 {
    let p = pt();
    if p.current.is_some() { p.cur as u64 } else { 2 + p.idx as u64 * 16 }
}

/// Wait briefly for another thread to make progress (e.g. finish loading a file): run other
/// threads of this P, or give up the CPU. True if this thread was interrupted
/// (Thread#raise/kill: the caller must unwind).
pub fn wait_a_moment() -> bool {
    if crate::gc::safepoint() {
        return true;
    }
    if active() {
        pass();
        if pst().pending_is_set() {
            return true;
        }
    }
    crate::gc::blocking(|| std::thread::sleep(Duration::from_micros(50)));
    false
}

/// Called from the GC safepoint when the preemption timer fired.
pub fn preempt() {
    let p = pt();
    let tick = TICK.load(Ordering::Relaxed);
    if p.seen_tick == tick && crate::mt::multi() {
        return;
    }
    p.seen_tick = tick;
    pass();
}

/// True when a Thread#raise / kill is waiting for this thread (checked at safepoints).
pub fn take_interrupt() -> bool {
    if !active() {
        return false;
    }
    deliver_interrupt();
    pst().pending_is_set()
}

/// Kernel#sleep
pub fn sleep(d: Option<Duration>) -> V {
    let alone = !active() || locked!(s => s.threads.len() <= 1);
    if alone {
        crate::kernel::flush_out();
        match d {
            Some(d) => crate::gc::blocking(|| std::thread::sleep(d)),
            None => loop {
                crate::gc::blocking(|| std::thread::sleep(Duration::from_secs(3600)))
            },
        }
        return QNIL;
    }
    match block_on(QNIL, d) {
        Wake::Interrupted => UNDEF,
        _ => QNIL,
    }
}

/// The current P's loop blocks in its own poller (the HTTP server): `wake` must interrupt
/// that wait whenever another P hands this one work, since `idle_timeout` may have been None.
pub fn set_wake_hook(wake: Box<dyn Fn() + Send + Sync>) {
    if !active() {
        return;
    }
    let idx = pt().idx;
    let pk = locked!(s => s.ps[idx].parker.clone());
    *pk.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake);
    pk.has_hook.store(true, Ordering::Release);
}

/// How long an IO loop may block waiting for events (None = forever: no other threads on
/// this P need the CPU).
pub fn idle_timeout() -> Option<Duration> {
    if !active() {
        return None;
    }
    let idx = pt().idx;
    let cur = pt().current;
    if P_NTHREADS[idx].load(Ordering::Relaxed) <= 1 {
        return None;
    }
    locked!(s => {
        let ps = &s.ps[idx];
        let others = ps.nthreads.saturating_sub(1);
        if others == 0 {
            return None;
        }
        if ps.runq.iter().any(|&t| Some(t) != cur && s.threads[t].state == St::Runnable) {
            return Some(Duration::ZERO);
        }
        let now = Instant::now();
        ps.sleepers.iter().filter_map(|&t| s.threads[t].wake_at).min().map(|w| w.saturating_duration_since(now)).or(Some(Duration::from_millis(50)))
    })
}

fn tid_of(v: V) -> usize {
    fix(ivar_get(v, sym("@__tid"))) as usize
}

/// Run `f` on a new thread of P `p` (used by the HTTP server to put a request loop on
/// every P). Returns the thread id.
pub fn spawn_native_on(p: usize, f: Box<dyn FnOnce() -> V>) -> usize {
    spawn_raw(Box::new(move |_| f()), THREAD_STACK, Some(p))
}
/// Number of Ps available (KENNY_THREADS).
pub fn max_ps() -> usize {
    sraw().map(|s| s.max_ps).unwrap_or(1)
}

// ---------- Fibers ----------

fn fiber_of(v: V) -> usize {
    fix(ivar_get(v, sym("@__fid"))) as usize
}
fn fiber_ptr(fid: usize) -> *mut GFiber {
    locked!(s => &mut *s.fibers[fid] as *mut GFiber)
}

/// "created", "resumed", "suspended" or "terminated" (for Fiber#inspect).
pub fn fiber_status(v: V) -> &'static str {
    let fid = fiber_of(v);
    let Some(f) = locked!(s => s.fibers.get(fid).map(|f| (f.done, f.started))) else { return "terminated" };
    if f.0 {
        "terminated"
    } else if pt().fiber_stack.contains(&fid) {
        "resumed"
    } else if f.1 {
        "suspended"
    } else {
        "created"
    }
}

pub fn fiber_new(blk: V) -> V {
    let stack = DefaultStack::new(FIBER_STACK).expect("fiber stack");
    let (base, limit) = (stack.base().get(), stack.limit().get());
    let fiber_class = sraw().unwrap().fiber_class;
    let o = object_new(fiber_class);
    let fid = locked!(s => {
        let fid = s.fibers.len();
        let ctx = new_ctx(s, base, limit);
        let co: FiberCo = Coroutine::with_stack(stack, move |y: &Yielder<V, V>, first: V| {
            unsafe { (*fiber_ptr(fid)).yielder = y as *const _ };
            let args = if is_ary(first) { rary(first).v.clone() } else { vec![] };
            let args = RVec::from(args);
            call_block(blk, &args)
        });
        s.fibers.push(Box::new(GFiber { co: Some(co), yielder: std::ptr::null(), ctx, started: false, done: false, resumer_ctx: 0, obj: o }));
        fid
    });
    ivar_set(o, sym("@__fid"), mkfix(fid as i64));
    ivar_set(o, sym("@__blk"), blk);
    // a new fiber starts with a copy of its creator's storage (Fiber.new(storage: true))
    let st = inherited_storage();
    if st != QNIL {
        ivar_set(o, sym("@__storage"), st);
    }
    o
}

fn current_fiber() -> V {
    send0(const_lookup(rt().c.object, sym("Fiber")).unwrap(), "current")
}
/// Fiber storage (`Fiber[:k]`) of the running fiber: a Hash, or nil while empty.
fn fiber_storage() -> V {
    ivar_get(current_fiber(), sym("@__storage"))
}
/// A copy of the running fiber's storage for a new fiber or thread, or nil.
fn inherited_storage() -> V {
    let st = fiber_storage();
    if is_hash(st) && !rhash(st).m.is_empty() { crate::kernel::dup(st, false) } else { QNIL }
}

pub fn fiber_resume(fv: V, args: &[V]) -> V {
    let fid = fiber_of(fv);
    let f = unsafe { &mut *fiber_ptr(fid) };
    let fe = sraw().unwrap().fiber_error;
    if f.done {
        return raise(fe, "attempt to resume a terminated fiber");
    }
    if pt().fiber_stack.contains(&fid) {
        return raise(fe, "attempt to resume the current fiber");
    }
    let input = if !f.started {
        f.started = true;
        ary_new(args.to_vec())
    } else {
        match args.len() {
            0 => QNIL,
            1 => args[0],
            _ => ary_new(args.to_vec()),
        }
    };
    let outer = pt().running_ctx.unwrap_or(0);
    leave_ctx(outer);
    let (outer_base, outer_limit) = (pst().heap.stack_base, pst().stack_limit);
    f.resumer_ctx = outer;
    pt().fiber_stack.push(fid);
    let ctx = f.ctx;
    enter_ctx(ctx);
    let mut co = f.co.take().unwrap();
    let res = co.resume(input);
    // Back on the resumer's stack: say so before anything that can stop for a collection
    // (taking a lock can): the GC scans this P's stack from here up to `stack_base`.
    pst().heap.stack_base = outer_base;
    crate::core::set_stack_limit(outer_limit);
    pt().running_ctx = Some(outer);
    pt().fiber_stack.pop();
    let f = unsafe { &mut *fiber_ptr(fid) };
    match res {
        CoroutineResult::Yield(v) => {
            f.co = Some(co);
            locked!(s => s.ctxs[ctx].running = false);
            enter_ctx(outer);
            v
        }
        CoroutineResult::Return(v) => {
            drop(co);
            f.done = true;
            locked!(s => {
                s.ctxs[ctx].live = false;
                s.ctxs[ctx].running = false;
            });
            enter_ctx(outer);
            v
        }
    }
}

pub fn fiber_yield(args: &[V]) -> V {
    let Some(&fid) = pt().fiber_stack.last() else {
        return raise(sraw().unwrap().fiber_error, "can't yield from root fiber");
    };
    let v = match args.len() {
        0 => QNIL,
        1 => args[0],
        _ => ary_new(args.to_vec()),
    };
    let f = unsafe { &*fiber_ptr(fid) };
    leave_ctx(f.ctx);
    let (y, obj) = (f.yielder, f.obj);
    // resumed again
    let r = unsafe { (*y).suspend(v) };
    // (Fiber#raise: resumed to raise its exception here)
    let pend = ivar_get(obj, sym("@__raise"));
    if pend != QNIL {
        ivar_set(obj, sym("@__raise"), QNIL);
        let args = crate::gc::Rooted::of(rary(pend).v.clone());
        return send(QNIL, sym("raise"), &args, QNIL);
    }
    r
}

// ---------- Mutex objects ----------

/// A Mutex's state: the owning thread (0 = unlocked) and how many threads wait for it.
pub struct MutexState {
    owner: AtomicU64,
    waiters: std::sync::atomic::AtomicU32,
}
unsafe extern "C" fn mutex_free(p: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(p as *mut MutexState) });
}
/// Mutex.allocate (the Mutex class has alloc kind Data).
pub fn mutex_alloc(c: *mut Class) -> V {
    let st = Box::into_raw(Box::new(MutexState { owner: AtomicU64::new(0), waiters: std::sync::atomic::AtomicU32::new(0) }));
    alloc(RData { h: Header { klass: c, kind: Kind::Data, flags: 0 }, data: st as *mut std::ffi::c_void, dmark: None, dfree: Some(mutex_free), roots: Vec::new() })
}
fn mutex_state(m: V) -> Option<&'static MutexState> {
    if !is_kind(m, Kind::Data) {
        return None;
    }
    let d = unsafe { &*(m as *const RData) };
    // (the free function identifies a Mutex's data)
    if d.dfree.map(|f| f as usize) != Some(mutex_free as unsafe extern "C" fn(*mut std::ffi::c_void) as usize) {
        return None;
    }
    Some(unsafe { &*(d.data as *const MutexState) })
}

// ---------- Ruby classes ----------

fn qbuf(q: V) -> V {
    ivar_get(q, sym("@__q"))
}
fn thread_error() -> *mut Class {
    sraw().unwrap().thread_error
}

pub fn init() {
    let o = rt().c.object;
    let th = new_class("Thread", o);
    set_global_const("Thread", th as V);
    let fb = new_class("Fiber", o);
    set_global_const("Fiber", fb as V);

    defs(th, "new", 0, -1, |_, a, b| {
        if !active() {
            return raise(rt().c.not_implemented_error, "threads can't be started outside the main program");
        }
        if !is_proc(b) {
            return raise(thread_error(), "must be called with a block");
        }
        let args = a.to_vec();
        // the new thread's root fiber starts with a copy of this fiber's storage
        let storage = inherited_storage();
        // the block, its arguments and self are reachable from the thread object below
        let tid = spawn_raw(
            Box::new(move |_| {
                if storage != QNIL {
                    ivar_set(current_fiber(), sym("@__storage"), storage);
                }
                let args = RVec::from(args);
                call_block(b, &args)
            }),
            THREAD_STACK,
            None,
        );
        let obj = locked!(s => s.threads[tid].obj);
        ivar_set(obj, sym("@__blk"), b);
        ivar_set(obj, sym("@__args"), ary_new(a.to_vec()));
        // (Thread.report_on_exception at its start)
        ivar_set(obj, sym("@__report"), crate::extra::thread_report_default());
        ivar_set(obj, sym("@__inherited_storage"), storage);
        obj
    });
    defs(th, "start", 0, -1, |s, a, b| send(s, sym("new"), a, b));
    defs(th, "fork", 0, -1, |s, a, b| send(s, sym("new"), a, b));
    defs(th, "current", 0, 0, |_, _, _| match pt().current {
        Some(_) => unsafe { (*pt().cur).obj },
        None => QNIL,
    });
    defs(th, "main", 0, 0, |_, _, _| match sraw() {
        Some(_) => locked!(s => s.threads.first().map(|t| t.obj).unwrap_or(QNIL)),
        None => QNIL,
    });
    defs(th, "list", 0, 0, |_, _, _| {
        let l = locked!(s => s.threads.iter().filter(|t| t.state != St::Done).map(|t| t.obj).collect());
        ary_new(l)
    });
    defs(th, "pass", 0, 0, |_, _, _| {
        pass();
        if matches!(pst().pending, Throw::None) { QNIL } else { UNDEF }
    });
    defs(th, "stop", 0, 0, |_, _, _| match block_on(QNIL, None) {
        Wake::Interrupted => UNDEF,
        _ => QNIL,
    });
    defs(th, "abort_on_exception=", 1, 1, |_, a, _| a[0]);
    defs(th, "report_on_exception=", 1, 1, |_, a, _| a[0]);
    defn(th, "report_on_exception=", 1, 1, |s, a, _| ivar_set(s, sym("@__report"), a[0]));
    defn(th, "abort_on_exception=", 1, 1, |_, a, _| a[0]);
    fn join(s: V, timeout: Option<Duration>) -> V {
        let tid = tid_of(s);
        if Some(tid) == pt().current {
            return raise(thread_error(), "Target thread must not be current thread");
        }
        let deadline = timeout.map(|d| Instant::now() + d);
        loop {
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            if left == Some(Duration::ZERO) && locked!(st => st.threads[tid].state != St::Done) {
                return QNIL;
            }
            // check and block in one locked step, so the thread's finish can't slip between
            let r = locked!(st => {
                if st.threads[tid].state == St::Done {
                    Ok(st.threads[tid].exc)
                } else {
                    Err(prepare_block(st, s, left))
                }
            });
            match r {
                Ok(e) => {
                    if e != QNIL {
                        return raise_exc(e);
                    }
                    return s;
                }
                Err(Some(i)) => {
                    pst().pending = i;
                    return UNDEF;
                }
                Err(None) => {
                    suspend_current();
                    match finish_block(s) {
                        Wake::Interrupted => return UNDEF,
                        Wake::Timeout => return QNIL,
                        Wake::Woken => {}
                    }
                }
            }
        }
    }
    defn(th, "join", 0, 1, |s, a, _| {
        let t = a.first().and_then(|x| crate::fmt::num_to_f(*x)).and_then(timeout_secs);
        join(s, t)
    });
    defn(th, "value", 0, 0, |s, _, _| {
        tri!(join(s, None));
        locked!(st => st.threads[tid_of(s)].value)
    });
    defn(th, "alive?", 0, 0, |s, _, _| bool_v(locked!(st => st.threads[tid_of(s)].state != St::Done)));
    defn(th, "status", 0, 0, |s, _, _| {
        let (state, exc) = locked!(st => {
            let t = &st.threads[tid_of(s)];
            (t.state, t.exc)
        });
        match state {
            St::Done => {
                if exc != QNIL {
                    QNIL
                } else {
                    QFALSE
                }
            }
            St::Blocked => str_from("sleep"),
            St::Runnable | St::Running => str_from("run"),
        }
    });
    defn(th, "stop?", 0, 0, |s, _, _| bool_v(locked!(st => !matches!(st.threads[tid_of(s)].state, St::Runnable | St::Running))));
    defn(th, "[]", 1, 1, |s, a, _| {
        let k = if is_str(a[0]) { mksym(intern(&rstr(a[0]).b.clone())) } else { a[0] };
        crate::hash::hget(ivar_get(s, sym("@__locals")), k)
    });
    defn(th, "[]=", 2, 2, |s, a, _| {
        let k = if is_str(a[0]) { mksym(intern(&rstr(a[0]).b.clone())) } else { a[0] };
        crate::hash::hset(ivar_get(s, sym("@__locals")), k, a[1])
    });
    defn(th, "key?", 1, 1, |s, a, _| bool_v(rhash(ivar_get(s, sym("@__locals"))).m.contains_key(&HKey(a[0]))));
    defn(th, "keys", 0, 0, |s, _, _| send0(ivar_get(s, sym("@__locals")), "keys"));
    defn(th, "thread_variable_get", 1, 1, |s, a, _| crate::hash::hget(ivar_get(s, sym("@__vars")), a[0]));
    defn(th, "thread_variable_set", 2, 2, |s, a, _| crate::hash::hset(ivar_get(s, sym("@__vars")), a[0], a[1]));
    defn(th, "thread_variable?", 1, 1, |s, a, _| bool_v(rhash(ivar_get(s, sym("@__vars"))).m.contains_key(&HKey(a[0]))));
    defn(th, "name", 0, 0, |s, _, _| ivar_get(s, sym("@__name")));
    defn(th, "name=", 1, 1, |s, a, _| ivar_set(s, sym("@__name"), a[0]));
    defn(th, "priority", 0, 0, |_, _, _| mkfix(0));
    defn(th, "priority=", 1, 1, |_, a, _| a[0]);
    defn(th, "inspect", 0, 0, |s, _, _| {
        let st = send0(s, "status");
        let st = if is_str(st) { String::from_utf8_lossy(&rstr(st).b).into_owned() } else { "dead".into() };
        new_str_v(format!("#<Thread:0x{:016x} {}>", s, st))
    });
    defn(th, "to_s", 0, 0, |s, _, _| send0(s, "inspect"));
    fn interrupt(s: V, what: Throw) -> V {
        let tid = tid_of(s);
        if Some(tid) == pt().current {
            pst().pending = what;
            return UNDEF;
        }
        let pk = locked!(st => {
            let t = &mut st.threads[tid];
            if t.state == St::Done {
                return s;
            }
            t.interrupt = Some(what);
            t.has_interrupt.store(true, Ordering::Release);
            let p = t.p;
            match t.state {
                St::Blocked => make_runnable(st, tid),
                _ => Some(st.ps[p].parker.clone()),
            }
        });
        if let Some(pk) = pk {
            pk.unpark();
        }
        // send a running target through its next safepoint's slow path
        crate::gc::KN_GC_PENDING.fetch_or(PREEMPT_BIT, Ordering::Relaxed);
        s
    }
    // raise(exception = RuntimeError, message, backtrace, cause:)
    defn(th, "raise", 0, -1, |s, a, _| {
        let mut av = a.to_vec();
        let mut cause = None;
        // (Ruby 3.4 takes keywords as the backtrace argument)
        if let Some(&h) = av.last().filter(|_| crate::misc::ruby4()) {
            if is_hash(h) && hdr(h).flags & FL_KWHASH != 0 {
                if let Some(&c) = rhash(h).m.get(&HKey(mksym(sym("cause")))) {
                    cause = Some(c);
                }
                av.pop();
            }
        }
        if av.len() > 3 {
            return arg_error(&format!("wrong number of arguments (given {}, expected 0..3)", av.len()));
        }
        let a = &av[..];
        let e = if a.is_empty() {
            exc_new(rt().c.runtime_error, "unhandled exception")
        } else if is_str(a[0]) {
            exc_new(rt().c.runtime_error, &String::from_utf8_lossy(&rstr(a[0]).b))
        } else if is_class(a[0]) {
            tri!(send(a[0], sym("new"), &a[1..a.len().min(2)], QNIL))
        } else {
            a[0]
        };
        if !is_a(e, rt().c.exception) {
            return type_error("exception class/object expected");
        }
        if let Some(&bt) = a.get(2) {
            tri!(send(e, sym("set_backtrace"), &[bt], QNIL));
        }
        if let Some(c) = cause {
            if c != QNIL && !is_a(c, rt().c.exception) {
                return type_error("exception object expected");
            }
            ivar_set(e, sym("@__cause"), c);
            ivar_set(e, sym("@__cause_set"), QTRUE);
        }
        interrupt(s, Throw::Raise(e))
    });
    for name in ["kill", "exit", "terminate"] {
        defn(th, name, 0, 0, |s, _, _| interrupt(s, Throw::Kill));
    }
    defn(th, "wakeup", 0, 0, |s, _, _| {
        let tid = tid_of(s);
        let pk = locked!(st => make_runnable(st, tid));
        if let Some(pk) = pk {
            pk.unpark();
        }
        s
    });
    defn(th, "run", 0, 0, |s, _, _| {
        tri!(send0(s, "wakeup"));
        pass();
        s
    });
    defs(th, "exit", 0, 0, |_, _, _| {
        pst().pending = Throw::Kill;
        UNDEF
    });

    // ---- Mutex: an atomic owner word; the scheduler lock only when there is a wait ----
    let mx = rt().c.mutex;
    const_set(th, sym("Mutex"), mx as V);
    unsafe {
        (*mx).alloc = Kind::Data; // instances come from `mutex_alloc`
    }
    fn me() -> V {
        match pt().current {
            Some(_) => unsafe { (*pt().cur).obj },
            None => QTRUE,
        }
    }
    fn st(m: V) -> Result<&'static MutexState, V> {
        mutex_state(m).ok_or_else(|| type_error("not a Mutex"))
    }
    fn lock(m: V) -> V {
        let st = match st(m) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let me = me();
        loop {
            if st.owner.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return m;
            }
            if st.owner.load(Ordering::Relaxed) == me {
                return raise(thread_error(), "deadlock; recursive locking");
            }
            if !active() {
                return raise(thread_error(), "deadlock; lock already owned by another thread");
            }
            // announce the wait, then try once more under the lock: an unlock either sees
            // the waiter (and wakes it) or this attempt sees the lock free
            let r = locked!(s => {
                st.waiters.fetch_add(1, Ordering::SeqCst);
                if st.owner.compare_exchange(0, me, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                    st.waiters.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                } else {
                    match prepare_block(s, m, None) {
                        Some(i) => {
                            st.waiters.fetch_sub(1, Ordering::SeqCst);
                            Err(Some(i))
                        }
                        None => Err(None),
                    }
                }
            });
            match r {
                Ok(()) => return m,
                Err(Some(i)) => {
                    pst().pending = i;
                    return UNDEF;
                }
                Err(None) => {
                    suspend_current();
                    let w = finish_block(m);
                    st.waiters.fetch_sub(1, Ordering::SeqCst);
                    if let Wake::Interrupted = w {
                        return UNDEF;
                    }
                }
            }
        }
    }
    /// Release `m` (owned by the caller); wakes one waiter. With the scheduler lock held,
    /// returns the parker to poke instead of taking the lock again.
    fn release(st: &MutexState, s: Option<&mut Sched>, m: V) -> Option<Arc<Parker>> {
        st.owner.store(0, Ordering::SeqCst);
        if st.waiters.load(Ordering::SeqCst) == 0 {
            return None;
        }
        match s {
            Some(s) => wake_one_locked(s, m).1,
            None => locked!(s => wake_one_locked(s, m).1),
        }
    }
    fn unlock(m: V) -> V {
        let st = match st(m) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if st.owner.load(Ordering::Relaxed) != me() {
            return raise(thread_error(), "Attempt to unlock a mutex which is locked by another thread/fiber");
        }
        if let Some(pk) = release(st, None, m) {
            pk.unpark();
        }
        m
    }
    fn owner(m: V) -> V {
        match mutex_state(m) {
            Some(st) => match st.owner.load(Ordering::Relaxed) {
                0 => QNIL,
                o => o,
            },
            None => QNIL,
        }
    }
    defn(mx, "lock", 0, 0, |s, _, _| lock(s));
    defn(mx, "unlock", 0, 0, |s, _, _| unlock(s));
    defn(mx, "try_lock", 0, 0, |s, _, _| match st(s) {
        Ok(st) => bool_v(st.owner.compare_exchange(0, me(), Ordering::Acquire, Ordering::Relaxed).is_ok()),
        Err(e) => e,
    });
    defn(mx, "locked?", 0, 0, |s, _, _| bool_v(truthy(owner(s))));
    defn(mx, "owned?", 0, 0, |s, _, _| bool_v(owner(s) == me()));
    defn(mx, "synchronize", 0, 0, |s, _, b| {
        tri!(lock(s));
        let r = call_block(b, &[]);
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        let u = unlock(s);
        if u == UNDEF {
            return UNDEF;
        }
        pst().pending = saved;
        r
    });
    defn(mx, "sleep", 0, 1, |s, a, _| {
        let d = match a.first().and_then(|x| crate::fmt::num_to_f(*x)) {
            Some(x) => Some(match time_interval(x) { Ok(d) => d, Err(e) => return e }),
            None => None,
        };
        tri!(unlock(s));
        let r = sleep(d);
        tri!(lock(s));
        r
    });

    // ---- ConditionVariable ----
    let cv = new_class("Thread::ConditionVariable", o);
    set_global_const("ConditionVariable", cv as V);
    const_set(th, sym("ConditionVariable"), cv as V);
    defn(cv, "wait", 1, 2, |s, a, _| {
        let m = a[0];
        let d = match a.get(1).and_then(|x| crate::fmt::num_to_f(*x)) {
            Some(x) => Some(match time_interval(x) { Ok(d) => d, Err(e) => return e }),
            None => None,
        };
        let native = mutex_state(m).filter(|st| st.owner.load(Ordering::Relaxed) == me());
        let w = if let (Some(mst), true) = (native, active()) {
            // release the mutex and start waiting in one locked step (no lost signal)
            let r = locked!(st => {
                let pk = release(mst, Some(&mut *st), m);
                (prepare_block(st, s, d), pk)
            });
            if let Some(pk) = r.1 {
                pk.unpark();
            }
            match r.0 {
                Some(i) => {
                    pst().pending = i;
                    Wake::Interrupted
                }
                None => {
                    suspend_current();
                    finish_block(s)
                }
            }
        } else {
            tri!(send0(m, "unlock"));
            block_on(s, d)
        };
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        tri!(send0(m, "lock"));
        if let Wake::Interrupted = w {
            pst().pending = saved;
            return UNDEF;
        }
        s
    });
    defn(cv, "signal", 0, 0, |s, _, _| {
        wake_one(s);
        s
    });
    defn(cv, "broadcast", 0, 0, |s, _, _| {
        wake_all(s);
        s
    });

    // ---- Queue / SizedQueue (buffer changes under the scheduler lock) ----
    let q = new_class("Thread::Queue", o);
    set_global_const("Queue", q as V);
    const_set(th, sym("Queue"), q as V);
    let closed_err = new_class("ClosedQueueError", const_lookup(o, sym("StopIteration")).unwrap() as *mut Class);
    set_global_const("ClosedQueueError", closed_err as V);
    defn(q, "initialize", 0, 1, |s, a, _| {
        let items = a.first().filter(|x| is_ary(**x)).map(|x| rary(*x).v.clone()).unwrap_or_default();
        ivar_set(s, sym("@__q"), ary_new(items));
        ivar_set(s, sym("@__closed"), QFALSE);
        QNIL
    });
    fn push(s: V, v: V) -> V {
        if truthy(ivar_get(s, sym("@__closed"))) {
            let c = const_lookup(rt().c.object, sym("ClosedQueueError")).unwrap();
            return raise(c as *mut Class, "queue closed");
        }
        // SizedQueue: wait for room
        loop {
            let max = ivar_get(s, sym("@__max"));
            let room = ivar_get(s, sym("@__room"));
            let r = locked!(st => {
                if !is_fix(max) || (rary(qbuf(s)).v.len() as i64) < fix(max) {
                    rary(qbuf(s)).v.push(v);
                    Ok(wake_one_locked(st, s).1)
                } else if !active() {
                    Err(None)
                } else {
                    Err(Some(prepare_block(st, room, None)))
                }
            });
            match r {
                Ok(pk) => {
                    if let Some(pk) = pk {
                        pk.unpark();
                    }
                    return s;
                }
                Err(None) => return raise(thread_error(), "queue full"),
                Err(Some(Some(i))) => {
                    pst().pending = i;
                    return UNDEF;
                }
                Err(Some(None)) => {
                    suspend_current();
                    if let Wake::Interrupted = finish_block(room) {
                        return UNDEF;
                    }
                }
            }
        }
    }
    for name in ["push", "<<", "enq"] {
        defn(q, name, 1, 2, |s, a, _| push(s, a[0]));
    }
    fn pop(s: V, a: &[V]) -> V {
        let non_block = a.first().is_some_and(|x| !is_hash(*x) && truthy(*x));
        let timeout = a.iter().find(|x| is_hash(**x)).and_then(|h| rhash(*h).m.get(&HKey(mksym(sym("timeout")))).copied()).and_then(crate::fmt::num_to_f);
        let deadline = timeout.and_then(timeout_secs).map(|t| Instant::now() + t);
        let room = ivar_get(s, sym("@__room"));
        loop {
            let closed = truthy(ivar_get(s, sym("@__closed")));
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            enum R {
                Got(V, Option<Arc<Parker>>),
                Nil,
                Empty,
                Fatal,
                Intr(Throw),
                Wait,
            }
            let r = locked!(st => {
                let buf = rary(qbuf(s));
                if !buf.v.is_empty() {
                    let v = buf.v.remove(0);
                    let pk = if room != QNIL { wake_one_locked(st, room).1 } else { None };
                    R::Got(v, pk)
                } else if closed || left == Some(Duration::ZERO) {
                    R::Nil
                } else if non_block {
                    R::Empty
                } else if !active() {
                    R::Fatal
                } else {
                    match prepare_block(st, s, left) {
                        Some(i) => R::Intr(i),
                        None => R::Wait,
                    }
                }
            });
            match r {
                R::Got(v, pk) => {
                    if let Some(pk) = pk {
                        pk.unpark();
                    }
                    return v;
                }
                R::Nil => return QNIL,
                R::Empty => return raise(thread_error(), "queue empty"),
                R::Fatal => return raise(sraw().unwrap().fatal, "No live threads left. Deadlock?"),
                R::Intr(i) => {
                    pst().pending = i;
                    return UNDEF;
                }
                R::Wait => {
                    suspend_current();
                    match finish_block(s) {
                        Wake::Interrupted => return UNDEF,
                        Wake::Timeout => return QNIL,
                        Wake::Woken => {}
                    }
                }
            }
        }
    }
    for name in ["pop", "shift", "deq"] {
        defn(q, name, 0, 2, |s, a, _| pop(s, a));
    }
    defn(q, "close", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__closed"), QTRUE);
        if active() {
            wake_all(s);
        }
        s
    });
    defn(q, "closed?", 0, 0, |s, _, _| ivar_get(s, sym("@__closed")));
    defn(q, "size", 0, 0, |s, _, _| mkfix(rary(qbuf(s)).v.len() as i64));
    defn(q, "length", 0, 0, |s, _, _| mkfix(rary(qbuf(s)).v.len() as i64));
    defn(q, "empty?", 0, 0, |s, _, _| bool_v(rary(qbuf(s)).v.is_empty()));
    defn(q, "clear", 0, 0, |s, _, _| {
        locked!(st => {
            let _ = st;
            rary(qbuf(s)).v.clear();
        });
        s
    });
    defn(q, "num_waiting", 0, 0, |s, _, _| match sraw() {
        Some(_) => mkfix(locked!(st => st.waiters.get(&(s as usize)).map(|w| w.len()).unwrap_or(0)) as i64),
        None => mkfix(0),
    });
    let sq = new_class("Thread::SizedQueue", q);
    set_global_const("SizedQueue", sq as V);
    const_set(th, sym("SizedQueue"), sq as V);
    defn(sq, "initialize", 1, 1, |s, a, _| {
        ivar_set(s, sym("@__q"), ary_new(vec![]));
        ivar_set(s, sym("@__closed"), QFALSE);
        ivar_set(s, sym("@__max"), a[0]);
        ivar_set(s, sym("@__room"), object_new(rt().c.object));
        QNIL
    });
    defn(sq, "max", 0, 0, |s, _, _| ivar_get(s, sym("@__max")));

    // ---- Monitor (reentrant) ----
    let mon = new_class("Monitor", o);
    set_global_const("Monitor", mon as V);
    fn mon_enter(s: V) -> V {
        if ivar_get(s, sym("@__owner")) == me() {
            let c = ivar_get(s, sym("@__count"));
            ivar_set(s, sym("@__count"), mkfix(fix(c) + 1));
            return s;
        }
        let m = ivar_get(s, sym("@__mutex"));
        let m = if m == QNIL {
            // two threads entering a new monitor at once must share one mutex
            let fresh = mutex_alloc(rt().c.mutex);
            locked!(st => {
                let _ = st;
                let cur = ivar_get(s, sym("@__mutex"));
                if cur == QNIL {
                    ivar_set(s, sym("@__mutex"), fresh);
                    fresh
                } else {
                    cur
                }
            })
        } else {
            m
        };
        tri!(lock(m));
        ivar_set(s, sym("@__owner"), me());
        ivar_set(s, sym("@__count"), mkfix(1));
        s
    }
    fn mon_exit(s: V) -> V {
        let c = fix(ivar_get(s, sym("@__count"))) - 1;
        ivar_set(s, sym("@__count"), mkfix(c));
        if c == 0 {
            ivar_set(s, sym("@__owner"), QNIL);
            tri!(unlock(ivar_get(s, sym("@__mutex"))));
        }
        s
    }
    defn(mon, "enter", 0, 0, |s, _, _| mon_enter(s));
    defn(mon, "mon_enter", 0, 0, |s, _, _| mon_enter(s));
    defn(mon, "exit", 0, 0, |s, _, _| mon_exit(s));
    defn(mon, "mon_exit", 0, 0, |s, _, _| mon_exit(s));
    fn mon_sync(s: V, b: V) -> V {
        tri!(mon_enter(s));
        let r = call_block(b, &[]);
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        tri!(mon_exit(s));
        pst().pending = saved;
        r
    }
    defn(mon, "synchronize", 0, 0, |s, _, b| mon_sync(s, b));
    defn(mon, "mon_synchronize", 0, 0, |s, _, b| mon_sync(s, b));
    defn(mon, "mon_owned?", 0, 0, |s, _, _| bool_v(ivar_get(s, sym("@__owner")) == me()));

    // ---- Fiber ----
    defs(fb, "new", 0, -1, |_, _, b| {
        if !is_proc(b) {
            return arg_error("tried to create Proc object without a block");
        }
        if !active() {
            return raise(rt().c.not_implemented_error, "fibers can't be created outside the main program");
        }
        fiber_new(b)
    });
    defs(fb, "yield", 0, -1, |_, a, _| fiber_yield(a));
    defn(fb, "resume", 0, -1, |s, a, _| fiber_resume(s, a));
    // raise(exception = RuntimeError, message, backtrace, cause:) inside the fiber, where it
    // last yielded
    defn(fb, "raise", 0, -1, |s, a, _| {
        let f = unsafe { &*fiber_ptr(fiber_of(s)) };
        if !f.started {
            return raise(sraw().unwrap().fiber_error, "cannot raise exception on unborn fiber");
        }
        if f.done {
            return raise(sraw().unwrap().fiber_error, "attempt to resume a terminated fiber");
        }
        let args = if a.is_empty() { vec![rt().c.runtime_error as V, new_str_v("unhandled exception".into())] } else { a.to_vec() };
        ivar_set(s, sym("@__raise"), ary_new(args));
        fiber_resume(s, &[])
    });
    defn(fb, "alive?", 0, 0, |s, _, _| {
        if ivar_get(s, sym("@__fid")) == QNIL {
            return QTRUE; // a thread's root fiber
        }
        bool_v(!unsafe { &*fiber_ptr(fiber_of(s)) }.done)
    });
    // the running fiber, or the current thread's root fiber
    defs(fb, "current", 0, 0, |_, _, _| {
        if let Some(&fid) = pt().fiber_stack.last() {
            return unsafe { &*fiber_ptr(fid) }.obj;
        }
        let t = send0(const_lookup(rt().c.object, sym("Thread")).unwrap(), "current");
        let root = ivar_get(t, sym("@__root_fiber"));
        if root != QNIL {
            return root;
        }
        let o = object_new(sraw().unwrap().fiber_class);
        ivar_set(t, sym("@__root_fiber"), o);
        o
    });

    // Fiber storage (Ruby 3.2): Fiber[:k], Fiber[:k] = v, Fiber#storage
    defs(fb, "[]", 1, 1, |_, a, _| {
        if !is_sym(a[0]) {
            return type_error(&format!("wrong argument type {} (expected Symbol)", class_name(real_class(a[0]))));
        }
        let st = fiber_storage();
        if is_hash(st) { crate::hash::hget(st, a[0]) } else { QNIL }
    });
    defs(fb, "[]=", 2, 2, |_, a, _| {
        if !is_sym(a[0]) {
            return type_error(&format!("wrong argument type {} (expected Symbol)", class_name(real_class(a[0]))));
        }
        let f = current_fiber();
        let mut st = ivar_get(f, sym("@__storage"));
        if !is_hash(st) {
            if a[1] == QNIL {
                return QNIL;
            }
            st = hash_new();
            ivar_set(f, sym("@__storage"), st);
        }
        if a[1] == QNIL {
            send(st, sym("delete"), &[a[0]], QNIL);
        } else {
            crate::hash::hset(st, a[0], a[1]);
        }
        a[1]
    });
    defn(fb, "storage", 0, 0, |s, _, _| {
        let st = ivar_get(s, sym("@__storage"));
        if is_hash(st) { crate::kernel::dup(st, false) } else { hash_new() }
    });
    defn(fb, "storage=", 1, 1, |s, a, _| {
        if a[0] != QNIL && !is_hash(a[0]) {
            return type_error("storage must be a hash");
        }
        let v = if a[0] == QNIL { QNIL } else { crate::kernel::dup(a[0], false) };
        ivar_set(s, sym("@__storage"), v);
        a[0]
    });

    // Kernel#sleep becomes thread-aware
    defn(rt().c.kernel, "sleep", 0, 1, |_, a, _| {
        let secs = a.first().and_then(|x| crate::fmt::num_to_f(*x));
        let start = Instant::now();
        let d = match secs {
            Some(s) => Some(match time_interval(s) { Ok(d) => d, Err(e) => return e }),
            None => None,
        };
        tri!(sleep(d));
        mkfix(start.elapsed().as_secs_f64().round() as i64)
    });

    // Methods a thread waits in (audited: they keep object references only on the stack):
    // a thread blocked in one doesn't keep the GC from moving objects.
    gc_transparent(rt().c.kernel, &["sleep", "loop", "catch"]);
    gc_transparent(th, &["join", "value"]);
    gc_transparent(singleton_class(th as V), &["pass", "stop"]);
    gc_transparent(mx, &["lock", "unlock", "synchronize", "sleep", "try_lock"]);
    gc_transparent(cv, &["wait", "signal", "broadcast"]);
    for c in [q, sq] {
        gc_transparent(c, &["pop", "shift", "deq", "push", "<<", "enq", "close"]);
    }
    gc_transparent(mon, &["synchronize", "mon_synchronize", "enter", "exit", "mon_enter", "mon_exit"]);
}

/// Enumerator#next / peek via a Fiber (works for infinite enumerators).
pub fn enum_next(e: V, peek: bool) -> V {
    let mut f = ivar_get(e, sym("@__fiber"));
    let buffered = ivar_get(e, sym("@__peeked"));
    if buffered != QNIL && is_ary(buffered) {
        let v = rary(buffered).v[0];
        if !peek {
            ivar_set(e, sym("@__peeked"), QNIL);
        }
        return v;
    }
    if f == QNIL {
        let target = e;
        let body = proc_new_native_rooted(
            Box::new(move |_, _, _| {
                let yielder = proc_new_native(Box::new(|_, a, _| fiber_yield(&[ary_new(a.to_vec())])), 0, -1);
                tri!(send(target, sym("each"), &[], yielder));
                let stop = rt().c.stop_iteration;
                ivar_set(target, sym("@__done"), QTRUE);
                let ex = exc_new(stop, "iteration reached an end");
                ex
            }),
            0,
            -1,
            vec![target],
        );
        f = fiber_new(body);
        ivar_set(e, sym("@__fiber"), f);
    }
    if truthy(ivar_get(e, sym("@__done"))) {
        return raise(rt().c.stop_iteration, "iteration reached an end");
    }
    let r = tri!(fiber_resume(f, &[]));
    if truthy(ivar_get(e, sym("@__done"))) {
        return raise_exc(r);
    }
    let a = rary(r);
    let v = if a.v.len() == 1 { a.v[0] } else { ary_new(a.v.clone()) };
    if peek {
        ivar_set(e, sym("@__peeked"), ary_new(vec![v]));
    }
    v
}
