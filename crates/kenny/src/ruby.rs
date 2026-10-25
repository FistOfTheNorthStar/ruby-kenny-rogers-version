//! The Ruby a program is built against: its standard library and gems are what Kenny
//! compiles in. In order: `KENNY_RUBY` (a ruby executable), the version a `.ruby-version` or
//! `.tool-versions` next to the program (or above it) names, found among rvm, rbenv, asdf,
//! chruby and mise installs, then `ruby` on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub struct Ruby {
    /// RUBY_VERSION, e.g. "3.4.12"
    pub version: String,
    /// the ABI version gem directories are named after, e.g. "3.4.0"
    pub abi: String,
    pub libdir: Option<PathBuf>,
    /// where this Ruby keeps its bundled and default gems
    pub default_gem_dir: Option<PathBuf>,
    pub gem_path: Vec<PathBuf>,
}

static RUBY: OnceLock<Option<Ruby>> = OnceLock::new();

/// Choose the Ruby for building `src` (the first call decides).
pub fn select(src: &Path) {
    RUBY.get_or_init(|| {
        let (exe, gem_env) = choose(src);
        query(&exe, gem_env)
    });
}

/// The chosen Ruby (None: no Ruby installed, or `select` not called).
pub fn get() -> Option<&'static Ruby> {
    RUBY.get().and_then(|r| r.as_ref())
}

/// The ruby executable, and the GEM_HOME / GEM_PATH to run it with (None: inherit them).
fn choose(src: &Path) -> (PathBuf, Option<(PathBuf, Vec<PathBuf>)>) {
    if let Some(r) = std::env::var_os("KENNY_RUBY") {
        let exe = PathBuf::from(r);
        let env = rvm_gem_env(&exe);
        return (exe, env);
    }
    let Some((want, file)) = wanted_version(src) else { return (PathBuf::from("ruby"), None) };
    match find_install(&want) {
        Some(exe) => {
            let env = rvm_gem_env(&exe);
            (exe, env)
        }
        None => {
            eprintln!("kenny: {} asks for Ruby {want}, which isn't installed (rvm, rbenv, asdf, chruby, mise); using `ruby` on PATH", file.display());
            (PathBuf::from("ruby"), None)
        }
    }
}

/// The version a `.ruby-version` / `.tool-versions` in `src`'s directory or above names.
fn wanted_version(src: &Path) -> Option<(String, PathBuf)> {
    let abs = std::fs::canonicalize(src).ok().or_else(|| std::env::current_dir().ok().map(|d| d.join(src)))?;
    let mut dir = if abs.is_dir() { Some(abs.as_path()) } else { abs.parent() };
    while let Some(d) = dir {
        let rv = d.join(".ruby-version");
        if let Ok(s) = std::fs::read_to_string(&rv) {
            // "ruby-3.4.12", "3.4.12", "3.4", rvm's "ruby-3.4.12@gemset"
            let v = s.lines().next().unwrap_or("").trim();
            let v = v.split('@').next().unwrap_or("").trim_start_matches("ruby-").trim();
            if !v.is_empty() {
                return Some((v.to_string(), rv));
            }
        }
        let tv = d.join(".tool-versions");
        if let Ok(s) = std::fs::read_to_string(&tv) {
            if let Some(v) = s.lines().find_map(|l| l.trim().strip_prefix("ruby ")).and_then(|r| r.split_whitespace().next()) {
                return Some((v.to_string(), tv));
            }
        }
        dir = d.parent();
    }
    None
}

/// Numeric parts of a version, for sorting ("3.4.12" > "3.4.9").
fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

/// An installed Ruby `want` (exact, or the newest one `want` is a prefix of: "3.4").
fn find_install(want: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    // (dir holding one directory per installed Ruby, prefix of those names, rvm-style)
    let roots: [(PathBuf, &str); 6] = [
        (home.join(".rvm/rubies"), "ruby-"),
        (home.join(".rbenv/versions"), ""),
        (home.join(".asdf/installs/ruby"), ""),
        (home.join(".rubies"), "ruby-"),
        (home.join(".local/share/mise/installs/ruby"), ""),
        (PathBuf::from("/opt/rubies"), "ruby-"),
    ];
    for (root, prefix) in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut found: Vec<(String, PathBuf)> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let v = name.strip_prefix(prefix)?.to_string();
                let matches = v == want || v.starts_with(&format!("{want}."));
                let exe = e.path().join("bin/ruby");
                (matches && exe.is_file()).then_some((v, exe))
            })
            .collect();
        found.sort_by_key(|(v, _)| version_key(v));
        if let Some((_, exe)) = found.pop() {
            return Some(exe);
        }
    }
    None
}

