//! Bundler projects: when the entry file has a `Gemfile.lock` next to it (or in a parent
//! directory), `kenny build` resolves the locked gems itself — from the lock file and the
//! installed gem directories (GEM_HOME / GEM_PATH, the system Ruby's `Gem.path`, ~/.gem, rvm,
//! rbenv) — so no `-I` flags are needed. The Gemfile's groups, `require:` options and
//! platforms are captured into a generated `kenny/bundle.rb`, which Kenny's `bundler` overlay
//! (lib/kenny/bundler.rb) uses for `require "bundler/setup"` and `Bundler.require(*groups)`.
//!
//! Groups left out of the binary: `BUNDLE_WITHOUT` (env or `.bundle/config`), else
//! `development:test`. `BUNDLE_WITH` adds groups back.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Bundle {
    #[allow(dead_code)]
    pub root: PathBuf,
    /// require paths of the gems compiled in, in dependency order
    pub load_paths: Vec<PathBuf>,
    /// the project's own Ruby directories (app/, config/, lib/, db/ ...), compiled in whole
    pub project_dirs: Vec<PathBuf>,
    /// `app/` and `config/` of gems that are Rails engines (loaded by path at runtime)
    pub engine_dirs: Vec<PathBuf>,
    /// generated `kenny/bundle.rb`
    pub ruby: String,
    pub without: Vec<String>,
    /// version of a psych gem in the lock file
    pub psych_version: Option<String>,
    /// the C extensions of the gems compiled in (`ext/<name>/extconf.rb`), except those
    /// Kenny replaces (`NATIVE`)
    pub exts: Vec<GemExt>,
}

/// A gem's C extension: built into the binary (crate::cext).
pub struct GemExt {
    /// "msgpack 1.8.5"
    pub gem: String,
    /// the directory of its extconf.rb
    pub dir: PathBuf,
}

#[derive(Default, Clone)]
struct Dep {
    name: String,
    groups: Vec<String>,
    /// None: require the gem's name; Some(list): these files (`require: false` is empty)
    autorequire: Option<Vec<String>>,
    /// platform filter, empty = all
    platforms: Vec<String>,
}

struct Spec {
    name: String,
    /// version as written in the lock, including any platform suffix (1.15.7-arm64-darwin)
    version: String,
    deps: Vec<String>,
    source: Source,
}

#[derive(Clone)]
enum Source {
    Gem,
    Path(PathBuf),
    Git { revision: String },
}

/// Gems whose job the Kenny binary does itself: Rack application servers (the binary has its
/// own HTTP server). They are not compiled in, and `Bundler.require` skips them.
const REPLACED: &[&str] = &["puma", "unicorn", "thin", "passenger", "falcon", "webrick", "pitchfork", "iodine"];

/// Gems whose C extensions Kenny replaces (natively in the runtime, or by an overlay file
/// that doesn't load the extension), or whose extensions need CRuby's VM internals: their
/// `ext/` sources are not built.
const NATIVE: &[&str] = &[
    "nokogiri", "racc", "bigdecimal", "json", "date", "strscan", "sqlite3", "openssl", "psych", "stringio",
    "io-console", "io-wait", "io-nonblock", "nio4r", "digest", "zlib", "etc", "erb", "cgi", "pathname", "ripper",
    "prism", "ffi", "fiddle", "bootsnap", "byebug", "debug", "debug_inspector", "puma", "monitor", "socket", "pty",
    "fcntl", "syslog", "readline-ext", "objspace", "coverage",
];

/// Project directories that are never compiled in (tests, generated files, vendored gems).
const SKIP_DIRS: &[&str] = &["test", "spec", "tmp", "log", "node_modules", "vendor", "bin", "public", "storage", "coverage", "features"];

