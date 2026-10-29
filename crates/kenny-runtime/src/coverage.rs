//! Coverage (the `coverage` library), as CRuby 3.4's ext/coverage.
//!
//! A program that requires "coverage" is compiled with counters (kenny-compiler
//! coverage.rs): each file has a data word pointing to its table of counters, which its
//! statements, branch targets and methods increment. `main` registers every file
//! (`kn_cov_register`: the word and a description of what the counters mean). The word
//! points to a scratch table until the file is measured: when its top level starts running
//! while coverage is set up (`kn_cov_file_start`), it gets a real table, which the word
//! points to while coverage is running (Coverage.resume / suspend swap it).
//!
//! The description ("meta"), one entry per line:
//!   N <lines>                        the size of the file's lines array
//!   S <ctr>:<line> ...               line events (lines mode): the counter, its line
//!   T <ctr>:<line> ...               the same when branches are measured too
//!   B <kind> l1 c1 l2 c2 | <target> <ctr> l1 c1 l2 c2 | ...   a branch and its targets
//!   M <ctr> l1 c1 l2 c2 <name>       a method (name: empty when not known statically)
//!
//! Not measured: code compiled at run time (eval, `load` of a changed file) and the code of
//! native libraries. Methods are found in the classes' method tables (CRuby looks at every
//! method entry in the heap, so it also reports methods since removed or redefined).

use crate::core::*;
use crate::hash::hset;
use crate::tri;
use std::collections::HashMap;
use std::sync::Mutex;

struct Base {
    kind: String,
    loc: [i64; 4],
    /// (kind, counter, location)
    targets: Vec<(String, usize, [i64; 4])>,
}

struct File {
    word: usize,
    path: String,
    nlines: usize,
    /// (counter, line) of the line events, without and with branch coverage
    lines: [Vec<(usize, usize)>; 2],
    bases: Vec<Base>,
    /// (counter, location, name)
    methods: Vec<(usize, [i64; 4], String)>,
    junk: Box<[u64]>,
    counters: Option<Box<[u64]>>,
    /// measured (its top level started while coverage was set up)
    covered: bool,
    /// oneshot_lines: the lines run (in order), and the counters that reported theirs
    oneshot: Vec<usize>,
    reported: Vec<bool>,
    /// counter -> line, in the current mode (oneshot_lines)
    line_of: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Idle,
    Suspended,
    Running,
}

const LINES: u32 = 1;
const BRANCHES: u32 = 2;
const METHODS: u32 = 4;
const ONESHOT: u32 = 8;
const EVAL: u32 = 16;

struct Cov {
    files: Vec<File>,
    by_word: HashMap<usize, usize>,
    state: State,
    /// 0: compatible mode (`Coverage.start` with no arguments: lines only, as arrays)
    mode: u32,
    /// the measured files, in the order they started
    order: Vec<usize>,
    /// method function address -> (file, counter)
    methods: HashMap<usize, (usize, usize)>,
}

static COV: Mutex<Option<Cov>> = Mutex::new(None);

/// Methods that only change an inherited method's visibility here (visibility.rs `set_vis`).
static VIS_COPIES: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

pub fn vis_copy(m: *const Method) {
    VIS_COPIES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(m as usize);
}

fn with<R>(f: impl FnOnce(&mut Cov) -> R) -> R {
    let mut g = COV.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.get_or_insert_with(|| Cov { files: Vec::new(), by_word: HashMap::new(), state: State::Idle, mode: 0, order: Vec::new(), methods: HashMap::new() });
    f(c)
}

fn nums(s: &str) -> Vec<i64> {
    s.split_whitespace().filter_map(|x| x.parse().ok()).collect()
}
fn loc4(v: &[i64]) -> [i64; 4] {
    [v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0), v.get(2).copied().unwrap_or(0), v.get(3).copied().unwrap_or(0)]
}

