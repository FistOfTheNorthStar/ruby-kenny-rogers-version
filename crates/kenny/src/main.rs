//! `kenny` — compile Ruby to a single native executable.
//!
//!   kenny build app.rb [-o app] [-I dir]... [--ext dir]...
//!   kenny run app.rb [args...]

mod bundle;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const USAGE: &str = "usage:
  kenny build <file.rb> [-o <output>] [-I <dir>]... [--ext <dir>]...
  kenny run <file.rb> [-I <dir>]... [--ext <dir>]... [-- args...]
  kenny --version

  -I <dir>      add a load path: `require \"x\"` compiles <dir>/x.rb into the binary
                (with a Gemfile.lock next to the program, or in a parent directory, the
                locked gems are found automatically; groups in BUNDLE_WITHOUT, by default
                development:test, are left out)
  --ext <dir>   compile a C extension (its *.c files) into the binary; `require \"name\"` runs Init_name";

const RUBY_H: &str = include_str!("../../../include/ruby.h");
const CAPI_SHIM: &str = include_str!("../../../include/kenny_capi.c");
/// Drop-in replacements for stdlib files that rely on features Kenny drops.
const OVERLAY: &[(&str, &str)] = &[
    ("forwardable.rb", include_str!("../../../lib/kenny/forwardable.rb")),
    ("openssl.rb", include_str!("../../../lib/kenny/openssl.rb")),
    ("securerandom.rb", include_str!("../../../lib/kenny/securerandom.rb")),
    ("random/formatter.rb", include_str!("../../../lib/kenny/random/formatter.rb")),
    ("json.rb", include_str!("../../../lib/kenny/json.rb")),
    ("json/ext.rb", include_str!("../../../lib/kenny/json/ext.rb")),
    ("json/common.rb", include_str!("../../../lib/kenny/json/common.rb")),
    ("digest.rb", include_str!("../../../lib/kenny/digest.rb")),
    ("tempfile.rb", include_str!("../../../lib/kenny/tempfile.rb")),
    ("pathname.so", include_str!("../../../lib/kenny/pathname_ext.rb")),
    ("kenny/gem.rb", include_str!("../../../lib/kenny/kenny/gem.rb")),
    ("bundler.rb", include_str!("../../../lib/kenny/bundler.rb")),
    ("bundler/setup.rb", include_str!("../../../lib/kenny/bundler/setup.rb")),
    ("bootsnap/setup.rb", include_str!("../../../lib/kenny/bootsnap/setup.rb")),
    ("ffi.rb", include_str!("../../../lib/kenny/ffi.rb")),
    ("socket.rb", include_str!("../../../lib/kenny/socket.rb")),
    ("nio.rb", include_str!("../../../lib/kenny/nio.rb")),
    ("pp.rb", include_str!("../../../lib/kenny/pp.rb")),
    ("yaml.rb", include_str!("../../../lib/kenny/yaml.rb")),
    ("psych.rb", include_str!("../../../lib/kenny/psych.rb")),
    ("bigdecimal.rb", include_str!("../../../lib/kenny/bigdecimal.rb")),
    ("bigdecimal/util.rb", include_str!("../../../lib/kenny/bigdecimal/util.rb")),
    ("racc/parser.rb", include_str!("../../../lib/kenny/racc/parser.rb")),
    // Nokogiri: its Ruby layer on the runtime's native core (kenny-runtime/src/nokogiri.rs)
    ("nokogiri.rb", include_str!("../../../lib/kenny/nokogiri.rb")),
    ("nokogiri/class_resolver.rb", include_str!("../../../lib/kenny/nokogiri/class_resolver.rb")),
    ("nokogiri/css.rb", include_str!("../../../lib/kenny/nokogiri/css.rb")),
    ("nokogiri/css/node.rb", include_str!("../../../lib/kenny/nokogiri/css/node.rb")),
    ("nokogiri/css/parser.rb", include_str!("../../../lib/kenny/nokogiri/css/parser.rb")),
    ("nokogiri/css/parser_extras.rb", include_str!("../../../lib/kenny/nokogiri/css/parser_extras.rb")),
    ("nokogiri/css/syntax_error.rb", include_str!("../../../lib/kenny/nokogiri/css/syntax_error.rb")),
    ("nokogiri/css/tokenizer.rb", include_str!("../../../lib/kenny/nokogiri/css/tokenizer.rb")),
    ("nokogiri/css/xpath_visitor.rb", include_str!("../../../lib/kenny/nokogiri/css/xpath_visitor.rb")),
    ("nokogiri/decorators/slop.rb", include_str!("../../../lib/kenny/nokogiri/decorators/slop.rb")),
    ("nokogiri/encoding_handler.rb", include_str!("../../../lib/kenny/nokogiri/encoding_handler.rb")),
    ("nokogiri/extension.rb", include_str!("../../../lib/kenny/nokogiri/extension.rb")),
    ("nokogiri/gumbo.rb", include_str!("../../../lib/kenny/nokogiri/gumbo.rb")),
    ("nokogiri/html.rb", include_str!("../../../lib/kenny/nokogiri/html.rb")),
    ("nokogiri/html4.rb", include_str!("../../../lib/kenny/nokogiri/html4.rb")),
    ("nokogiri/html4/builder.rb", include_str!("../../../lib/kenny/nokogiri/html4/builder.rb")),
    ("nokogiri/html4/document.rb", include_str!("../../../lib/kenny/nokogiri/html4/document.rb")),
    ("nokogiri/html4/document_fragment.rb", include_str!("../../../lib/kenny/nokogiri/html4/document_fragment.rb")),
    ("nokogiri/html4/element_description.rb", include_str!("../../../lib/kenny/nokogiri/html4/element_description.rb")),
    ("nokogiri/html4/element_description_defaults.rb", include_str!("../../../lib/kenny/nokogiri/html4/element_description_defaults.rb")),
    ("nokogiri/html4/encoding_reader.rb", include_str!("../../../lib/kenny/nokogiri/html4/encoding_reader.rb")),
    ("nokogiri/html4/entity_lookup.rb", include_str!("../../../lib/kenny/nokogiri/html4/entity_lookup.rb")),
    ("nokogiri/html4/sax/parser.rb", include_str!("../../../lib/kenny/nokogiri/html4/sax/parser.rb")),
    ("nokogiri/html4/sax/parser_context.rb", include_str!("../../../lib/kenny/nokogiri/html4/sax/parser_context.rb")),
    ("nokogiri/html4/sax/push_parser.rb", include_str!("../../../lib/kenny/nokogiri/html4/sax/push_parser.rb")),
    ("nokogiri/html5.rb", include_str!("../../../lib/kenny/nokogiri/html5.rb")),
    ("nokogiri/html5/document.rb", include_str!("../../../lib/kenny/nokogiri/html5/document.rb")),
    ("nokogiri/html5/document_fragment.rb", include_str!("../../../lib/kenny/nokogiri/html5/document_fragment.rb")),
    ("nokogiri/html5/node.rb", include_str!("../../../lib/kenny/nokogiri/html5/node.rb")),
    ("nokogiri/kenny.rb", include_str!("../../../lib/kenny/nokogiri/kenny.rb")),
    ("nokogiri/syntax_error.rb", include_str!("../../../lib/kenny/nokogiri/syntax_error.rb")),
    ("nokogiri/version.rb", include_str!("../../../lib/kenny/nokogiri/version.rb")),
    ("nokogiri/version/constant.rb", include_str!("../../../lib/kenny/nokogiri/version/constant.rb")),
    ("nokogiri/version/info.rb", include_str!("../../../lib/kenny/nokogiri/version/info.rb")),
    ("nokogiri/xml.rb", include_str!("../../../lib/kenny/nokogiri/xml.rb")),
    ("nokogiri/xml/attr.rb", include_str!("../../../lib/kenny/nokogiri/xml/attr.rb")),
    ("nokogiri/xml/attribute_decl.rb", include_str!("../../../lib/kenny/nokogiri/xml/attribute_decl.rb")),
    ("nokogiri/xml/builder.rb", include_str!("../../../lib/kenny/nokogiri/xml/builder.rb")),
    ("nokogiri/xml/cdata.rb", include_str!("../../../lib/kenny/nokogiri/xml/cdata.rb")),
    ("nokogiri/xml/character_data.rb", include_str!("../../../lib/kenny/nokogiri/xml/character_data.rb")),
    ("nokogiri/xml/document.rb", include_str!("../../../lib/kenny/nokogiri/xml/document.rb")),
    ("nokogiri/xml/document_fragment.rb", include_str!("../../../lib/kenny/nokogiri/xml/document_fragment.rb")),
    ("nokogiri/xml/dtd.rb", include_str!("../../../lib/kenny/nokogiri/xml/dtd.rb")),
    ("nokogiri/xml/element_content.rb", include_str!("../../../lib/kenny/nokogiri/xml/element_content.rb")),
    ("nokogiri/xml/element_decl.rb", include_str!("../../../lib/kenny/nokogiri/xml/element_decl.rb")),
    ("nokogiri/xml/entity_decl.rb", include_str!("../../../lib/kenny/nokogiri/xml/entity_decl.rb")),
    ("nokogiri/xml/entity_reference.rb", include_str!("../../../lib/kenny/nokogiri/xml/entity_reference.rb")),
    ("nokogiri/xml/namespace.rb", include_str!("../../../lib/kenny/nokogiri/xml/namespace.rb")),
    ("nokogiri/xml/node.rb", include_str!("../../../lib/kenny/nokogiri/xml/node.rb")),
    ("nokogiri/xml/node/save_options.rb", include_str!("../../../lib/kenny/nokogiri/xml/node/save_options.rb")),
    ("nokogiri/xml/node_set.rb", include_str!("../../../lib/kenny/nokogiri/xml/node_set.rb")),
    ("nokogiri/xml/notation.rb", include_str!("../../../lib/kenny/nokogiri/xml/notation.rb")),
    ("nokogiri/xml/parse_options.rb", include_str!("../../../lib/kenny/nokogiri/xml/parse_options.rb")),
    ("nokogiri/xml/pp.rb", include_str!("../../../lib/kenny/nokogiri/xml/pp.rb")),
    ("nokogiri/xml/pp/character_data.rb", include_str!("../../../lib/kenny/nokogiri/xml/pp/character_data.rb")),
    ("nokogiri/xml/pp/node.rb", include_str!("../../../lib/kenny/nokogiri/xml/pp/node.rb")),
    ("nokogiri/xml/processing_instruction.rb", include_str!("../../../lib/kenny/nokogiri/xml/processing_instruction.rb")),
    ("nokogiri/xml/reader.rb", include_str!("../../../lib/kenny/nokogiri/xml/reader.rb")),
    ("nokogiri/xml/relax_ng.rb", include_str!("../../../lib/kenny/nokogiri/xml/relax_ng.rb")),
    ("nokogiri/xml/sax.rb", include_str!("../../../lib/kenny/nokogiri/xml/sax.rb")),
    ("nokogiri/xml/sax/document.rb", include_str!("../../../lib/kenny/nokogiri/xml/sax/document.rb")),
    ("nokogiri/xml/sax/parser.rb", include_str!("../../../lib/kenny/nokogiri/xml/sax/parser.rb")),
    ("nokogiri/xml/sax/parser_context.rb", include_str!("../../../lib/kenny/nokogiri/xml/sax/parser_context.rb")),
    ("nokogiri/xml/sax/push_parser.rb", include_str!("../../../lib/kenny/nokogiri/xml/sax/push_parser.rb")),
    ("nokogiri/xml/schema.rb", include_str!("../../../lib/kenny/nokogiri/xml/schema.rb")),
    ("nokogiri/xml/searchable.rb", include_str!("../../../lib/kenny/nokogiri/xml/searchable.rb")),
    ("nokogiri/xml/syntax_error.rb", include_str!("../../../lib/kenny/nokogiri/xml/syntax_error.rb")),
    ("nokogiri/xml/text.rb", include_str!("../../../lib/kenny/nokogiri/xml/text.rb")),
    ("nokogiri/xml/xpath.rb", include_str!("../../../lib/kenny/nokogiri/xml/xpath.rb")),
    ("nokogiri/xml/xpath/syntax_error.rb", include_str!("../../../lib/kenny/nokogiri/xml/xpath/syntax_error.rb")),
    ("nokogiri/xml/xpath_context.rb", include_str!("../../../lib/kenny/nokogiri/xml/xpath_context.rb")),
    ("nokogiri/xslt.rb", include_str!("../../../lib/kenny/nokogiri/xslt.rb")),
    ("nokogiri/xslt/stylesheet.rb", include_str!("../../../lib/kenny/nokogiri/xslt/stylesheet.rb")),
];

