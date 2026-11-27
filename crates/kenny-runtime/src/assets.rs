//! The asset compilers of the Rails asset pipeline, without Ruby, libsass or Node:
//!
//! - `Kenny::Sass.render`: Sass/SCSS through `grass` (a Sass compiler in Rust), behind the
//!   sassc gem's Ruby API (lib/kenny/sassc.rb). Custom functions (sprockets' `asset-path`,
//!   `image-url`, ...) call back into Ruby.
//! - `Kenny::JS`: a QuickJS engine per context, behind an ExecJS runtime
//!   (lib/kenny/execjs.rb), so ExecJS users such as uglifier and terser run in-process.
//!
//! Linked only into programs whose gems include sassc or execjs: `kenny build` then registers
//! `Init_kenny_assets` (as the extension "kenny_assets"); other binaries don't carry grass or
//! QuickJS (the linker drops what nothing references).

use crate::core::*;
use grass_compiler as grass;
use grass::sass_value::{ArgumentResult, Brackets, Color, ListSeparator, Number, QuoteKind, SassMap, SassNumber, Unit, Value};
use grass::codemap::{Span, Spanned};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn kenny_module() -> *mut Class {
    match const_lookup(rt().c.object, sym("Kenny")) {
        Some(m) => m as *mut Class,
        None => {
            let m = new_module("Kenny");
            set_global_const("Kenny", m as V);
            m
        }
    }
}

fn string_of(v: V) -> String {
    if is_str(v) { String::from_utf8_lossy(&rstr(v).b).into_owned() } else { String::new() }
}

/// `require "kenny_assets"`
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "C" fn Init_kenny_assets() {
    let kenny = kenny_module();
    let sass = new_module("Kenny::Sass");
    const_set(kenny, sym("Sass"), sass as V);
    defs(sass, "render", 9, 9, sass_render);
    let js = new_module("Kenny::JS");
    const_set(kenny, sym("JS"), js as V);
    defs(js, "context", 0, 0, |_, _, _| js_context());
    defs(js, "eval", 2, 2, |_, a, _| js_eval(a[0], a[1]));
}

// ---------- Sass ----------

/// The file system grass sees: the entry file is the template in memory; every other file read
/// is recorded (the stylesheet's dependencies).
///
/// With `importer` (sassc-rails' importer is in use, lib/kenny/sassc.rb) it also shows grass
/// files that aren't there, for what that importer accepts beyond Sass's own imports:
/// - `x.scss` / `x.sass` / `x.css` where only `x.scss.erb` / `x.sass.erb` / `x.css.erb` exists:
///   the ERB's result (`importer.call("erb", path, nil, nil)`);
/// - `x.scss` / `x.sass` where only `x.css.scss` / `x.css.sass` exists: that file;
/// - `dir/*.scss` and `dir/**/*.scss`, what grass looks for on `@import "dir/*"` and
///   `@import "dir/**/*"`: a stylesheet importing every Sass file there
///   (`importer.call("glob", dir, "*" or "**/*", [files not to import])`).
#[derive(Debug)]
struct SassFs {
    entry: PathBuf,
    src: Vec<u8>,
    read: RefCell<Vec<PathBuf>>,
    importer: V,
    /// Files read so far that have a glob import: a glob never imports the file it is in
    /// (grass doesn't say which file an import is in; any of these could be it).
    globbers: RefCell<Vec<PathBuf>>,
}

enum Virtual {
    File(PathBuf),
    Erb(PathBuf),
    Glob(PathBuf, &'static str),
}

fn has_glob_import(src: &[u8]) -> bool {
    src.split(|b| *b == b'\n').any(|l| l.contains(&b'*') && l.windows(7).any(|w| w == b"@import"))
}

impl SassFs {
    fn virtual_file(&self, path: &Path) -> Option<Virtual> {
        if self.importer == QNIL {
            return None;
        }
        let name = path.file_name()?.to_str()?;
        let dir = path.parent()?;
        if name == "*.scss" {
            let (base, glob) = if dir.file_name().is_some_and(|d| d == "**") { (dir.parent()?, "**/*") } else { (dir, "*") };
            // (a directory that isn't there imports nothing, as with sassc-rails)
            return Some(Virtual::Glob(base.to_path_buf(), glob));
        }
        let (stem, ext) = name.rsplit_once('.')?;
        if !matches!(ext, "scss" | "sass" | "css") {
            return None;
        }
        if ext != "css" {
            let p = dir.join(format!("{stem}.css.{ext}"));
            if p.is_file() {
                return Some(Virtual::File(p));
            }
        }
        let p = dir.join(format!("{name}.erb"));
        p.is_file().then_some(Virtual::Erb(p))
    }

