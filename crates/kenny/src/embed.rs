//! The file image embedded in a built app (read by `kenny-runtime/src/vfs.rs`): the app's
//! directory (views, config, locales, public/ ...) and the compiled gems' directories, so the
//! binary runs without the source tree.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

struct Item {
    dir: bool,
    mode: u32,
    mtime: i64,
    size: u64,
    deflated: bool,
    data: Vec<u8>,
}

/// Directories of the app's own directory that hold runtime data, not part of the build.
const APP_SKIP_DIRS: &[&str] = &["log", "tmp", "node_modules", "storage", "coverage", ".bundle", "vendor/bundle"];
/// Files never embedded: runtime data and secrets (the master key is given at runtime, via
/// RAILS_MASTER_KEY or a config/master.key file in the run directory).
fn app_skip_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel == "config/master.key"
        || name == ".DS_Store"
        || [".sqlite3", ".sqlite", ".sqlite3-journal", ".sqlite3-wal", ".sqlite3-shm", ".log", ".pid"].iter().any(|s| name.ends_with(s))
}
/// Native code (compiled extensions, executables): never read as data.
fn native(path: &Path, head: &[u8]) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ["o", "a", "so", "bundle", "dylib", "dll", "exe", "jar", "class", "obj", "lib"].contains(&ext.as_str()) {
        return true;
    }
    head.starts_with(b"\x7fELF")
        || head.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || head.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
        || head.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || head.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
}
fn precompressed(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    ["gz", "br", "zst", "zip", "png", "jpg", "jpeg", "gif", "webp", "ico", "woff", "woff2", "mp4", "mp3", "pdf"].contains(&ext.as_str())
}

const MAX_FILE: u64 = 64 << 20;

pub struct Embedded {
    pub files: usize,
    pub bytes: usize,
}

/// Collect `app_root` (overlaid on the run directory at runtime) and the read-only `dirs`,
/// and write the image to `out`. `skip` are files never to embed (e.g. the output binary).
pub fn write_image(app_root: &Path, dirs: &[PathBuf], skip: &[PathBuf], out: &Path) -> Result<Embedded, String> {
    let mut items: BTreeMap<Vec<u8>, Item> = BTreeMap::new();
    let mut roots: Vec<(bool, Vec<u8>)> = Vec::new();
    let skip: HashSet<PathBuf> = skip.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let app = app_root.canonicalize().map_err(|e| format!("{}: {e}", app_root.display()))?;
    roots.push((true, app.as_os_str().as_bytes().to_vec()));
    walk(&app, &app, true, &skip, &mut items, &mut HashSet::new());
    for d in dirs {
        let Ok(d) = d.canonicalize() else { continue };
        if d.starts_with(&app) || !d.is_dir() {
            continue;
        }
        roots.push((false, d.as_os_str().as_bytes().to_vec()));
        walk(&d, &d, false, &skip, &mut items, &mut HashSet::new());
    }
    // the roots' parent directories, so the roots can be found by walking up (Rails finds an
    // engine's root that way) and `realpath` works on them
    for (_, r) in &roots {
        let mut p = Path::new(std::ffi::OsStr::from_bytes(r)).parent();
        while let Some(d) = p.filter(|d| d.parent().is_some()) {
            let key = d.as_os_str().as_bytes().to_vec();
            if !items.contains_key(&key) {
                let md = std::fs::metadata(d).ok();
                items.insert(key, Item { dir: true, mode: md.as_ref().map_or(0o755, |m| m.mode() & 0o7777), mtime: md.as_ref().map_or(0, |m| m.mtime()), size: 0, deflated: false, data: Vec::new() });
            }
            p = d.parent();
        }
    }
    let mut head = Vec::new();
    head.extend_from_slice(b"KNVFS001");
    head.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    for (is_app, p) in &roots {
        head.push(*is_app as u8);
        head.extend_from_slice(&(p.len() as u32).to_le_bytes());
        head.extend_from_slice(p);
    }
    head.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let mut data_len = 0u64;
    let mut files = 0;
    for (path, it) in &items {
        head.extend_from_slice(&(path.len() as u32).to_le_bytes());
        head.extend_from_slice(path);
        head.push(it.dir as u8);
        head.extend_from_slice(&it.mode.to_le_bytes());
        head.extend_from_slice(&it.mtime.to_le_bytes());
        head.extend_from_slice(&it.size.to_le_bytes());
        head.push(it.deflated as u8);
        head.extend_from_slice(&data_len.to_le_bytes());
        head.extend_from_slice(&(it.data.len() as u64).to_le_bytes());
        data_len += it.data.len() as u64;
        files += !it.dir as usize;
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?);
    let w = |f: &mut std::io::BufWriter<std::fs::File>, b: &[u8]| f.write_all(b).map_err(|e| format!("{}: {e}", out.display()));
    w(&mut f, &head)?;
    for it in items.values() {
        w(&mut f, &it.data)?;
    }
    f.flush().map_err(|e| e.to_string())?;
    Ok(Embedded { files, bytes: head.len() + data_len as usize })
}

fn walk(root: &Path, dir: &Path, app: bool, skip: &HashSet<PathBuf>, items: &mut BTreeMap<Vec<u8>, Item>, seen: &mut HashSet<(u64, u64)>) {
    let Ok(md) = std::fs::metadata(dir) else { return };
    if !seen.insert((md.dev(), md.ino())) {
        return; // a symlink loop
    }
    items.insert(dir.as_os_str().as_bytes().to_vec(), Item { dir: true, mode: md.mode() & 0o7777, mtime: md.mtime(), size: md.len(), deflated: false, data: Vec::new() });
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut ents: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    ents.sort();
    for p in ents {
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(md) = std::fs::metadata(&p) else { continue };
        if md.is_dir() {
            if name == ".git" || (app && APP_SKIP_DIRS.contains(&rel.as_str())) {
                continue;
            }
            walk(root, &p, app, skip, items, seen);
            continue;
        }
        if !md.is_file() || md.len() > MAX_FILE || (app && app_skip_file(&rel)) {
            continue;
        }
        if p.canonicalize().is_ok_and(|c| skip.contains(&c)) {
            continue;
        }
        let Ok(raw) = std::fs::read(&p) else { continue };
        if native(&p, &raw[..raw.len().min(4)]) {
            continue;
        }
        let (deflated, data) = if raw.len() > 256 && !precompressed(&p) {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            let _ = e.write_all(&raw);
            match e.finish() {
                Ok(z) if z.len() < raw.len() * 9 / 10 => (true, z),
                _ => (false, raw),
            }
        } else {
            (false, raw)
        };
        items.insert(p.as_os_str().as_bytes().to_vec(), Item { dir: false, mode: md.mode() & 0o7777, mtime: md.mtime(), size: md.len(), deflated, data });
    }
}

/// A C file that links the image into the program and registers it before `main`.
pub fn image_c_source(image: &Path, len: u64) -> String {
    let p = image.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"extern void kn_register_vfs(const unsigned char *, unsigned long long);
#ifdef __APPLE__
#define KN_SECTION ".section __TEXT,__const\n"
#define KN_SYM "_kenny_vfs_image"
#else
#define KN_SECTION ".section .rodata\n"
#define KN_SYM "kenny_vfs_image"
#endif
__asm__(KN_SECTION ".p2align 4\n.globl " KN_SYM "\n" KN_SYM ":\n.incbin \"{p}\"\n.byte 0\n.text\n");
extern const unsigned char kenny_vfs_image[];
__attribute__((constructor)) static void kn_register_embedded_files(void) {{
  kn_register_vfs(kenny_vfs_image, {len}ULL);
}}
"#
    )
}