fn system_stdlib() -> Option<PathBuf> {
    if std::env::var_os("KENNY_NO_SYSTEM_STDLIB").is_some() {
        return None;
    }
    if let Ok(p) = std::env::var("KENNY_STDLIB") {
        return Some(PathBuf::from(p));
    }
    let out = Command::new("ruby").args(["-e", "print RbConfig::CONFIG['rubylibdir']"]).output().ok()?;
    let p = PathBuf::from(String::from_utf8(out.stdout).ok()?);
    p.is_dir().then_some(p)
}

struct Opts {
    src: PathBuf,
    out: Option<PathBuf>,
    load_paths: Vec<PathBuf>,
    exts: Vec<PathBuf>,
    run_args: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { src: PathBuf::new(), out: None, load_paths: Vec::new(), exts: Vec::new(), run_args: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let next = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{a} needs an argument"));
        match a.as_str() {
            "-o" => {
                o.out = Some(PathBuf::from(next(i)?));
                i += 2;
            }
            "-I" => {
                o.load_paths.push(PathBuf::from(next(i)?));
                i += 2;
            }
            "--ext" => {
                o.exts.push(PathBuf::from(next(i)?));
                i += 2;
            }
            "--" => {
                o.run_args.extend(args[i + 1..].iter().cloned());
                break;
            }
            _ if o.src.as_os_str().is_empty() => {
                o.src = PathBuf::from(a);
                i += 1;
            }
            _ => {
                o.run_args.push(a.clone());
                i += 1;
            }
        }
    }
    if o.src.as_os_str().is_empty() {
        return Err(USAGE.into());
    }
    if let Ok(lp) = std::env::var("KENNY_LOAD_PATH") {
        o.load_paths.extend(std::env::split_paths(&lp));
    }
    Ok(o)
}