/// rvm installs gems outside a Ruby's own directory and sets GEM_HOME for the shell's current
/// Ruby only: for an rvm Ruby `exe` (`~/.rvm/rubies/ruby-X/bin/ruby`), its own gem directories.
fn rvm_gem_env(exe: &Path) -> Option<(PathBuf, Vec<PathBuf>)> {
    let name = exe.parent()?.parent()?.file_name()?.to_string_lossy().into_owned();
    let rubies = exe.parent()?.parent()?.parent()?;
    if rubies.file_name()? != "rubies" || !name.starts_with("ruby-") {
        return None;
    }
    let gems = rubies.parent()?.join("gems");
    let own = gems.join(&name);
    let global = gems.join(format!("{name}@global"));
    let path = [own.clone(), global].into_iter().filter(|p| p.is_dir()).collect();
    Some((own, path))
}

fn query(exe: &Path, gem_env: Option<(PathBuf, Vec<PathBuf>)>) -> Option<Ruby> {
    let mut cmd = Command::new(exe);
    cmd.args([
        "-e",
        "puts RUBY_VERSION, RbConfig::CONFIG['ruby_version'], RbConfig::CONFIG['rubylibdir'], (Gem.default_dir rescue ''), (Gem.path.join(File::PATH_SEPARATOR) rescue '')",
    ]);
    match gem_env {
        Some((home, path)) => {
            cmd.env("GEM_HOME", home);
            cmd.env("GEM_PATH", std::env::join_paths(path).unwrap_or_default());
        }
        None if exe != Path::new("ruby") => {
            // (the shell's GEM_HOME belongs to its own Ruby)
            cmd.env_remove("GEM_HOME").env_remove("GEM_PATH");
        }
        None => {}
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut l = text.lines();
    let version = l.next()?.to_string();
    let abi = l.next().unwrap_or("").to_string();
    let dir = |s: Option<&str>| s.map(PathBuf::from).filter(|p| p.is_dir());
    let libdir = dir(l.next());
    let default_gem_dir = dir(l.next());
    let gem_path = l.next().map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    Some(Ruby { version, abi, libdir, default_gem_dir, gem_path })
}

impl Ruby {
    /// Whether gem directory `d` belongs to this Ruby (rvm's `ruby-3.4.12[@set]`, or a
    /// directory named after its ABI version: `~/.gem/ruby/3.4.0`, `.../lib/ruby/gems/3.4.0`).
    pub fn owns_gem_dir(&self, d: &Path) -> bool {
        let rvm = format!("ruby-{}", self.version);
        d.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            c == rvm || c.starts_with(&format!("{rvm}@"))
        }) || (!self.abi.is_empty() && d.file_name().is_some_and(|n| n.to_string_lossy() == self.abi))
    }

    /// The `lib` directories of the gems this Ruby bundles in place of former standard
    /// libraries (csv, bigdecimal, base64 ... in 3.4), which it lets you require without a
    /// Gemfile: the newest installed copy of each.
    pub fn bundled_gem_libs(&self) -> Vec<PathBuf> {
        let (Some(lib), Some(gems)) = (&self.libdir, &self.default_gem_dir) else { return Vec::new() };
        let Ok(src) = std::fs::read_to_string(lib.join("bundled_gems.rb")) else { return Vec::new() };
        // `SINCE = { "csv" => "3.4.0", ... }`: the gems that left the standard library, and when
        let Some(block) = src.split("SINCE = {").nth(1).and_then(|s| s.split('}').next()) else { return Vec::new() };
        let me = version_key(&self.version);
        let names: Vec<&str> = block
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| {
                let mut q = l.split('"');
                let (name, since) = (q.nth(1)?, q.nth(1)?);
                (version_key(since) <= me).then_some(name)
            })
            .collect();
        let Ok(rd) = std::fs::read_dir(gems.join("gems")) else { return Vec::new() };
        let mut all: Vec<(String, PathBuf)> = rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        all.sort_by_key(|(n, _)| version_key(n));
        let mut out = Vec::new();
        for name in names {
            let pre = format!("{name}-");
            let newest = all.iter().rev().find(|(n, _)| n.strip_prefix(&pre).is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit())));
            if let Some((_, dir)) = newest {
                let l = dir.join("lib");
                if l.is_dir() {
                    out.push(l);
                }
            }
        }
        out
    }
}
