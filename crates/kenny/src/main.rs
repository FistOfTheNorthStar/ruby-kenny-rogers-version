//! `kenny` — compile Ruby to a single native executable.
//!
//!   kenny build app.rb [-o app] [-I dir]... [--ext dir]...
//!   kenny run app.rb [args...]

mod bundle;
mod cext;
mod ruby;
mod embed;
mod prism;

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const USAGE: &str = "usage:
  kenny build <file.rb> [-o <output>] [-I <dir>]... [--ext <dir>]... [--embed <dir> | --no-embed]
  kenny run <file.rb> [-I <dir>]... [--ext <dir>]... [-- args...]
  kenny --version

  -I <dir>      add a load path: `require \"x\"` compiles <dir>/x.rb into the binary
                (with a Gemfile.lock next to the program, or in a parent directory, the
                locked gems are found automatically; groups in BUNDLE_WITHOUT, by default
                development:test, are left out)
  --ext <dir>   compile a C extension (its *.c files) into the binary; `require \"name\"` runs Init_name
  --embed <dir> embed the files of <dir> (the app's directory) and of the compiled gems'
                directories in the binary, so it runs without them; the default for a
                config.ru is its directory. At runtime the binary reads those files from
                itself; everything else under <dir> (database, log/, tmp/) is in the
                directory it is started in (or $KENNY_APP_ROOT)
  --no-embed    don't embed files: read them from where they are at build time";

const RUBY_H: &str = include_str!("../../../include/ruby.h");
const CAPI_SHIM: &str = include_str!("../../../include/kenny_capi.c");
/// Drop-in replacements for stdlib files that rely on features Kenny drops.
const OVERLAY: &[(&str, &str)] = &[
    ("forwardable.rb", include_str!("../../../lib/kenny/forwardable.rb")),
    ("openssl.rb", include_str!("../../../lib/kenny/openssl.rb")),
    ("kenny/openssl_ssl.rb", include_str!("../../../lib/kenny/kenny/openssl_ssl.rb")),
    ("securerandom.rb", include_str!("../../../lib/kenny/securerandom.rb")),
    ("random/formatter.rb", include_str!("../../../lib/kenny/random/formatter.rb")),
    ("json.rb", include_str!("../../../lib/kenny/json.rb")),
    ("json/ext.rb", include_str!("../../../lib/kenny/json/ext.rb")),
    ("json/common.rb", include_str!("../../../lib/kenny/json/common.rb")),
    ("digest.rb", include_str!("../../../lib/kenny/digest.rb")),
    ("tempfile.rb", include_str!("../../../lib/kenny/tempfile.rb")),
    ("stringio.rb", include_str!("../../../lib/kenny/stringio.rb")),
    ("pathname.so", include_str!("../../../lib/kenny/pathname_ext.rb")),
    ("ripper.so", include_str!("../../../lib/kenny/ripper_ext.rb")),
    // Prism's native part and Ripper's parser on the runtime's libprism (see prism.rs; both are
    // replaced by stand-ins that raise when there is no prism Ruby library to pair with it)
    ("prism/prism.rb", include_str!("../../../lib/kenny/prism/prism.rb")),
    ("kenny/ripper_prism.rb", include_str!("../../../lib/kenny/kenny/ripper_prism.rb")),
    ("kenny/gem.rb", include_str!("../../../lib/kenny/kenny/gem.rb")),
    ("kenny/gem_specification.rb", include_str!("../../../lib/kenny/kenny/gem_specification.rb")),
    ("kenny/data.rb", include_str!("../../../lib/kenny/kenny/data.rb")),
    ("kenny/argf.rb", include_str!("../../../lib/kenny/kenny/argf.rb")),
    ("kenny/pathname4.rb", include_str!("../../../lib/kenny/kenny/pathname4.rb")),
    ("kenny/pathname_builtin.rb", include_str!("../../../lib/kenny/kenny/pathname_builtin.rb")),
    ("bundler.rb", include_str!("../../../lib/kenny/bundler.rb")),
    ("bundler/setup.rb", include_str!("../../../lib/kenny/bundler/setup.rb")),
    ("bootsnap/setup.rb", include_str!("../../../lib/kenny/bootsnap/setup.rb")),
    ("ffi.rb", include_str!("../../../lib/kenny/ffi.rb")),
    // the asset pipeline's compilers on the runtime's Sass compiler and JavaScript engine
    ("sassc.rb", include_str!("../../../lib/kenny/sassc.rb")),
    ("execjs.rb", include_str!("../../../lib/kenny/execjs.rb")),
    ("kenny/assets_precompile.rb", include_str!("../../../lib/kenny/kenny/assets_precompile.rb")),
    ("socket.rb", include_str!("../../../lib/kenny/socket.rb")),
    ("coverage.rb", include_str!("../../../lib/kenny/coverage.rb")),
    ("nio.rb", include_str!("../../../lib/kenny/nio.rb")),
    ("pp.rb", include_str!("../../../lib/kenny/pp.rb")),
    ("yaml.rb", include_str!("../../../lib/kenny/yaml.rb")),
    ("psych.rb", include_str!("../../../lib/kenny/psych.rb")),
    ("kenny/psych_tree.rb", include_str!("../../../lib/kenny/kenny/psych_tree.rb")),
    // OpenSSL::ASN1 and OpenSSL::X509 (openssl.rb requires them, then the stdlib's openssl/x509.rb)
    ("kenny/openssl_asn1.rb", include_str!("../../../lib/kenny/kenny/openssl_asn1.rb")),
    ("kenny/openssl_x509.rb", include_str!("../../../lib/kenny/kenny/openssl_x509.rb")),
    ("kenny/openssl_x509_name.rb", include_str!("../../../lib/kenny/kenny/openssl_x509_name.rb")),
    ("kenny/openssl_x509_ext.rb", include_str!("../../../lib/kenny/kenny/openssl_x509_ext.rb")),
    ("kenny/openssl_x509_cert.rb", include_str!("../../../lib/kenny/kenny/openssl_x509_cert.rb")),
    ("kenny/openssl_x509_store.rb", include_str!("../../../lib/kenny/kenny/openssl_x509_store.rb")),
    ("kenny/openssl_ec.rb", include_str!("../../../lib/kenny/kenny/openssl_ec.rb")),
    ("kenny/openssl_ed25519.rb", include_str!("../../../lib/kenny/kenny/openssl_ed25519.rb")),
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
    ruby::get()?.libdir.clone()
}

struct Opts {
    src: PathBuf,
    out: Option<PathBuf>,
    load_paths: Vec<PathBuf>,
    exts: Vec<PathBuf>,
    run_args: Vec<String>,
    /// Some(dir): embed dir; None with no_embed false: the default (a config.ru's directory)
    embed: Option<PathBuf>,
    no_embed: bool,
    /// `kenny run`: build into a temporary file and run it (no build chatter)
    running: bool,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { src: PathBuf::new(), out: None, load_paths: Vec::new(), exts: Vec::new(), run_args: Vec::new(), embed: None, no_embed: false, running: false };
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
            "--embed" => {
                o.embed = Some(PathBuf::from(next(i)?));
                i += 2;
            }
            "--no-embed" => {
                o.no_embed = true;
                i += 1;
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

/// The C extensions to build: `--ext` dirs, then the locked gems' (a gem's that can't be set
/// up is left out with a warning).
fn ext_list(o: &Opts, gem_exts: &[bundle::GemExt], work: &Path) -> Result<Vec<cext::Ext>, String> {
    let mut exts = Vec::new();
    for e in &o.exts {
        exts.push(cext::from_dir(e)?);
    }
    if std::env::var_os("KENNY_NO_GEM_EXTS").is_none() {
        for g in gem_exts {
            match cext::from_gem(&g.gem, &g.dir, work) {
                Ok(e) => exts.push(e),
                Err(why) if std::env::var_os("KENNY_QUIET").is_none() => {
                    eprintln!("kenny: warning: the C extension of {} is left out (`require` of it raises LoadError): {why}", g.gem)
                }
                Err(_) => {}
            }
        }
    }
    Ok(exts)
}

/// Write the overlay files into `overlay` and work out the compiler options: the overlay,
/// the -I dirs, the locked gems of a Bundler project, then the standard library.
fn compile_options(o: &Opts, overlay: &Path) -> Result<(kenny_compiler::Options, prism::Pairing, Vec<bundle::GemExt>), String> {
    ruby::select(&o.src);
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
    let mut gem_exts = Vec::new();
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
        gem_exts = b.exts;
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
    // (ARGF too, loaded on first use: kenny-runtime's const_missing)
    let mut embed_files = vec![gem_rb, overlay.join("kenny/data.rb"), overlay.join("kenny/argf.rb")];
    let ruby4 = ruby::get().is_some_and(|r| r.is_ruby4());
    // `Set` is loaded on first use, as Ruby 3.2+ does (kenny-runtime's const_missing): from the
    // stdlib's set.rb, or in Ruby 4.0, where Set is a core class, Kenny's (as the overlay's
    // set.rb, so `require "set"` finds it too)
    if ruby4 {
        let set_rb = overlay.join("set.rb");
        std::fs::write(&set_rb, include_str!("../../../lib/kenny/kenny/set4.rb")).map_err(|e| e.to_string())?;
        embed_files.push(set_rb);
        // Pathname too is core in 4.0 (loaded on first use)
        embed_files.push(overlay.join("kenny/pathname4.rb"));
    } else if let Some(set_rb) = stdlib.as_ref().map(|d| d.join("set.rb")).filter(|p| p.is_file()) {
        embed_files.push(set_rb);
    }
    // `DidYouMean` too (loaded on first use, or to report an uncaught NameError)
    if let Some(dym) = stdlib.as_ref().map(|d| d.join("did_you_mean.rb")).filter(|p| p.is_file()) {
        embed_files.push(dym);
    }
    let mut load_paths = vec![overlay.to_path_buf()];
    load_paths.extend(gem_paths);
    load_paths.extend(stdlib.clone());
    // then the gems that left the standard library (csv, bigdecimal ... in Ruby 3.4), which
    // Ruby lets you require without a Gemfile (with one, as with Bundler, they must be in it)
    let bundled = bundle::find_lockfile(&o.src).is_none();
    if bundled && std::env::var_os("KENNY_NO_SYSTEM_STDLIB").is_none() && std::env::var_os("KENNY_STDLIB").is_none() {
        load_paths.extend(ruby::get().map(|r| r.bundled_gem_libs()).unwrap_or_default());
    }
    // every file of the -I dirs goes in (runtime-computed requires/autoloads, e.g. Rails)
    let pairing = prism::pair(&mut load_paths, stdlib.as_deref(), overlay)?;
    let ruby = ruby::get().map(|r| r.release_line()).unwrap_or_default();
    // Ripper's Prism backend: compiled in only when the program's code names Ripper's parser
    let ripper_backend = Some(overlay.join("kenny/ripper_prism.rb"));
    Ok((kenny_compiler::Options { load_paths, embed_dirs, embed_files, dep_dirs, ruby, coverage: false, ripper_backend }, pairing, gem_exts))
}

/// Returns whether the asset compilers were linked in (see `needs_assets`).
fn build(o: &Opts, out: &Path) -> Result<bool, String> {
    let work = std::env::temp_dir().join(format!("kenny-build-{}", std::process::id()));
    let (mut opts, pairing, gem_exts) = match compile_options(o, &work.join("overlay")) {
        Ok(x) => x,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    let assets = needs_assets(&opts);
    if assets {
        // the binary can precompile its own assets (see precompile_assets)
        opts.embed_files.push(work.join("overlay/kenny/assets_precompile.rb"));
    }
    // the C extensions, and the Ruby files their C code requires (oj's Init_oj does
    // `rb_require("date")`): compiled in, as the program's own requires are
    let exts = match ext_list(o, &gem_exts, &work) {
        Ok(e) => e,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
    };
    for f in cext::required_features(&exts) {
        let found = opts.load_paths.iter().map(|d| d.join(format!("{f}.rb"))).find(|p| p.is_file());
        if let Some(p) = found.filter(|p| !opts.embed_files.contains(p)) {
            opts.embed_files.push(p);
        }
    }
    let compiled = match kenny_compiler::compile_file_with(&o.src, &opts) {
        Ok(c) => c,
        Err(errs) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(errs.join("\n"));
        }
    };
    let (libprism, prism_warning) = prism::needs_libprism(&pairing, &work.join("overlay"), &compiled.files);
    if std::env::var_os("KENNY_QUIET").is_none() {
        for w in &compiled.warnings {
            eprintln!("warning: {w}");
        }
        if let Some(w) = prism_warning {
            eprintln!("kenny: warning: {w}");
        }
    }
    let obj = work.join("program.o");
    std::fs::write(&obj, &compiled.object).map_err(|e| format!("{}: {e}", obj.display()))?;
    let result = (|| {
        let mut objs = vec![obj.clone()];
        let mut exts = if exts.is_empty() { None } else { Some(cext::compile(exts, &work, RUBY_H, CAPI_SHIM)?) };
        if assets {
            objs.push(ext_stub(&work, "kenny_assets")?);
        }
        if libprism {
            objs.push(ext_stub(&work, "kenny_prism")?);
        }
        let rt = runtime_lib(compiled.needs_jit)?;
        // A Rails app precompiles its own assets (the program with the asset compilers linked
        // in) before its files are embedded, so the binary carries the fresh ones.
        if assets && !o.running {
            if let Some(reason) = precompile_reason(o)? {
                link_exts(&objs, &mut exts, &rt, out)?;
                precompile_assets(o, out, reason)?;
            }
        }
        objs.extend(embed_files(o, &opts, out, &work)?);
        link_exts(&objs, &mut exts, &rt, out)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result.map(|_| assets)
}

/// Whether the program uses the sassc or execjs gem (the Rails asset pipeline): then the
/// runtime's Sass compiler and JavaScript engine are linked in (`require "kenny_assets"`).
/// KENNY_ASSETS=1 / 0 forces it either way.
fn needs_assets(opts: &kenny_compiler::Options) -> bool {
    if let Some(v) = std::env::var_os("KENNY_ASSETS") {
        return v != "0";
    }
    opts.load_paths.iter().any(|p| {
        let gem = if p.file_name().is_some_and(|n| n == "lib") { p.parent() } else { Some(p.as_path()) };
        gem.and_then(|g| g.file_name()).and_then(|n| n.to_str()).is_some_and(|n| {
            let name = n.rsplit_once('-').map_or(n, |(name, _)| name);
            name == "sassc" || name == "execjs"
        })
    })
}

fn newest_mtime(p: &Path, newest: &mut Option<std::time::SystemTime>) {
    let Ok(md) = std::fs::metadata(p) else { return };
    if md.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                newest_mtime(&e.path(), newest);
            }
        }
    } else if let Ok(t) = md.modified() {
        if newest.is_none_or(|n| t > n) {
            *newest = Some(t);
        }
    }
}

/// During `kenny build` of a Rails app that uses the asset pipeline (sassc/execjs gems and an
/// app/assets directory next to config.ru): why its assets need precompiling, if they do —
/// public/assets has no Sprockets manifest, or the app's assets, Gemfile.lock or asset config
/// are newer than it. The build then runs the linked program with KENNY_ASSETS_PRECOMPILE=1
/// (`rails assets:precompile`, without Ruby) before embedding the app's files.
/// KENNY_PRECOMPILE=1 always precompiles, KENNY_PRECOMPILE=0 never.
fn precompile_reason(o: &Opts) -> Result<Option<&'static str>, String> {
    let force = std::env::var("KENNY_PRECOMPILE").ok();
    if force.as_deref() == Some("0") {
        return Ok(None);
    }
    let src = std::fs::canonicalize(&o.src).map_err(|e| format!("{}: {e}", o.src.display()))?;
    let root = src.parent().unwrap_or(Path::new(".")).to_path_buf();
    if !root.join("app/assets").is_dir() || !root.join("config/application.rb").is_file() {
        return Ok(None);
    }
    let out_dir = root.join("public/assets");
    let manifest = std::fs::read_dir(&out_dir).ok().and_then(|rd| {
        rd.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".sprockets-manifest-") || n == "manifest.json"))
    });
    let reason = if force.as_deref() == Some("1") {
        Some("KENNY_PRECOMPILE=1")
    } else {
        match manifest.as_ref().and_then(|m| std::fs::metadata(m).ok()?.modified().ok()) {
            None => Some("public/assets has no manifest"),
            Some(built) => {
                let mut newest = None;
                for p in ["app/assets", "lib/assets", "vendor/assets", "Gemfile.lock", "config/initializers/assets.rb"] {
                    newest_mtime(&root.join(p), &mut newest);
                }
                if newest.is_some_and(|n| n > built) { Some("the assets changed") } else { None }
            }
        }
    };
    Ok(reason)
}