pub fn find_lockfile(entry: &Path) -> Option<PathBuf> {
    if std::env::var_os("KENNY_NO_BUNDLE").is_some() {
        return None;
    }
    let start = entry.canonicalize().ok()?;
    let mut dir = start.parent()?.to_path_buf();
    loop {
        for (lock, gemfile) in [("Gemfile.lock", "Gemfile"), ("gems.locked", "gems.rb")] {
            if dir.join(lock).is_file() && dir.join(gemfile).is_file() {
                return Some(dir.join(lock));
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn resolve(lockfile: &Path) -> Result<Bundle, String> {
    let root = lockfile.parent().unwrap().to_path_buf();
    let gemfile = if lockfile.file_name().is_some_and(|n| n == "gems.locked") { root.join("gems.rb") } else { root.join("Gemfile") };
    let lock = std::fs::read_to_string(lockfile).map_err(|e| format!("{}: {e}", lockfile.display()))?;
    let (specs, lock_reqs, bundled_with) = parse_lock(&lock, &root);
    let lock_deps: Vec<String> = lock_reqs.iter().map(|(n, _)| n.clone()).collect();
    let gemfile_src = std::fs::read(&gemfile).map_err(|e| format!("{}: {e}", gemfile.display()))?;
    let mut deps = parse_gemfile(&gemfile_src);
    // the lock's DEPENDENCIES is the authority on which gems the Gemfile declares
    deps.retain(|d| lock_deps.contains(&d.name));
    let mut seen: HashSet<String> = deps.iter().map(|d| d.name.clone()).collect();
    for n in &lock_deps {
        if seen.insert(n.clone()) {
            deps.push(Dep { name: n.clone(), groups: vec!["default".into()], ..Default::default() });
        }
    }

    let (without, with) = group_settings(&root);
    let included = |d: &Dep| platform_ok(&d.platforms) && d.groups.iter().any(|g| !without.contains(g) || with.contains(g));

    // gems reachable from the included dependencies
    let by_name: HashMap<&str, Vec<&Spec>> = specs.iter().fold(HashMap::new(), |mut m, s| {
        m.entry(s.name.as_str()).or_default().push(s);
        m
    });
    let gem_dirs = gem_dirs();
    let mut order: Vec<&str> = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    fn visit<'a>(n: &'a str, by_name: &HashMap<&str, Vec<&'a Spec>>, visited: &mut HashSet<&'a str>, order: &mut Vec<&'a str>) {
        let Some(list) = by_name.get(n) else { return };
        let Some(s) = list.first() else { return };
        if !visited.insert(s.name.as_str()) {
            return;
        }
        for d in &s.deps {
            visit(d, by_name, visited, order);
        }
        order.push(s.name.as_str());
    }
    for d in deps.iter().filter(|d| included(d) && !REPLACED.contains(&d.name.as_str())) {
        if let Some((k, _)) = by_name.get_key_value(d.name.as_str()) {
            visit(k, &by_name, &mut visited, &mut order);
        }
    }

    let mut load_paths = Vec::new();
    let mut spec_rows = Vec::new();
    let mut missing = Vec::new();
    let mut engine_dirs = Vec::new();
    let mut exts = Vec::new();
    for name in &order {
        // Kenny provides Bundler itself (lib/kenny/bundler.rb)
        if *name == "bundler" {
            continue;
        }
        let candidates = &by_name[name];
        let mut found = None;
        for s in candidates {
            if let Some(f) = locate(s, &gem_dirs, &root) {
                found = Some((s, f));
                break;
            }
        }
        match found {
            Some((s, Located::Dir(dir, req, ext))) => {
                if !NATIVE.contains(&s.name.as_str()) {
                    for e in ext.iter().filter(|e| e.ends_with("extconf.rb")) {
                        let d = dir.join(e);
                        if let Some(p) = d.parent().filter(|_| d.is_file()) {
                            exts.push(GemExt { gem: format!("{} {}", s.name, s.version), dir: p.to_path_buf() });
                        }
                    }
                }
                let paths: Vec<PathBuf> = req.iter().map(|r| dir.join(r)).filter(|p| p.is_dir()).collect();
                if dir.join("app").is_dir() {
                    engine_dirs.extend(["app", "config"].iter().map(|d| dir.join(d)).filter(|d| d.is_dir()));
                }
                load_paths.extend(paths.iter().cloned());
                spec_rows.push((s.name.clone(), s.version.clone(), dir, paths));
            }
            // a default gem: its files are part of the standard library
            Some((s, Located::Default)) => spec_rows.push((s.name.clone(), s.version.clone(), PathBuf::new(), vec![])),
            None => missing.push(format!("{} ({})", name, candidates[0].version)),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "{}: these locked gems are not installed (run `bundle install`): {}\n(searched: {})",
            lockfile.display(),
            missing.join(", "),
            gem_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
        ));
    }

    let mut project_dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        entries.sort();
        for p in entries {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            if n.starts_with('.') || SKIP_DIRS.contains(&n.as_str()) {
                continue;
            }
            project_dirs.push(p);
        }
    }

    let compiled: HashSet<&str> = spec_rows.iter().map(|r| r.0.as_str()).collect();
    let mut rb = String::new();
    rb.push_str("# Generated by `kenny build` from the Gemfile and its lock file.\nmodule Bundler\n  module Kenny\n");
    rb.push_str(&format!("    ROOT = {}\n", rstr(&root.display().to_string())));
    rb.push_str(&format!("    GEMFILE = {}\n", rstr(&gemfile.display().to_string())));
    rb.push_str(&format!("    LOCKFILE = {}\n", rstr(&lockfile.display().to_string())));
    rb.push_str(&format!("    WITHOUT = [{}]\n", without.iter().filter(|g| !with.contains(*g)).map(|g| format!(":{g}")).collect::<Vec<_>>().join(", ")));
    rb.push_str(&format!("    # gems replaced by the binary itself (its HTTP server)\n    REPLACED = [{}].freeze\n", REPLACED.iter().map(|g| rstr(g)).collect::<Vec<_>>().join(", ")));
    rb.push_str(&format!("    # the lock's BUNDLED WITH\n    BUNDLED_WITH = {}\n", bundled_with.as_deref().map(rstr).unwrap_or_else(|| "nil".into())));
    rb.push_str("    # name, groups, autorequire (nil: the gem's name), for this platform, compiled in,\n    # version requirements (from the lock)\n    DEPENDENCIES = [\n");
    for d in &deps {
        let auto = match &d.autorequire {
            None => "nil".to_string(),
            Some(v) => format!("[{}]", v.iter().map(|s| rstr(s)).collect::<Vec<_>>().join(", ")),
        };
        let reqs = lock_reqs.iter().find(|(n, _)| *n == d.name).map(|(_, r)| r.iter().map(|x| rstr(x)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        rb.push_str(&format!(
            "      [{}, [{}], {}, {}, {}, [{}]],\n",
            rstr(&d.name),
            d.groups.iter().map(|g| format!(":{g}")).collect::<Vec<_>>().join(", "),
            auto,
            platform_ok(&d.platforms),
            compiled.contains(d.name.as_str()) && included(d),
            reqs
        ));
    }
    rb.push_str("    ].freeze\n    # name, version, gem directory, require paths\n    SPECS = [\n");
    for (n, v, dir, paths) in &spec_rows {
        rb.push_str(&format!(
            "      [{}, {}, {}, [{}]],\n",
            rstr(n),
            rstr(v),
            rstr(&dir.display().to_string()),
            paths.iter().map(|p| rstr(&p.display().to_string())).collect::<Vec<_>>().join(", ")
        ));
    }
    rb.push_str("    ].freeze\n  end\nend\n");
    let without = without.into_iter().filter(|g| !with.contains(g)).collect();
    let psych_version = specs.iter().find(|s| s.name == "psych").map(|s| s.version.split('-').next().unwrap_or("").to_string());
    Ok(Bundle { root, load_paths, project_dirs, engine_dirs, ruby: rb, without, psych_version, exts })
}

fn rstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' | '#' => {
                o.push('\\');
                o.push(c);
            }
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Kenny runs as the `ruby` platform (like MRI); Windows/JRuby-only gems are skipped.
fn platform_ok(platforms: &[String]) -> bool {
    platforms.is_empty()
        || platforms.iter().any(|p| {
            let p = p.as_str();
            p == "ruby" || p == "mri" || p.starts_with("ruby_") || p.starts_with("mri_")
        })
}

fn group_settings(root: &Path) -> (Vec<String>, Vec<String>) {
    let split = |s: &str| s.split([':', ' ', ',']).filter(|x| !x.is_empty()).map(|x| x.to_string()).collect::<Vec<_>>();
    let config = std::fs::read_to_string(root.join(".bundle/config")).unwrap_or_default();
    let from_config = |key: &str| {
        config.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        })
    };
    let without = std::env::var("BUNDLE_WITHOUT").ok().or_else(|| from_config("BUNDLE_WITHOUT")).unwrap_or_else(|| "development:test".into());
    let with = std::env::var("BUNDLE_WITH").ok().or_else(|| from_config("BUNDLE_WITH")).unwrap_or_default();
    (split(&without), split(&with))
}

