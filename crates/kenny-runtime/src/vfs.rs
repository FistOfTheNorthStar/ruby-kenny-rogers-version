//! Files embedded in the binary by `kenny build` (a Rack app's directory and the compiled
//! gems' directories), so the binary runs without the source tree.
//!
//! The image is keyed by the absolute paths the files had at build time. Every path-based
//! file operation looks there first:
//! - an embedded file is read from the image (read-only; the build's copy always wins);
//! - anything else goes to the real filesystem. Paths under the app's build directory are
//!   moved to the directory the binary runs in (the current directory at startup, or
//!   `KENNY_APP_ROOT`), so a deployed binary keeps its database, logs and `tmp/` there.
//!   Paths under that run directory also see the embedded files, so relative paths work.
//!   Started inside (or above) its build directory, while that still exists, the binary
//!   uses the build directory itself.
//! - writing to or deleting an embedded file goes to the real file in the run directory
//!   (copied from the image first), which from then on replaces the embedded copy.
//! - `Dir.chdir` into an embedded directory that has no real counterpart is virtual.
//! - live app files: in development (`RAILS_ENV`/`RACK_ENV` = development, run in its build
//!   directory) or with `KENNY_LIVE_FILES=1`, the app's own files come from disk, not the
//!   image, so edits show up (Rails code reloading, templates, routes); `KENNY_LIVE_FILES=0`
//!   turns that off. The gems' files still come from the image.
//!
//! Image layout (little-endian), written by `crates/kenny/src/embed.rs`:
//!   "KNVFS001", u32 root count, roots (u8 is_app, u32 len, bytes),
//!   u32 entry count, entries (u32 path len, path, u8 is_dir, u32 mode, i64 mtime,
//!   u64 size, u8 deflated, u64 data offset, u64 data len), then the data.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub struct Entry {
    pub dir: bool,
    pub mode: u32,
    pub mtime: i64,
    pub size: u64,
    pub ino: u64,
    deflated: bool,
    data: &'static [u8],
    inflated: OnceLock<&'static [u8]>,
    children: Vec<&'static [u8]>,
}

struct Image {
    entries: HashMap<&'static [u8], Entry>,
    /// the app's files are read from disk (see "live app files" above)
    live: bool,
    /// the app's directory at build time
    app_root: Option<&'static [u8]>,
    /// where the app's real (non-embedded) files live at runtime, if not `app_root`
    run_root: Option<Vec<u8>>,
}

static RAW: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static RAW_LEN: AtomicUsize = AtomicUsize::new(0);
static IMAGE: OnceLock<Option<Image>> = OnceLock::new();
/// Embedded paths written or deleted at runtime: the real file replaces them.
static SHADOWED: Mutex<Option<HashSet<Vec<u8>>>> = Mutex::new(None);
static ANY_SHADOWED: AtomicBool = AtomicBool::new(false);
/// The current directory while it is one that only exists in the image (`Dir.chdir` into an
/// embedded directory leaves the process's own directory alone).
static VCWD: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static HAS_VCWD: AtomicBool = AtomicBool::new(false);

/// Called by a constructor in the program (before `main`, no allocation).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_register_vfs(data: *const u8, len: u64) {
    RAW_LEN.store(len as usize, Ordering::Relaxed);
    RAW.store(data as *mut u8, Ordering::Release);
}

