//! Native subset of Ruby's OpenSSL extension (what Rails' message signing and
//! encryption need) built on pure-Rust crypto: digests, HMAC, PBKDF2/HKDF,
//! AES in ECB/CBC/CTR/GCM modes, RSA keys (openssl_pkey.rs), X509 signature checks
//! (openssl_x509.rs) and the OS random source. Also hosts
//! `Random::Formatter` (the formatting half of SecureRandom).
//!
//! `require "openssl"` compiles the overlay `lib/kenny/openssl.rb`, which calls
//! `__kenny_openssl_init` to define the classes below and adds the Ruby-level
//! conveniences (digest subclasses, `OpenSSL::KDF`, `initialize_copy`).

use crate::compat::FL_BINARY;
use crate::core::*;
use crate::tri;
use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use sha2::Digest as _;

const FL_USASCII: u32 = FL_BINARY << 1;

static CTX: G<*mut Class> = G::new(std::ptr::null_mut());
static OSSL_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static DIGEST_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static HMAC_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static CIPHER_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static AUTH_TAG_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static DIGEST_CLASS: G<*mut Class> = G::new(std::ptr::null_mut());
static CIPHER_CLASS: G<*mut Class> = G::new(std::ptr::null_mut());
static KDF_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());

// ---------- byte helpers ----------

fn bin(b: Vec<u8>) -> V {
    let s = str_new(b);
    hdr(s).flags |= FL_BINARY;
    s
}
fn ascii(s: String) -> V {
    let v = str_new(s.into_bytes());
    hdr(v).flags |= FL_USASCII;
    v
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}
/// Cryptographically secure random bytes: getentropy(2) (no file descriptor per call; it
/// returns at most 256 bytes at a time), falling back to /dev/urandom.
pub fn urandom(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    let ok = b.chunks_mut(256).all(|c| unsafe { libc::getentropy(c.as_mut_ptr() as *mut libc::c_void, c.len()) } == 0);
    if !ok {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut b);
        }
    }
    b
}

fn implicit_err(v: V, into: &str) -> V {
    let n = match v {
        QNIL => "nil".to_string(),
        QTRUE => "true".to_string(),
        QFALSE => "false".to_string(),
        _ => class_name(real_class(v)),
    };
    type_error(&format!("no implicit conversion of {n} into {into}"))
}
/// Bytes of a String argument (or anything with `to_str`); Err holds UNDEF with the exception pending.
fn bytes(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    if is_heap(v) && respond_to(v, sym("to_str")) {
        let s = send(v, sym("to_str"), &[], QNIL);
        if s == UNDEF {
            return Err(UNDEF);
        }
        if is_str(s) {
            return Ok(rstr(s).b.clone());
        }
    }
    Err(implicit_err(v, "String"))
}
fn int(v: V) -> Result<i64, V> {
    if is_fix(v) {
        return Ok(fix(v));
    }
    if is_float(v) {
        return Ok(float_val(v) as i64);
    }
    Err(implicit_err(v, "Integer"))
}
macro_rules! get {
    ($e:expr) => {
        match $e {
            Ok(x) => x,
            Err(e) => return e,
        }
    };
}

