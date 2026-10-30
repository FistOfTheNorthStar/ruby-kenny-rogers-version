//! `OpenSSL::PKey::RSA` on the RustCrypto `rsa` crate (constant-time bignums, blinded
//! private-key operations): key generation, PKCS#1/PKCS#8/SPKI in DER and PEM, and
//! PKCS#1 v1.5 and PSS signatures. The Ruby-level API (`initialize`'s argument forms,
//! `PKey.read`, `n`/`e`/`params` as `OpenSSL::BN`, `sign_pss`'s keywords) is in
//! lib/kenny/openssl.rb on top of the `__` methods defined here.

use super::*;
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPrivateKey};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, Pkcs1v15Sign, Pss, RsaPrivateKey, RsaPublicKey};

static PKEY_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());
static RSA_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());

#[derive(Clone)]
pub(super) enum Rsa {
    Priv(RsaPrivateKey),
    Pub(RsaPublicKey),
}
impl Rsa {
    fn public(&self) -> &RsaPublicKey {
        match self {
            Rsa::Priv(k) => k.as_public_key(),
            Rsa::Pub(k) => k,
        }
    }
}

/// The OS random source as a `rand_core` CSPRNG (key generation, blinding, PSS salts).
pub(super) struct OsRng;
impl rsa::rand_core::TryRng for OsRng {
    type Error = std::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut b = [0u8; 4];
        self.try_fill_bytes(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut b = [0u8; 8];
        self.try_fill_bytes(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn try_fill_bytes(&mut self, d: &mut [u8]) -> Result<(), Self::Error> {
        let ok = d.chunks_mut(256).all(|c| unsafe { libc::getentropy(c.as_mut_ptr() as *mut libc::c_void, c.len()) } == 0);
        if !ok {
            use std::io::Read;
            // (key material must never come from a zeroed buffer)
            std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(d)).expect("no OS random source for key generation");
        }
        Ok(())
    }
}
impl rsa::rand_core::TryCryptoRng for OsRng {}

/// Runs `$body` with `$D` bound to the digest type of `$alg`.
macro_rules! with_digest {
    ($alg:expr, $D:ident => $body:expr) => {
        match $alg {
            Alg::Md5 => {
                type $D = md5::Md5;
                $body
            }
            Alg::Sha1 => {
                type $D = sha1::Sha1;
                $body
            }
            Alg::Sha224 => {
                type $D = sha2::Sha224;
                $body
            }
            Alg::Sha256 => {
                type $D = sha2::Sha256;
                $body
            }
            Alg::Sha384 => {
                type $D = sha2::Sha384;
                $body
            }
            Alg::Sha512 => {
                type $D = sha2::Sha512;
                $body
            }
        }
    };
}

fn rsa_of(s: V) -> Result<&'static mut Rsa, V> {
    match ctx_of(s) {
        Some(Ctx::Rsa(k)) => Ok(k),
        _ => Err(uninit(s)),
    }
}
fn priv_of(s: V) -> Result<&'static RsaPrivateKey, V> {
    match rsa_of(s)? {
        Rsa::Priv(k) => Ok(k),
        Rsa::Pub(_) => Err(arg_error("private key is needed")),
    }
}
fn rsa_err(msg: &str) -> V {
    ossl_raise(*RSA_ERROR.get(), msg, None)
}
fn pkey_err(msg: &str) -> V {
    ossl_raise(*PKEY_ERROR.get(), msg, None)
}