    fn record(&self, path: &Path, src: &[u8]) {
        let mut r = self.read.borrow_mut();
        if !r.iter().any(|p| p == path) {
            r.push(path.to_path_buf());
        }
        if self.importer != QNIL && has_glob_import(src) {
            self.globbers.borrow_mut().push(path.to_path_buf());
        }
    }

    /// `importer.call(...)` -> the source it returns. What it raises is re-raised after grass
    /// returns (see `SassCall::pending`); nothing more is asked of the importer then.
    fn call_importer(&self, args: &[V]) -> std::io::Result<Vec<u8>> {
        if SASS_CALLS.with(|c| c.borrow().last().is_some_and(|c| c.pending.is_some())) {
            return Ok(Vec::new());
        }
        let r = send(self.importer, sym("call"), args, QNIL);
        let pending = std::mem::replace(&mut pst().pending, Throw::None);
        if !matches!(pending, Throw::None) || r == UNDEF {
            SASS_CALLS.with(|c| {
                if let Some(c) = c.borrow_mut().last_mut() {
                    c.pending = Some(pending);
                }
            });
            // (grass panics on an error from a read here: the file is empty instead, and the
            // render's result is dropped for the exception)
            return Ok(Vec::new());
        }
        Ok(if is_str(r) { rstr(r).b.to_vec() } else { Vec::new() })
    }
}

impl grass::Fs for SassFs {
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_file(&self, path: &Path) -> bool {
        path == self.entry || path.is_file() || self.virtual_file(path).is_some()
    }
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        if path == self.entry {
            return Ok(self.src.clone());
        }
        let virt = if path.is_file() { None } else { self.virtual_file(path) };
        match virt {
            None => {
                let b = std::fs::read(path)?;
                self.record(path, &b);
                Ok(b)
            }
            Some(Virtual::File(p)) => {
                let b = std::fs::read(&p)?;
                self.record(&p, &b);
                Ok(b)
            }
            Some(Virtual::Erb(p)) => {
                let b = self.call_importer(&[str_from("erb"), str_from(&p.to_string_lossy()), QNIL, QNIL])?;
                self.record(&p, &b);
                Ok(b)
            }
            Some(Virtual::Glob(base, glob)) => {
                let mut skip = vec![str_from(&self.entry.to_string_lossy())];
                skip.extend(self.globbers.borrow().iter().map(|p| str_from(&p.to_string_lossy())));
                self.call_importer(&[str_from("glob"), str_from(&base.to_string_lossy()), str_from(glob), ary_new(skip)])
            }
        }
    }
}

/// The render in progress on this thread: the Ruby callback and each custom function's
/// parameter names. A callback that raised something other than a StandardError (or threw)
/// leaves it in `pending`; it is re-raised after grass returns.
struct SassCall {
    callback: V,
    params: Vec<Vec<String>>,
    pending: Option<Throw>,
}

thread_local! {
    static SASS_CALLS: RefCell<Vec<SassCall>> = const { RefCell::new(Vec::new()) };
}