// ---------- digests ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Alg {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}
impl Alg {
    fn parse(n: &[u8]) -> Option<Alg> {
        let u = String::from_utf8_lossy(n).to_ascii_uppercase().replace('-', "");
        Some(match u.as_str() {
            "MD5" => Alg::Md5,
            "SHA1" => Alg::Sha1,
            "SHA224" => Alg::Sha224,
            "SHA256" => Alg::Sha256,
            "SHA384" => Alg::Sha384,
            "SHA512" => Alg::Sha512,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Alg::Md5 => "MD5",
            Alg::Sha1 => "SHA1",
            Alg::Sha224 => "SHA224",
            Alg::Sha256 => "SHA256",
            Alg::Sha384 => "SHA384",
            Alg::Sha512 => "SHA512",
        }
    }
    fn out_len(self) -> usize {
        match self {
            Alg::Md5 => 16,
            Alg::Sha1 => 20,
            Alg::Sha224 => 28,
            Alg::Sha256 => 32,
            Alg::Sha384 => 48,
            Alg::Sha512 => 64,
        }
    }
    fn block_len(self) -> usize {
        match self {
            Alg::Sha384 | Alg::Sha512 => 128,
            _ => 64,
        }
    }
}

#[derive(Clone)]
enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha224(sha2::Sha224),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
}
impl Hasher {
    fn new(a: Alg) -> Hasher {
        match a {
            Alg::Md5 => Hasher::Md5(md5::Md5::new()),
            Alg::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            Alg::Sha224 => Hasher::Sha224(sha2::Sha224::new()),
            Alg::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            Alg::Sha384 => Hasher::Sha384(sha2::Sha384::new()),
            Alg::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }
    fn update(&mut self, d: &[u8]) {
        match self {
            Hasher::Md5(h) => h.update(d),
            Hasher::Sha1(h) => h.update(d),
            Hasher::Sha224(h) => h.update(d),
            Hasher::Sha256(h) => h.update(d),
            Hasher::Sha384(h) => h.update(d),
            Hasher::Sha512(h) => h.update(d),
        }
    }
    /// Digest of everything so far; the running state is left untouched.
    fn finish(&self) -> Vec<u8> {
        match self.clone() {
            Hasher::Md5(h) => h.finalize().to_vec(),
            Hasher::Sha1(h) => h.finalize().to_vec(),
            Hasher::Sha224(h) => h.finalize().to_vec(),
            Hasher::Sha256(h) => h.finalize().to_vec(),
            Hasher::Sha384(h) => h.finalize().to_vec(),
            Hasher::Sha512(h) => h.finalize().to_vec(),
        }
    }
}
fn hash(a: Alg, d: &[u8]) -> Vec<u8> {
    let mut h = Hasher::new(a);
    h.update(d);
    h.finish()
}

/// HMAC with the keyed inner/outer states precomputed (cheap to clone per PBKDF2 round).
#[derive(Clone)]
struct Mac {
    inner: Hasher,
    outer: Hasher,
}
impl Mac {
    fn new(a: Alg, key: &[u8]) -> Mac {
        let bl = a.block_len();
        let mut k = if key.len() > bl { hash(a, key) } else { key.to_vec() };
        k.resize(bl, 0);
        let mut inner = Hasher::new(a);
        let mut outer = Hasher::new(a);
        inner.update(&k.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
        outer.update(&k.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
        Mac { inner, outer }
    }
    fn finish(&self) -> Vec<u8> {
        let mut o = self.outer.clone();
        o.update(&self.inner.finish());
        o.finish()
    }
}
fn hmac(a: Alg, key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Mac::new(a, key);
    m.inner.update(data);
    m.finish()
}
fn pbkdf2(a: Alg, pass: &[u8], salt: &[u8], iter: u64, len: usize) -> Vec<u8> {
    let base = Mac::new(a, pass);
    let mut out = Vec::with_capacity(len + a.out_len());
    let mut block: u32 = 1;
    while out.len() < len {
        let mut m = base.clone();
        m.inner.update(salt);
        m.inner.update(&block.to_be_bytes());
        let mut u = m.finish();
        let mut t = u.clone();
        for _ in 1..iter.max(1) {
            let mut m = base.clone();
            m.inner.update(&u);
            u = m.finish();
            for (x, y) in t.iter_mut().zip(&u) {
                *x ^= y;
            }
        }
        out.extend(t);
        block += 1;
    }
    out.truncate(len);
    out
}
fn hkdf(a: Alg, ikm: &[u8], salt: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = hmac(a, salt, ikm);
    let mut out = Vec::new();
    let mut t = Vec::new();
    let mut i: u8 = 1;
    while out.len() < len {
        let mut m = Mac::new(a, &prk);
        m.inner.update(&t);
        m.inner.update(info);
        m.inner.update(&[i]);
        t = m.finish();
        out.extend(&t);
        i = i.wrapping_add(1);
    }
    out.truncate(len);
    out
}

// ---------- AES modes ----------

#[derive(Clone)]
enum Aes {
    A128(aes::Aes128),
    A192(aes::Aes192),
    A256(aes::Aes256),
}
impl Aes {
    fn new(k: &[u8]) -> Aes {
        match k.len() {
            16 => Aes::A128(aes::Aes128::new_from_slice(k).unwrap()),
            24 => Aes::A192(aes::Aes192::new_from_slice(k).unwrap()),
            _ => Aes::A256(aes::Aes256::new_from_slice(k).unwrap()),
        }
    }
    fn enc(&self, b: &mut [u8; 16]) {
        let mut a = Array::from(*b);
        match self {
            Aes::A128(c) => c.encrypt_block(&mut a),
            Aes::A192(c) => c.encrypt_block(&mut a),
            Aes::A256(c) => c.encrypt_block(&mut a),
        }
        b.copy_from_slice(a.as_slice());
    }
    fn dec(&self, b: &mut [u8; 16]) {
        let mut a = Array::from(*b);
        match self {
            Aes::A128(c) => c.decrypt_block(&mut a),
            Aes::A192(c) => c.decrypt_block(&mut a),
            Aes::A256(c) => c.decrypt_block(&mut a),
        }
        b.copy_from_slice(a.as_slice());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ecb,
    Cbc,
    Ctr,
    Gcm,
}

/// Multiplication in GF(2^128) as used by GHASH (big-endian bit order).
fn gmul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        let lsb = v & 1;
        v >>= 1;
        if lsb == 1 {
            v ^= 0xE1u128 << 120;
        }
    }
    z
}
fn ghash_absorb(x: &mut u128, h: u128, data: &[u8]) {
    for c in data.chunks(16) {
        let mut b = [0u8; 16];
        b[..c.len()].copy_from_slice(c);
        *x = gmul(*x ^ u128::from_be_bytes(b), h);
    }
}

/// Per-operation state, created lazily at the first update after (re)configuration.
#[derive(Clone)]
struct Run {
    aes: Aes,
    buf: Vec<u8>,
    prev: [u8; 16],
    ks: [u8; 16],
    ks_pos: usize,
    h: u128,
    j0: [u8; 16],
    x: u128,
    gbuf: Vec<u8>,
    aad_len: u64,
    ct_len: u64,
}
impl Run {
    fn keystream(&mut self, data: &[u8], inc32: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        for &b in data {
            if self.ks_pos == 16 {
                self.ks = self.prev;
                self.aes.enc(&mut self.ks);
                let n = if inc32 { 4 } else { 16 };
                for i in (16 - n..16).rev() {
                    self.prev[i] = self.prev[i].wrapping_add(1);
                    if self.prev[i] != 0 {
                        break;
                    }
                }
                self.ks_pos = 0;
            }
            out.push(b ^ self.ks[self.ks_pos]);
            self.ks_pos += 1;
        }
        out
    }
    fn ghash_feed(&mut self, ct: &[u8]) {
        self.gbuf.extend_from_slice(ct);
        let n = self.gbuf.len() / 16 * 16;
        let full: Vec<u8> = self.gbuf.drain(..n).collect();
        ghash_absorb(&mut self.x, self.h, &full);
        self.ct_len += ct.len() as u64;
    }
    fn gcm_tag(&mut self) -> Vec<u8> {
        let rest = std::mem::take(&mut self.gbuf);
        ghash_absorb(&mut self.x, self.h, &rest);
        let mut l = [0u8; 16];
        l[..8].copy_from_slice(&(self.aad_len * 8).to_be_bytes());
        l[8..].copy_from_slice(&(self.ct_len * 8).to_be_bytes());
        ghash_absorb(&mut self.x, self.h, &l);
        let mut ek = self.j0;
        self.aes.enc(&mut ek);
        (u128::from_be_bytes(ek) ^ self.x).to_be_bytes().to_vec()
    }
}

#[derive(Clone)]
struct Cipher {
    bits: usize,
    mode: Mode,
    enc: bool,
    key: Option<Vec<u8>>,
    iv: Option<Vec<u8>>,
    iv_len: usize,
    padding: bool,
    aad: Vec<u8>,
    tag_in: Option<Vec<u8>>,
    tag_out: Option<Vec<u8>>,
    run: Option<Run>,
}
impl Cipher {
    fn parse(n: &[u8]) -> Option<Cipher> {
        let l = String::from_utf8_lossy(n).to_ascii_lowercase();
        let l = l.strip_prefix("id-").unwrap_or(&l);
        let rest = l.strip_prefix("aes")?;
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        let (b, m) = rest.split_once('-')?;
        let bits: usize = b.parse().ok()?;
        if ![128, 192, 256].contains(&bits) {
            return None;
        }
        let mode = match m {
            "ecb" => Mode::Ecb,
            "cbc" => Mode::Cbc,
            "ctr" => Mode::Ctr,
            "gcm" => Mode::Gcm,
            _ => return None,
        };
        let iv_len = match mode {
            Mode::Ecb => 0,
            Mode::Gcm => 12,
            _ => 16,
        };
        Some(Cipher { bits, mode, enc: true, key: None, iv: None, iv_len, padding: true, aad: Vec::new(), tag_in: None, tag_out: None, run: None })
    }
    fn name(&self) -> String {
        match self.mode {
            Mode::Gcm => format!("id-aes{}-GCM", self.bits),
            Mode::Ecb => format!("AES-{}-ECB", self.bits),
            Mode::Cbc => format!("AES-{}-CBC", self.bits),
            Mode::Ctr => format!("AES-{}-CTR", self.bits),
        }
    }
    fn key_len(&self) -> usize {
        self.bits / 8
    }
    fn block_size(&self) -> usize {
        if matches!(self.mode, Mode::Ecb | Mode::Cbc) { 16 } else { 1 }
    }
    fn start(&mut self) -> Result<&mut Run, &'static str> {
        if self.run.is_none() {
            let key = self.key.as_ref().ok_or("key not set")?;
            let aes = Aes::new(key);
            let iv = self.iv.clone().unwrap_or_else(|| vec![0; self.iv_len]);
            let mut r = Run { aes, buf: Vec::new(), prev: [0; 16], ks: [0; 16], ks_pos: 16, h: 0, j0: [0; 16], x: 0, gbuf: Vec::new(), aad_len: 0, ct_len: 0 };
            match self.mode {
                Mode::Ecb => {}
                Mode::Cbc | Mode::Ctr => r.prev.copy_from_slice(&iv[..16]),
                Mode::Gcm => {
                    let mut hb = [0u8; 16];
                    r.aes.enc(&mut hb);
                    r.h = u128::from_be_bytes(hb);
                    if iv.len() == 12 {
                        r.j0[..12].copy_from_slice(&iv);
                        r.j0[15] = 1;
                    } else {
                        let mut s = 0u128;
                        ghash_absorb(&mut s, r.h, &iv);
                        let mut l = [0u8; 16];
                        l[8..].copy_from_slice(&((iv.len() as u64) * 8).to_be_bytes());
                        ghash_absorb(&mut s, r.h, &l);
                        r.j0 = s.to_be_bytes();
                    }
                    r.prev = r.j0;
                    for i in (12..16).rev() {
                        r.prev[i] = r.prev[i].wrapping_add(1);
                        if r.prev[i] != 0 {
                            break;
                        }
                    }
                    ghash_absorb(&mut r.x, r.h, &self.aad);
                    r.aad_len = self.aad.len() as u64;
                }
            }
            self.run = Some(r);
        }
        Ok(self.run.as_mut().unwrap())
    }
    fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
        let (enc, padding, mode) = (self.enc, self.padding, self.mode);
        let r = self.start()?;
        Ok(match mode {
            Mode::Ctr => r.keystream(data, false),
            Mode::Gcm => {
                if enc {
                    let out = r.keystream(data, true);
                    r.ghash_feed(&out);
                    out
                } else {
                    r.ghash_feed(data);
                    r.keystream(data, true)
                }
            }
            Mode::Ecb | Mode::Cbc => {
                r.buf.extend_from_slice(data);
                let mut n = r.buf.len() / 16 * 16;
                // with padding the last full block may be the padding block: hold it until final
                if !enc && padding && n == r.buf.len() && n > 0 {
                    n -= 16;
                }
                let blocks: Vec<u8> = r.buf.drain(..n).collect();
                let mut out = Vec::with_capacity(n);
                for c in blocks.chunks(16) {
                    let mut b: [u8; 16] = c.try_into().unwrap();
                    if enc {
                        if mode == Mode::Cbc {
                            for i in 0..16 {
                                b[i] ^= r.prev[i];
                            }
                        }
                        r.aes.enc(&mut b);
                        r.prev = b;
                    } else {
                        r.aes.dec(&mut b);
                        if mode == Mode::Cbc {
                            for i in 0..16 {
                                b[i] ^= r.prev[i];
                            }
                            r.prev.copy_from_slice(c);
                        }
                    }
                    out.extend_from_slice(&b);
                }
                out
            }
        })
    }
    fn finish(&mut self) -> Result<Vec<u8>, &'static str> {
        let (enc, padding, mode) = (self.enc, self.padding, self.mode);
        let tag_in = self.tag_in.clone();
        self.start()?;
        let mut r = self.run.take().unwrap();
        match mode {
            Mode::Ctr => Ok(Vec::new()),
            Mode::Gcm => {
                let tag = r.gcm_tag();
                if enc {
                    self.tag_out = Some(tag);
                    Ok(Vec::new())
                } else {
                    match tag_in {
                        Some(t) if !t.is_empty() && t.len() <= 16 && ct_eq(&t, &tag[..t.len()]) => Ok(Vec::new()),
                        _ => Err(""),
                    }
                }
            }
            Mode::Ecb | Mode::Cbc => {
                let cbc = mode == Mode::Cbc;
                if enc {
                    if !padding {
                        return if r.buf.is_empty() { Ok(Vec::new()) } else { Err("wrong final block length") };
                    }
                    let p = 16 - r.buf.len();
                    let mut b = [p as u8; 16];
                    b[..r.buf.len()].copy_from_slice(&r.buf);
                    if cbc {
                        for i in 0..16 {
                            b[i] ^= r.prev[i];
                        }
                    }
                    r.aes.enc(&mut b);
                    Ok(b.to_vec())
                } else {
                    if !padding {
                        return if r.buf.is_empty() { Ok(Vec::new()) } else { Err("wrong final block length") };
                    }
                    if r.buf.len() != 16 {
                        return Err("wrong final block length");
                    }
                    let mut b: [u8; 16] = r.buf[..].try_into().unwrap();
                    r.aes.dec(&mut b);
                    if cbc {
                        for i in 0..16 {
                            b[i] ^= r.prev[i];
                        }
                    }
                    let p = b[15] as usize;
                    if p == 0 || p > 16 || b[16 - p..].iter().any(|&x| x as usize != p) {
                        return Err("bad decrypt");
                    }
                    Ok(b[..16 - p].to_vec())
                }
            }
        }
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------- native state attached to Ruby objects ----------

enum Ctx {
    Digest(Alg, Hasher),
    Hmac(Alg, Mac, Mac),
    Cipher(Box<Cipher>),
    Rsa(Box<pkey::Rsa>),
}
impl Ctx {
    fn dup(&self) -> Ctx {
        match self {
            Ctx::Digest(a, h) => Ctx::Digest(*a, h.clone()),
            Ctx::Hmac(a, m, m0) => Ctx::Hmac(*a, m.clone(), m0.clone()),
            Ctx::Cipher(c) => Ctx::Cipher(c.clone()),
            Ctx::Rsa(k) => Ctx::Rsa(k.clone()),
        }
    }
}
unsafe extern "C" fn free_ctx(p: *mut std::ffi::c_void) {
    unsafe { drop(Box::from_raw(p as *mut Ctx)) };
}
fn set_ctx(s: V, c: Ctx) {
    let d = alloc(RData {
        h: Header { klass: *CTX.get(), kind: Kind::Data, flags: 0 },
        data: Box::into_raw(Box::new(c)) as *mut std::ffi::c_void,
        dmark: None,
        dfree: Some(free_ctx),
        roots: Vec::new(),
    });
    ivar_set(s, sym("@__ctx"), d);
}
fn ctx_of(s: V) -> Option<&'static mut Ctx> {
    let v = ivar_get(s, sym("@__ctx"));
    if is_kind(v, Kind::Data) && hdr(v).klass == *CTX.get() {
        Some(unsafe { &mut *((*(v as *mut RData)).data as *mut Ctx) })
    } else {
        None
    }
}
fn uninit(s: V) -> V {
    // (CRuby's wording; Digest/Cipher raise a RuntimeError)
    if is_a(s, *DIGEST_CLASS.get()) {
        return raise(rt().c.runtime_error, "Digest CTX wasn't initialized!");
    }
    if is_a(s, *CIPHER_CLASS.get()) {
        return raise(rt().c.runtime_error, "Cipher not initialized!");
    }
    raise(*OSSL_ERROR.get(), &format!("{} not initialized", class_name(real_class(s))))
}