struct Rd {
    b: &'static [u8],
    i: usize,
}
impl Rd {
    fn take(&mut self, n: usize) -> Option<&'static [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

fn parse(b: &'static [u8]) -> Option<Image> {
    let mut r = Rd { b, i: 0 };
    if r.take(8)? != b"KNVFS001" {
        return None;
    }
    let mut app_root = None;
    for _ in 0..r.u32()? {
        let is_app = r.u8()? != 0;
        let n = r.u32()? as usize;
        let p = r.take(n)?;
        if is_app {
            app_root = Some(p);
        }
    }
    let n = r.u32()? as usize;
    let mut raw = Vec::with_capacity(n);
    for _ in 0..n {
        let pl = r.u32()? as usize;
        let path = r.take(pl)?;
        let dir = r.u8()? != 0;
        let mode = r.u32()?;
        let mtime = r.u64()? as i64;
        let size = r.u64()?;
        let deflated = r.u8()? != 0;
        let off = r.u64()? as usize;
        let len = r.u64()? as usize;
        raw.push((path, dir, mode, mtime, size, deflated, off, len));
    }
    let data = &b[r.i..];
    let mut entries: HashMap<&'static [u8], Entry> = HashMap::with_capacity(n);
    for (i, (path, dir, mode, mtime, size, deflated, off, len)) in raw.into_iter().enumerate() {
        let data = data.get(off..off + len)?;
        entries.insert(path, Entry { dir, mode, mtime, size, ino: 0x4b4e_0000_0000 + i as u64, deflated, data, inflated: OnceLock::new(), children: Vec::new() });
    }
    let paths: Vec<&'static [u8]> = entries.keys().copied().collect();
    for p in paths {
        if let Some(cut) = p.iter().rposition(|c| *c == b'/') {
            let parent: &[u8] = if cut == 0 { b"/" } else { &p[..cut] };
            if let Some(e) = entries.get_mut(parent) {
                e.children.push(&p[cut + 1..]);
            }
        }
    }
    for e in entries.values_mut() {
        e.children.sort();
    }
    let run_root = app_root.and_then(|r| {
        let explicit = std::env::var_os("KENNY_APP_ROOT").map(std::path::PathBuf::from);
        let t = explicit.clone().or_else(|| std::env::current_dir().ok())?;
        let t = std::fs::canonicalize(&t).unwrap_or(t);
        let t = t.as_os_str().as_bytes().to_vec();
        if t == r {
            return None;
        }
        // started inside (or above) the build directory, which is still there: use it
        let nested = under(&t, r).is_some() || under(r, &t).is_some() || t == b"/";
        if explicit.is_none() && nested && std::fs::metadata(OsStr::from_bytes(r)).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        Some(t)
    });
    let live = match std::env::var("KENNY_LIVE_FILES").ok().as_deref() {
        Some("1") | Some("true") => true,
        Some(_) => false,
        None => {
            let env = std::env::var("RAILS_ENV").or_else(|_| std::env::var("RACK_ENV")).unwrap_or_default();
            env == "development" && run_root.is_none() && app_root.is_some_and(|r| std::fs::metadata(OsStr::from_bytes(r)).is_ok_and(|m| m.is_dir()))
        }
    };
    Some(Image { entries, live, app_root, run_root })
}

fn image() -> Option<&'static Image> {
    let p = RAW.load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    IMAGE
        .get_or_init(|| {
            let b = unsafe { std::slice::from_raw_parts(p as *const u8, RAW_LEN.load(Ordering::Relaxed)) };
            parse(b)
        })
        .as_ref()
}

/// Is there an embedded image at all? (Cheap: plain programs have none.)
pub fn active() -> bool {
    image().is_some()
}

fn under<'a>(p: &'a [u8], root: &[u8]) -> Option<&'a [u8]> {
    let rest = p.strip_prefix(root)?;
    (rest.is_empty() || rest[0] == b'/').then_some(rest)
}

fn needs_normalizing(p: &[u8]) -> bool {
    p.windows(2).any(|w| w == b"//" || w == b"/.") || (p.len() > 1 && p.ends_with(b"/"))
}