/// Run the just-linked binary `bin` with KENNY_ASSETS_PRECOMPILE=1 in the app directory.
fn precompile_assets(o: &Opts, bin: &Path, reason: &str) -> Result<(), String> {
    let src = std::fs::canonicalize(&o.src).map_err(|e| format!("{}: {e}", o.src.display()))?;
    let root = src.parent().unwrap_or(Path::new(".")).to_path_buf();
    let out_dir = root.join("public/assets");
    if std::env::var_os("KENNY_QUIET").is_none() {
        eprintln!("kenny: precompiling assets into {} ({reason})", out_dir.display());
    }
    let bin = std::fs::canonicalize(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let mut cmd = Command::new(&bin);
    cmd.current_dir(&root).env("KENNY_ASSETS_PRECOMPILE", "1");
    if std::env::var_os("RAILS_ENV").is_none() && std::env::var_os("RACK_ENV").is_none() {
        cmd.env("RAILS_ENV", "production");
    }
    if std::env::var_os("SECRET_KEY_BASE").is_none() {
        // what Rails' own SECRET_KEY_BASE_DUMMY does: booting for a build task needs no secrets
        cmd.env("SECRET_KEY_BASE", "kenny-assets-precompile");
    }
    let status = cmd.status().map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "kenny: precompiling the assets failed (the binary {} was built; KENNY_PRECOMPILE=0 skips this step)",
            bin.display()
        ))
    }
}