// ---------- Gemfile.lock ----------

/// The lock's specs, its DEPENDENCIES (name, requirements) and BUNDLED WITH version.
fn parse_lock(text: &str, root: &Path) -> (Vec<Spec>, Vec<(String, Vec<String>)>, Option<String>) {
    let mut specs = Vec::new();
    let mut deps = Vec::new();
    let mut bundled_with = None;
    let mut section = "";
    let mut source = Source::Gem;
    let mut remote = String::new();
    let mut revision = String::new();
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            section = match line.trim() {
                "GEM" => "GEM",
                "PATH" => "PATH",
                "GIT" => "GIT",
                "DEPENDENCIES" => "DEPENDENCIES",
                "BUNDLED WITH" => "BUNDLED WITH",
                _ => "",
            };
            remote.clear();
            revision.clear();
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        match section {
            "GEM" | "PATH" | "GIT" => {
                if indent == 2 {
                    if let Some(r) = t.strip_prefix("remote: ") {
                        remote = r.to_string();
                    } else if let Some(r) = t.strip_prefix("revision: ") {
                        revision = r.to_string();
                    }
                    source = match section {
                        "PATH" => Source::Path(root.join(&remote)),
                        "GIT" => Source::Git { revision: revision.clone() },
                        _ => Source::Gem,
                    };
                } else if indent == 4 {
                    if let Some((n, v)) = t.split_once(" (") {
                        specs.push(Spec { name: n.to_string(), version: v.trim_end_matches(')').to_string(), deps: vec![], source: source.clone() });
                    }
                } else if indent == 6 {
                    if let Some(s) = specs.last_mut() {
                        let n = t.split(' ').next().unwrap_or("");
                        s.deps.push(n.to_string());
                    }
                }
            }
            "DEPENDENCIES" if indent == 2 => {
                let n = t.split([' ', '!']).next().unwrap_or("");
                if !n.is_empty() {
                    // `rails (~> 8.0, >= 8.0.1)`
                    let reqs = t.split_once(" (").map(|(_, r)| r.trim_end_matches(')').split(", ").map(str::to_string).collect()).unwrap_or_default();
                    deps.push((n.to_string(), reqs));
                }
            }
            "BUNDLED WITH" if !t.is_empty() => bundled_with = Some(t.to_string()),
            _ => {}
        }
    }
    (specs, deps, bundled_with)
}