// ---------- errors ----------
//
// The openssl gem 4.0 (Ruby 4.0's) puts the entries of OpenSSL's error queue in the
// exception's `@errors` (OpenSSLError#errors), and some messages end with the last entry's
// reason. Kenny's crypto isn't OpenSSL, so it writes the entries OpenSSL 3 would have.

/// OpenSSL 3's error-queue entry for an algorithm name it doesn't know.
fn unsupported_entry(name: &str) -> String {
    format!("error:0308010C:digital envelope routines:inner_evp_generic_fetch:unsupported (Global default library context, Algorithm ({name} : 0), Properties (<null>))")
}
const BAD_DECRYPT: &str = "error:1C800064:Provider routines:ossl_cipher_unpadblock:bad decrypt";
const WRONG_FINAL_BLOCK: &str = "error:1C80006B:Provider routines:ossl_cipher_generic_block_final:wrong final block length";
const INVALID_TAG: &str = "error:1C80006E:Provider routines:ossl_gcm_get_ctx_params:invalid tag";
const INVALID_KEY_LENGTH: &str = "error:03000082:digital envelope routines:EVP_CIPHER_CTX_set_key_length:invalid key length";

/// The reason text of an error-queue entry (what follows `error:CODE:library:function:`).
fn reason(entry: &str) -> &str {
    entry.splitn(5, ':').nth(4).unwrap_or(entry)
}