macro_rules! trampolines {
    ($($i:literal => $f:ident),* $(,)?) => {
        $(fn $f(args: ArgumentResult, _: &mut grass::Visitor) -> grass::Result<Value> { call_custom($i, args) })*
        const TRAMPOLINES: &[fn(ArgumentResult, &mut grass::Visitor) -> grass::Result<Value>] = &[$($f),*];
    };
}
trampolines!(
    0 => t0, 1 => t1, 2 => t2, 3 => t3, 4 => t4, 5 => t5, 6 => t6, 7 => t7, 8 => t8, 9 => t9,
    10 => t10, 11 => t11, 12 => t12, 13 => t13, 14 => t14, 15 => t15, 16 => t16, 17 => t17,
    18 => t18, 19 => t19, 20 => t20, 21 => t21, 22 => t22, 23 => t23, 24 => t24, 25 => t25,
    26 => t26, 27 => t27, 28 => t28, 29 => t29, 30 => t30, 31 => t31, 32 => t32, 33 => t33,
    34 => t34, 35 => t35, 36 => t36, 37 => t37, 38 => t38, 39 => t39, 40 => t40, 41 => t41,
    42 => t42, 43 => t43, 44 => t44, 45 => t45, 46 => t46, 47 => t47, 48 => t48, 49 => t49,
    50 => t50, 51 => t51, 52 => t52, 53 => t53, 54 => t54, 55 => t55, 56 => t56, 57 => t57,
    58 => t58, 59 => t59, 60 => t60, 61 => t61, 62 => t62, 63 => t63,
);

fn sep_name(s: ListSeparator) -> &'static str {
    match s {
        ListSeparator::Comma => "comma",
        ListSeparator::Slash => "slash",
        _ => "space",
    }
}

/// A Sass value as the tagged arrays lib/kenny/sassc.rb turns into SassC::Script::Value objects.
fn sass_to_ruby(v: &Value, span: Span) -> V {
    match v {
        Value::Null => ary_new(vec![str_from("null")]),
        Value::True => ary_new(vec![str_from("b"), QTRUE]),
        Value::False => ary_new(vec![str_from("b"), QFALSE]),
        Value::String(s, q) => ary_new(vec![str_from("s"), str_from(s), bool_v(matches!(q, QuoteKind::Quoted))]),
        Value::Dimension(n) => ary_new(vec![str_from("n"), float_new(n.num.0), str_from(&n.unit.to_string())]),
        Value::Color(c) => ary_new(vec![
            str_from("c"),
            float_new(c.red().0),
            float_new(c.green().0),
            float_new(c.blue().0),
            float_new(c.alpha().0),
        ]),
        Value::List(items, sep, br) => {
            let items: Vec<V> = items.iter().map(|x| sass_to_ruby(x, span)).collect();
            ary_new(vec![str_from("l"), ary_new(items), str_from(sep_name(*sep)), bool_v(matches!(br, Brackets::Bracketed))])
        }
        Value::ArgList(_) => {
            let items: Vec<V> = v.clone().as_list().iter().map(|x| sass_to_ruby(x, span)).collect();
            ary_new(vec![str_from("l"), ary_new(items), str_from("comma"), QFALSE])
        }
        Value::Map(m) => {
            let pairs: Vec<V> = m.iter().map(|(k, val)| ary_new(vec![sass_to_ruby(&k.node, span), sass_to_ruby(val, span)])).collect();
            ary_new(vec![str_from("m"), ary_new(pairs)])
        }
        other => {
            let s = other.inspect(span).unwrap_or_default();
            ary_new(vec![str_from("s"), str_from(&s), QFALSE])
        }
    }
}

fn num_of(v: V) -> f64 {
    if is_fix(v) { fix(v) as f64 } else if is_float(v) { float_val(v) } else { 0.0 }
}

/// A tagged array from Ruby back to a Sass value.
fn ruby_to_sass(v: V, span: Option<Span>) -> Value {
    if !is_ary(v) {
        return Value::Null;
    }
    let a: Vec<V> = rary(v).v.iter().copied().collect();
    let tag = a.first().map(|t| string_of(*t)).unwrap_or_default();
    let at = |i: usize| a.get(i).copied().unwrap_or(QNIL);
    match tag.as_str() {
        "s" => Value::String(string_of(at(1)), if truthy(at(2)) { QuoteKind::Quoted } else { QuoteKind::None }),
        "n" => {
            let unit = string_of(at(2));
            let unit = if unit.is_empty() { Unit::None } else { Unit::from(unit) };
            Value::Dimension(SassNumber { num: Number(num_of(at(1))), unit, as_slash: None })
        }
        "c" => Value::Color(Arc::new(Color::from_rgba(Number(num_of(at(1))), Number(num_of(at(2))), Number(num_of(at(3))), Number(num_of(at(4)))))),
        "b" => {
            if truthy(at(1)) {
                Value::True
            } else {
                Value::False
            }
        }
        "l" => {
            let items: Vec<Value> = if is_ary(at(1)) { rary(at(1)).v.iter().map(|x| ruby_to_sass(*x, span)).collect() } else { Vec::new() };
            let sep = match string_of(at(2)).as_str() {
                "comma" => ListSeparator::Comma,
                "slash" => ListSeparator::Slash,
                _ => ListSeparator::Space,
            };
            Value::List(items, sep, if truthy(at(3)) { Brackets::Bracketed } else { Brackets::None })
        }
        "m" => match span {
            Some(span) => {
                let mut m = SassMap::new();
                if is_ary(at(1)) {
                    for p in rary(at(1)).v.iter() {
                        if is_ary(*p) && rary(*p).v.len() == 2 {
                            let k = ruby_to_sass(rary(*p).v[0], Some(span));
                            let val = ruby_to_sass(rary(*p).v[1], Some(span));
                            m.insert(Spanned { node: k, span }, val);
                        }
                    }
                }
                Value::Map(m)
            }
            None => Value::Null,
        },
        _ => Value::Null,
    }
}