/// Called by `main` before the program runs: a compiled file's counters word and description.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_cov_register(word: *mut *mut u64, meta: *const u8, mlen: u64, path: *const u8, plen: u64) {
    let meta = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(meta, mlen as usize) }).into_owned();
    let path = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(path, plen as usize) }).into_owned();
    let mut f = File {
        word: word as usize,
        path,
        nlines: 0,
        lines: [Vec::new(), Vec::new()],
        bases: Vec::new(),
        methods: Vec::new(),
        junk: Box::new([]),
        counters: None,
        covered: false,
        oneshot: Vec::new(),
        reported: Vec::new(),
        line_of: Vec::new(),
    };
    let mut n = 0usize;
    for l in meta.lines() {
        let (tag, rest) = l.split_once(' ').unwrap_or((l, ""));
        match tag {
            "N" => f.nlines = rest.trim().parse().unwrap_or(0),
            "S" | "T" => {
                let v: Vec<(usize, usize)> = rest
                    .split_whitespace()
                    .filter_map(|p| {
                        let (c, line) = p.split_once(':')?;
                        Some((c.parse().ok()?, line.parse().ok()?))
                    })
                    .collect();
                for &(c, _) in &v {
                    n = n.max(c + 1);
                }
                f.lines[(tag == "T") as usize] = v;
            }
            "B" => {
                let mut parts = rest.split(" | ");
                let head: Vec<&str> = parts.next().unwrap_or("").splitn(2, ' ').collect();
                let mut b = Base { kind: head[0].to_string(), loc: loc4(&nums(head.get(1).unwrap_or(&""))), targets: Vec::new() };
                for t in parts {
                    let (kind, r) = t.split_once(' ').unwrap_or((t, ""));
                    let v = nums(r);
                    let c = v.first().copied().unwrap_or(0) as usize;
                    n = n.max(c + 1);
                    b.targets.push((kind.to_string(), c, loc4(&v[1.min(v.len())..])));
                }
                f.bases.push(b);
            }
            "M" => {
                // "<ctr> l1 c1 l2 c2 <name>"
                let mut it = rest.splitn(6, ' ');
                let v: Vec<i64> = (0..5).filter_map(|_| it.next()?.parse().ok()).collect();
                let name = it.next().unwrap_or("").to_string();
                let c = v.first().copied().unwrap_or(0) as usize;
                n = n.max(c + 1);
                f.methods.push((c, loc4(&v[1.min(v.len())..]), name));
            }
            _ => {}
        }
    }
    f.junk = vec![0u64; n.max(1)].into_boxed_slice();
    unsafe { *word = f.junk.as_ptr() as *mut u64 };
    with(|c| {
        c.by_word.insert(word as usize, c.files.len());
        c.files.push(f);
    });
}

/// Called by `main`: (function address, file word, counter) of every compiled method.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_cov_methods(table: *const u64, n: u64) {
    let t = unsafe { std::slice::from_raw_parts(table, n as usize * 3) };
    with(|c| {
        for e in t.chunks(3) {
            if let Some(&f) = c.by_word.get(&(e[1] as usize)) {
                c.methods.insert(e[0] as usize, (f, e[2] as usize));
            }
        }
    });
}

impl File {
    fn point(&self, running: bool) {
        let p = match (&self.counters, running && self.covered) {
            (Some(c), true) => c.as_ptr(),
            _ => self.junk.as_ptr(),
        };
        unsafe { *(self.word as *mut *const u64) = p };
    }
}

impl Cov {
    fn branches_on(&self) -> bool {
        self.mode & BRANCHES != 0
    }
}

/// A compiled file's top level starts: measured from now on if coverage is set up.
#[unsafe(no_mangle)]
pub extern "C" fn kn_cov_file_start(word: *mut *mut u64) {
    with(|c| {
        let Some(&i) = c.by_word.get(&(word as usize)) else { return };
        if c.state == State::Idle {
            return;
        }
        let mode = c.branches_on() as usize;
        let running = c.state == State::Running;
        let f = &mut c.files[i];
        // (loaded again: a fresh table, as CRuby's new coverage array for the path)
        f.counters = Some(vec![0u64; f.junk.len()].into_boxed_slice());
        f.oneshot.clear();
        f.reported = vec![false; f.junk.len()];
        f.line_of = vec![0; f.junk.len()];
        for &(ctr, line) in &f.lines[mode] {
            if let Some(x) = f.line_of.get_mut(ctr) {
                *x = line;
            }
        }
        let first = !f.covered;
        f.covered = true;
        f.point(running);
        if first {
            c.order.push(i);
        }
    });
}

/// A line event's counter went from 0 to 1 (oneshot_lines records its line, once).
#[unsafe(no_mangle)]
pub extern "C" fn kn_cov_first(word: *mut *mut u64, ctr: u64) {
    with(|c| {
        if c.mode & ONESHOT == 0 || c.state != State::Running {
            return;
        }
        let Some(&i) = c.by_word.get(&(word as usize)) else { return };
        let f = &mut c.files[i];
        let ctr = ctr as usize;
        if !f.covered || f.reported.get(ctr).copied().unwrap_or(true) {
            return;
        }
        let line = f.line_of.get(ctr).copied().unwrap_or(0);
        if line == 0 {
            return;
        }
        f.reported[ctr] = true;
        f.oneshot.push(line);
    });
}