/// Raise `cls` with `msg` as the openssl gem does: in a 4.0 build with `@errors` holding the
/// error queue's `entry` (none: an empty queue).
fn ossl_raise(cls: *mut Class, msg: &str, entry: Option<&str>) -> V {
    let e = exc_new(cls, msg);
    if crate::misc::ruby4() {
        ivar_set(e, sym("@errors"), ary_new(entry.into_iter().map(str_from).collect()));
    }
    raise_exc(e)
}
fn digest_ctx(s: V) -> Result<(Alg, &'static mut Hasher), V> {
    match ctx_of(s) {
        Some(Ctx::Digest(a, h)) => Ok((*a, h)),
        _ => Err(uninit(s)),
    }
}
fn hmac_ctx(s: V) -> Result<(Alg, &'static mut Mac, &'static mut Mac), V> {
    match ctx_of(s) {
        Some(Ctx::Hmac(a, m, m0)) => Ok((*a, m, m0)),
        _ => Err(uninit(s)),
    }
}
fn cipher_ctx(s: V) -> Result<&'static mut Cipher, V> {
    match ctx_of(s) {
        Some(Ctx::Cipher(c)) => Ok(c),
        _ => Err(uninit(s)),
    }
}

/// A digest given as a name ("SHA256", :sha256), an OpenSSL::Digest instance or a digest class.
fn alg_arg(v: V) -> Result<Alg, V> {
    if let Some(Ctx::Digest(a, _)) = (is_heap(v) && !is_str(v)).then(|| ctx_of(v)).flatten() {
        return Ok(*a);
    }
    if is_class(v) {
        let inst = send(v, sym("new"), &[], QNIL);
        if inst == UNDEF {
            return Err(UNDEF);
        }
        return alg_arg(inst);
    }
    let name = if is_sym(v) {
        sym_name(sym_id(v)).to_vec()
    } else if is_heap(v) && !is_str(v) && respond_to(v, sym("name")) {
        let n = send(v, sym("name"), &[], QNIL);
        if n == UNDEF {
            return Err(UNDEF);
        }
        bytes(n)?
    } else {
        bytes(v)?
    };
    Alg::parse(&name).ok_or_else(|| {
        let name = String::from_utf8_lossy(&name);
        // (the openssl gem 4.0 of Ruby 4.0 raises DigestError, 3.x a RuntimeError)
        if crate::misc::ruby4() {
            let entry = unsupported_entry(&name);
            ossl_raise(*DIGEST_ERROR.get(), &format!("unsupported digest algorithm: {name}: {}", reason(&entry)), Some(&entry))
        } else {
            raise(rt().c.runtime_error, &format!("Unsupported digest algorithm ({name}).: unknown object name"))
        }
    })
}

fn class_under(m: *mut Class, name: &str, sup: *mut Class) -> *mut Class {
    let full = format!("{}::{name}", class_name(m));
    let c = new_class(&full, sup);
    rclass(c as V).lexical_parent = m;
    const_set(m, sym(name), c as V);
    c
}
fn module_under(m: *mut Class, name: &str) -> *mut Class {
    let full = format!("{}::{name}", class_name(m));
    let c = new_module(&full);
    rclass(c as V).lexical_parent = m;
    const_set(m, sym(name), c as V);
    c
}

// ---------- OpenSSL::Digest ----------

fn digest_out(s: V, a: &[V], fmt: fn(Vec<u8>) -> V) -> V {
    let (alg, h) = get!(digest_ctx(s));
    match a.first() {
        Some(&d) => {
            let d = get!(bytes(d));
            *h = Hasher::new(alg);
            h.update(&d);
            let r = h.finish();
            *h = Hasher::new(alg);
            fmt(r)
        }
        None => fmt(h.finish()),
    }
}
fn fmt_bin(b: Vec<u8>) -> V {
    bin(b)
}
fn fmt_hex(b: Vec<u8>) -> V {
    ascii(hex(&b))
}
fn fmt_b64(b: Vec<u8>) -> V {
    ascii(base64(&b))
}