/// The first PEM block in `s` (OpenSSL skips text before `-----BEGIN`): its label and DER.
fn pem_block(s: &[u8]) -> Option<(String, Vec<u8>)> {
    let s = std::str::from_utf8(s).ok()?;
    let start = s.find("-----BEGIN ")?;
    let rest = &s[start + 11..];
    let label_end = rest.find("-----")?;
    let label = rest[..label_end].to_string();
    let body_start = start + 11 + label_end + 5;
    let end = s[body_start..].find(&format!("-----END {label}-----"))? + body_start;
    let body = &s[body_start..end];
    if body.contains(':') {
        // (RFC 1421 headers: an encrypted traditional key, Proc-Type: 4,ENCRYPTED)
        return Some((format!("ENCRYPTED {label}"), Vec::new()));
    }
    let b64: Vec<u8> = body.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    Some((label, unbase64(&b64)?))
}
fn unbase64(s: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s: Vec<u8> = s.iter().copied().take_while(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

enum Loaded {
    Key(Rsa),
    Encrypted,
    NotRsa,
}
/// An RSA key from PEM or DER: PKCS#1, PKCS#8 or SPKI.
fn load(data: &[u8]) -> Loaded {
    if let Some((label, der)) = pem_block(data) {
        let k = match label.as_str() {
            "RSA PRIVATE KEY" => RsaPrivateKey::from_pkcs1_der(&der).ok().map(Rsa::Priv),
            "PRIVATE KEY" => RsaPrivateKey::from_pkcs8_der(&der).ok().map(Rsa::Priv),
            "PUBLIC KEY" => RsaPublicKey::from_public_key_der(&der).ok().map(Rsa::Pub),
            "RSA PUBLIC KEY" => RsaPublicKey::from_pkcs1_der(&der).ok().map(Rsa::Pub),
            l if l.starts_with("ENCRYPTED ") => return Loaded::Encrypted,
            _ => None,
        };
        return k.map_or(Loaded::NotRsa, Loaded::Key);
    }
    if let Ok(k) = RsaPrivateKey::from_pkcs1_der(data) {
        return Loaded::Key(Rsa::Priv(k));
    }
    if let Ok(k) = RsaPrivateKey::from_pkcs8_der(data) {
        return Loaded::Key(Rsa::Priv(k));
    }
    if let Ok(k) = RsaPublicKey::from_public_key_der(data) {
        return Loaded::Key(Rsa::Pub(k));
    }
    if let Ok(k) = RsaPublicKey::from_pkcs1_der(data) {
        return Loaded::Key(Rsa::Pub(k));
    }
    Loaded::NotRsa
}

/// Big-endian bytes without leading zeros (OpenSSL::BN.new(s, 2) reads them back).
fn uint_bytes(u: &BoxedUint) -> V {
    bin(u.to_be_bytes_trimmed_vartime().into_vec())
}

/// The largest PSS salt for this key and digest (OpenSSL's `:max`).
fn pss_max_salt(k: &RsaPublicKey, alg: Alg) -> usize {
    let em_len = (k.n().bits() as usize - 1).div_ceil(8);
    em_len.saturating_sub(alg.out_len() + 2)
}

/// A salt length argument: an Integer, or :digest / :max / :auto (`None` when verifying: detect it).
fn salt_arg(v: V, k: &RsaPublicKey, alg: Alg, signing: bool) -> Result<Option<usize>, V> {
    if is_sym(v) {
        return match sym_name(sym_id(v)) {
            b"digest" => Ok(Some(alg.out_len())),
            b"max" => Ok(Some(pss_max_salt(k, alg))),
            b"auto" if signing => Ok(Some(pss_max_salt(k, alg))),
            b"auto" => Ok(None),
            n => Err(arg_error(&format!("unknown salt_length: {}", String::from_utf8_lossy(n)))),
        };
    }
    let n = int(v)?;
    match n {
        -1 => Ok(Some(alg.out_len())),
        -2 | -3 if signing => Ok(Some(pss_max_salt(k, alg))),
        -2 => Ok(None),
        n if n >= 0 => Ok(Some(n as usize)),
        _ => Err(rsa_err("invalid salt length")),
    }
}

/// (returns OpenSSL::PKey and OpenSSL::PKey::PKey, for openssl_ec.rs)
pub(super) fn init_pkey(ossl: *mut Class) -> (*mut Class, *mut Class) {
    let pk = module_under(ossl, "PKey");
    *PKEY_ERROR.get() = class_under(pk, "PKeyError", *OSSL_ERROR.get());
    let base = class_under(pk, "PKey", rt().c.object);
    let rsa = class_under(pk, "RSA", base);
    // (the openssl gem 4.0 of Ruby 4.0: RSAError, DSAError, ... are other names of PKeyError)
    *RSA_ERROR.get() = if crate::misc::ruby4() {
        const_set(pk, sym("RSAError"), *PKEY_ERROR.get() as V);
        *PKEY_ERROR.get()
    } else {
        class_under(pk, "RSAError", *PKEY_ERROR.get())
    };

    defn(rsa, "__generate", 2, 2, |s, a, _| {
        let bits = get!(int(a[0]));
        let e = get!(int(a[1]));
        if !(512..=16384).contains(&bits) {
            return rsa_err(&format!("key size {bits} is not supported"));
        }
        if e < 3 || e % 2 == 0 {
            return rsa_err("bad e value");
        }
        let e = BoxedUint::from(e as u64);
        match RsaPrivateKey::new_with_exp(&mut OsRng, bits as usize, e) {
            Ok(k) => {
                set_ctx(s, Ctx::Rsa(Box::new(Rsa::Priv(k))));
                s
            }
            Err(e) => rsa_err(&e.to_string()),
        }
    });
    // true when `data` held an RSA key; false when it's not one
    defn(rsa, "__load", 1, 1, |s, a, _| {
        let d = get!(bytes(a[0]));
        match load(&d) {
            Loaded::Key(k) => {
                set_ctx(s, Ctx::Rsa(Box::new(k)));
                QTRUE
            }
            Loaded::Encrypted => pkey_err("encrypted private keys are not supported by Kenny"),
            Loaded::NotRsa => QFALSE,
        }
    });
    defn(rsa, "__copy", 1, 1, |s, a, _| {
        if let Some(c) = ctx_of(a[0]) {
            set_ctx(s, c.dup());
        }
        s
    });
    // self becomes the public half of `other`
    defn(rsa, "__public_of", 1, 1, |s, a, _| {
        let k = get!(rsa_of(a[0])).public().clone();
        set_ctx(s, Ctx::Rsa(Box::new(Rsa::Pub(k))));
        s
    });
    defn(rsa, "private?", 0, 0, |s, _, _| bool_v(matches!(rsa_of(s), Ok(Rsa::Priv(_)))));
    defn(rsa, "public?", 0, 0, |s, _, _| bool_v(rsa_of(s).is_ok()));
    defn(rsa, "__public_eq", 1, 1, |s, a, _| {
        let x = get!(rsa_of(s)).public();
        let y = get!(rsa_of(a[0])).public();
        bool_v(x.n().as_ref() == y.n().as_ref() && x.e() == y.e())
    });
    // [n, e, d, p, q, dmp1, dmq1, iqmp] as big-endian bytes (nil: not in the key)
    defn(rsa, "__parts", 0, 0, |s, _, _| {
        let k = get!(rsa_of(s));
        let p = k.public();
        let mut v = vec![uint_bytes(p.n().as_ref()), uint_bytes(p.e())];
        match k {
            Rsa::Priv(k) => {
                let pr = k.primes();
                v.push(uint_bytes(k.d()));
                v.push(pr.first().map_or(QNIL, uint_bytes));
                v.push(pr.get(1).map_or(QNIL, uint_bytes));
                v.push(k.dp().map_or(QNIL, uint_bytes));
                v.push(k.dq().map_or(QNIL, uint_bytes));
                v.push(k.qinv().map_or(QNIL, |q| uint_bytes(&q.retrieve())));
            }
            Rsa::Pub(_) => v.extend([QNIL; 6]),
        }
        ary_new(v)
    });
    // kind: 0 traditional (PKCS#1 private, or SPKI public), 1 PKCS#8 private, 2 SPKI public
    fn encode(s: V, kind: i64, pem: bool) -> V {
        let k = get!(rsa_of(s));
        let r: Result<Vec<u8>, String> = match (kind, &*k) {
            (0, Rsa::Priv(k)) if pem => k.to_pkcs1_pem(LineEnding::LF).map(|p| p.as_bytes().to_vec()).map_err(|e| e.to_string()),
            (0, Rsa::Priv(k)) => k.to_pkcs1_der().map(|d| d.as_bytes().to_vec()).map_err(|e| e.to_string()),
            (1, Rsa::Priv(k)) if pem => k.to_pkcs8_pem(LineEnding::LF).map(|p| p.as_bytes().to_vec()).map_err(|e| e.to_string()),
            (1, Rsa::Priv(k)) => k.to_pkcs8_der().map(|d| d.as_bytes().to_vec()).map_err(|e| e.to_string()),
            (1, Rsa::Pub(_)) => return arg_error("private key is needed"),
            (_, k) if pem => k.public().to_public_key_pem(LineEnding::LF).map(String::into_bytes).map_err(|e| e.to_string()),
            (_, k) => k.public().to_public_key_der().map(|d| d.as_bytes().to_vec()).map_err(|e| e.to_string()),
        };
        match r {
            Ok(b) if pem => ascii(String::from_utf8(b).unwrap_or_default()),
            Ok(b) => bin(b),
            Err(e) => rsa_err(&e),
        }
    }
    defn(rsa, "__pem", 1, 1, |s, a, _| encode(s, get!(int(a[0])), true));
    defn(rsa, "__der", 1, 1, |s, a, _| encode(s, get!(int(a[0])), false));

    // PKCS#1 v1.5 signatures
    defn(rsa, "__sign", 2, 2, |s, a, _| {
        let alg = get!(alg_arg(a[0]));
        let data = get!(bytes(a[1]));
        let k = get!(priv_of(s));
        let r = with_digest!(alg, D => k.sign_with_rng(&mut OsRng, Pkcs1v15Sign::new::<D>(), &hash(alg, &data)));
        match r {
            Ok(sig) => bin(sig),
            Err(e) => pkey_err(&e.to_string()),
        }
    });
    defn(rsa, "__verify", 3, 3, |s, a, _| {
        let alg = get!(alg_arg(a[0]));
        let sig = get!(bytes(a[1]));
        let data = get!(bytes(a[2]));
        let k = get!(rsa_of(s)).public();
        bool_v(with_digest!(alg, D => k.verify(Pkcs1v15Sign::new::<D>(), &hash(alg, &data), &sig)).is_ok())
    });
    // PSS signatures (MGF1 with the same digest)
    defn(rsa, "__sign_pss", 3, 3, |s, a, _| {
        let alg = get!(alg_arg(a[0]));
        let data = get!(bytes(a[1]));
        let k = get!(priv_of(s));
        let salt = get!(salt_arg(a[2], k.as_public_key(), alg, true)).unwrap_or(alg.out_len());
        if salt > pss_max_salt(k.as_public_key(), alg) {
            return rsa_err("data too large for key size");
        }
        let r = with_digest!(alg, D => k.sign_with_rng(&mut OsRng, Pss::<D>::new_with_salt(salt), &hash(alg, &data)));
        match r {
            Ok(sig) => bin(sig),
            Err(e) => rsa_err(&e.to_string()),
        }
    });
    defn(rsa, "__verify_pss", 4, 4, |s, a, _| {
        let alg = get!(alg_arg(a[0]));
        let sig = get!(bytes(a[1]));
        let data = get!(bytes(a[2]));
        let k = get!(rsa_of(s)).public();
        let salt = get!(salt_arg(a[3], k, alg, false));
        let ok = with_digest!(alg, D => {
            let mut p = Pss::<D>::new();
            p.salt_len = salt;
            k.verify(p, &hash(alg, &data), &sig)
        });
        bool_v(ok.is_ok())
    });
    (pk, base)
}