fn call_custom(i: usize, mut args: ArgumentResult) -> grass::Result<Value> {
    let Some((callback, params)) = SASS_CALLS.with(|c| c.borrow().last().map(|c| (c.callback, c.params.get(i).cloned().unwrap_or_default()))) else {
        return Ok(Value::Null);
    };
    // positional arguments first, then the remaining parameters by name
    let mut vals = Vec::new();
    let mut span: Option<Span> = None;
    let mut n = 0;
    while let Some(sv) = args.get_positional(n) {
        span.get_or_insert(sv.span);
        vals.push(sv);
        n += 1;
    }
    for name in params.iter().skip(n) {
        match args.get_named(name.as_str()) {
            Some(sv) => {
                span.get_or_insert(sv.span);
                vals.push(sv);
            }
            None => break,
        }
    }
    let ruby_args: Vec<V> = vals.iter().map(|sv| sass_to_ruby(&sv.node, sv.span)).collect();
    let r = send(callback, sym("call"), &[mkfix(i as i64), ary_new(ruby_args)], QNIL);
    let pending = std::mem::replace(&mut pst().pending, Throw::None);
    if !matches!(pending, Throw::None) || r == UNDEF {
        SASS_CALLS.with(|c| {
            if let Some(c) = c.borrow_mut().last_mut() {
                c.pending = Some(pending);
            }
        });
        return error_value("error in a custom Sass function", span);
    }
    if is_ary(r) && rary(r).v.first().is_some_and(|t| string_of(*t) == "error") {
        let msg = rary(r).v.get(1).map(|m| string_of(*m)).unwrap_or_default();
        return error_value(&msg, span);
    }
    Ok(ruby_to_sass(r, span))
}

fn error_value(msg: &str, span: Option<Span>) -> grass::Result<Value> {
    match span {
        Some(span) => Err((msg.to_string(), span).into()),
        // grass can only report an error with a source location; a function called without
        // arguments has none here, so (like sassc's handler) the message goes to stderr
        None => {
            crate::kernel::err_out(format!("[SassC::FunctionsHandler] {msg}\n").as_bytes());
            Ok(Value::Null)
        }
    }
}

