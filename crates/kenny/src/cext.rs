//! C extensions compiled into the binary: those `--ext <dir>` names, and those of the gems in a
//! Gemfile.lock (`ext/<name>/extconf.rb`). A Kenny binary doesn't load shared libraries, so an
//! extension's C (and C++) sources are compiled against Kenny's `ruby.h` and linked in, and
//! `require "<its feature>"` runs its `Init_<name>`.
//!
//! What a gem's extension is built with comes from mkmf, as `gem install` built it: the
//! Makefile its extconf.rb wrote (RubyGems leaves it in the gem's ext directory; without one,
//! extconf.rb is run with the Ruby the program is built against, in a scratch directory).
//! From it: the feature name (`create_makefile("msgpack/msgpack")`), the sources, the
//! `HAVE_*` macros of `have_func`/`have_header` and the other `-D`/`-I`/`-std` flags, and
//! the libraries to link (`-lstdc++` makes it C++). The Ruby's own header directories are
//! left out (Kenny's `ruby.h` replaces them); the extconf checks ran against CRuby's
//! headers, so a check can find a function Kenny's API lacks — then the extension doesn't
//! compile or link, and is left out with a warning (requiring it raises LoadError, as before).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ext {
    /// what the warnings call it: "msgpack 1.8.5 (ext/msgpack)", or the --ext directory
    pub label: String,
    /// `require` name: "msgpack/msgpack" (`--ext`: the Init name)
    pub features: Vec<(String, String)>,
    pub sources: Vec<PathBuf>,
    pub cflags: Vec<String>,
    pub cxxflags: Vec<String>,
    /// linker arguments (-l, -L, -framework)
    pub libs: Vec<String>,
    /// from a lock file: a failure leaves the extension out instead of failing the build
    pub optional: bool,
}

impl Ext {
    fn cxx(&self) -> bool {
        self.sources.iter().any(|s| is_cxx(s))
    }
}

// (".S": assembler sources run through the C preprocessor, e.g. bcrypt's x86.S)
const C_EXTS: &[&str] = &["c", "m", "S"];
const CXX_EXTS: &[&str] = &["cc", "cpp", "cxx", "C", "mm"];

fn is_cxx(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| CXX_EXTS.contains(&e))
}

fn quiet() -> bool {
    std::env::var_os("KENNY_QUIET").is_some()
}

/// An `--ext <dir>` (or file): its C/C++ files, each `Init_x` they define registered as `x`.
pub fn from_dir(e: &Path) -> Result<Ext, String> {
    let files: Vec<PathBuf> = if e.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(e)
            .map_err(|x| format!("{}: {x}", e.display()))?
            .flatten()
            .map(|d| d.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| x == "c" || CXX_EXTS.contains(&x)))
            .collect();
        v.sort();
        v
    } else {
        vec![e.to_path_buf()]
    };
    let dir = if e.is_dir() { e.to_path_buf() } else { e.parent().unwrap_or(Path::new(".")).to_path_buf() };
    let mut inits: Vec<String> = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for line in text.lines() {
            let l = line.trim_start();
            let l = l.strip_prefix("extern \"C\"").map(str::trim_start).unwrap_or(l);
            if let Some(rest) = l.strip_prefix("void Init_").or_else(|| l.strip_prefix("RUBY_FUNC_EXPORTED void Init_")) {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() && rest[name.len()..].trim_start().starts_with('(') && !inits.contains(&name) {
                    inits.push(name);
                }
            }
        }
    }
    let inc = format!("-I{}", dir.display());
    Ok(Ext {
        label: e.display().to_string(),
        features: inits.into_iter().map(|n| (n.clone(), n)).collect(),
        sources: files,
        cflags: vec![inc.clone()],
        cxxflags: vec![inc],
        libs: vec![],
        optional: false,
    })
}