/// A constructor registering `Init_<name>` as the extension `name`, for the runtime's optional
/// native parts: referencing it is what keeps them in the binary (kenny_assets: grass and
/// QuickJS; kenny_prism: libprism).
fn ext_stub(work: &Path, name: &str) -> Result<PathBuf, String> {
    let src = work.join(format!("{name}.c"));
    let obj = work.join(format!("{name}.o"));
    std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
    std::fs::write(
        &src,
        format!(
            "extern void kn_register_ext(const char *, void (*)(void));\nextern void Init_{name}(void);\n\
             __attribute__((constructor)) static void kn_register_{name}(void) {{ kn_register_ext(\"{name}\", Init_{name}); }}\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let out = cc().args(["-O2", "-c"]).arg(&src).arg("-o").arg(&obj).output().map_err(|e| format!("failed to run the C compiler: {e}"))?;
    if !out.status.success() {
        return Err(format!("compiling {} failed:\n{}", src.display(), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(obj)
}

/// The app directory to embed: --embed, or a config.ru's directory (see `embed.rs`).
fn app_root(o: &Opts) -> Option<PathBuf> {
    if o.no_embed || std::env::var_os("KENNY_NO_EMBED").is_some() {
        return None;
    }
    if let Some(d) = &o.embed {
        return Some(d.clone());
    }
    if o.src.file_name().is_some_and(|n| n == "config.ru") {
        let dir = o.src.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        return Some(dir.to_path_buf());
    }
    None
}

/// Embed the app's files and the gems' directories; returns the object file holding them.
fn embed_files(o: &Opts, opts: &kenny_compiler::Options, out: &Path, work: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(root) = app_root(o) else { return Ok(vec![]) };
    let mut dirs = opts.embed_dirs.clone();
    dirs.extend(opts.dep_dirs.iter().cloned());
    let image = work.join("embedded-files.bin");
    let e = embed::write_image(&root, &dirs, &[out.to_path_buf()], &image)?;
    if std::env::var_os("KENNY_QUIET").is_none() && !o.running {
        eprintln!("kenny: embedded {} files ({:.1} MB) from {} and the gems' directories", e.files, e.bytes as f64 / 1048576.0, root.display());
    }
    let c = work.join("kenny_embedded_files.c");
    std::fs::write(&c, embed::image_c_source(&image, e.bytes as u64)).map_err(|e| e.to_string())?;
    let obj = work.join("kenny_embedded_files.o");
    let r = cc().args(["-c", "-O0", "-w"]).arg(&c).arg("-o").arg(&obj).output().map_err(|e| format!("failed to run the C compiler: {e}"))?;
    if !r.status.success() {
        return Err(format!("embedding files failed:\n{}", String::from_utf8_lossy(&r.stderr)));
    }
    Ok(vec![obj])
}

/// Link the program with the C extensions; an extension of a gem that uses a C API function
/// Kenny lacks is left out (see cext.rs), and the rest linked again.
fn link_exts(objs: &[PathBuf], exts: &mut Option<cext::Built>, rt: &Path, out: &Path) -> Result<(), String> {
    loop {
        let mut all = objs.to_vec();
        let mut extra = Vec::new();
        if let Some(b) = exts.as_ref() {
            all.extend(b.objects());
            extra = b.link_args();
        }
        match link(&all, rt, out, &extra) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let retry = match exts.as_mut() {
                    Some(b) => b.drop_unresolved(&e)?,
                    None => false,
                };
                if !retry {
                    return Err(e);
                }
            }
        }
    }
}

fn link(objs: &[PathBuf], rt: &Path, out: &Path, extra: &[String]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if std::env::var_os("KENNY_LINK_UMBRELLA").is_none() {
        if let Some(stubs) = macos::system_lib_stubs() {
            if link_with(objs, rt, out, &stubs, extra).is_ok() && macos::direct_binds_ok(out) {
                return Ok(());
            }
        }
    }
    link_with(objs, rt, out, &[], extra)
}

/// Link, binding libSystem's symbols to these sub-library stubs (macOS, `macos`).
fn link_with(objs: &[PathBuf], rt: &Path, out: &Path, stubs: &[PathBuf], extra: &[String]) -> Result<(), String> {
    let mut c = cc();
    c.arg("-o").arg(out).args(objs).arg(rt).args(extra);
    if cfg!(target_os = "macos") {
        c.args(["-Wl,-dead_strip", "-lm"]);
        for s in stubs {
            c.arg(format!("-Wl,{}", s.display()));
        }
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

/// Faster program start on macOS. A program's imports from libSystem are bound when it starts;
/// through the umbrella each one is looked up in its ~40 re-exported libraries in turn (tens of
/// thousands of instructions each: 0.6 ms of an empty program's 3.2 ms). Linked against the
/// sub-libraries that define them (libsystem_c, libsystem_kernel, ...), each is found directly.
/// Apple only promises where symbols are at the umbrella level, so after linking each import is
/// checked against the running system (`dlsym`); a mismatch, or no SDK stubs, links the usual
/// way. `KENNY_LINK_UMBRELLA=1` always does.
#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const SUB_LIBS: &[&str] = &[
        "libsystem_kernel",
        "libsystem_c",
        "libsystem_pthread",
        "libsystem_m",
        "libsystem_malloc",
        "libsystem_platform",
        "libunwind",
        "libdyld",
        "libsystem_info",
        "libdispatch",
        "libcommonCrypto",
    ];

    /// The SDK's stubs for the sub-libraries, if it has them all.
    pub fn system_lib_stubs() -> Option<Vec<PathBuf>> {
        let sdk = match std::env::var("SDKROOT") {
            Ok(s) if !s.is_empty() => PathBuf::from(s),
            _ => {
                let o = Command::new("xcrun").arg("--show-sdk-path").output().ok()?;
                if !o.status.success() {
                    return None;
                }
                PathBuf::from(String::from_utf8(o.stdout).ok()?.trim())
            }
        };
        SUB_LIBS.iter().map(|l| Some(sdk.join(format!("usr/lib/system/{l}.tbd"))).filter(|p| p.is_file())).collect()
    }

    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void;
        fn dlsym(handle: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    }
    const RTLD_LAZY: i32 = 0x1;
    const RTLD_NOLOAD: i32 = 0x10;
    /// lookups through the handle search that library only (with what it re-exports), as a
    /// two-level bind to it does
    const RTLD_FIRST: i32 = 0x100;

    /// Every import bound to a sub-library is found in that library on this system.
    pub fn direct_binds_ok(bin: &Path) -> bool {
        let mut handles = std::collections::HashMap::new();
        let Ok(o) = Command::new("nm").args(["-m", "-u"]).arg(bin).output() else { return false };
        if !o.status.success() {
            return false;
        }
        let text = String::from_utf8_lossy(&o.stdout);
        for line in text.lines() {
            // "(undefined) external _abort (from libsystem_c)"
            let Some((head, from)) = line.rsplit_once(" (from ") else { continue };
            let lib = from.trim_end_matches(')');
            if !SUB_LIBS.contains(&lib) {
                continue;
            }
            let Some(sym) = head.split_whitespace().last().and_then(|s| s.strip_prefix('_')) else { return false };
            let Ok(c) = std::ffi::CString::new(sym) else { return false };
            let h = *handles.entry(lib).or_insert_with(|| {
                let path = std::ffi::CString::new(format!("/usr/lib/system/{lib}.dylib")).unwrap();
                unsafe { dlopen(path.as_ptr(), RTLD_LAZY | RTLD_NOLOAD | RTLD_FIRST) }
            });
            if h.is_null() || unsafe { dlsym(h, c.as_ptr()) }.is_null() {
                return false;
            }
        }
        true
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
        // (a debugging aid: the coverage structure of files, as CRuby's Coverage.peek_result
        // reports it for a file compiled with lines and branches, or lines only)
        Some("--coverage-dump") => {
            let lines_only = std::env::var_os("KENNY_COV_LINES_ONLY").is_some();
            for f in &args[2..] {
                let p = Path::new(f);
                let p = p.canonicalize().unwrap_or(p.to_path_buf());
                match std::fs::read(&p) {
                    Ok(src) => print!("{}", kenny_compiler::coverage::dump(&src, &p, !lines_only)),
                    Err(e) => eprintln!("{f}: {e}"),
                }
            }
        }
        Some("build") => {
            let o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            let out = o.out.clone().unwrap_or_else(|| PathBuf::from(o.src.file_stem().unwrap_or_default()));
            build(&o, &out).unwrap_or_else(|e| fail(e));
        }
        Some("check") => {
            // compile only (no linking): report compile errors/warnings for one file
            let o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            let work = std::env::temp_dir().join(format!("kenny-check-{}", std::process::id()));
            let (mut opts, _, _) = compile_options(&o, &work.join("overlay")).unwrap_or_else(|e| fail(e));
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
            let mut o = parse(&args[2..]).unwrap_or_else(|e| fail(e));
            o.running = true;
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