/// `Kenny::Sass.render(template, filename, syntax, style, load_paths, functions, callback, importer, source_map)`
/// -> `["ok", css, [dependency paths], source map JSON or nil]` or `["error", message, line, file]`.
/// `functions` is `[[name, [param names]], ...]`; `callback.call(index, [tagged args])` runs one.
/// `importer` is nil or what gives the ERB and glob imports of sassc-rails their source (see
/// `SassFs`). `source_map` is nil or `[map file or nil, contents?, embed?, omit url?]`, sassc's
/// options of those names (see `SourceMapOpts`).
fn sass_render(_: V, a: &[V], _: V) -> V {
    let template = if is_str(a[0]) { rstr(a[0]).b.to_vec() } else { Vec::new() };
    let filename = if is_str(a[1]) { Some(string_of(a[1])) } else { None };
    let indented = string_of(a[2]) == "sass";
    let style = string_of(a[3]);
    let mut load_paths: Vec<PathBuf> = Vec::new();
    let entry = match &filename {
        Some(f) => {
            let p = PathBuf::from(f);
            let p = if p.is_absolute() { p } else { std::env::current_dir().map(|d| d.join(&p)).unwrap_or(p) };
            if let Some(d) = p.parent() {
                load_paths.push(d.to_path_buf());
            }
            p
        }
        None => std::env::current_dir().unwrap_or_default().join(if indented { "stdin.sass" } else { "stdin.scss" }),
    };
    if is_ary(a[4]) {
        for p in rary(a[4]).v.iter() {
            let p = PathBuf::from(string_of(*p));
            if !p.as_os_str().is_empty() && !load_paths.contains(&p) {
                load_paths.push(p);
            }
        }
    }
    let callback = a[6];
    let mut params = Vec::new();
    let mut names = Vec::new();
    if is_ary(a[5]) {
        for spec in rary(a[5]).v.iter().take(TRAMPOLINES.len()) {
            if !is_ary(*spec) {
                continue;
            }
            let s = rary(*spec);
            let name = s.v.first().map(|n| string_of(*n)).unwrap_or_default();
            let ps: Vec<String> = s.v.get(1).filter(|x| is_ary(**x)).map(|x| rary(*x).v.iter().map(|p| string_of(*p)).collect()).unwrap_or_default();
            names.push(name);
            params.push(ps);
        }
    }
    let fs = SassFs { entry: entry.clone(), src: template, read: RefCell::new(Vec::new()), importer: a[7], globbers: RefCell::new(Vec::new()) };
    let mut opts = grass::Options::default()
        .fs(&fs)
        .style(if style == "compressed" { grass::OutputStyle::Compressed } else { grass::OutputStyle::Expanded })
        .input_syntax(if indented { grass::InputSyntax::Sass } else { grass::InputSyntax::Scss })
        .load_paths(&load_paths)
        .quiet(true);
    for (i, name) in names.iter().enumerate() {
        // Sass doesn't distinguish `-` and `_` in names
        opts = opts.add_custom_fn(name.replace('_', "-"), grass::Builtin::new(TRAMPOLINES[i]));
        if name.contains('_') {
            opts = opts.add_custom_fn(name.clone(), grass::Builtin::new(TRAMPOLINES[i]));
        }
    }
    let map_opts = SourceMapOpts::from_ruby(a[8]);
    SASS_CALLS.with(|c| c.borrow_mut().push(SassCall { callback, params, pending: None }));
    let result = match &map_opts {
        Some(_) => grass::from_path_with_source_map(&entry, &opts).map(|(css, map)| (css, Some(map))),
        None => grass::from_path(&entry, &opts).map(|css| (css, None)),
    };
    let call = SASS_CALLS.with(|c| c.borrow_mut().pop());
    std::hint::black_box(callback);
    std::hint::black_box(a[7]);
    if let Some(Some(p)) = call.map(|c| c.pending) {
        if !matches!(p, Throw::None) {
            pst().pending = p;
            return UNDEF;
        }
    }
    match result {
        Ok((mut css, map)) => {
            let deps: Vec<V> = fs.read.borrow().iter().map(|p| str_from(&p.to_string_lossy())).collect();
            let json = match (map, &map_opts) {
                (Some(map), Some(o)) => Some(o.finish(&mut css, &map, filename.is_some(), &entry)),
                _ => None,
            };
            // with only `source_map_embed` the map is in the CSS, and sassc's #source_map raises
            let json = json.filter(|_| map_opts.as_ref().is_some_and(|o| o.file.is_some()));
            ary_new(vec![str_from("ok"), str_new(css.into_bytes()), ary_new(deps), json.map_or(QNIL, |j| str_new(j.into_bytes()))])
        }
        Err(e) => match e.kind() {
            grass::ErrorKind::ParseError { message, loc, .. } => ary_new(vec![
                str_from("error"),
                str_from(&format!("Error: {message}\n        on line {} of {}", loc.begin.line + 1, loc.file.name())),
                mkfix(loc.begin.line as i64 + 1),
                str_from(loc.file.name()),
            ]),
            grass::ErrorKind::IoError(io) => ary_new(vec![str_from("error"), str_from(&format!("Error: {io}")), QNIL, QNIL]),
            grass::ErrorKind::FromUtf8Error(s) => ary_new(vec![str_from("error"), str_from(&format!("Error: {s}")), QNIL, QNIL]),
            _ => ary_new(vec![str_from("error"), str_from("Error: Sass compilation failed"), QNIL, QNIL]),
        },
    }
}