fn runtime_lib(jit: bool) -> Result<PathBuf, String> {
    let file = if jit { "libkenny_runtime_jit.a" } else { "libkenny_runtime.a" };
    if let Ok(p) = std::env::var(if jit { "KENNY_RUNTIME_JIT" } else { "KENNY_RUNTIME" }) {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    for cand in [dir.join(file), dir.join("../release").join(file), dir.join("../lib").join(file)] {
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err(format!("can't find {} next to {} (build it with `cargo build --release`)", file, exe.display()))
}

fn cc() -> Command {
    Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
}

/// Compile C extensions + the C-API shim; returns object files.
fn build_exts(exts: &[PathBuf], work: &Path) -> Result<Vec<PathBuf>, String> {
    if exts.is_empty() {
        return Ok(vec![]);
    }
    let inc = work.join("include");
    std::fs::create_dir_all(inc.join("ruby")).map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby.h"), RUBY_H).map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby/ruby.h"), "#include \"../ruby.h\"\n").map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby/encoding.h"), "#include \"../ruby.h\"\ntypedef struct rb_encoding rb_encoding;\n").map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby/thread.h"), "#include \"../ruby.h\"\n").map_err(|e| e.to_string())?;
    std::fs::write(inc.join("ruby/io.h"), "#include \"../ruby.h\"\n").map_err(|e| e.to_string())?;
    std::fs::write(work.join("kenny_capi.c"), CAPI_SHIM).map_err(|e| e.to_string())?;
    let mut sources = vec![work.join("kenny_capi.c")];
    let mut inits = Vec::new();
    let mut ext_dirs = Vec::new();
    for e in exts {
        let files: Vec<PathBuf> = if e.is_dir() {
            std::fs::read_dir(e)
                .map_err(|x| format!("{}: {x}", e.display()))?
                .flatten()
                .map(|d| d.path())
                .filter(|p| p.extension().is_some_and(|x| x == "c"))
                .collect()
        } else {
            vec![e.clone()]
        };
        ext_dirs.push(if e.is_dir() { e.clone() } else { e.parent().unwrap_or(Path::new(".")).to_path_buf() });
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for line in text.lines() {
                if let Some(rest) = line.trim_start().strip_prefix("void Init_").or_else(|| line.trim_start().strip_prefix("RUBY_FUNC_EXPORTED void Init_")) {
                    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() && rest[name.len()..].trim_start().starts_with('(') && !inits.contains(&name) {
                        inits.push(name);
                    }
                }
            }
            sources.push(f);
        }
    }
    let mut reg = String::from("extern void kn_register_ext(const char *, void (*)(void));\n");
    for n in &inits {
        reg.push_str(&format!("void Init_{n}(void);\n"));
    }
    reg.push_str("__attribute__((constructor)) static void kn_register_all_exts(void) {\n");
    for n in &inits {
        reg.push_str(&format!("  kn_register_ext(\"{n}\", Init_{n});\n"));
    }
    reg.push_str("}\n");
    std::fs::write(work.join("kenny_exts.c"), reg).map_err(|e| e.to_string())?;
    sources.push(work.join("kenny_exts.c"));
    let mut objs = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        let o = work.join(format!("ext{i}.o"));
        let mut c = cc();
        c.args(["-std=gnu17", "-O2", "-fPIC", "-w", "-c"]).arg("-I").arg(&inc);
        for d in &ext_dirs {
            c.arg("-I").arg(d);
        }
        c.arg(s).arg("-o").arg(&o);
        let out = c.output().map_err(|e| format!("failed to run the C compiler: {e}"))?;
        if !out.status.success() {
            return Err(format!("compiling {} failed:\n{}", s.display(), String::from_utf8_lossy(&out.stderr)));
        }
        objs.push(o);
    }
    Ok(objs)
}