// ---------- installed gems ----------

enum Located {
    /// the gem's directory, its require paths and its extensions (extconf.rb paths)
    Dir(PathBuf, Vec<String>, Vec<String>),
    Default,
}

fn gem_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    // the gems of the Ruby being built against (crate::ruby) come first
    let ruby = crate::ruby::get();
    if let Some(r) = ruby {
        dirs.extend(r.gem_path.iter().cloned());
    }
    for var in ["GEM_HOME", "GEM_PATH"] {
        if let Some(v) = std::env::var_os(var) {
            dirs.extend(std::env::split_paths(&v));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for pat in [".gem/ruby", ".rvm/gems", ".local/share/gem/ruby"] {
            if let Ok(rd) = std::fs::read_dir(home.join(pat)) {
                let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                v.sort();
                dirs.extend(v);
            }
        }
        if let Ok(rd) = std::fs::read_dir(home.join(".rbenv/versions")) {
            for e in rd.flatten() {
                if let Ok(g) = std::fs::read_dir(e.path().join("lib/ruby/gems")) {
                    dirs.extend(g.flatten().map(|x| x.path()));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| d.join("gems").is_dir() && seen.insert(d.clone()));
    // other Rubies' gems only after that Ruby's own (a gem installed for both resolves to its
    // copy; one installed for another Ruby only is still found)
    if let Some(r) = ruby {
        dirs.sort_by_key(|d| !r.owns_gem_dir(d));
    }
    dirs
}

/// The require paths of an installed gem (`name-version` in a gem directory), if it is installed.
pub fn installed_gem(name: &str, version: &str) -> Option<Vec<PathBuf>> {
    let s = Spec { name: name.into(), version: version.into(), deps: Vec::new(), source: Source::Gem };
    match locate(&s, &gem_dirs(), Path::new(".")) {
        Some(Located::Dir(dir, req, _)) => Some(req.iter().map(|r| dir.join(r)).filter(|p| p.is_dir()).collect()),
        _ => None,
    }
}

fn locate(s: &Spec, gem_dirs: &[PathBuf], _root: &Path) -> Option<Located> {
    let full = format!("{}-{}", s.name, s.version);
    match &s.source {
        Source::Path(dir) => {
            // (Bundler looks for each gem's gemspec at `{,*,*/*}.gemspec` under the path: a
            // repository of several gems, like rails/rails, has them in subdirectories)
            let dir = path_gem_dir(dir, &s.name);
            let req = gemspec_in(&dir).map(|g| require_paths(&g)).unwrap_or_else(|| vec!["lib".into()]);
            let dir = dir.canonicalize().unwrap_or(dir.clone());
            let ext = find_extconfs(&dir);
            return Some(Located::Dir(dir, req, ext));
        }
        Source::Git { revision } => {
            let short: String = revision.chars().take(12).collect();
            for d in gem_dirs {
                let dir = d.join("bundler/gems").join(format!("{}-{}", s.name, short));
                if dir.is_dir() {
                    let req = gemspec_in(&dir).map(|g| require_paths(&g)).unwrap_or_else(|| vec!["lib".into()]);
                    let ext = find_extconfs(&dir);
                    return Some(Located::Dir(dir, req, ext));
                }
            }
            return None;
        }
        Source::Gem => {}
    }
    let installed = |d: &Path, full: &str| -> Option<Located> {
        let dir = d.join("gems").join(full);
        if !dir.is_dir() {
            return None;
        }
        let spec = std::fs::read_to_string(d.join("specifications").join(format!("{full}.gemspec"))).unwrap_or_default();
        let mut ext = spec_list(&spec, ".extensions").unwrap_or_default();
        // a precompiled platform gem (pg-1.6.3-arm64-darwin) lists no extensions, its
        // binaries being built for CRuby: build the C sources it ships instead
        if ext.is_empty() && full.len() > s.name.len() + 1 + version_part(&s.version).len() {
            ext = find_extconfs(&dir);
        }
        Some(Located::Dir(dir, require_paths(&spec), ext))
    };
    for d in gem_dirs {
        if let Some(l) = installed(d, &full) {
            return Some(l);
        }
    }
    for d in gem_dirs {
        if d.join("specifications/default").join(format!("{full}.gemspec")).is_file() {
            return Some(Located::Default);
        }
    }
    // locked for the `ruby` platform but installed for this one (pg-1.6.3-arm64-darwin), or
    // the other way round
    for d in gem_dirs {
        let base = format!("{}-{}", s.name, version_part(&s.version));
        let Ok(rd) = std::fs::read_dir(d.join("gems")) else { continue };
        let mut cands: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| *n == base || n.strip_prefix(&format!("{base}-")).is_some_and(host_platform))
            .collect();
        cands.sort();
        for c in cands {
            if let Some(l) = installed(d, &c) {
                return Some(l);
            }
        }
    }
    None
}

/// The version without a platform suffix ("1.6.3-arm64-darwin" -> "1.6.3").
fn version_part(v: &str) -> &str {
    v.split_once('-').map(|x| x.0).unwrap_or(v)
}

/// Whether a gem platform ("arm64-darwin", "x86_64-linux-gnu") is this machine's.
fn host_platform(p: &str) -> bool {
    let arch_ok = match std::env::consts::ARCH {
        "aarch64" => p.starts_with("arm64-") || p.starts_with("aarch64-"),
        "x86_64" => p.starts_with("x86_64-") || p.starts_with("x64-"),
        _ => false,
    };
    let os_ok = match std::env::consts::OS {
        "macos" => p.contains("darwin"),
        "linux" => p.contains("linux"),
        _ => false,
    };
    arch_ok && os_ok
}

/// The directory of gem `name` in a path source: where its `<name>.gemspec` is (the path
/// itself, or one or two levels down), else the path.
fn path_gem_dir(dir: &Path, name: &str) -> PathBuf {
    let file = format!("{name}.gemspec");
    if dir.join(&file).is_file() {
        return dir.to_path_buf();
    }
    let subdirs = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        v.sort();
        v
    };
    for d1 in subdirs(dir) {
        if d1.join(&file).is_file() {
            return d1;
        }
    }
    for d1 in subdirs(dir) {
        for d2 in subdirs(&d1) {
            if d2.join(&file).is_file() {
                return d2;
            }
        }
    }
    dir.to_path_buf()
}

fn gemspec_in(dir: &Path) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    let p = rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "gemspec"))?;
    std::fs::read_to_string(p).ok()
}

