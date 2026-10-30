//! The arithmetic under `OpenSSL::PKey::EC` (P-256, P-384, P-521 and secp256k1, on the
//! RustCrypto curve crates) and Ed25519 keys (`ring`). Keys are plain bytes on the Ruby side:
//! a private key is its scalar (big-endian, the field's size) and a point its SEC1 octets.
//! The key formats (SEC1, PKCS#8, SPKI) and the rest of the API are Ruby, in
//! lib/kenny/kenny/openssl_ec.rb and lib/kenny/kenny/openssl_ed25519.rb.

use super::pkey::OsRng;
use super::*;
use ecdsa::signature::hazmat::{PrehashVerifier, RandomizedPrehashSigner};
use elliptic_curve::group::Group as _;
use elliptic_curve::Generate as _;
use elliptic_curve::ff::PrimeField;
use elliptic_curve::sec1::{FromSec1Point, Sec1Point, ToSec1Point};
use elliptic_curve::{AffinePoint, ProjectivePoint, Scalar, SecretKey};

/// Runs `$body` with `$C` bound to the curve named by `$name` (OpenSSL's short names);
/// `$none` when Kenny doesn't have that curve.
macro_rules! with_curve {
    ($name:expr, $C:ident => $body:expr, $none:expr) => {
        match &$name[..] {
            b"prime256v1" => {
                type $C = p256::NistP256;
                $body
            }
            b"secp384r1" => {
                type $C = p384::NistP384;
                $body
            }
            b"secp521r1" => {
                type $C = p521::NistP521;
                $body
            }
            b"secp256k1" => {
                type $C = k256::Secp256k1;
                $body
            }
            _ => $none,
        }
    };
}

/// The point SEC1 octets encode, or None when they're not a point on the curve. ("\x00"
/// is the point at infinity.)
fn point_of<C>(b: &[u8]) -> Option<ProjectivePoint<C>>
where
    C: elliptic_curve::CurveArithmetic,
    elliptic_curve::FieldBytesSize<C>: elliptic_curve::sec1::ModulusSize,
    AffinePoint<C>: FromSec1Point<C> + ToSec1Point<C>,
{
    let p = Sec1Point::<C>::from_bytes(b).ok()?;
    let a: Option<AffinePoint<C>> = AffinePoint::<C>::from_sec1_point(&p).into_option();
    a.map(Into::into)
}

fn octets<C>(p: &ProjectivePoint<C>, compressed: bool) -> Vec<u8>
where
    C: elliptic_curve::CurveArithmetic,
    elliptic_curve::FieldBytesSize<C>: elliptic_curve::sec1::ModulusSize,
    AffinePoint<C>: FromSec1Point<C> + ToSec1Point<C>,
{
    let a: AffinePoint<C> = (*p).into();
    a.to_sec1_point(compressed).as_bytes().to_vec()
}

/// A scalar from big-endian bytes no longer than the field (Ruby reduces it mod the order first).
fn scalar_of<C>(b: &[u8]) -> Option<Scalar<C>>
where
    C: elliptic_curve::CurveArithmetic,
{
    let mut fb = elliptic_curve::FieldBytes::<C>::default();
    if b.len() > fb.len() {
        return None;
    }
    let off = fb.len() - b.len();
    fb[off..].copy_from_slice(b);
    let s: Option<Scalar<C>> = PrimeField::from_repr(fb).into_option();
    s
}

fn secret_of<C>(b: &[u8]) -> Option<SecretKey<C>>
where
    C: elliptic_curve::CurveArithmetic,
{
    let mut fb = elliptic_curve::FieldBytes::<C>::default();
    if b.len() > fb.len() {
        return None;
    }
    let off = fb.len() - b.len();
    fb[off..].copy_from_slice(b);
    SecretKey::<C>::from_bytes(&fb).ok()
}