fn normalize(full: &[u8]) -> Vec<u8> {
    let mut segs: Vec<&[u8]> = Vec::new();
    for s in full.split(|c| *c == b'/') {
        match s {
            b"" | b"." => {}
            b".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    let mut out = Vec::new();
    for s in segs {
        out.push(b'/');
        out.extend_from_slice(s);
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

/// The current directory as Ruby sees it: an embedded directory after `Dir.chdir` into one.
pub fn virtual_cwd() -> Option<Vec<u8>> {
    if !HAS_VCWD.load(Ordering::Relaxed) {
        return None;
    }
    VCWD.lock().unwrap().clone()
}

fn set_vcwd(d: Option<Vec<u8>>) {
    let mut g = VCWD.lock().unwrap();
    HAS_VCWD.store(d.is_some(), Ordering::Relaxed);
    *g = d;
}

/// `Dir.chdir`: into an embedded directory that has no real counterpart, only virtually.
pub fn chdir(p: &[u8]) -> std::io::Result<()> {
    if image().is_none() {
        return std::env::set_current_dir(OsStr::from_bytes(p));
    }
    if let Some((k, e)) = find_with_path(p) {
        if e.dir {
            let d = disk(&k).into_owned();
            if std::fs::metadata(OsStr::from_bytes(&d)).is_ok_and(|m| m.is_dir()) {
                std::env::set_current_dir(OsStr::from_bytes(&d))?;
                set_vcwd(None);
            } else {
                set_vcwd(Some(k));
            }
            return Ok(());
        }
    }
    let d = disk(p).into_owned();
    std::env::set_current_dir(OsStr::from_bytes(&d))?;
    set_vcwd(None);
    Ok(())
}

/// The absolute, normalized form of `p` (relative to the current directory).
fn absolute(p: &[u8]) -> Cow<'_, [u8]> {
    if p.starts_with(b"/") {
        if needs_normalizing(p) { Cow::Owned(normalize(p)) } else { Cow::Borrowed(p) }
    } else {
        let mut v = virtual_cwd().unwrap_or_else(|| std::env::current_dir().unwrap_or_default().as_os_str().as_bytes().to_vec());
        v.push(b'/');
        v.extend_from_slice(p);
        Cow::Owned(normalize(&v))
    }
}

/// The image key for `p`: absolute, with the run directory mapped to the app's build directory.
fn key<'a>(img: &Image, p: &'a [u8]) -> Cow<'a, [u8]> {
    let a = absolute(p);
    if let (Some(t), Some(r)) = (&img.run_root, img.app_root) {
        if under(&a, r).is_some() {
            return a; // the build directory may be inside the run directory
        }
        if let Some(rest) = under(&a, t) {
            let mut v = r.to_vec();
            v.extend_from_slice(rest);
            return Cow::Owned(v);
        }
    }
    a
}

/// Live app files: the image doesn't answer for paths in the app's directory.
fn live_skip(img: &Image, k: &[u8]) -> bool {
    img.live && img.app_root.is_some_and(|r| under(k, r).is_some())
}

fn shadowed(k: &[u8]) -> bool {
    ANY_SHADOWED.load(Ordering::Relaxed) && SHADOWED.lock().unwrap().as_ref().is_some_and(|s| s.contains(k))
}

/// The embedded file or directory at `p`, if any.
pub fn find(p: &[u8]) -> Option<&'static Entry> {
    let img = image()?;
    if p.is_empty() {
        return None;
    }
    let k = key(img, p);
    let e = img.entries.get(&*k)?;
    if shadowed(&k) || live_skip(img, &k) {
        return None;
    }
    Some(e)
}

/// Like `find`, also returning the entry's path in the image (the build-time path).
pub fn find_with_path(p: &[u8]) -> Option<(Vec<u8>, &'static Entry)> {
    let img = image()?;
    if p.is_empty() {
        return None;
    }
    let k = key(img, p);
    let e = img.entries.get(&*k)?;
    if shadowed(&k) || live_skip(img, &k) {
        return None;
    }
    Some((k.into_owned(), e))
}

/// Where `p` lives on the real filesystem: paths under the app's build directory are moved
/// to the run directory.
pub fn disk(p: &[u8]) -> Cow<'_, [u8]> {
    let Some(img) = image() else { return Cow::Borrowed(p) };
    static TRACE: OnceLock<bool> = OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var_os("KENNY_VFS_TRACE").is_some()) && find(p).is_none() {
        eprintln!("kenny-vfs: disk {}", String::from_utf8_lossy(&absolute(p)));
    }
    let rel = !p.starts_with(b"/");
    if rel && !HAS_VCWD.load(Ordering::Relaxed) {
        return Cow::Borrowed(p); // relative to the process's directory
    }
    let a = absolute(p);
    if let (Some(t), Some(r)) = (&img.run_root, img.app_root) {
        if let Some(rest) = under(&a, r) {
            let mut v = t.clone();
            v.extend_from_slice(rest);
            return Cow::Owned(v);
        }
    }
    if rel { Cow::Owned(a.into_owned()) } else { Cow::Borrowed(p) }
}

/// `p` is about to be written, created or removed: the real file replaces an embedded one
/// from now on, and directories that only exist in the image are created on disk.
pub fn prepare_write(p: &[u8]) {
    let Some(img) = image() else { return };
    let k = key(img, p);
    if let Some(cut) = k.iter().rposition(|c| *c == b'/') {
        ensure_dir(&k[..cut]);
    }
    if let Some(e) = img.entries.get(&*k).filter(|e| !e.dir) {
        if shadowed(&k) {
            return;
        }
        // copy-on-write: the real file starts as the embedded one
        let d = disk(&k).into_owned();
        let dp = OsStr::from_bytes(&d);
        if std::fs::symlink_metadata(dp).is_err() {
            let _ = std::fs::write(dp, contents(e));
            let _ = std::fs::set_permissions(dp, std::os::unix::fs::PermissionsExt::from_mode(e.mode & 0o7777));
        }
        SHADOWED.lock().unwrap().get_or_insert_with(HashSet::new).insert(k.into_owned());
        ANY_SHADOWED.store(true, Ordering::Relaxed);
    }
}