/// `s.require_paths = ["lib".freeze]` from an installed gemspec (default: lib).
fn require_paths(spec: &str) -> Vec<String> {
    spec_list(spec, ".require_paths").filter(|v| !v.is_empty()).unwrap_or_else(|| vec!["lib".into()])
}

/// A list attribute of an installed gemspec: `s.extensions = ["ext/x/extconf.rb".freeze]`.
fn spec_list(spec: &str, attr: &str) -> Option<Vec<String>> {
    for line in spec.lines() {
        if let Some((lhs, rhs)) = line.split_once('=') {
            if lhs.trim().ends_with(attr) {
                return Some(rhs.split('"').skip(1).step_by(2).map(|s| s.to_string()).collect());
            }
        }
    }
    None
}

/// The extconf.rb files under a source checkout's ext/ (a path or git gem, whose gemspec is
/// Ruby code that isn't evaluated).
fn find_extconfs(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, rel: &str, depth: usize, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if p.is_dir() && depth < 3 {
                walk(&p, &format!("{rel}/{name}"), depth + 1, out);
            } else if name == "extconf.rb" {
                out.push(format!("{rel}/{name}"));
            }
        }
    }
    let mut out = Vec::new();
    walk(&dir.join("ext"), "ext", 0, &mut out);
    out
}