fn rt_err(msg: &str) -> V {
    raise(rt().c.runtime_error, msg)
}

fn cname(v: V) -> String {
    match v {
        QNIL => "nil".into(),
        QTRUE => "true".into(),
        QFALSE => "false".into(),
        _ => class_name(real_class(v)),
    }
}

fn opt(h: V, key: &str) -> bool {
    truthy(crate::hash::hget(h, mksym(sym(key))))
}

/// Coverage.setup's argument: the measuring mode.
fn parse_mode(a: &[V]) -> Result<u32, V> {
    let Some(&o) = a.first() else { return Ok(0) };
    if o == mksym(sym("all")) {
        return Ok(LINES | BRANCHES | METHODS | EVAL);
    }
    let h = if is_hash(o) {
        o
    } else if respond_to(o, sym("to_hash")) {
        let h = send(o, sym("to_hash"), &[], QNIL);
        if h == UNDEF {
            return Err(UNDEF);
        }
        h
    } else {
        return Err(type_error(&format!("no implicit conversion of {} into Hash", cname(o))));
    };
    let mut m = 0;
    if opt(h, "lines") {
        m |= LINES;
    }
    if opt(h, "branches") {
        m |= BRANCHES;
    }
    if opt(h, "methods") {
        m |= METHODS;
    }
    if opt(h, "oneshot_lines") {
        if m & LINES != 0 {
            return Err(rt_err("cannot enable lines and oneshot_lines simultaneously"));
        }
        m |= LINES | ONESHOT;
    }
    if opt(h, "eval") {
        m |= EVAL;
    }
    Ok(m)
}

fn setup(a: &[V]) -> V {
    if with(|c| c.state != State::Idle) {
        return rt_err("coverage measurement is already setup");
    }
    let mode = match parse_mode(a) {
        Ok(m) => m,
        Err(e) => return e,
    };
    with(|c| {
        c.mode = mode;
        c.state = State::Suspended;
    });
    QNIL
}

fn resume() -> V {
    match with(|c| c.state) {
        State::Idle => return rt_err("coverage measurement is not set up yet"),
        State::Running => return rt_err("coverage measurement is already running"),
        State::Suspended => {}
    }
    with(|c| {
        c.state = State::Running;
        for &i in &c.order {
            c.files[i].point(true);
        }
    });
    QNIL
}

fn suspend() -> V {
    if with(|c| c.state != State::Running) {
        return rt_err("coverage measurement is not running");
    }
    with(|c| {
        c.state = State::Suspended;
        for &i in &c.order {
            c.files[i].point(false);
        }
    });
    QNIL
}

/// What a result is made of, copied out of the registry (no Ruby objects are made while it
/// is locked).
struct Snap {
    path: String,
    lines: Vec<Option<u64>>,
    oneshot: Vec<usize>,
    /// (kind, location, [(kind, location, count)])
    branches: Vec<(String, [i64; 4], Vec<(String, [i64; 4], u64)>)>,
    /// counter -> (location, name, count)
    methods: HashMap<usize, ([i64; 4], String, u64)>,
    /// the order of `methods`
    method_order: Vec<usize>,
}

fn snapshot(c: &Cov) -> Vec<Snap> {
    let mode = c.branches_on() as usize;
    c.order
        .iter()
        .map(|&i| {
            let f = &c.files[i];
            let ctr = |k: usize| f.counters.as_ref().and_then(|t| t.get(k).copied()).unwrap_or(0);
            let mut lines = vec![None; f.nlines];
            for &(k, line) in &f.lines[mode] {
                if line >= 1 && line <= f.nlines {
                    *lines[line - 1].get_or_insert(0) += ctr(k);
                }
            }
            let branches = f.bases.iter().map(|b| (b.kind.clone(), b.loc, b.targets.iter().map(|(k, n, l)| (k.clone(), *l, ctr(*n))).collect())).collect();
            let methods = f.methods.iter().map(|(n, l, name)| (*n, (*l, name.clone(), ctr(*n)))).collect();
            let method_order = f.methods.iter().map(|m| m.0).collect();
            Snap { path: f.path.clone(), lines, oneshot: f.oneshot.clone(), branches, methods, method_order }
        })
        .collect()
}

fn freeze(v: V) -> V {
    hdr(v).flags |= FL_FROZEN;
    v
}