fn init_digest(ossl: *mut Class) {
    let d = class_under(ossl, "Digest", rt().c.object);
    *DIGEST_CLASS.get() = d;
    *DIGEST_ERROR.get() = class_under(d, "DigestError", *OSSL_ERROR.get());
    defn(d, "initialize", 0, 2, |s, a, _| {
        let Some(&n) = a.first() else {
            return arg_error("wrong number of arguments (given 0, expected 1..2)");
        };
        let alg = get!(alg_arg(n));
        let mut h = Hasher::new(alg);
        if let Some(&data) = a.get(1).filter(|x| **x != QNIL) {
            h.update(&get!(bytes(data)));
        }
        set_ctx(s, Ctx::Digest(alg, h));
        QNIL
    });
    defn(d, "__kenny_copy", 1, 1, |s, a, _| {
        if let Some(c) = ctx_of(a[0]) {
            set_ctx(s, c.dup());
        }
        s
    });
    fn update(s: V, a: &[V], _: V) -> V {
        let (_, h) = get!(digest_ctx(s));
        h.update(&get!(bytes(a[0])));
        s
    }
    defn(d, "update", 1, 1, update);
    defn(d, "<<", 1, 1, update);
    // Digest::Instance#file and Digest::Class.file (CRuby's OpenSSL::Digest is a Digest::Class):
    // ActiveStorage checksums uploads with OpenSSL::Digest::MD5.file
    defn(d, "file", 1, 1, |s, a, _| {
        // (a path, or anything with to_path: a Tempfile, a Pathname)
        let p = if !is_str(a[0]) && respond_to(a[0], sym("to_path")) { tri!(send(a[0], sym("to_path"), &[], QNIL)) } else { a[0] };
        let path = match crate::fmt::to_s(p) {
            Ok(b) => b,
            Err(()) => return UNDEF,
        };
        match crate::vfs::read_file(&path) {
            Ok(b) => {
                tri!(send(s, sym("update"), &[bin(b.into_owned())], QNIL));
                s
            }
            Err(e) => crate::io::raise_errno(e.raw_os_error().unwrap_or(libc::ENOENT), Some("rb_sysopen"), &path),
        }
    });
    defs(d, "file", 1, -1, |s, a, _| {
        let o = tri!(send(s, sym("new"), &a[1..], QNIL));
        send(o, sym("file"), &[a[0]], QNIL)
    });
    defn(d, "reset", 0, 0, |s, _, _| {
        let (alg, h) = get!(digest_ctx(s));
        *h = Hasher::new(alg);
        s
    });
    defn(d, "digest", 0, 1, |s, a, _| digest_out(s, a, fmt_bin));
    defn(d, "hexdigest", 0, 1, |s, a, _| digest_out(s, a, fmt_hex));
    defn(d, "base64digest", 0, 1, |s, a, _| digest_out(s, a, fmt_b64));
    defn(d, "to_s", 0, 0, |s, _, _| digest_out(s, &[], fmt_hex));
    defn(d, "inspect", 0, 0, |s, _, _| {
        let (_, h) = get!(digest_ctx(s));
        str_from(&format!("#<{}: {}>", class_name(real_class(s)), hex(&h.finish())))
    });
    fn bang(s: V, f: fn(Vec<u8>) -> V) -> V {
        let (alg, h) = get!(digest_ctx(s));
        let r = h.finish();
        *h = Hasher::new(alg);
        f(r)
    }
    defn(d, "digest!", 0, 0, |s, _, _| bang(s, fmt_bin));
    defn(d, "hexdigest!", 0, 0, |s, _, _| bang(s, fmt_hex));
    defn(d, "name", 0, 0, |s, _, _| str_from(get!(digest_ctx(s)).0.name()));
    fn dlen(s: V, _: &[V], _: V) -> V {
        mkfix(get!(digest_ctx(s)).0.out_len() as i64)
    }
    defn(d, "digest_length", 0, 0, dlen);
    defn(d, "size", 0, 0, dlen);
    defn(d, "length", 0, 0, dlen);
    defn(d, "block_length", 0, 0, |s, _, _| mkfix(get!(digest_ctx(s)).0.block_len() as i64));
    defn(d, "==", 1, 1, |s, a, _| {
        let (_, h) = get!(digest_ctx(s));
        let mine = h.finish();
        let o = a[0];
        if is_str(o) {
            let ob = &rstr(o).b;
            return bool_v(*ob == mine || *ob == hex(&mine).into_bytes());
        }
        match ctx_of(o) {
            Some(Ctx::Digest(_, oh)) => bool_v(oh.finish() == mine),
            _ => QFALSE,
        }
    });
    // OpenSSL::Digest.digest("SHA256", data) / OpenSSL::Digest::SHA256.digest(data)
    fn class_out(s: V, a: &[V], m: &str) -> V {
        let (ctor, data): (&[V], V) = match a {
            [n, d] => (std::slice::from_ref(n), *d),
            [d] => (&[], *d),
            _ => return arg_error(&format!("wrong number of arguments (given {}, expected 1..2)", a.len())),
        };
        let inst = tri!(send(s, sym("new"), ctor, QNIL));
        send(inst, sym(m), &[data], QNIL)
    }
    defs(d, "digest", 1, 2, |s, a, _| class_out(s, a, "digest"));
    defs(d, "hexdigest", 1, 2, |s, a, _| class_out(s, a, "hexdigest"));
    defs(d, "base64digest", 1, 2, |s, a, _| class_out(s, a, "base64digest"));
}

// ---------- OpenSSL::HMAC ----------

fn init_hmac(ossl: *mut Class) {
    let h = class_under(ossl, "HMAC", rt().c.object);
    *HMAC_ERROR.get() = class_under(ossl, "HMACError", *OSSL_ERROR.get());
    defn(h, "initialize", 2, 2, |s, a, _| {
        let key = get!(bytes(a[0]));
        let alg = get!(alg_arg(a[1]));
        let m = Mac::new(alg, &key);
        set_ctx(s, Ctx::Hmac(alg, m.clone(), m));
        QNIL
    });
    defn(h, "__kenny_copy", 1, 1, |s, a, _| {
        if let Some(c) = ctx_of(a[0]) {
            set_ctx(s, c.dup());
        }
        s
    });
    fn update(s: V, a: &[V], _: V) -> V {
        let (_, m, _) = get!(hmac_ctx(s));
        m.inner.update(&get!(bytes(a[0])));
        s
    }
    defn(h, "update", 1, 1, update);
    defn(h, "<<", 1, 1, update);
    defn(h, "reset", 0, 0, |s, _, _| {
        let (_, m, m0) = get!(hmac_ctx(s));
        *m = m0.clone();
        s
    });
    fn out(s: V, f: fn(Vec<u8>) -> V) -> V {
        let (_, m, _) = get!(hmac_ctx(s));
        f(m.finish())
    }
    defn(h, "digest", 0, 0, |s, _, _| out(s, fmt_bin));
    defn(h, "hexdigest", 0, 0, |s, _, _| out(s, fmt_hex));
    defn(h, "base64digest", 0, 0, |s, _, _| out(s, fmt_b64));
    defn(h, "to_s", 0, 0, |s, _, _| out(s, fmt_hex));
    defn(h, "inspect", 0, 0, |s, _, _| out(s, fmt_hex));
    defn(h, "==", 1, 1, |s, a, _| {
        let (_, m, _) = get!(hmac_ctx(s));
        match ctx_of(a[0]) {
            Some(Ctx::Hmac(_, o, _)) => bool_v(ct_eq(&o.finish(), &m.finish())),
            _ => QFALSE,
        }
    });
    fn one_shot(a: &[V], f: fn(Vec<u8>) -> V) -> V {
        let alg = get!(alg_arg(a[0]));
        let key = get!(bytes(a[1]));
        let data = get!(bytes(a[2]));
        f(hmac(alg, &key, &data))
    }
    defs(h, "digest", 3, 3, |_, a, _| one_shot(a, fmt_bin));
    defs(h, "hexdigest", 3, 3, |_, a, _| one_shot(a, fmt_hex));
    defs(h, "base64digest", 3, 3, |_, a, _| one_shot(a, fmt_b64));
}