/// An embedded directory that is about to be used on disk (written into, or `chdir`ed to):
/// create it in the run directory.
pub fn ensure_dir(p: &[u8]) {
    let Some(img) = image() else { return };
    let k = key(img, p);
    if img.entries.get(&*k).is_some_and(|e| e.dir) {
        let d = disk(&k).into_owned();
        let dp = OsStr::from_bytes(&d);
        if !dp.is_empty() && std::fs::metadata(dp).is_err() {
            let _ = std::fs::create_dir_all(dp);
        }
    }
}

/// `File.realpath` of an embedded path: its path in the image.
pub fn realpath(p: &[u8]) -> Option<Vec<u8>> {
    find_with_path(p).map(|(k, _)| k)
}

/// Read a whole file, embedded or real (for native code that reads files by path).
pub fn read_file(p: &[u8]) -> std::io::Result<Cow<'static, [u8]>> {
    if let Some(b) = read(p) {
        return Ok(Cow::Borrowed(b));
    }
    let path = disk(p);
    crate::gc::blocking(|| std::fs::read(OsStr::from_bytes(&path))).map(Cow::Owned)
}

/// The real path to write `p` to (see `prepare_write`).
pub fn write_path(p: &[u8]) -> Vec<u8> {
    prepare_write(p);
    disk(p).into_owned()
}

/// The contents of an embedded file.
pub fn contents(e: &'static Entry) -> &'static [u8] {
    if !e.deflated {
        return e.data;
    }
    e.inflated.get_or_init(|| {
        let mut out = Vec::with_capacity(e.size as usize);
        let _ = flate2::read::DeflateDecoder::new(e.data).read_to_end(&mut out);
        Box::leak(out.into_boxed_slice())
    })
}

/// The contents of the embedded file at `p`.
pub fn read(p: &[u8]) -> Option<&'static [u8]> {
    find(p).filter(|e| !e.dir).map(contents)
}

/// A `stat` for an embedded entry: owned by the running user, read-only unless the mode
/// says otherwise, timestamps from the build.
pub fn stat(e: &Entry) -> libc::stat {
    let mut s: libc::stat = unsafe { std::mem::zeroed() };
    s.st_mode = (if e.dir { libc::S_IFDIR } else { libc::S_IFREG }) | (e.mode & 0o7777) as libc::mode_t;
    s.st_nlink = if e.dir { 2 } else { 1 };
    s.st_uid = unsafe { libc::geteuid() };
    s.st_gid = unsafe { libc::getegid() };
    s.st_size = e.size as libc::off_t;
    s.st_blksize = 4096;
    s.st_blocks = e.size.div_ceil(512) as _;
    s.st_ino = e.ino as _;
    s.st_dev = 0x4b4e as _;
    s.st_mtime = e.mtime as _;
    s.st_atime = e.mtime as _;
    s.st_ctime = e.mtime as _;
    #[cfg(target_os = "macos")]
    {
        s.st_birthtime = e.mtime as _;
    }
    s
}

/// Names in an embedded directory (without "." and "..").
pub fn children(e: &'static Entry) -> &'static [&'static [u8]] {
    &e.children
}

/// The names in directory `p`: the embedded ones (minus replaced files) plus those on disk.
/// None if neither has the directory.
pub fn list_dir(p: &[u8]) -> Option<Vec<Vec<u8>>> {
    let (k, e) = find_with_path(p).filter(|(_, e)| e.dir)?;
    let mut names: Vec<Vec<u8>> = Vec::with_capacity(e.children.len());
    for c in &e.children {
        if ANY_SHADOWED.load(Ordering::Relaxed) {
            let mut full = k.clone();
            full.push(b'/');
            full.extend_from_slice(c);
            if shadowed(&full) {
                continue;
            }
        }
        names.push(c.to_vec());
    }
    let d = disk(&k).into_owned();
    if let Ok(rd) = std::fs::read_dir(OsStr::from_bytes(&d)) {
        let have: HashSet<Vec<u8>> = names.iter().cloned().collect();
        for x in rd.flatten() {
            let n = x.file_name().as_bytes().to_vec();
            if !have.contains(&n) {
                names.push(n);
            }
        }
    }
    Some(names)
}