fn loc_vals(l: &[i64; 4]) -> [V; 4] {
    [mkfix(l[0]), mkfix(l[1]), mkfix(l[2]), mkfix(l[3])]
}

/// The methods of the measured files that are defined somewhere: file index (into the
/// snapshot) -> [(owner, name, counter)], in the order of the classes and their tables.
fn defined_methods(c: &Cov) -> HashMap<usize, Vec<(V, u32, usize)>> {
    let pos: HashMap<usize, usize> = c.order.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut out: HashMap<usize, Vec<(V, u32, usize)>> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let copies = VIS_COPIES.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    for k in crate::gc::all_classes() {
        let entries: Vec<(u32, *const Method)> = unsafe { (*k).methods.iter().map(|(n, m)| (*n, *m)).collect() };
        for (name, m) in entries {
            if m.is_null() || copies.contains(&(m as usize)) {
                continue;
            }
            let addr = match unsafe { &(*m).body } {
                Body::Compiled(f) => *f as usize,
                Body::Proc(p) if is_proc(*p) => rproc(*p).func as usize,
                _ => continue,
            };
            let Some(&(fi, ctr)) = c.methods.get(&addr) else { continue };
            let Some(&si) = pos.get(&fi) else { continue };
            let owner = unsafe { (*m).owner };
            let owner = if owner.is_null() { k } else { owner };
            if seen.insert((owner as usize, fi, ctr)) {
                out.entry(si).or_default().push((owner as V, name, ctr));
            }
        }
    }
    out
}

fn peek() -> V {
    let (mode, snaps, defined) = {
        let mut g = COV.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = g.as_mut().filter(|c| c.state != State::Idle) else {
            drop(g);
            return rt_err("coverage measurement is not enabled");
        };
        let defined = if c.mode & METHODS != 0 { defined_methods(c) } else { HashMap::new() };
        (c.mode, snapshot(c), defined)
    };
    let result = hash_new();
    for (si, s) in snaps.iter().enumerate() {
        let lines = || {
            if mode & ONESHOT != 0 {
                freeze(ary_new(s.oneshot.iter().map(|&l| mkfix(l as i64)).collect()))
            } else {
                freeze(ary_new(s.lines.iter().map(|c| c.map_or(QNIL, |n| mkfix(n as i64))).collect()))
            }
        };
        let v = if mode == 0 {
            lines()
        } else {
            let h = hash_new();
            if mode & LINES != 0 {
                let l = lines();
                hset(h, mksym(sym(if mode & ONESHOT != 0 { "oneshot_lines" } else { "lines" })), l);
            }
            if mode & BRANCHES != 0 {
                let bh = hash_new();
                hset(h, mksym(sym("branches")), bh);
                let mut id = 0i64;
                for (kind, loc, targets) in &s.branches {
                    let l = loc_vals(loc);
                    let key = ary_new(vec![mksym(sym(kind)), mkfix(id), l[0], l[1], l[2], l[3]]);
                    id += 1;
                    let th = hash_new();
                    hset(bh, key, th);
                    for (tk, tl, n) in targets {
                        let l = loc_vals(tl);
                        let key = ary_new(vec![mksym(sym(tk)), mkfix(id), l[0], l[1], l[2], l[3]]);
                        id += 1;
                        hset(th, key, mkfix(*n as i64));
                    }
                }
            }
            if mode & METHODS != 0 {
                let mh = hash_new();
                hset(h, mksym(sym("methods")), mh);
                let mut list = defined.get(&si).cloned().unwrap_or_default();
                // (in the order of the definitions in the file)
                list.sort_by_key(|&(_, _, ctr)| s.method_order.iter().position(|&c| c == ctr).unwrap_or(usize::MAX));
                for (owner, name, ctr) in list {
                    let Some((loc, sname, n)) = s.methods.get(&ctr) else { continue };
                    let name = if sname.is_empty() { name } else { sym(sname) };
                    let l = loc_vals(loc);
                    let key = ary_new(vec![owner, mksym(name), l[0], l[1], l[2], l[3]]);
                    hset(mh, key, mkfix(*n as i64));
                }
            }
            h
        };
        hset(result, str_from(&s.path), v);
    }
    freeze(result)
}

fn clear(c: &mut Cov) {
    for &i in &c.order {
        let f = &mut c.files[i];
        if let Some(t) = f.counters.as_mut() {
            t.iter_mut().for_each(|x| *x = 0);
        }
        // (oneshot_lines: a line reported once isn't again)
        f.oneshot.clear();
    }
}