// ---------- OpenSSL::PKCS5 / KDF ----------

fn kdf_len(v: V) -> Result<usize, V> {
    let n = int(v)?;
    if n < 0 {
        return Err(arg_error("negative string size (or size too big)"));
    }
    Ok(n as usize)
}

fn init_kdf(ossl: *mut Class) {
    let p = module_under(ossl, "PKCS5");
    const_set(p, sym("PKCS5Error"), *OSSL_ERROR.get() as V);
    defs(p, "pbkdf2_hmac", 5, 5, |_, a, _| {
        let pass = get!(bytes(a[0]));
        let salt = get!(bytes(a[1]));
        let iter = get!(int(a[2]));
        let len = get!(kdf_len(a[3]));
        let alg = get!(alg_arg(a[4]));
        bin(pbkdf2(alg, &pass, &salt, iter.max(1) as u64, len))
    });
    defs(p, "pbkdf2_hmac_sha1", 4, 4, |_, a, _| {
        let pass = get!(bytes(a[0]));
        let salt = get!(bytes(a[1]));
        let iter = get!(int(a[2]));
        let len = get!(kdf_len(a[3]));
        bin(pbkdf2(Alg::Sha1, &pass, &salt, iter.max(1) as u64, len))
    });
    let k = module_under(ossl, "KDF");
    *KDF_ERROR.get() = class_under(k, "KDFError", *OSSL_ERROR.get());
    // keyword front-ends (pbkdf2_hmac / hkdf) live in lib/kenny/openssl.rb
    defs(k, "__hkdf", 5, 5, |_, a, _| {
        let ikm = get!(bytes(a[0]));
        let salt = get!(bytes(a[1]));
        let info = get!(bytes(a[2]));
        let len = get!(kdf_len(a[3]));
        let alg = get!(alg_arg(a[4]));
        if len > 255 * alg.out_len() {
            return ossl_raise(*KDF_ERROR.get(), "EVP_PKEY_derive", None);
        }
        bin(hkdf(alg, &ikm, &salt, &info, len))
    });
}

// ---------- OpenSSL::Cipher ----------

fn cipher_err(m: &str) -> V {
    ossl_raise(*CIPHER_ERROR.get(), m, None)
}
/// A failed Cipher#final: OpenSSL's reason (`""` for a bad GCM tag), which openssl 4.0
/// prefixes with "cipher final failed".
fn cipher_final_err(m: &str) -> V {
    let entry = match m {
        "bad decrypt" => Some(BAD_DECRYPT),
        "wrong final block length" => Some(WRONG_FINAL_BLOCK),
        _ => None,
    };
    if crate::misc::ruby4() {
        ossl_raise(*CIPHER_ERROR.get(), &format!("cipher final failed: {m}"), entry)
    } else {
        cipher_err(m)
    }
}