// ---------- Gemfile (read statically with Prism) ----------

fn parse_gemfile(src: &[u8]) -> Vec<Dep> {
    let result = ruby_prism::parse(src);
    let mut out = Vec::new();
    let node = result.node();
    if let Some(p) = node.as_program_node() {
        walk(&p.statements().as_node(), &["default".to_string()], &[], &mut out);
    }
    out
}

fn str_of(n: &ruby_prism::Node) -> Option<String> {
    if let Some(s) = n.as_string_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).into_owned());
    }
    if let Some(s) = n.as_symbol_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).into_owned());
    }
    None
}

fn strs_of(n: &ruby_prism::Node) -> Vec<String> {
    if let Some(a) = n.as_array_node() {
        return a.elements().iter().filter_map(|e| str_of(&e)).collect();
    }
    str_of(n).into_iter().collect()
}

fn walk(n: &ruby_prism::Node, groups: &[String], platforms: &[String], out: &mut Vec<Dep>) {
    if let Some(s) = n.as_statements_node() {
        for x in s.body().iter() {
            walk(&x, groups, platforms, out);
        }
        return;
    }
    if let Some(i) = n.as_if_node() {
        // conditions can't be evaluated at build time: take both branches (the lock decides)
        if let Some(s) = i.statements() {
            walk(&s.as_node(), groups, platforms, out);
        }
        if let Some(e) = i.subsequent() {
            walk(&e, groups, platforms, out);
        }
        return;
    }
    if let Some(e) = n.as_else_node() {
        if let Some(s) = e.statements() {
            walk(&s.as_node(), groups, platforms, out);
        }
        return;
    }
    if let Some(u) = n.as_unless_node() {
        if let Some(s) = u.statements() {
            walk(&s.as_node(), groups, platforms, out);
        }
        return;
    }
    let Some(c) = n.as_call_node() else { return };
    if c.receiver().is_some() {
        return;
    }
    let args: Vec<ruby_prism::Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
    let mut pos = Vec::new();
    let mut opts: Vec<(String, ruby_prism::Node)> = Vec::new();
    for a in args {
        if let Some(h) = a.as_keyword_hash_node() {
            for e in h.elements().iter() {
                if let Some(assoc) = e.as_assoc_node() {
                    if let Some(k) = str_of(&assoc.key()) {
                        opts.push((k, assoc.value()));
                    }
                }
            }
        } else {
            pos.push(a);
        }
    }
    let body = c.block().and_then(|b| b.as_block_node()).and_then(|b| b.body());
    match c.name().as_slice() {
        b"gem" => {
            let Some(name) = pos.first().and_then(str_of) else { return };
            let mut d = Dep { name, groups: groups.to_vec(), autorequire: None, platforms: platforms.to_vec() };
            for (k, v) in &opts {
                match k.as_str() {
                    "group" | "groups" => d.groups = strs_of(v),
                    "platform" | "platforms" => d.platforms = strs_of(v),
                    "require" => {
                        d.autorequire = if v.as_false_node().is_some() {
                            Some(vec![])
                        } else if v.as_true_node().is_some() {
                            None
                        } else {
                            Some(strs_of(v))
                        }
                    }
                    _ => {}
                }
            }
            out.push(d);
        }
        b"group" | b"groups" => {
            let g: Vec<String> = pos.iter().flat_map(strs_of).collect();
            if let Some(b) = body {
                walk(&b, &g, platforms, out);
            }
        }
        b"platforms" | b"platform" => {
            let p: Vec<String> = pos.iter().flat_map(strs_of).collect();
            if let Some(b) = body {
                walk(&b, groups, &p, out);
            }
        }
        _ => {
            // source/path/git/github blocks
            if let Some(b) = body {
                walk(&b, groups, platforms, out);
            }
        }
    }
}