fn result(a: &[V]) -> V {
    if with(|c| c.state == State::Idle) {
        return rt_err("coverage measurement is not enabled");
    }
    let (mut stop, mut clr) = (true, true);
    if let Some(&o) = a.first() {
        let h = if is_hash(o) {
            o
        } else if respond_to(o, sym("to_hash")) {
            tri!(send(o, sym("to_hash"), &[], QNIL))
        } else {
            return type_error(&format!("no implicit conversion of {} into Hash", cname(o)));
        };
        stop = opt(h, "stop");
        clr = opt(h, "clear");
    }
    let r = tri!(peek());
    if stop && !clr {
        let loc = current_loc().map(|l| format!("{l}: ")).unwrap_or_default();
        eprintln!("{loc}warning: stop implies clear");
        clr = true;
    }
    with(|c| {
        if clr {
            clear(c);
        }
        if stop {
            c.state = State::Idle;
            for &i in &c.order {
                let f = &mut c.files[i];
                f.covered = false;
                f.counters = None;
                f.point(false);
            }
            c.order.clear();
        }
    });
    r
}

/// Coverage.line_stub(file): the file's lines array with every measured line 0.
fn line_stub(path: V) -> V {
    let p = if is_str(path) { path } else { tri!(send(path, sym("to_str"), &[], QNIL)) };
    let name = rstr(p).b.to_vec();
    let src = match crate::vfs::read_file(&name) {
        Ok(s) => s.into_owned(),
        Err(e) => {
            let n = String::from_utf8_lossy(&name).into_owned();
            let k = if e.kind() == std::io::ErrorKind::NotFound { "ENOENT" } else { "EACCES" };
            let cls = const_lookup(rt().c.object, sym("Errno")).and_then(|m| const_lookup(m as *mut Class, sym(k)));
            return match cls {
                Some(c) => raise(c as *mut Class, &format!("{} @ rb_sysopen - {n}", if k == "ENOENT" { "No such file or directory" } else { "Permission denied" })),
                None => raise(rt().c.runtime_error, &format!("cannot read {n}")),
            };
        }
    };
    // (File.foreach: a last line without a newline counts too)
    let mut n = src.iter().filter(|&&b| b == b'\n').count();
    if src.last().is_some_and(|&b| b != b'\n') {
        n += 1;
    }
    let mut lines = vec![QNIL; n];
    let real = std::fs::canonicalize(String::from_utf8_lossy(&name).as_ref()).map(|p| p.display().to_string()).unwrap_or_else(|_| String::from_utf8_lossy(&name).into_owned());
    with(|c| {
        let mode = c.branches_on() as usize;
        if let Some(f) = c.files.iter().find(|f| f.path == real || f.path.as_bytes() == name.as_slice()) {
            for &(_, line) in &f.lines[mode] {
                if line >= 1 {
                    if line > lines.len() {
                        lines.resize(line, QNIL);
                    }
                    lines[line - 1] = mkfix(0);
                }
            }
        }
    });
    ary_new(lines)
}

fn init_coverage() {
    let m = new_module("Coverage");
    set_global_const("Coverage", m as V);
    defs(m, "supported?", 1, 1, |_, a, _| {
        if !is_sym(a[0]) {
            return type_error(&format!("wrong argument type {} (expected Symbol)", cname(a[0])));
        }
        let n = sym_str(sym_id(a[0]));
        bool_v(matches!(n.as_str(), "lines" | "oneshot_lines" | "branches" | "methods" | "eval"))
    });
    defs(m, "setup", 0, 1, |_, a, _| setup(a));
    defs(m, "start", 0, 1, |_, a, _| {
        tri!(setup(a));
        resume()
    });
    defs(m, "resume", 0, 0, |_, _, _| resume());
    defs(m, "suspend", 0, 0, |_, _, _| suspend());
    defs(m, "result", 0, 1, |_, a, _| result(a));
    defs(m, "peek_result", 0, 0, |_, _, _| peek());
    defs(m, "state", 0, 0, |_, _, _| {
        mksym(sym(match with(|c| c.state) {
            State::Idle => "idle",
            State::Suspended => "suspended",
            State::Running => "running",
        }))
    });
    defs(m, "running?", 0, 0, |_, _, _| bool_v(with(|c| c.state == State::Running)));
    defs(m, "line_stub", 1, 1, |_, a, _| line_stub(a[0]));
}

pub fn init() {
    crate::compat::provide_feature(&["kenny/coverage_native"], Some(init_coverage));
}