fn init_cipher(ossl: *mut Class) {
    let c = class_under(ossl, "Cipher", rt().c.object);
    *CIPHER_CLASS.get() = c;
    *CIPHER_ERROR.get() = class_under(c, "CipherError", *OSSL_ERROR.get());
    if crate::misc::ruby4() {
        // openssl 4.0: a failed AEAD tag check (Cipher#final)
        *AUTH_TAG_ERROR.get() = class_under(c, "AuthTagError", *CIPHER_ERROR.get());
    }
    defn(c, "initialize", 1, 1, |s, a, _| {
        let n = get!(bytes(a[0]));
        match Cipher::parse(&n) {
            Some(ci) => {
                set_ctx(s, Ctx::Cipher(Box::new(ci)));
                QNIL
            }
            None if crate::misc::ruby4() => {
                let name = String::from_utf8_lossy(&n);
                let entry = unsupported_entry(&name);
                ossl_raise(*CIPHER_ERROR.get(), &format!("unsupported cipher algorithm: {name}: {}", reason(&entry)), Some(&entry))
            }
            None => raise(rt().c.runtime_error, &format!("unsupported cipher algorithm ({})", String::from_utf8_lossy(&n))),
        }
    });
    defn(c, "__kenny_copy", 1, 1, |s, a, _| {
        if let Some(x) = ctx_of(a[0]) {
            set_ctx(s, x.dup());
        }
        s
    });
    defs(c, "ciphers", 0, 0, |_, _, _| {
        let mut v = Vec::new();
        for b in [128, 192, 256] {
            for m in ["ecb", "cbc", "ctr", "gcm"] {
                v.push(str_from(&format!("aes-{b}-{m}")));
            }
        }
        ary_new(v)
    });
    fn set_dir(s: V, enc: bool) -> V {
        let ci = get!(cipher_ctx(s));
        ci.enc = enc;
        ci.run = None;
        ci.tag_out = None;
        s
    }
    defn(c, "encrypt", 0, -1, |s, _, _| set_dir(s, true));
    defn(c, "decrypt", 0, -1, |s, _, _| set_dir(s, false));
    defn(c, "reset", 0, 0, |s, _, _| {
        get!(cipher_ctx(s)).run = None;
        s
    });
    defn(c, "name", 0, 0, |s, _, _| str_from(&get!(cipher_ctx(s)).name()));
    defn(c, "key_len", 0, 0, |s, _, _| mkfix(get!(cipher_ctx(s)).key_len() as i64));
    defn(c, "iv_len", 0, 0, |s, _, _| mkfix(get!(cipher_ctx(s)).iv_len as i64));
    defn(c, "block_size", 0, 0, |s, _, _| mkfix(get!(cipher_ctx(s)).block_size() as i64));
    defn(c, "authenticated?", 0, 0, |s, _, _| bool_v(get!(cipher_ctx(s)).mode == Mode::Gcm));
    defn(c, "key_len=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        if get!(int(a[0])) as usize != ci.key_len() {
            return ossl_raise(*CIPHER_ERROR.get(), "invalid key length", Some(INVALID_KEY_LENGTH));
        }
        a[0]
    });
    defn(c, "iv_len=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        let n = get!(int(a[0]));
        if ci.mode != Mode::Gcm {
            return arg_error("cipher does not support AEAD");
        }
        if n < 1 {
            return arg_error("iv_len must be positive");
        }
        ci.iv_len = n as usize;
        ci.iv = None;
        ci.run = None;
        a[0]
    });
    defn(c, "key=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        let k = get!(bytes(a[0]));
        if k.len() != ci.key_len() {
            return arg_error(&format!("key must be {} bytes", ci.key_len()));
        }
        ci.key = Some(k);
        ci.run = None;
        a[0]
    });
    defn(c, "iv=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        let iv = get!(bytes(a[0]));
        if iv.len() != ci.iv_len {
            return arg_error(&format!("iv must be {} bytes", ci.iv_len));
        }
        ci.iv = Some(iv);
        ci.run = None;
        a[0]
    });
    defn(c, "padding=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        ci.padding = !matches!(a[0], QFALSE | QNIL) && !(is_fix(a[0]) && fix(a[0]) == 0);
        a[0]
    });
    defn(c, "random_key", 0, 0, |s, _, _| {
        let ci = get!(cipher_ctx(s));
        let k = urandom(ci.key_len());
        ci.key = Some(k.clone());
        ci.run = None;
        bin(k)
    });
    defn(c, "random_iv", 0, 0, |s, _, _| {
        let ci = get!(cipher_ctx(s));
        let iv = urandom(ci.iv_len);
        ci.iv = Some(iv.clone());
        ci.run = None;
        bin(iv)
    });
    defn(c, "auth_data=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        if ci.mode != Mode::Gcm {
            return cipher_err("AEAD not supported by this cipher");
        }
        ci.aad = get!(bytes(a[0]));
        ci.run = None;
        a[0]
    });
    defn(c, "auth_tag=", 1, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        if ci.mode != Mode::Gcm {
            return cipher_err("authentication tag not supported by this cipher");
        }
        ci.tag_in = Some(get!(bytes(a[0])));
        a[0]
    });
    defn(c, "auth_tag", 0, 1, |s, a, _| {
        let ci = get!(cipher_ctx(s));
        let n = match a.first() {
            Some(&v) => get!(int(v)),
            None => 16,
        };
        if ci.mode != Mode::Gcm {
            return cipher_err("authentication tag not supported by this cipher");
        }
        match &ci.tag_out {
            Some(t) if ci.enc && n > 0 && n <= 16 => bin(t[..n as usize].to_vec()),
            _ => ossl_raise(*CIPHER_ERROR.get(), &format!("retrieving the authentication tag failed: {}", reason(INVALID_TAG)), Some(INVALID_TAG)),
        }
    });
    defn(c, "update", 1, 2, |s, a, _| {
        // (CRuby checks for a key before the context)
        if ctx_of(s).is_none() {
            return cipher_err("key not set");
        }
        let ci = get!(cipher_ctx(s));
        let data = get!(bytes(a[0]));
        if data.is_empty() {
            return arg_error("data must not be empty");
        }
        match ci.update(&data) {
            Ok(out) => match a.get(1) {
                Some(&buf) if is_str(buf) => {
                    tri!(check_frozen(buf));
                    rstr(buf).b.set(out);
                    hdr(buf).flags |= FL_BINARY;
                    buf
                }
                _ => bin(out),
            },
            Err(m) => cipher_err(m),
        }
    });
    defn(c, "final", 0, 0, |s, _, _| {
        let ci = get!(cipher_ctx(s));
        let aead = ci.mode == Mode::Gcm;
        match ci.finish() {
            Ok(out) => bin(out),
            // (openssl 4.0 says which call failed, and has its own class for a bad AEAD tag)
            Err(_) if aead && crate::misc::ruby4() => ossl_raise(*AUTH_TAG_ERROR.get(), "AEAD authentication tag verification failed", None),
            Err(m) => cipher_final_err(m),
        }
    });
}

// (after get! and the helpers it uses: macros are in scope from their definition on)
#[path = "openssl_pkey.rs"]
mod pkey;
#[path = "openssl_tls.rs"]
mod tls;
#[path = "openssl_x509.rs"]
mod x509;

// ---------- OpenSSL module ----------

fn init_openssl() -> V {
    if let Some(m) = const_lookup(rt().c.object, sym("OpenSSL")) {
        return m;
    }
    let ossl = new_module("OpenSSL");
    set_global_const("OpenSSL", ossl as V);
    *CTX.get() = new_class("OpenSSL::__Ctx", rt().c.object);
    *OSSL_ERROR.get() = class_under(ossl, "OpenSSLError", rt().c.standard_error);
    // the openssl gem of the Ruby the program is built against
    const_set(ossl, sym("VERSION"), str_from(if crate::misc::ruby4() { "4.0.0" } else { "3.3.0" }));
    // (openssl 4.0's OpenSSLError#errors and #detailed_message are in lib/kenny/openssl.rb)
    const_set(ossl, sym("OPENSSL_VERSION"), str_from("OpenSSL 3.0.0 (Kenny native: RustCrypto)"));
    const_set(ossl, sym("OPENSSL_LIBRARY_VERSION"), str_from("OpenSSL 3.0.0 (Kenny native: RustCrypto)"));
    const_set(ossl, sym("OPENSSL_VERSION_NUMBER"), mkfix(0x30000000));
    const_set(ossl, sym("OPENSSL_FIPS"), QFALSE);
    let meta = ossl;
    defs(meta, "fips_mode", 0, 0, |_, _, _| QFALSE);
    defs(meta, "fixed_length_secure_compare", 2, 2, |_, a, _| {
        let x = get!(bytes(a[0]));
        let y = get!(bytes(a[1]));
        if x.len() != y.len() {
            return arg_error("inputs must be of equal length");
        }
        bool_v(ct_eq(&x, &y))
    });
    defs(meta, "secure_compare", 2, 2, |_, a, _| {
        let x = get!(bytes(a[0]));
        let y = get!(bytes(a[1]));
        bool_v(ct_eq(&hash(Alg::Sha256, &x), &hash(Alg::Sha256, &y)) && x == y)
    });

    let r = module_under(ossl, "Random");
    class_under(r, "RandomError", *OSSL_ERROR.get());
    fn rand_bytes(_: V, a: &[V], _: V) -> V {
        let n = get!(int(a[0]));
        if n < 0 {
            return arg_error("negative string size (or size too big)");
        }
        bin(urandom(n as usize))
    }
    defs(r, "random_bytes", 1, 1, rand_bytes);
    defs(r, "pseudo_bytes", 1, 1, rand_bytes);
    defs(r, "random_add", 2, 2, |s, _, _| s);
    defs(r, "seed", 1, 1, |_, a, _| a[0]);
    defs(r, "status?", 0, 0, |_, _, _| QTRUE);

    let ssl = module_under(ossl, "SSL");
    for (n, v) in [("VERIFY_NONE", 0), ("VERIFY_PEER", 1), ("VERIFY_FAIL_IF_NO_PEER_CERT", 2), ("VERIFY_CLIENT_ONCE", 4)] {
        const_set(ssl, sym(n), mkfix(v));
    }
    class_under(ssl, "SSLError", *OSSL_ERROR.get());

    init_digest(ossl);
    init_hmac(ossl);
    init_kdf(ossl);
    init_cipher(ossl);
    pkey::init_pkey(ossl);
    tls::init_tls(ssl);
    x509::init_x509(ossl);
    ossl as V
}

