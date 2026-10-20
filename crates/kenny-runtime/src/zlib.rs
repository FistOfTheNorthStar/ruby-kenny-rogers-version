//! Zlib (the `zlib` C extension) on top of flate2's pure-Rust (miniz_oxide) backend.
//!
//! Compressed bytes are valid zlib/deflate/gzip streams but not byte-identical to C zlib's.
//! Streaming state of Zlib::Deflate / Zlib::Inflate / Zlib::GzipWriter lives in a side
//! table keyed by the hidden `@__zid` ivar (released on finish/close).
//! Zlib::GzipReader reads its whole source on construction.

use crate::compat::FL_BINARY;
use crate::core::*;
use crate::tri;
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use std::collections::HashMap;

struct ZErrs {
    error: *mut Class,
    data: *mut Class,
    buf: *mut Class,
    stream: *mut Class,
    gz_error: *mut Class,
    crc: *mut Class,
    length: *mut Class,
    no_footer: *mut Class,
}
static ERRS: G<Option<ZErrs>> = G::new(None);
fn errs() -> &'static ZErrs {
    ERRS.get().as_ref().unwrap()
}

enum Engine {
    Deflate(Compress),
    Inflate(Decompress),
}
struct ZState {
    eng: Engine,
    /// gzip framing for Deflate with wbits > 15 / GzipWriter
    gzip: bool,
    header_done: bool,
    crc: crc32fast::Hasher,
    isize: u64,
    /// Inflate: input left after the end of the stream
    unused: Vec<u8>,
    /// Inflate in gzip/auto mode: buffered input (decoded at once)
    gz_in: Vec<u8>,
    ended: bool,
    /// output collected by `<<`
    pending_out: Vec<u8>,
}
// Boxed, so a state's address stays put while the table changes (other threads may add theirs).
static STATES: crate::mt::L<Option<HashMap<i64, Box<ZState>>>> = crate::mt::L::new(None);
static NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

fn new_state(s: V, eng: Engine, gzip: bool) {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    STATES.get().get_or_insert_with(HashMap::new).insert(
        id,
        Box::new(ZState { eng, gzip, header_done: false, crc: crc32fast::Hasher::new(), isize: 0, unused: Vec::new(), gz_in: Vec::new(), ended: false, pending_out: Vec::new() }),
    );
    ivar_set(s, sym("@__zid"), mkfix(id));
}
fn state(s: V) -> Result<&'static mut ZState, V> {
    let id = ivar_get(s, sym("@__zid"));
    if is_fix(id) {
        let p = STATES.get().as_mut().and_then(|t| t.get_mut(&fix(id)).map(|b| &mut **b as *mut ZState));
        if let Some(p) = p {
            return Ok(unsafe { &mut *p });
        }
    }
    Err(raise(errs().error, "stream is not ready"))
}
fn drop_state(s: V) {
    let id = ivar_get(s, sym("@__zid"));
    if is_fix(id) {
        let b = STATES.get().as_mut().and_then(|t| t.remove(&fix(id)));
        drop(b);
    }
    ivar_set(s, sym("@__zid"), QNIL);
}

fn bin(b: Vec<u8>) -> V {
    let s = str_new(b);
    hdr(s).flags |= FL_BINARY;
    s
}
fn bytes_arg(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        Ok(rstr(v).b.clone())
    } else if v == QNIL {
        Ok(Vec::new())
    } else {
        Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(v)))))
    }
}
fn level_of(v: Option<V>) -> Compression {
    match v {
        Some(x) if is_fix(x) && (0..=9).contains(&fix(x)) => Compression::new(fix(x) as u32),
        _ => Compression::default(),
    }
}
fn flush_of(v: Option<V>) -> FlushCompress {
    match v.filter(|x| is_fix(*x)).map(fix) {
        Some(1) => FlushCompress::Partial,
        Some(2) => FlushCompress::Sync,
        Some(3) => FlushCompress::Full,
        Some(4) => FlushCompress::Finish,
        _ => FlushCompress::None,
    }
}

// ---------- raw engines ----------

fn run_compress(c: &mut Compress, input: &[u8], flush: FlushCompress) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2 + 128);
    let mut consumed = 0usize;
    loop {
        if out.capacity() - out.len() < 128 {
            out.reserve(out.capacity().max(1024));
        }
        let (bi, bo) = (c.total_in(), c.total_out());
        let st = c.compress_vec(&input[consumed..], &mut out, flush).unwrap_or(Status::BufError);
        consumed += (c.total_in() - bi) as usize;
        if st == Status::StreamEnd {
            break;
        }
        let progressed = c.total_in() != bi || c.total_out() != bo;
        let room = out.len() < out.capacity();
        if consumed >= input.len() && room && flush != FlushCompress::Finish {
            break;
        }
        if !progressed && room {
            break;
        }
    }
    out
}

enum InflateErr {
    Data(String),
}
/// Returns (output, stream ended, bytes of input consumed).
fn run_decompress(d: &mut Decompress, input: &[u8]) -> Result<(Vec<u8>, bool, usize), InflateErr> {
    let mut out = Vec::with_capacity(input.len() * 3 + 64);
    let mut consumed = 0usize;
    loop {
        if out.capacity() - out.len() < 4096 {
            out.reserve(out.capacity().max(16384));
        }
        let (bi, bo) = (d.total_in(), d.total_out());
        let st = d.decompress_vec(&input[consumed..], &mut out, FlushDecompress::None).map_err(|e| InflateErr::Data(e.to_string()))?;
        consumed += (d.total_in() - bi) as usize;
        if st == Status::StreamEnd {
            return Ok((out, true, consumed));
        }
        let progressed = d.total_in() != bi || d.total_out() != bo;
        let room = out.len() < out.capacity();
        if (consumed >= input.len() && room) || (!progressed && room) {
            return Ok((out, false, consumed));
        }
    }
}
fn zlib_header_ok(b: &[u8]) -> bool {
    b.len() < 2 || (b[0] & 0x0f == 8 && ((b[0] as u32) << 8 | b[1] as u32) % 31 == 0)
}
fn data_error(zlib: bool, input: &[u8], _msg: String) -> V {
    if zlib && !zlib_header_ok(input) {
        raise(errs().data, "incorrect header check")
    } else {
        raise(errs().data, "invalid compressed data -- format violated")
    }
}