/// What sassc's source map options ask for. Like libsass: a map is made when `source_map_file`
/// is given or with `source_map_embed`; its `file` and `sources` are relative to the map file's
/// directory, and the CSS ends with a `sourceMappingURL` comment (the map relative to the CSS,
/// which is the input file with `.css`) unless `omit_source_map_url`.
struct SourceMapOpts {
    file: Option<PathBuf>,
    contents: bool,
    embed: bool,
    omit_url: bool,
}

impl SourceMapOpts {
    fn from_ruby(v: V) -> Option<Self> {
        if !is_ary(v) {
            return None;
        }
        let v = &rary(v).v;
        let flag = |i: usize| v.get(i).is_some_and(|x| truthy(*x));
        let file = v.first().filter(|x| is_str(**x)).map(|x| string_of(*x)).filter(|f| !f.is_empty()).map(PathBuf::from);
        let o = SourceMapOpts { file, contents: flag(1), embed: flag(2), omit_url: flag(3) };
        (o.file.is_some() || o.embed).then_some(o)
    }

    /// The map's JSON (formatted as libsass does); adds the `sourceMappingURL` comment to `css`.
    /// Without a filename the entry is `stdin`, as with libsass.
    fn finish(&self, css: &mut String, map: &grass::SourceMap, named: bool, entry: &Path) -> String {
        let cwd = std::env::current_dir().unwrap_or_default();
        let map_path = self.file.as_deref().map_or_else(|| cwd.clone(), |f| absolute(&cwd, f));
        // the directory part of the path as given (for sassc-rails' `source_map_file: "."`, the
        // current directory)
        let map_dir = absolute(&cwd, self.file.as_deref().and_then(Path::parent).unwrap_or(Path::new("")));
        let output = if named { entry.with_extension("css") } else { cwd.join("stdin.css") };

        let mut json = String::from("{\n\t\"version\": 3,\n\t\"file\": ");
        json_str(&mut json, &relative(&map_dir, &output));
        json.push_str(",\n\t\"sources\": [");
        for (i, src) in map.sources.iter().enumerate() {
            json.push_str(if i == 0 { "\n\t\t" } else { ",\n\t\t" });
            if i == 0 && !named {
                json_str(&mut json, "stdin");
            } else {
                json_str(&mut json, &relative(&map_dir, &absolute(&cwd, Path::new(&src.name))));
            }
        }
        json.push_str("\n\t],");
        if self.contents {
            json.push_str("\n\t\"sourcesContent\": [");
            for (i, src) in map.sources.iter().enumerate() {
                json.push_str(if i == 0 { "\n\t\t" } else { ",\n\t\t" });
                json_str(&mut json, &src.contents);
            }
            json.push_str("\n\t],");
        }
        json.push_str("\n\t\"names\": [],\n\t\"mappings\": ");
        json_str(&mut json, &map.encode_mappings());
        json.push_str("\n}");

        if self.embed || !self.omit_url {
            let url = if self.embed {
                let b64 = crate::digest::base64(json.as_bytes());
                format!("data:application/json;base64,{}", String::from_utf8_lossy(&b64))
            } else {
                relative(output.parent().unwrap_or(&cwd), &map_path)
            };
            if !css.ends_with('\n') {
                css.push('\n');
            }
            css.push_str(&format!("\n/*# sourceMappingURL={url} */"));
        }
        json
    }
}