/// Write the overlay files into `overlay` and work out the compiler options: the overlay,
/// the -I dirs, the locked gems of a Bundler project, then the standard library.
fn compile_options(o: &Opts, overlay: &Path) -> Result<kenny_compiler::Options, String> {
    std::fs::create_dir_all(overlay).map_err(|e| e.to_string())?;
    for (name, src) in OVERLAY {
        let path = overlay.join(name);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, src).map_err(|e| e.to_string())?;
    }
    let mut gem_paths = o.load_paths.clone();
    let mut embed_dirs = o.load_paths.clone();
    let mut dep_dirs = Vec::new();
    let mut psych_version = None;
    let mut bundle_rb = String::from("module Bundler\n  module Kenny\n    ROOT = nil\n    GEMFILE = nil\n    LOCKFILE = nil\n    WITHOUT = [].freeze\n    REPLACED = [].freeze\n    DEPENDENCIES = [].freeze\n    SPECS = [].freeze\n  end\nend\n");
    if let Some(lock) = bundle::find_lockfile(&o.src) {
        let b = bundle::resolve(&lock)?;
        if std::env::var_os("KENNY_QUIET").is_none() {
            eprintln!(
                "kenny: {}: {} gem directories{}",
                lock.display(),
                b.load_paths.len(),
                if b.without.is_empty() { String::new() } else { format!(" (without groups: {})", b.without.join(", ")) }
            );
        }
        for p in &b.load_paths {
            if !gem_paths.contains(p) {
                gem_paths.push(p.clone());
                embed_dirs.push(p.clone());
            }
        }
        // the project's own code (app/, config/, lib/ ...): loaded at runtime by path
        embed_dirs.extend(b.project_dirs.iter().cloned());
        embed_dirs.extend(b.engine_dirs.iter().cloned());
        dep_dirs.extend(b.engine_dirs.iter().cloned());
        bundle_rb = b.ruby;
        psych_version = b.psych_version;
    }
    std::fs::write(overlay.join("kenny/bundle.rb"), bundle_rb).map_err(|e| e.to_string())?;
    // YAML follows the psych of the Ruby the program is built against (psych 3 and 4+ differ
    // in what `YAML.load` accepts); a psych gem in the Gemfile.lock wins
    let stdlib = system_stdlib();
    let psych = psych_version
        .or_else(|| {
            let src = std::fs::read_to_string(stdlib.as_ref()?.join("psych/versions.rb")).ok()?;
            let line = src.lines().find(|l| l.trim_start().starts_with("VERSION"))?;
            Some(line.split(['\'', '"']).nth(1)?.to_string())
        })
        .unwrap_or_else(|| "5.2.2".into());
    std::fs::write(overlay.join("kenny/psych_version.rb"), format!("module Psych\n  VERSION = \"{psych}\"\nend\n")).map_err(|e| e.to_string())?;
    let gem_rb = overlay.join("kenny/gem.rb");
    let mut load_paths = vec![overlay.to_path_buf()];
    load_paths.extend(gem_paths);
    load_paths.extend(stdlib);
    // every file of the -I dirs goes in (runtime-computed requires/autoloads, e.g. Rails)
    Ok(kenny_compiler::Options { load_paths, embed_dirs, embed_files: vec![gem_rb], dep_dirs })
}