/// A gem's extension (the directory of its extconf.rb), from its mkmf Makefile.
pub fn from_gem(gem: &str, dir: &Path, work: &Path) -> Result<Ext, String> {
    let label = format!("{gem} ({})", dir.file_name().unwrap_or_default().to_string_lossy());
    let in_place = dir.join("Makefile");
    let (mk, build_dir) = if in_place.is_file() && std::fs::read_to_string(&in_place).is_ok_and(|t| t.contains("create_makefile") || t.contains("TARGET =")) {
        (std::fs::read_to_string(&in_place).map_err(|e| e.to_string())?, dir.to_path_buf())
    } else {
        let b = work.join(format!("extconf-{}", gem.replace(' ', "-")));
        std::fs::create_dir_all(&b).map_err(|e| e.to_string())?;
        let ruby = crate::ruby::get().ok_or("no Ruby to run extconf.rb with")?;
        let out = ruby.command().arg(dir.join("extconf.rb")).current_dir(&b).output().map_err(|e| format!("running extconf.rb: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
            return Err(format!("{label}: extconf.rb failed:\n{}", tail(&err, 10)));
        }
        (std::fs::read_to_string(b.join("Makefile")).map_err(|e| format!("{label}: extconf.rb wrote no Makefile ({e})"))?, b)
    };
    let mk = Makefile::parse(&mk);
    let rel = |p: &str| -> PathBuf {
        let p = Path::new(p);
        if p.is_absolute() { p.to_path_buf() } else { build_dir.join(p) }
    };
    let srcdir = rel(&mk.get("srcdir"));
    // the Ruby's header directories (Kenny's ruby.h stands in for them)
    let ruby_hdrs: Vec<PathBuf> = ["topdir", "hdrdir", "arch_hdrdir", "rubyhdrdir", "rubyarchhdrdir"]
        .iter()
        .map(|v| mk.get(v))
        .filter(|v| !v.is_empty())
        .map(|v| rel(&v))
        .collect();
    let rubyish = |d: &Path| ruby_hdrs.iter().any(|h| d.starts_with(h)) || d.join("ruby.h").is_file() || d.join("ruby/config.h").is_file();
    let target = mk.get("TARGET");
    if target.is_empty() {
        return Err(format!("{label}: its Makefile names no TARGET"));
    }
    let prefix = mk.get("target_prefix");
    let feature = format!("{}/{target}", prefix.trim_matches('/')).trim_start_matches('/').to_string();
    // the sources: one per object file, found in srcdir or VPATH
    let mut dirs = vec![srcdir.clone()];
    for v in mk.get("VPATH").split(':').filter(|s| !s.is_empty()) {
        let d = rel(v);
        if !rubyish(&d) && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    let mut sources = Vec::new();
    for o in words(&mk.get("OBJS")) {
        let Some(stem) = o.strip_suffix(".o") else { continue };
        let found = dirs.iter().find_map(|d| C_EXTS.iter().chain(CXX_EXTS).map(|x| d.join(format!("{stem}.{x}"))).find(|p| p.is_file()));
        match found {
            Some(p) => sources.push(p),
            None => return Err(format!("{label}: no source for {o}")),
        }
    }
    if sources.is_empty() {
        return Err(format!("{label}: its Makefile names no sources"));
    }
    // compiler flags that say what to compile: macros, include directories, the language standard
    let pick = |vars: &[&str]| -> Vec<String> {
        let mut out = Vec::new();
        let mut ws = vars.iter().flat_map(|v| words(&mk.get(v))).peekable();
        while let Some(w) = ws.next() {
            if w.starts_with("-D") || w.starts_with("-U") || w.starts_with("-std=") {
                out.push(w);
            } else if w == "-include" {
                if let Some(f) = ws.next() {
                    out.push(w);
                    out.push(rel(&f).display().to_string());
                }
            } else if let Some(d) = w.strip_prefix("-I") {
                let d = if d.is_empty() { ws.next().unwrap_or_default() } else { d.to_string() };
                let p = rel(&d);
                if !rubyish(&p) {
                    out.push(format!("-I{}", p.display()));
                }
            }
        }
        out
    };
    let mut cflags = pick(&["INCFLAGS", "CPPFLAGS", "CFLAGS"]);
    let mut cxxflags = pick(&["INCFLAGS", "CPPFLAGS", "CXXFLAGS"]);
    // (a `-std=` for the other language, from the shared CPPFLAGS: charlock_holmes puts its
    // -std=c++20 there; clang refuses it for C)
    cflags.retain(|w| !(w.starts_with("-std=") && w.contains("++")));
    cxxflags.retain(|w| !(w.starts_with("-std=") && !w.contains("++")));
    // (mkmf's own `-I. -I$(srcdir)` come first anyway; an extconf.h is in the build directory)
    for f in [&mut cflags, &mut cxxflags] {
        f.insert(0, format!("-I{}", build_dir.display()));
        f.insert(1, format!("-I{}", srcdir.display()));
    }
    let mut libs = Vec::new();
    let mut ws = ["LOCAL_LIBS", "LIBS", "LIBPATH", "DLDFLAGS", "LDFLAGS"].iter().flat_map(|v| words(&mk.get(v))).peekable();
    while let Some(w) = ws.next() {
        if let Some(l) = w.strip_prefix("-l") {
            // (the C++ runtime is linked for C++ sources; Ruby's own libraries are Kenny)
            if !(l.starts_with("ruby") || ["pthread", "m", "dl", "c", "objc", "stdc++", "c++", "gmp", "crypt"].contains(&l)) && !libs.contains(&w) {
                libs.push(w);
            }
        } else if w == "-framework" {
            if let Some(f) = ws.next() {
                if !libs.contains(&f) {
                    libs.push(w);
                    libs.push(f);
                }
            }
        } else if let Some(d) = w.strip_prefix("-L") {
            let p = rel(d);
            if p.is_dir() && !rubyish(&p) && p != build_dir && !libs.contains(&format!("-L{}", p.display())) {
                libs.push(format!("-L{}", p.display()));
            }
        }
    }
    let wants_cxx = words(&mk.get("LIBS")).iter().any(|w| w == "-lstdc++" || w == "-lc++");
    let mut e = Ext { label, features: vec![(feature, target)], sources, cflags, cxxflags, libs, optional: true };
    if wants_cxx && !e.cxx() {
        // a C extension linking the C++ runtime (it calls a C++ library)
        e.libs.push(cxx_runtime().into());
    }
    Ok(e)
}

/// The features the extensions' C code requires by name: `rb_require("date")`.
pub fn required_features(exts: &[Ext]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for src in exts.iter().flat_map(|e| &e.sources) {
        let Ok(text) = std::fs::read(src) else { continue };
        let text = String::from_utf8_lossy(&text);
        for (i, _) in text.match_indices("rb_require(\"") {
            let rest = &text[i + "rb_require(\"".len()..];
            if let Some(end) = rest.find('"') {
                let f = &rest[..end];
                if !f.is_empty() && !f.contains('\\') && !out.iter().any(|x| x == f) {
                    out.push(f.to_string());
                }
            }
        }
    }
    out
}

fn cxx_runtime() -> &'static str {
    if cfg!(target_os = "macos") { "-lc++" } else { "-lstdc++" }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// A Makefile's variables (`NAME = value`, with `\` continuations), expanded on lookup.
struct Makefile {
    vars: HashMap<String, String>,
}

impl Makefile {
    fn parse(text: &str) -> Makefile {
        let mut vars = HashMap::new();
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            if line.starts_with('\t') || line.trim_start().starts_with('#') {
                continue;
            }
            let mut l = line.to_string();
            while l.ends_with('\\') {
                l.pop();
                l.push(' ');
                match lines.next() {
                    Some(n) => l.push_str(n),
                    None => break,
                }
            }
            let Some(eq) = l.find('=') else { continue };
            let (name, value) = (&l[..eq], &l[eq + 1..]);
            let name = name.trim_end_matches([':', '?', '+']).trim();
            if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            vars.insert(name.to_string(), value.trim().to_string());
        }
        Makefile { vars }
    }

    fn get(&self, name: &str) -> String {
        self.expand(self.vars.get(name).map(String::as_str).unwrap_or(""), 0)
    }

    fn expand(&self, s: &str, depth: usize) -> String {
        if depth > 20 || !s.contains('$') {
            return s.to_string();
        }
        let b = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'$' && i + 1 < b.len() {
                let (open, close) = match b[i + 1] {
                    b'(' => (b'(', b')'),
                    b'{' => (b'{', b'}'),
                    b'$' => {
                        out.push('$');
                        i += 2;
                        continue;
                    }
                    _ => {
                        i += 2;
                        continue;
                    }
                };
                let mut d = 0;
                let mut j = i + 1;
                while j < b.len() {
                    if b[j] == open {
                        d += 1;
                    } else if b[j] == close {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                let inner = &s[i + 2..j.min(s.len())];
                // `$(V:0=)` substitutions and functions: not needed for what is read here
                if !inner.contains([':', ' ']) {
                    let v = self.vars.get(inner).map(String::as_str).unwrap_or("");
                    out.push_str(&self.expand(v, depth + 1));
                }
                i = j + 1;
            } else {
                let start = i;
                while i < b.len() && b[i] != b'$' {
                    i += 1;
                }
                out.push_str(&s[start..i]);
            }
        }
        out
    }
}

/// Shell words (the Makefile's flags go through `sh`): quotes and backslashes.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            '\'' => {
                have = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                have = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                if !matches!(n, '"' | '\\' | '$' | '`') {
                                    cur.push('\\');
                                }
                                cur.push(n);
                            }
                        }
                        _ => cur.push(c),
                    }
                }
            }
            '\\' => {
                have = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            _ => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

/// The compiled extensions: the C-API shim's objects, each extension's objects and the
/// registration object (which names the extensions linked in).
pub struct Built {
    shim: Vec<PathBuf>,
    exts: Vec<(Ext, Vec<PathBuf>)>,
    reg: PathBuf,
    work: PathBuf,
}

/// The headers an extension may include: Kenny's ruby.h under the names CRuby's API uses.
const HEADERS: &[&str] = &[
    "ruby/ruby.h", "ruby/encoding.h", "ruby/thread.h", "ruby/io.h", "ruby/st.h", "ruby/intern.h", "ruby/version.h",
    "ruby/re.h", "ruby/util.h", "ruby/defines.h", "ruby/config.h", "ruby/missing.h", "ruby/vm.h", "ruby/assert.h",
    "ruby/atomic.h", "ruby/memory_view.h", "ruby/oniguruma.h", "ruby/onigmo.h", "ruby/regex.h", "ruby/thread_native.h",
    "ruby/debug.h", "ruby/backward.h", "ruby/internal/symbol.h", "ruby/internal/encoding/encoding.h",
    "ruby/internal/intern/string.h",
];

pub fn compile(exts: Vec<Ext>, work: &Path, ruby_h: &str, shim: &str) -> Result<Built, String> {
    let inc = work.join("include");
    std::fs::create_dir_all(inc.join("ruby")).map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby.h"), ruby_h).map_err(|e| e.to_string())?;
    for h in HEADERS {
        let p = inc.join(h);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let up = "../".repeat(h.matches('/').count());
        std::fs::write(&p, format!("#include \"{up}ruby.h\"\n")).map_err(|e| e.to_string())?;
    }
    std::fs::write(work.join("kenny_capi.c"), shim).map_err(|e| e.to_string())?;
    // one job per source file, compiled in parallel
    struct Job {
        src: PathBuf,
        obj: PathBuf,
        cxx: bool,
        flags: Vec<String>,
        ext: Option<usize>,
    }
    let mut jobs = vec![Job { src: work.join("kenny_capi.c"), obj: work.join("kenny_capi.o"), cxx: false, flags: vec![], ext: None }];
    for (i, e) in exts.iter().enumerate() {
        for (k, s) in e.sources.iter().enumerate() {
            let cxx = is_cxx(s);
            let stem = s.file_stem().unwrap_or_default().to_string_lossy();
            jobs.push(Job { src: s.clone(), obj: work.join(format!("ext{i}-{k}-{stem}.o")), cxx, flags: if cxx { e.cxxflags.clone() } else { e.cflags.clone() }, ext: Some(i) });
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<(), String>>>> = jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(jobs.len()).max(1);
    std::thread::scope(|sc| {
        for _ in 0..n {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(j) = jobs.get(i) else { break };
                let mut c = if j.cxx { Command::new(std::env::var("CXX").unwrap_or_else(|_| "c++".into())) } else { crate::cc() };
                c.args([if j.cxx { "-std=gnu++17" } else { "-std=gnu17" }, "-O2", "-fPIC", "-w", "-c"]);
                c.arg("-I").arg(&inc).args(&j.flags).arg(&j.src).arg("-o").arg(&j.obj);
                let r = match c.output() {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(format!("compiling {} failed:\n{}", j.src.display(), tail(&String::from_utf8_lossy(&o.stderr), 20))),
                    Err(e) => Err(format!("failed to run the {} compiler: {e}", if j.cxx { "C++" } else { "C" })),
                };
                *results[i].lock().unwrap() = Some(r);
            });
        }
    });
    let mut failed: Vec<Option<String>> = vec![None; exts.len()];
    for (j, r) in jobs.iter().zip(results) {
        if let Some(Err(e)) = r.into_inner().unwrap() {
            match j.ext {
                None => return Err(e),
                Some(i) if !exts[i].optional => return Err(e),
                Some(i) => {
                    failed[i].get_or_insert(e);
                }
            }
        }
    }
    let mut built = Vec::new();
    for (i, e) in exts.into_iter().enumerate() {
        if let Some(why) = &failed[i] {
            warn_left_out(&e, &first_error(why));
            continue;
        }
        let objs = jobs.iter().filter(|j| j.ext == Some(i)).map(|j| j.obj.clone()).collect();
        built.push((e, objs));
    }
    let mut b = Built { shim: vec![work.join("kenny_capi.o")], exts: built, reg: work.join("kenny_exts.o"), work: work.to_path_buf() };
    b.register()?;
    Ok(b)
}

/// The first compiler error of a failed compile (for a one-line warning).
fn first_error(log: &str) -> String {
    log.lines().find(|l| l.contains("error:")).map(|l| l.trim().to_string()).unwrap_or_else(|| log.lines().next().unwrap_or("").to_string())
}

fn warn_left_out(e: &Ext, why: &str) {
    if !quiet() {
        eprintln!("kenny: warning: the C extension of {} is left out (`require` of it raises LoadError): {why}", e.label);
    }
}

impl Built {
    /// (Re)write the object that registers the extensions: a constructor calling
    /// `kn_register_ext(feature, Init_x)` for each.
    fn register(&mut self) -> Result<(), String> {
        let mut reg = String::from("extern void kn_register_ext(const char *, void (*)(void));\n");
        let mut inits: Vec<&str> = Vec::new();
        for (e, _) in &self.exts {
            for (_, init) in &e.features {
                if !inits.contains(&init.as_str()) {
                    inits.push(init);
                }
            }
        }
        for n in &inits {
            reg.push_str(&format!("void Init_{n}(void);\n"));
        }
        reg.push_str("__attribute__((constructor)) static void kn_register_all_exts(void) {\n");
        for (e, _) in &self.exts {
            for (f, init) in &e.features {
                reg.push_str(&format!("  kn_register_ext(\"{}\", Init_{init});\n", f.replace('\\', "\\\\").replace('"', "\\\"")));
            }
        }
        reg.push_str("}\n");
        let c = self.work.join("kenny_exts.c");
        std::fs::write(&c, reg).map_err(|e| e.to_string())?;
        let o = crate::cc().args(["-O2", "-w", "-c"]).arg(&c).arg("-o").arg(&self.reg).output().map_err(|e| format!("failed to run the C compiler: {e}"))?;
        if !o.status.success() {
            return Err(format!("compiling {} failed:\n{}", c.display(), String::from_utf8_lossy(&o.stderr)));
        }
        Ok(())
    }

    pub fn objects(&self) -> Vec<PathBuf> {
        let mut v = self.shim.clone();
        for (_, o) in &self.exts {
            v.extend(o.iter().cloned());
        }
        v.push(self.reg.clone());
        v
    }

    /// Linker arguments the extensions need (libraries, the C++ runtime).
    pub fn link_args(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (e, _) in &self.exts {
            v.extend(e.libs.iter().cloned());
        }
        if self.exts.iter().any(|(e, _)| e.cxx()) && !v.iter().any(|l| l == cxx_runtime()) {
            v.push(cxx_runtime().into());
        }
        v
    }

    /// After a link failed with `log`: leave out the gems' extensions that use a symbol the
    /// link couldn't find (a C API function Kenny lacks). Whether any was left out.
    pub fn drop_unresolved(&mut self, log: &str) -> Result<bool, String> {
        let missing = undefined_symbols(log);
        if missing.is_empty() {
            return Ok(false);
        }
        let mut dropped = false;
        let mut keep = Vec::new();
        for (e, objs) in std::mem::take(&mut self.exts) {
            let used = e.optional.then(|| undefined_in(&objs).into_iter().find(|s| missing.contains(s))).flatten();
            match used {
                Some(sym) => {
                    warn_left_out(&e, &format!("Kenny's C API has no `{sym}`"));
                    dropped = true;
                }
                None => keep.push((e, objs)),
            }
        }
        self.exts = keep;
        if dropped {
            self.register()?;
        }
        Ok(dropped)
    }
}

/// The symbols a failed link reports undefined (without the platform's `_` prefix).
fn undefined_symbols(log: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in log.lines() {
        let t = l.trim();
        // macOS ld: `  "_rb_foo", referenced from:`
        if let Some(rest) = t.strip_prefix('"') {
            if let Some((s, tail)) = rest.split_once('"') {
                if tail.contains("referenced from") {
                    out.push(s.strip_prefix('_').unwrap_or(s).to_string());
                }
            }
        }
        // GNU ld / lld: undefined reference to `rb_foo'  /  undefined symbol: rb_foo
        if let Some((_, rest)) = t.split_once("undefined reference to ") {
            out.push(rest.trim_matches(|c| c == '`' || c == '\'' || c == '"').to_string());
        } else if let Some((_, rest)) = t.split_once("undefined symbol: ") {
            out.push(rest.trim().to_string());
        }
    }
    out
}

fn undefined_in(objs: &[PathBuf]) -> Vec<String> {
    let Ok(o) = Command::new("nm").arg("-u").args(objs).output() else { return vec![] };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .map(|s| if cfg!(target_os = "macos") { s.strip_prefix('_').unwrap_or(s) } else { s }.to_string())
        .collect()
}