/// `p` against `cwd`, with `.` and `..` resolved as text (as libsass does: no symlinks followed).
fn absolute(cwd: &Path, p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in cwd.join(p).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// `to` relative to the directory `from` (both absolute), with `/`.
fn relative(from: &Path, to: &Path) -> String {
    let (f, t): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); f.len() - common];
    parts.extend(t[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}

/// `s` as a JSON string (non-ASCII as is, as libsass writes it).
fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------- JavaScript ----------

struct JsCtx {
    // the context first: it must be dropped before its runtime
    ctx: rquickjs::Context,
    _rt: rquickjs::Runtime,
}

/// QuickJS runtimes aren't thread-safe; every use of one goes through its mutex.
struct JsBox(Mutex<JsCtx>);
unsafe impl Send for JsBox {}
unsafe impl Sync for JsBox {}

unsafe extern "C" fn js_free(p: *mut std::ffi::c_void) {
    drop(unsafe { Box::from_raw(p as *mut JsBox) });
}

fn js_context() -> V {
    let rt_ = match rquickjs::Runtime::new() {
        Ok(r) => r,
        Err(e) => return raise(rt().c.runtime_error, &format!("can't start QuickJS: {e}")),
    };
    // Kenny's threads have stacks of 8 MB or more; UglifyJS recurses deeply on big files
    rt_.set_max_stack_size(4 << 20);
    let ctx = match rquickjs::Context::full(&rt_) {
        Ok(c) => c,
        Err(e) => return raise(rt().c.runtime_error, &format!("can't start QuickJS: {e}")),
    };
    let b = Box::new(JsBox(Mutex::new(JsCtx { ctx, _rt: rt_ })));
    alloc(RData {
        h: Header { klass: rt().c.object, kind: Kind::Data, flags: 0 },
        data: Box::into_raw(b) as *mut std::ffi::c_void,
        dmark: None,
        dfree: Some(js_free),
        roots: Vec::new(),
    })
}

/// `Kenny::JS.eval(context, source)`: evaluates `source` and returns `["ok", json]` (json is
/// nil when the value has no JSON form, e.g. undefined or a function) or
/// `["error", "syntax" | "runtime", message]`.
fn js_eval(c: V, src: V) -> V {
    if !is_kind(c, Kind::Data) || !is_str(src) {
        return type_error("Kenny::JS.eval(context, String)");
    }
    let jb = unsafe { &*((*(c as *mut RData)).data as *const JsBox) };
    let source = rstr(src).b.to_vec();
    let guard = jb.0.lock().unwrap_or_else(|e| e.into_inner());
    let out: Result<Option<String>, (bool, String)> = guard.ctx.with(|ctx| {
        // QuickJS measures its stack limit from where the runtime was created; each Kenny
        // thread runs on its own stack, so start from this one (rquickjs only does this with
        // its "parallel" feature). Otherwise any thread but the first gets "RangeError:
        // Maximum call stack size exceeded" (e.g. a JavaScript compressor run in a request)
        unsafe { rquickjs::qjs::JS_UpdateStackTop(rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr())) };
        // sloppy-mode global code, as in the browser and Node (rquickjs defaults to strict)
        let mut opts = rquickjs::context::EvalOptions::default();
        opts.strict = false;
        opts.filename = Some("(execjs)".into());
        let r: rquickjs::Result<rquickjs::Value> = ctx.eval_with_options(source, opts);
        match r {
            Ok(v) => match ctx.json_stringify(v) {
                Ok(s) => Ok(s.and_then(|s| s.to_string().ok())),
                Err(_) => Err(exception(&ctx)),
            },
            Err(_) => Err(exception(&ctx)),
        }
    });
    drop(guard);
    match out {
        Ok(Some(json)) => ary_new(vec![str_from("ok"), str_new(json.into_bytes())]),
        Ok(None) => ary_new(vec![str_from("ok"), QNIL]),
        Err((syntax, msg)) => ary_new(vec![str_from("error"), str_from(if syntax { "syntax" } else { "runtime" }), str_from(&msg)]),
    }
}

/// The pending JS exception: (is a SyntaxError, "Name: message" plus its JS stack).
fn exception(ctx: &rquickjs::Ctx) -> (bool, String) {
    let e = ctx.catch();
    if let Some(obj) = e.as_object() {
        let name: String = obj.get("name").unwrap_or_default();
        let msg: String = obj.get("message").unwrap_or_default();
        let stack: String = obj.get("stack").unwrap_or_default();
        let mut text = if name.is_empty() { msg } else { format!("{name}: {msg}") };
        if !stack.is_empty() {
            text.push('\n');
            text.push_str(stack.trim_end());
        }
        return (name == "SyntaxError", text);
    }
    let text = match e.as_string() {
        Some(s) => s.to_string().unwrap_or_default(),
        None => format!("{e:?}"),
    };
    (false, text)
}