pub fn deflate_bytes(input: &[u8], level: Compression, zlib: bool) -> Vec<u8> {
    let mut c = Compress::new(level, zlib);
    run_compress(&mut c, input, FlushCompress::Finish)
}
fn inflate_bytes(input: &[u8], zlib: bool) -> Result<Vec<u8>, V> {
    let mut d = Decompress::new(zlib);
    match run_decompress(&mut d, input) {
        Ok((out, true, _)) => Ok(out),
        Ok((_, false, _)) => Err(raise(errs().buf, "buffer error")),
        Err(InflateErr::Data(m)) => Err(data_error(zlib, input, m)),
    }
}

// ---------- gzip framing ----------

fn now_secs() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}
fn gzip_header(level: Compression, mtime: u32, name: Option<&[u8]>, comment: Option<&[u8]>) -> Vec<u8> {
    let mut flg = 0u8;
    if name.is_some() {
        flg |= 8;
    }
    if comment.is_some() {
        flg |= 16;
    }
    let xfl = match level.level() {
        9 => 2,
        1 => 4,
        _ => 0,
    };
    let mut h = vec![0x1f, 0x8b, 8, flg];
    h.extend_from_slice(&mtime.to_le_bytes());
    h.push(xfl);
    h.push(3);
    for x in [name, comment].into_iter().flatten() {
        h.extend(x.iter().copied().filter(|b| *b != 0));
        h.push(0);
    }
    h
}
pub fn gzip_bytes(input: &[u8], level: Compression, mtime: u32) -> Vec<u8> {
    let mut out = gzip_header(level, mtime, None, None);
    out.extend(deflate_bytes(input, level, false));
    out.extend_from_slice(&crc32fast::hash(input).to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out
}

struct GzInfo {
    data: Vec<u8>,
    mtime: u32,
    os: u8,
    name: Option<Vec<u8>>,
    comment: Option<Vec<u8>>,
    level: i64,
    unused: Vec<u8>,
}
enum GzErr {
    /// raise this exception (already raised: UNDEF)
    Raised(V),
    /// input ended early
    Short,
}
fn gunzip_bytes(b: &[u8]) -> Result<GzInfo, GzErr> {
    let gz = errs().gz_error;
    if b.len() < 10 {
        if b.len() >= 2 && b[..2] != [0x1f, 0x8b] {
            return Err(GzErr::Raised(raise(gz, "not in gzip format")));
        }
        return Err(GzErr::Short);
    }
    if b[0] != 0x1f || b[1] != 0x8b {
        return Err(GzErr::Raised(raise(gz, "not in gzip format")));
    }
    if b[2] != 8 {
        return Err(GzErr::Raised(raise(gz, &format!("unsupported compression method {}", b[2]))));
    }
    let flg = b[3];
    let mtime = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let level = match b[8] {
        2 => 9,
        4 => 1,
        _ => -1,
    };
    let os = b[9];
    let mut i = 10usize;
    if flg & 4 != 0 {
        if i + 2 > b.len() {
            return Err(GzErr::Short);
        }
        let xlen = u16::from_le_bytes([b[i], b[i + 1]]) as usize;
        i += 2 + xlen;
    }
    let cstr = |i: &mut usize| -> Result<Vec<u8>, GzErr> {
        let start = *i;
        while *i < b.len() && b[*i] != 0 {
            *i += 1;
        }
        if *i >= b.len() {
            return Err(GzErr::Short);
        }
        let s = b[start..*i].to_vec();
        *i += 1;
        Ok(s)
    };
    let name = if flg & 8 != 0 { Some(cstr(&mut i)?) } else { None };
    let comment = if flg & 16 != 0 { Some(cstr(&mut i)?) } else { None };
    if flg & 2 != 0 {
        i += 2;
    }
    if i > b.len() {
        return Err(GzErr::Short);
    }
    let mut d = Decompress::new(false);
    let (data, ended, used) = match run_decompress(&mut d, &b[i..]) {
        Ok(r) => r,
        Err(InflateErr::Data(m)) => return Err(GzErr::Raised(data_error(false, &[], m))),
    };
    if !ended {
        return Err(GzErr::Short);
    }
    let j = i + used;
    if j + 8 > b.len() {
        return Err(GzErr::Raised(raise(errs().no_footer, "footer is not found")));
    }
    let crc = u32::from_le_bytes([b[j], b[j + 1], b[j + 2], b[j + 3]]);
    let len = u32::from_le_bytes([b[j + 4], b[j + 5], b[j + 6], b[j + 7]]);
    if crc != crc32fast::hash(&data) {
        return Err(GzErr::Raised(raise(errs().crc, "invalid compressed data -- crc error")));
    }
    if len != data.len() as u32 {
        return Err(GzErr::Raised(raise(errs().length, "invalid compressed data -- length error")));
    }
    Ok(GzInfo { data, mtime, os, name, comment, level, unused: b[j + 8..].to_vec() })
}
fn gunzip_or_raise(b: &[u8]) -> Result<GzInfo, V> {
    match gunzip_bytes(b) {
        Ok(g) => Ok(g),
        Err(GzErr::Raised(e)) => Err(e),
        Err(GzErr::Short) => Err(raise(errs().gz_error, "unexpected end of string")),
    }
}

fn kwarg(a: &[V], name: &str) -> Option<V> {
    let h = *a.last()?;
    if !is_hash(h) {
        return None;
    }
    let v = crate::hash::hget(h, mksym(sym(name)));
    if v == QNIL { None } else { Some(v) }
}
fn positional(a: &[V]) -> &[V] {
    match a.last() {
        Some(h) if is_hash(*h) && hdr(*h).flags & FL_KWHASH != 0 => &a[..a.len() - 1],
        _ => a,
    }
}

// ---------- Zlib::Deflate / Zlib::Inflate ----------

fn deflate_chunk(s: V, input: &[u8], flush: FlushCompress) -> Result<Vec<u8>, V> {
    let st = state(s)?;
    let Engine::Deflate(c) = &mut st.eng else { return Err(raise(errs().stream, "stream error")) };
    let mut out = Vec::new();
    if st.gzip && !st.header_done {
        st.header_done = true;
        out.extend(gzip_header(Compression::default(), now_secs(), None, None));
    }
    if st.gzip {
        st.crc.update(input);
        st.isize += input.len() as u64;
    }
    out.extend(run_compress(c, input, flush));
    if flush == FlushCompress::Finish && st.gzip && !st.ended {
        out.extend_from_slice(&st.crc.clone().finalize().to_le_bytes());
        out.extend_from_slice(&(st.isize as u32).to_le_bytes());
    }
    if flush == FlushCompress::Finish {
        st.ended = true;
    }
    Ok(out)
}

/// Feed input to an Inflate stream; returns the newly inflated bytes.
fn inflate_chunk(s: V, input: &[u8]) -> Result<Vec<u8>, V> {
    let st = state(s)?;
    if st.ended {
        st.unused.extend_from_slice(input);
        return Ok(Vec::new());
    }
    if st.gzip {
        st.gz_in.extend_from_slice(input);
        let buf = st.gz_in.clone();
        // auto-detect (wbits 32+): zlib stream unless it starts with the gzip magic
        let looks_gzip = buf.len() < 2 || buf[..2] == [0x1f, 0x8b];
        if !looks_gzip {
            st.gzip = false;
            st.gz_in.clear();
            return inflate_chunk(s, &buf);
        }
        return match gunzip_bytes(&buf) {
            Ok(g) => {
                let st = state(s)?;
                st.ended = true;
                st.unused = g.unused;
                st.gz_in.clear();
                Ok(g.data)
            }
            Err(GzErr::Short) => Ok(Vec::new()),
            Err(GzErr::Raised(e)) => Err(e),
        };
    }
    let Engine::Inflate(d) = &mut st.eng else { return Err(raise(errs().stream, "stream error")) };
    let zlib = d.total_in() == 0;
    match run_decompress(d, input) {
        Ok((out, ended, used)) => {
            if ended {
                st.ended = true;
                st.unused = input[used..].to_vec();
            }
            Ok(out)
        }
        Err(InflateErr::Data(m)) => Err(data_error(zlib && is_zlib_mode(s), input, m)),
    }
}
fn is_zlib_mode(s: V) -> bool {
    truthy(ivar_get(s, sym("@__zlibhdr")))
}

fn init_streams(z: *mut Class) {
    let zstream = new_class("Zlib::ZStream", rt().c.object);
    const_set(z, sym("ZStream"), zstream as V);
    defn(zstream, "finished?", 0, 0, |s, _, _| bool_v(state(s).map(|st| st.ended).unwrap_or(true)));
    defn(zstream, "stream_end?", 0, 0, |s, _, _| bool_v(state(s).map(|st| st.ended).unwrap_or(true)));
    defn(zstream, "closed?", 0, 0, |s, _, _| bool_v(state(s).is_err() && { pst().pending = Throw::None; true }));
    defn(zstream, "ended?", 0, 0, |s, _, _| bool_v(state(s).is_err() && { pst().pending = Throw::None; true }));
    defn(zstream, "close", 0, 0, |s, _, _| {
        drop_state(s);
        QNIL
    });
    defn(zstream, "end", 0, 0, |s, _, _| {
        drop_state(s);
        QNIL
    });
    defn(zstream, "total_in", 0, 0, |s, _, _| match state(s) {
        Ok(st) => mkfix(match &st.eng {
            Engine::Deflate(c) => c.total_in(),
            Engine::Inflate(d) => d.total_in(),
        } as i64),
        Err(e) => e,
    });
    defn(zstream, "total_out", 0, 0, |s, _, _| match state(s) {
        Ok(st) => mkfix(match &st.eng {
            Engine::Deflate(c) => c.total_out(),
            Engine::Inflate(d) => d.total_out(),
        } as i64),
        Err(e) => e,
    });
    defn(zstream, "data_type", 0, 0, |_, _, _| mkfix(2));
    defn(zstream, "adler", 0, 0, |_, _, _| mkfix(1));
    defn(zstream, "avail_in", 0, 0, |_, _, _| mkfix(0));
    defn(zstream, "avail_out", 0, 0, |_, _, _| mkfix(0));
    defn(zstream, "flush_next_out", 0, 0, |s, _, _| match state(s) {
        Ok(st) => bin(std::mem::take(&mut st.pending_out)),
        Err(e) => e,
    });
    defn(zstream, "flush_next_in", 0, 0, |_, _, _| bin(Vec::new()));

    let deflate = new_class("Zlib::Deflate", zstream);
    const_set(z, sym("Deflate"), deflate as V);
    defs(deflate, "deflate", 1, 2, |_, a, _| match bytes_arg(a[0]) {
        Ok(b) => bin(deflate_bytes(&b, level_of(a.get(1).copied()), true)),
        Err(e) => e,
    });
    defn(deflate, "initialize", 0, 4, |s, a, _| {
        let wbits = a.get(1).copied().filter(|x| is_fix(*x)).map(fix).unwrap_or(15);
        let level = level_of(a.first().copied());
        ivar_set(s, sym("@__level"), mkfix(level.level() as i64));
        new_state(s, Engine::Deflate(Compress::new(level, wbits > 0 && wbits <= 15)), wbits > 15);
        QNIL
    });
    defn(deflate, "deflate", 1, 2, |s, a, _| {
        let b = match bytes_arg(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let flush = flush_of(a.get(1).copied());
        match deflate_chunk(s, &b, flush) {
            Ok(mut out) => {
                if let Ok(st) = state(s) {
                    let mut p = std::mem::take(&mut st.pending_out);
                    p.append(&mut out);
                    out = p;
                }
                bin(out)
            }
            Err(e) => e,
        }
    });
    defn(deflate, "<<", 1, 1, |s, a, _| {
        let b = match bytes_arg(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match deflate_chunk(s, &b, FlushCompress::None) {
            Ok(out) => {
                if let Ok(st) = state(s) {
                    st.pending_out.extend(out);
                }
                s
            }
            Err(e) => e,
        }
    });
    defn(deflate, "flush", 0, 1, |s, a, _| {
        let flush = match a.first() {
            Some(&x) => flush_of(Some(x)),
            None => FlushCompress::Sync,
        };
        match deflate_chunk(s, &[], flush) {
            Ok(out) => {
                let st = match state(s) {
                    Ok(st) => st,
                    Err(e) => return e,
                };
                let mut p = std::mem::take(&mut st.pending_out);
                p.extend(out);
                bin(p)
            }
            Err(e) => e,
        }
    });
    defn(deflate, "finish", 0, 0, |s, _, _| match deflate_chunk(s, &[], FlushCompress::Finish) {
        Ok(out) => {
            let st = match state(s) {
                Ok(st) => st,
                Err(e) => return e,
            };
            let mut p = std::mem::take(&mut st.pending_out);
            p.extend(out);
            bin(p)
        }
        Err(e) => e,
    });
    defn(deflate, "params", 2, 2, |_, _, _| QNIL);
    defn(deflate, "set_dictionary", 1, 1, |_, a, _| a[0]);

    let inflate = new_class("Zlib::Inflate", zstream);
    const_set(z, sym("Inflate"), inflate as V);
    defs(inflate, "inflate", 1, 1, |_, a, _| match bytes_arg(a[0]) {
        Ok(b) => match inflate_bytes(&b, true) {
            Ok(o) => bin(o),
            Err(e) => e,
        },
        Err(e) => e,
    });
    defn(inflate, "initialize", 0, 1, |s, a, _| {
        let wbits = a.first().copied().filter(|x| is_fix(*x)).map(fix).unwrap_or(15);
        let zlib = wbits > 0 && wbits <= 15;
        ivar_set(s, sym("@__zlibhdr"), bool_v(zlib));
        new_state(s, Engine::Inflate(Decompress::new(zlib)), wbits > 15);
        QNIL
    });
    defn(inflate, "inflate", 1, 2, |s, a, _| {
        let b = match bytes_arg(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match inflate_chunk(s, &b) {
            Ok(mut out) => {
                if let Ok(st) = state(s) {
                    let mut p = std::mem::take(&mut st.pending_out);
                    p.append(&mut out);
                    out = p;
                }
                bin(out)
            }
            Err(e) => e,
        }
    });
    defn(inflate, "<<", 1, 1, |s, a, _| {
        let b = match bytes_arg(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        match inflate_chunk(s, &b) {
            Ok(out) => {
                if let Ok(st) = state(s) {
                    st.pending_out.extend(out);
                }
                s
            }
            Err(e) => e,
        }
    });
    defn(inflate, "finish", 0, 0, |s, _, _| {
        let st = match state(s) {
            Ok(st) => st,
            Err(e) => return e,
        };
        if !st.ended {
            return raise(errs().buf, "buffer error");
        }
        bin(std::mem::take(&mut st.pending_out))
    });
    defn(inflate, "unused", 0, 0, |s, _, _| match state(s) {
        Ok(st) if !st.unused.is_empty() => bin(st.unused.clone()),
        Ok(_) => QNIL,
        Err(e) => e,
    });
    defn(inflate, "sync", 1, 1, |_, _, _| QFALSE);
    defn(inflate, "sync_point?", 0, 0, |_, _, _| QFALSE);
    defn(inflate, "set_dictionary", 1, 1, |_, a, _| a[0]);
}

// ---------- GzipWriter / GzipReader ----------

fn io_write(io: V, b: Vec<u8>) -> V {
    if b.is_empty() {
        return QNIL;
    }
    send1(io, "write", bin(b))
}
fn gzw_header(s: V) -> Result<Vec<u8>, V> {
    let st = state(s)?;
    if st.header_done {
        return Ok(Vec::new());
    }
    st.header_done = true;
    let lv = ivar_get(s, sym("@__level"));
    let level = level_of(Some(lv));
    let mt = ivar_get(s, sym("@__mtime"));
    let mtime = if is_fix(mt) && fix(mt) != 0 { fix(mt) as u32 } else { now_secs() };
    let name = ivar_get(s, sym("@__orig_name"));
    let comment = ivar_get(s, sym("@__comment"));
    let name = is_str(name).then(|| rstr(name).b.clone());
    let comment = is_str(comment).then(|| rstr(comment).b.clone());
    Ok(gzip_header(level, mtime, name.as_deref(), comment.as_deref()))
}
fn gzw_write(s: V, input: &[u8], flush: FlushCompress) -> V {
    let mut out = match gzw_header(s) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let st = match state(s) {
        Ok(st) => st,
        Err(e) => return e,
    };
    st.crc.update(input);
    st.isize += input.len() as u64;
    let Engine::Deflate(c) = &mut st.eng else { return QNIL };
    out.extend(run_compress(c, input, flush));
    if flush == FlushCompress::Finish {
        out.extend_from_slice(&st.crc.clone().finalize().to_le_bytes());
        out.extend_from_slice(&(st.isize as u32).to_le_bytes());
    }
    let io = ivar_get(s, sym("@__io"));
    if io == QNIL {
        // GzipWriter.open(path): collect and write the file on close
        let b = ivar_get(s, sym("@__fbuf"));
        if is_str(b) {
            rstr(b).b.extend(out);
        }
        return QNIL;
    }
    io_write(io, out)
}
fn gzw_finish(s: V) -> V {
    tri!(gzw_write(s, &[], FlushCompress::Finish));
    drop_state(s);
    let path = ivar_get(s, sym("@__path"));
    if is_str(path) {
        let p = String::from_utf8_lossy(&rstr(path).b).into_owned();
        let data = rstr(ivar_get(s, sym("@__fbuf"))).b.clone();
        if let Err(e) = std::fs::write(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&crate::vfs::write_path(p.as_bytes())), data) {
            return raise(rt().c.io_error, &format!("{p}: {e}"));
        }
    }
    ivar_get(s, sym("@__io"))
}
fn str_of(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    crate::fmt::to_s(v).map_err(|_| UNDEF)
}
/// Close a GzipFile: finish (writer) and close the underlying io if it can be closed.
fn gz_close(s: V) -> V {
    if truthy(ivar_get(s, sym("@__closed"))) {
        let gz = errs().gz_error;
        return raise(gz, "closed gzip stream");
    }
    let io = if truthy(ivar_get(s, sym("@__writer"))) { tri!(gzw_finish(s)) } else { ivar_get(s, sym("@__io")) };
    ivar_set(s, sym("@__closed"), QTRUE);
    if io != QNIL && respond_to(io, sym("close")) {
        tri!(send0(io, "close"));
    }
    io
}
fn wrap_impl(c: V, a: &[V], blk: V) -> V {
    let gz = tri!(send(c, sym("new"), a, QNIL));
    if blk == QNIL {
        return gz;
    }
    let r = call_block(blk, &[gz]);
    let saved = std::mem::replace(&mut pst().pending, Throw::None);
    if !truthy(ivar_get(gz, sym("@__closed"))) {
        let c = gz_close(gz);
        if c == UNDEF && matches!(saved, Throw::None) {
            return UNDEF;
        }
        pst().pending = Throw::None;
    }
    if !matches!(saved, Throw::None) {
        pst().pending = saved;
        return UNDEF;
    }
    r
}

fn reader_data(s: V) -> V {
    ivar_get(s, sym("@__data"))
}
fn reader_pos(s: V) -> usize {
    let p = ivar_get(s, sym("@__rpos"));
    if is_fix(p) { fix(p) as usize } else { 0 }
}
fn set_rpos(s: V, p: usize) {
    ivar_set(s, sym("@__rpos"), mkfix(p as i64));
}
fn reader_gets(s: V, sep: Option<&[u8]>) -> V {
    let d = &rstr(reader_data(s)).b;
    let pos = reader_pos(s);
    if pos >= d.len() {
        return QNIL;
    }
    let end = match sep {
        Some(sep) if !sep.is_empty() => crate::string::find(d, sep, pos).map(|i| i + sep.len()).unwrap_or(d.len()),
        _ => d.len(),
    };
    let line = str_new(d[pos..end].to_vec());
    set_rpos(s, end);
    let ln = ivar_get(s, sym("@__lineno"));
    ivar_set(s, sym("@__lineno"), mkfix(if is_fix(ln) { fix(ln) } else { 0 } + 1));
    line
}
fn sep_arg(a: &[V]) -> Option<Vec<u8>> {
    match a.first() {
        Some(&x) if is_str(x) => Some(rstr(x).b.clone()),
        Some(&QNIL) => None,
        _ => Some(b"\n".to_vec()),
    }
}

fn init_gzip(z: *mut Class) {
    let file = new_class("Zlib::GzipFile", rt().c.object);
    const_set(z, sym("GzipFile"), file as V);
    let gz_error = new_class("Zlib::GzipFile::Error", errs().error);
    const_set(file, sym("Error"), gz_error as V);
    let no_footer = new_class("Zlib::GzipFile::NoFooter", gz_error);
    const_set(file, sym("NoFooter"), no_footer as V);
    let crc = new_class("Zlib::GzipFile::CRCError", gz_error);
    const_set(file, sym("CRCError"), crc as V);
    let length = new_class("Zlib::GzipFile::LengthError", gz_error);
    const_set(file, sym("LengthError"), length as V);
    {
        let e = ERRS.get().as_mut().unwrap();
        e.gz_error = gz_error;
        e.no_footer = no_footer;
        e.crc = crc;
        e.length = length;
    }
    defs(file, "wrap", 1, -1, |c, a, b| wrap_impl(c, a, b));
    defn(file, "close", 0, 0, |s, _, _| gz_close(s));
    defn(file, "closed?", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__closed")))));
    defn(file, "to_io", 0, 0, |s, _, _| ivar_get(s, sym("@__io")));
    defn(file, "sync", 0, 0, |s, _, _| bool_v(truthy(ivar_get(s, sym("@__sync")))));
    defn(file, "sync=", 1, 1, |s, a, _| ivar_set(s, sym("@__sync"), a[0]));
    defn(file, "level", 0, 0, |s, _, _| {
        let l = ivar_get(s, sym("@__level"));
        if is_fix(l) { l } else { mkfix(-1) }
    });
    defn(file, "os_code", 0, 0, |s, _, _| {
        let o = ivar_get(s, sym("@__os"));
        if is_fix(o) { o } else { mkfix(3) }
    });
    defn(file, "mtime", 0, 0, |s, _, _| {
        let m = ivar_get(s, sym("@__mtime"));
        let secs = if is_fix(m) { m } else { mkfix(0) };
        send1(rt().c.time as V, "at", secs)
    });
    defn(file, "orig_name", 0, 0, |s, _, _| ivar_get(s, sym("@__orig_name")));
    defn(file, "comment", 0, 0, |s, _, _| ivar_get(s, sym("@__comment")));
    defn(file, "crc", 0, 0, |s, _, _| match state(s) {
        Ok(st) => mkfix(st.crc.clone().finalize() as i64),
        Err(_) => {
            pst().pending = Throw::None;
            let d = reader_data(s);
            mkfix(if is_str(d) { crc32fast::hash(&rstr(d).b) as i64 } else { 0 })
        }
    });

    // ---- writer ----
    let w = new_class("Zlib::GzipWriter", file);
    const_set(z, sym("GzipWriter"), w as V);
    defn(w, "initialize", 1, -1, |s, a, _| {
        let pa = positional(a);
        ivar_set(s, sym("@__io"), pa[0]);
        let level = level_of(pa.get(1).copied());
        ivar_set(s, sym("@__level"), mkfix(level.level() as i64));
        ivar_set(s, sym("@__writer"), QTRUE);
        ivar_set(s, sym("@__mtime"), mkfix(0));
        new_state(s, Engine::Deflate(Compress::new(level, false)), true);
        QNIL
    });
    defs(w, "open", 1, -1, |c, a, b| {
        let mut args = a.to_vec();
        args[0] = QNIL;
        let gz = tri!(send(c, sym("new"), &args, QNIL));
        ivar_set(gz, sym("@__path"), a[0]);
        ivar_set(gz, sym("@__fbuf"), bin(Vec::new()));
        if b == QNIL {
            return gz;
        }
        let r = call_block(b, &[gz]);
        let saved = std::mem::replace(&mut pst().pending, Throw::None);
        if !truthy(ivar_get(gz, sym("@__closed"))) {
            tri!(gz_close(gz));
        }
        if !matches!(saved, Throw::None) {
            pst().pending = saved;
            return UNDEF;
        }
        r
    });
    defn(w, "mtime=", 1, 1, |s, a, _| {
        let t = a[0];
        let secs = if is_int(t) { t } else { tri!(send0(t, "to_i")) };
        ivar_set(s, sym("@__mtime"), secs)
    });
    defn(w, "orig_name=", 1, 1, |s, a, _| ivar_set(s, sym("@__orig_name"), a[0]));
    defn(w, "comment=", 1, 1, |s, a, _| ivar_set(s, sym("@__comment"), a[0]));
    fn write(s: V, a: &[V]) -> Result<usize, V> {
        let mut n = 0;
        for &x in a {
            let b = str_of(x)?;
            n += b.len();
            let r = gzw_write(s, &b, FlushCompress::None);
            if r == UNDEF {
                return Err(UNDEF);
            }
        }
        Ok(n)
    }
    defn(w, "write", 0, -1, |s, a, _| match write(s, a) {
        Ok(n) => mkfix(n as i64),
        Err(e) => e,
    });
    defn(w, "<<", 1, 1, |s, a, _| match write(s, a) {
        Ok(_) => s,
        Err(e) => e,
    });
    defn(w, "print", 0, -1, |s, a, _| match write(s, a) {
        Ok(_) => QNIL,
        Err(e) => e,
    });
    defn(w, "putc", 1, 1, |s, a, _| {
        let b = if is_fix(a[0]) { vec![fix(a[0]) as u8] } else { rstr(a[0]).b[..1].to_vec() };
        tri!(gzw_write(s, &b, FlushCompress::None));
        a[0]
    });
    defn(w, "puts", 0, -1, |s, a, _| {
        let mut buf = Vec::new();
        let r = crate::kernel::puts_to(a, &mut |b| buf.extend_from_slice(b));
        if r == UNDEF {
            return UNDEF;
        }
        tri!(gzw_write(s, &buf, FlushCompress::None));
        QNIL
    });
    defn(w, "printf", 1, -1, |s, a, _| {
        let f = match str_of(a[0]) {
            Ok(f) => f,
            Err(e) => return e,
        };
        match crate::fmt::sprintf(&f, &a[1..]) {
            Ok(b) => {
                tri!(gzw_write(s, &b, FlushCompress::None));
                QNIL
            }
            Err(e) => e,
        }
    });
    defn(w, "flush", 0, 1, |s, a, _| {
        let f = match a.first() {
            Some(&x) => flush_of(Some(x)),
            None => FlushCompress::Sync,
        };
        if f != FlushCompress::None {
            tri!(gzw_write(s, &[], f));
        }
        let io = ivar_get(s, sym("@__io"));
        if io != QNIL && respond_to(io, sym("flush")) {
            tri!(send0(io, "flush"));
        }
        s
    });
    defn(w, "finish", 0, 0, |s, _, _| {
        let io = tri!(gzw_finish(s));
        ivar_set(s, sym("@__closed"), QTRUE);
        io
    });
    fn wpos(s: V) -> V {
        match state(s) {
            Ok(st) => mkfix(st.isize as i64),
            Err(e) => e,
        }
    }
    defn(w, "pos", 0, 0, |s, _, _| wpos(s));
    defn(w, "tell", 0, 0, |s, _, _| wpos(s));

    // ---- reader ----
    let r = new_class("Zlib::GzipReader", file);
    const_set(z, sym("GzipReader"), r as V);
    unsafe { (*r).includes.push(rt().c.enumerable) };
    fn reader_init(s: V, src: Vec<u8>) -> V {
        match gunzip_or_raise(&src) {
            Ok(g) => {
                ivar_set(s, sym("@__data"), str_new(g.data));
                ivar_set(s, sym("@__mtime"), mkfix(g.mtime as i64));
                ivar_set(s, sym("@__os"), mkfix(g.os as i64));
                ivar_set(s, sym("@__level"), mkfix(g.level));
                ivar_set(s, sym("@__orig_name"), g.name.map(str_new).unwrap_or(QNIL));
                ivar_set(s, sym("@__comment"), g.comment.map(str_new).unwrap_or(QNIL));
                ivar_set(s, sym("@__unused"), if g.unused.is_empty() { QNIL } else { bin(g.unused) });
                set_rpos(s, 0);
                ivar_set(s, sym("@__lineno"), mkfix(0));
                QNIL
            }
            Err(e) => e,
        }
    }
    defn(r, "initialize", 1, -1, |s, a, _| {
        let io = a[0];
        ivar_set(s, sym("@__io"), io);
        let data = tri!(send0(io, "read"));
        let src = if is_str(data) { rstr(data).b.clone() } else { Vec::new() };
        reader_init(s, src)
    });
    defs(r, "open", 1, -1, |c, a, b| {
        let p = match str_of(a[0]) {
            Ok(p) => String::from_utf8_lossy(&p).into_owned(),
            Err(e) => return e,
        };
        let src = match crate::vfs::read_file(p.as_bytes()) {
            Ok(x) => x.into_owned(),
            Err(_) => {
                let errno = const_lookup(rt().c.object, sym("Errno")).unwrap() as *mut Class;
                let enoent = const_get_under(errno, sym("ENOENT")).unwrap() as *mut Class;
                return raise(enoent, &format!("No such file or directory @ rb_sysopen - {p}"));
            }
        };
        let gz = object_new(c as *mut Class);
        tri!(reader_init(gz, src));
        if b == QNIL {
            return gz;
        }
        let r = call_block(b, &[gz]);
        ivar_set(gz, sym("@__closed"), QTRUE);
        r
    });
    defs(r, "zcat", 1, -1, |c, a, b| {
        let gz = tri!(send(c, sym("new"), a, QNIL));
        let d = reader_data(gz);
        if b != QNIL {
            tri!(call_block(b, &[d]));
            return QNIL;
        }
        str_new(rstr(d).b.clone())
    });
    defn(r, "read", 0, 2, |s, a, _| {
        let d = &rstr(reader_data(s)).b;
        let pos = reader_pos(s);
        match a.first() {
            Some(&n) if n != QNIL => {
                if !is_fix(n) || fix(n) < 0 {
                    return arg_error(&format!("negative length {} given", crate::fmt::inspect_lossy(n)));
                }
                if pos >= d.len() && fix(n) > 0 {
                    return QNIL;
                }
                let end = (pos + fix(n) as usize).min(d.len());
                set_rpos(s, end);
                bin(d[pos..end].to_vec())
            }
            _ => {
                set_rpos(s, d.len());
                str_new(d[pos.min(d.len())..].to_vec())
            }
        }
    });
    defn(r, "readpartial", 1, 2, |s, a, _| {
        let d = &rstr(reader_data(s)).b;
        let pos = reader_pos(s);
        if pos >= d.len() {
            let eof = const_lookup(rt().c.object, sym("EOFError")).unwrap() as *mut Class;
            return raise(eof, "end of file reached");
        }
        let end = (pos + fix(a[0]).max(0) as usize).min(d.len());
        set_rpos(s, end);
        bin(d[pos..end].to_vec())
    });
    defn(r, "gets", 0, 2, |s, a, _| reader_gets(s, sep_arg(a).as_deref()));
    defn(r, "readline", 0, 2, |s, a, _| {
        let l = reader_gets(s, sep_arg(a).as_deref());
        if l == QNIL {
            let eof = const_lookup(rt().c.object, sym("EOFError")).unwrap() as *mut Class;
            return raise(eof, "end of file reached");
        }
        l
    });
    defn(r, "readlines", 0, 2, |s, a, _| {
        let sep = sep_arg(a);
        let mut out = crate::gc::RVec::new();
        loop {
            let l = reader_gets(s, sep.as_deref());
            if l == QNIL {
                break;
            }
            out.push(l);
        }
        ary_new(out.take())
    });
    fn each_line(s: V, a: &[V], b: V) -> V {
        if b == QNIL {
            return crate::misc::enum_for(s, "each_line", a);
        }
        let sep = sep_arg(a);
        loop {
            let l = reader_gets(s, sep.as_deref());
            if l == QNIL {
                return s;
            }
            tri!(call_block(b, &[l]));
        }
    }
    defn(r, "each_line", 0, 2, |s, a, b| each_line(s, a, b));
    defn(r, "each", 0, 2, |s, a, b| each_line(s, a, b));
    defn(r, "each_byte", 0, 0, |s, _, b| {
        loop {
            let d = &rstr(reader_data(s)).b;
            let pos = reader_pos(s);
            if pos >= d.len() {
                return s;
            }
            let x = d[pos];
            set_rpos(s, pos + 1);
            tri!(call_block(b, &[mkfix(x as i64)]));
        }
    });
    defn(r, "getc", 0, 0, |s, _, _| {
        let d = &rstr(reader_data(s)).b;
        let pos = reader_pos(s);
        if pos >= d.len() {
            return QNIL;
        }
        let n = match d[pos] {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        let end = (pos + n).min(d.len());
        set_rpos(s, end);
        str_new(d[pos..end].to_vec())
    });
    defn(r, "getbyte", 0, 0, |s, _, _| {
        let d = &rstr(reader_data(s)).b;
        let pos = reader_pos(s);
        if pos >= d.len() {
            return QNIL;
        }
        set_rpos(s, pos + 1);
        mkfix(d[pos] as i64)
    });
    defn(r, "eof?", 0, 0, |s, _, _| bool_v(reader_pos(s) >= rstr(reader_data(s)).b.len()));
    defn(r, "eof", 0, 0, |s, _, _| bool_v(reader_pos(s) >= rstr(reader_data(s)).b.len()));
    defn(r, "pos", 0, 0, |s, _, _| mkfix(reader_pos(s) as i64));
    defn(r, "tell", 0, 0, |s, _, _| mkfix(reader_pos(s) as i64));
    defn(r, "lineno", 0, 0, |s, _, _| ivar_get(s, sym("@__lineno")));
    defn(r, "lineno=", 1, 1, |s, a, _| ivar_set(s, sym("@__lineno"), a[0]));
    defn(r, "rewind", 0, 0, |s, _, _| {
        set_rpos(s, 0);
        ivar_set(s, sym("@__lineno"), mkfix(0));
        mkfix(0)
    });
    defn(r, "unused", 0, 0, |s, _, _| ivar_get(s, sym("@__unused")));
    defn(r, "finish", 0, 0, |s, _, _| {
        ivar_set(s, sym("@__closed"), QTRUE);
        ivar_get(s, sym("@__io"))
    });
}

fn init_zlib() {
    let r = rt();
    let z = new_module("Zlib");
    set_global_const("Zlib", z as V);
    let error = new_class("Zlib::Error", r.c.standard_error);
    const_set(z, sym("Error"), error as V);
    let mk = |n: &str| {
        let k = new_class(&format!("Zlib::{n}"), error);
        const_set(z, sym(n), k as V);
        k
    };
    let data = mk("DataError");
    let buf = mk("BufError");
    let stream = mk("StreamError");
    for n in ["StreamEnd", "NeedDict", "MemError", "VersionError", "InProgressError"] {
        mk(n);
    }
    *ERRS.get() = Some(ZErrs { error, data, buf, stream, gz_error: error, crc: error, length: error, no_footer: error });
    for (n, v) in [
        ("NO_COMPRESSION", 0),
        ("BEST_SPEED", 1),
        ("BEST_COMPRESSION", 9),
        ("DEFAULT_COMPRESSION", -1),
        ("FILTERED", 1),
        ("HUFFMAN_ONLY", 2),
        ("RLE", 3),
        ("FIXED", 4),
        ("DEFAULT_STRATEGY", 0),
        ("MAX_WBITS", 15),
        ("DEF_MEM_LEVEL", 8),
        ("MAX_MEM_LEVEL", 9),
        ("NO_FLUSH", 0),
        ("PARTIAL_FLUSH", 1),
        ("SYNC_FLUSH", 2),
        ("FULL_FLUSH", 3),
        ("FINISH", 4),
        ("BLOCK", 5),
        ("BINARY", 0),
        ("ASCII", 1),
        ("TEXT", 1),
        ("UNKNOWN", 2),
        ("OS_CODE", 3),
        ("OS_MSDOS", 0),
        ("OS_AMIGA", 1),
        ("OS_VMS", 2),
        ("OS_UNIX", 3),
        ("OS_VMCMS", 4),
        ("OS_ATARI", 5),
        ("OS_OS2", 6),
        ("OS_MACOS", 7),
        ("OS_ZSYSTEM", 8),
        ("OS_CPM", 9),
        ("OS_TOPS20", 10),
        ("OS_WIN32", 11),
        ("OS_QDOS", 12),
        ("OS_RISCOS", 13),
        ("OS_UNKNOWN", 255),
    ] {
        const_set(z, sym(n), mkfix(v));
    }
    const_set(z, sym("VERSION"), str_from("3.2.1"));
    const_set(z, sym("ZLIB_VERSION"), str_from("1.3.1"));
    defs(z, "zlib_version", 0, 0, |_, _, _| str_from("1.3.1"));

    defs(z, "deflate", 1, 2, |_, a, _| match bytes_arg(a[0]) {
        Ok(b) => bin(deflate_bytes(&b, level_of(a.get(1).copied()), true)),
        Err(e) => e,
    });
    defs(z, "inflate", 1, 1, |_, a, _| match bytes_arg(a[0]) {
        Ok(b) => match inflate_bytes(&b, true) {
            Ok(o) => bin(o),
            Err(e) => e,
        },
        Err(e) => e,
    });
    defs(z, "gzip", 1, -1, |_, a, _| {
        let b = match bytes_arg(a[0]) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let level = level_of(kwarg(a, "level"));
        let mtime = match kwarg(a, "mtime") {
            Some(t) if is_fix(t) => fix(t) as u32,
            Some(t) => {
                let i = tri!(send0(t, "to_i"));
                if is_fix(i) { fix(i) as u32 } else { 0 }
            }
            None => now_secs(),
        };
        bin(gzip_bytes(&b, level, mtime))
    });
    defs(z, "gunzip", 1, 1, |_, a, _| match bytes_arg(a[0]) {
        Ok(b) => match gunzip_or_raise(&b) {
            Ok(g) => bin(g.data),
            Err(e) => e,
        },
        Err(e) => e,
    });
    fn checksum_args(a: &[V]) -> Result<(Vec<u8>, Option<u32>), V> {
        let b = match a.first() {
            Some(&x) => bytes_arg(x)?,
            None => Vec::new(),
        };
        let init = match a.get(1) {
            Some(&x) if is_int(x) => Some(if is_fix(x) { fix(x) as u32 } else { 0 }),
            _ => None,
        };
        Ok((b, init))
    }
    defs(z, "crc32", 0, 2, |_, a, _| match checksum_args(a) {
        Ok((b, init)) => {
            if a.is_empty() || a[0] == QNIL {
                return mkfix(init.unwrap_or(0) as i64);
            }
            let mut h = crc32fast::Hasher::new_with_initial(init.unwrap_or(0));
            h.update(&b);
            mkfix(h.finalize() as i64)
        }
        Err(e) => e,
    });
    defs(z, "adler32", 0, 2, |_, a, _| match checksum_args(a) {
        Ok((b, init)) => {
            if a.is_empty() || a[0] == QNIL {
                return mkfix(init.unwrap_or(1) as i64);
            }
            let mut h = adler2::Adler32::from_checksum(init.unwrap_or(1));
            h.write_slice(&b);
            mkfix(h.checksum() as i64)
        }
        Err(e) => e,
    });
    defs(z, "crc_table", 0, 0, |_, _, _| {
        let t: Vec<V> = (0u32..256)
            .map(|n| {
                let mut c = n;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
                }
                mkfix(c as i64)
            })
            .collect();
        ary_new(t)
    });
    init_streams(z);
    init_gzip(z);
}

pub fn init() {
    crate::compat::provide_feature(&["zlib"], Some(init_zlib));
}