pub(super) fn init_ec(pk: *mut Class, base: *mut Class) {
    let ec = class_under(pk, "EC", base);

    // EC.__kenny_generate(curve): [private scalar, uncompressed public point]
    defs(ec, "__kenny_generate", 1, 1, |_, a, _| {
        let curve = get!(bytes(a[0]));
        with_curve!(curve, C => {
            let k = SecretKey::<C>::generate_from_rng(&mut OsRng);
            let p = octets::<C>(&k.public_key().to_projective(), false);
            ary_new(vec![bin(k.to_bytes().to_vec()), bin(p)])
        }, QNIL)
    });
    // EC.__kenny_public(curve, private scalar): its public point (uncompressed); nil when the
    // scalar isn't a private key on the curve (0, or not below the order)
    defs(ec, "__kenny_public", 2, 2, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let d = get!(bytes(a[1]));
        with_curve!(curve, C => match secret_of::<C>(&d) {
            Some(k) => bin(octets::<C>(&k.public_key().to_projective(), false)),
            None => QNIL,
        }, QNIL)
    });
    // EC.__kenny_point(curve, octets, compressed): the point re-encoded (compressed or
    // uncompressed; "\x00" for infinity), nil when the octets aren't a point on the curve
    defs(ec, "__kenny_point", 3, 3, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let b = get!(bytes(a[1]));
        let compressed = truthy(a[2]);
        with_curve!(curve, C => match point_of::<C>(&b) {
            Some(p) => bin(octets::<C>(&p, compressed)),
            None => QNIL,
        }, QNIL)
    });
    // EC.__kenny_add(curve, a, b) and EC.__kenny_mul(curve, point, scalar): uncompressed
    // octets, nil when an argument isn't a point
    defs(ec, "__kenny_add", 3, 3, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let x = get!(bytes(a[1]));
        let y = get!(bytes(a[2]));
        with_curve!(curve, C => match (point_of::<C>(&x), point_of::<C>(&y)) {
            (Some(p), Some(q)) => bin(octets::<C>(&(p + q), false)),
            _ => QNIL,
        }, QNIL)
    });
    defs(ec, "__kenny_mul", 3, 3, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let x = get!(bytes(a[1]));
        let k = get!(bytes(a[2]));
        with_curve!(curve, C => {
            let p = if x.is_empty() { Some(ProjectivePoint::<C>::generator()) } else { point_of::<C>(&x) };
            match (p, scalar_of::<C>(&k)) {
                (Some(p), Some(k)) => bin(octets::<C>(&(p * k), false)),
                _ => QNIL,
            }
        }, QNIL)
    });
    // EC.__kenny_sign(curve, private scalar, digest): an ECDSA signature (DER) of the digest,
    // with a random nonce as OpenSSL's (RFC 6979's, hedged with fresh randomness)
    defs(ec, "__kenny_sign", 3, 3, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let d = get!(bytes(a[1]));
        let z = get!(bytes(a[2]));
        with_curve!(curve, C => {
            let Some(k) = secret_of::<C>(&d) else { return arg_error("invalid private key") };
            let sk = ecdsa::SigningKey::<C>::from(k);
            let r: Result<ecdsa::Signature<C>, _> = sk.sign_prehash_with_rng(&mut OsRng, &z);
            match r {
                Ok(sig) => bin(sig.to_der().as_bytes().to_vec()),
                Err(_) => arg_error("ECDSA signing failed"),
            }
        }, QNIL)
    });
    // EC.__kenny_verify(curve, point, digest, signature DER): true or false. (A high-S
    // signature verifies, as in OpenSSL: k256 accepts only low-S ones, so it's normalized.)
    defs(ec, "__kenny_verify", 4, 4, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let pt = get!(bytes(a[1]));
        let z = get!(bytes(a[2]));
        let der = get!(bytes(a[3]));
        with_curve!(curve, C => {
            let Ok(vk) = ecdsa::VerifyingKey::<C>::from_sec1_bytes(&pt) else { return QFALSE };
            let Ok(sig) = ecdsa::Signature::<C>::from_der(&der) else { return QFALSE };
            // (OpenSSL's ECDSA_verify: the signature must be the DER of its own (r, s))
            if sig.to_der().as_bytes() != &der[..] {
                return QFALSE;
            }
            let sig = sig.normalize_s();
            bool_v(vk.verify_prehash(&z, &sig).is_ok())
        }, QNIL)
    });
    // EC.__kenny_dh(curve, private scalar, peer point): the shared secret (the x coordinate)
    defs(ec, "__kenny_dh", 3, 3, |_, a, _| {
        let curve = get!(bytes(a[0]));
        let d = get!(bytes(a[1]));
        let pt = get!(bytes(a[2]));
        with_curve!(curve, C => {
            let Some(k) = secret_of::<C>(&d) else { return arg_error("invalid private key") };
            let Ok(peer) = elliptic_curve::PublicKey::<C>::from_sec1_bytes(&pt) else { return QNIL };
            let s = elliptic_curve::ecdh::diffie_hellman(k.to_nonzero_scalar(), peer.as_affine());
            bin(s.raw_secret_bytes().to_vec())
        }, QNIL)
    });
    // Ed25519: PKey.__kenny_ed25519_public(seed) and __kenny_ed25519_sign(seed, data)
    defs(pk, "__kenny_ed25519_public", 1, 1, |_, a, _| {
        let seed = get!(bytes(a[0]));
        match ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed) {
            Ok(k) => bin(ring::signature::KeyPair::public_key(&k).as_ref().to_vec()),
            Err(_) => QNIL,
        }
    });
    defs(pk, "__kenny_ed25519_sign", 2, 2, |_, a, _| {
        let seed = get!(bytes(a[0]));
        let data = get!(bytes(a[1]));
        match ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed) {
            Ok(k) => bin(k.sign(&data).as_ref().to_vec()),
            Err(_) => arg_error("invalid Ed25519 private key"),
        }
    });
}