fn build(o: &Opts, out: &Path) -> Result<(), String> {
    let work = std::env::temp_dir().join(format!("kenny-build-{}", std::process::id()));
    let opts = match compile_options(o, &work.join("overlay")) {
        Ok(x) => x,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    let compiled = match kenny_compiler::compile_file_with(&o.src, &opts) {
        Ok(c) => c,
        Err(errs) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(errs.join("\n"));
        }
    };
    if std::env::var_os("KENNY_QUIET").is_none() {
        for w in &compiled.warnings {
            eprintln!("warning: {w}");
        }
    }
    let obj = work.join("program.o");
    std::fs::write(&obj, &compiled.object).map_err(|e| format!("{}: {e}", obj.display()))?;
    let result = (|| {
        let mut objs = vec![obj.clone()];
        objs.extend(build_exts(&o.exts, &work)?);
        let rt = runtime_lib(compiled.needs_jit)?;
        link(&objs, &rt, out)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn link(objs: &[PathBuf], rt: &Path, out: &Path) -> Result<(), String> {
    let mut c = cc();
    c.arg("-o").arg(out).args(objs).arg(rt);
    if cfg!(target_os = "macos") {
        c.args(["-Wl,-dead_strip", "-lm"]);
    } else {
        c.args(["-Wl,--gc-sections", "-lm", "-lpthread", "-ldl"]);
    }
    let o = c.output().map_err(|e| format!("failed to run the C linker: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!("linking failed:\n{}", String::from_utf8_lossy(&o.stderr)))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fail = |e: String| -> ! {
        eprintln!("{e}");
        exit(1)
    };
    match args.get(1).map(|s| s.as_str()) {
        Some("--version") | Some("-v") => println!("kenny {} (Ruby 3.4 syntax via Prism, Cranelift backend)", env!("CARGO_PKG_VERSION")),
        Some("build") => {
            let o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            let out = o.out.clone().unwrap_or_else(|| PathBuf::from(o.src.file_stem().unwrap_or_default()));
            build(&o, &out).unwrap_or_else(|e| fail(e));
        }
        Some("check") => {
            // compile only (no linking): report compile errors/warnings for one file
            let o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            let work = std::env::temp_dir().join(format!("kenny-check-{}", std::process::id()));
            let mut opts = compile_options(&o, &work.join("overlay")).unwrap_or_else(|e| fail(e));
            // only the file itself (and what it requires), not every file of the gems
            opts.embed_dirs.clear();
            let r = kenny_compiler::compile_file_with(&o.src, &opts);
            let _ = std::fs::remove_dir_all(&work);
            match r {
                Ok(c) => {
                    for w in &c.warnings {
                        eprintln!("warning: {w}");
                    }
                }
                Err(errs) => fail(errs.join("\n")),
            }
        }
        Some("run") => {
            let o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            let dir = std::env::temp_dir().join(format!("kenny-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let out = dir.join(o.src.file_stem().unwrap_or_default());
            build(&o, &out).unwrap_or_else(|e| fail(e));
            let status = Command::new(&out).args(&o.run_args).status();
            let _ = std::fs::remove_dir_all(&dir);
            match status {
                Ok(s) => exit(s.code().unwrap_or(1)),
                Err(e) => fail(format!("failed to run {}: {e}", out.display())),
            }
        }
        _ => fail(USAGE.into()),
    }
}