// ---------- Random::Formatter (SecureRandom) ----------

fn fmt_bytes(s: V, n: usize) -> Result<Vec<u8>, V> {
    let b = send(s, sym("bytes"), &[mkfix(n as i64)], QNIL);
    if b == UNDEF {
        return Err(UNDEF);
    }
    let v = bytes(b)?;
    if v.len() < n {
        return Err(raise(rt().c.not_implemented_error, "Unexpected partial read from random device"));
    }
    Ok(v)
}
fn opt_len(a: &[V]) -> Result<usize, V> {
    match a.first() {
        None | Some(&QNIL) => Ok(16),
        Some(&v) => {
            let n = int(v)?;
            if n < 0 {
                return Err(arg_error("negative string size (or size too big)"));
            }
            Ok(n as usize)
        }
    }
}
fn rand_u64(s: V) -> Result<u64, V> {
    let b = fmt_bytes(s, 8)?;
    Ok(u64::from_le_bytes(b[..8].try_into().unwrap()))
}
fn rand_below(s: V, n: u64) -> Result<u64, V> {
    let zone = u64::MAX - (u64::MAX % n);
    loop {
        let r = rand_u64(s)?;
        if r < zone {
            return Ok(r % n);
        }
    }
}
fn rand_unit(s: V) -> Result<f64, V> {
    Ok((rand_u64(s)? >> 11) as f64 / (1u64 << 53) as f64)
}
fn random_number(s: V, a: &[V]) -> Result<V, V> {
    let arg = a.first().copied().unwrap_or(QNIL);
    if is_fix(arg) && fix(arg) > 0 {
        return Ok(mkfix(rand_below(s, fix(arg) as u64)? as i64));
    }
    if is_float(arg) && float_val(arg) > 0.0 {
        return Ok(float_new(rand_unit(s)? * float_val(arg)));
    }
    if is_range(arg) {
        let rg = rrange(arg);
        if is_fix(rg.lo) && is_fix(rg.hi) {
            let hi = fix(rg.hi) - if rg.excl { 1 } else { 0 };
            if hi < fix(rg.lo) {
                return Ok(QNIL);
            }
            let span = (hi - fix(rg.lo)) as u64 + 1;
            return Ok(mkfix(fix(rg.lo) + rand_below(s, span)? as i64));
        }
        if let (Some(lo), Some(hi)) = (crate::fmt::num_to_f(rg.lo), crate::fmt::num_to_f(rg.hi)) {
            return Ok(float_new(lo + rand_unit(s)? * (hi - lo)));
        }
    }
    Ok(float_new(rand_unit(s)?))
}
fn uuid_fmt(b: &[u8]) -> String {
    let h = hex(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn init() {
    defn(rt().c.kernel, "__kenny_openssl_init", 0, 0, |_, _, _| init_openssl());

    let random = const_lookup(rt().c.object, sym("Random")).unwrap() as *mut Class;
    let f = module_under(random, "Formatter");
    defn(f, "random_bytes", 0, 1, |s, a, _| {
        let n = get!(opt_len(a));
        bin(get!(fmt_bytes(s, n)))
    });
    defn(f, "gen_random", 1, 1, |s, a, _| {
        let n = get!(opt_len(a));
        bin(get!(fmt_bytes(s, n)))
    });
    defn(f, "hex", 0, 1, |s, a, _| {
        let n = get!(opt_len(a));
        ascii(hex(&get!(fmt_bytes(s, n))))
    });
    defn(f, "base64", 0, 1, |s, a, _| {
        let n = get!(opt_len(a));
        ascii(base64(&get!(fmt_bytes(s, n))))
    });
    defn(f, "urlsafe_base64", 0, 2, |s, a, _| {
        let n = get!(opt_len(a));
        let pad = a.get(1).is_some_and(|v| truthy(*v));
        let mut t = base64(&get!(fmt_bytes(s, n))).replace('+', "-").replace('/', "_");
        if !pad {
            t = t.trim_end_matches('=').to_string();
        }
        ascii(t)
    });
    fn uuid4(s: V, _: &[V], _: V) -> V {
        let mut b = get!(fmt_bytes(s, 16));
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        ascii(uuid_fmt(&b))
    }
    defn(f, "uuid", 0, 0, uuid4);
    defn(f, "uuid_v4", 0, 0, uuid4);
    defn(f, "uuid_v7", 0, -1, |s, _, _| {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let mut b = get!(fmt_bytes(s, 16));
        b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
        b[6] = (b[6] & 0x0f) | 0x70;
        b[8] = (b[8] & 0x3f) | 0x80;
        ascii(uuid_fmt(&b))
    });
    defn(f, "random_number", 0, 1, |s, a, _| get!(random_number(s, a)));
    defn(f, "rand", 0, 1, |s, a, _| get!(random_number(s, a)));
    fn choose(s: V, src: V, n: usize) -> V {
        if !is_ary(src) || rary(src).v.is_empty() {
            return arg_error("chars must be a non-empty Array");
        }
        let size = rary(src).v.len() as u64;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let i = get!(rand_below(s, size)) as usize;
            let c = rary(src).v[i];
            out.extend(get!(bytes(c)));
        }
        str_new(out)
    }
    defn(f, "choose", 2, 2, |s, a, _| {
        let n = get!(int(a[1]));
        choose(s, a[0], n.max(0) as usize)
    });
    defn(f, "alphanumeric", 0, 2, |s, a, _| {
        let (pos, kw): (&[V], V) = match a.last() {
            Some(&h) if is_hash(h) => (&a[..a.len() - 1], h),
            _ => (a, QNIL),
        };
        let n = get!(opt_len(pos));
        let chars = if kw != QNIL { crate::hash::hget(kw, mksym(sym("chars"))) } else { QNIL };
        let chars = if chars == QNIL {
            let f = const_get_under(
                const_lookup(rt().c.object, sym("Random")).unwrap() as *mut Class,
                sym("Formatter"),
            )
            .unwrap() as *mut Class;
            const_get_under(f, sym("ALPHANUMERIC")).unwrap_or(QNIL)
        } else {
            chars
        };
        choose(s, chars, n)
    });
    let mut alnum = Vec::new();
    for r in [b'A'..=b'Z', b'a'..=b'z', b'0'..=b'9'] {
        alnum.extend(r.map(|c| str_new(vec![c])));
    }
    const_set(f, sym("ALPHANUMERIC"), ary_new(alnum));
}
