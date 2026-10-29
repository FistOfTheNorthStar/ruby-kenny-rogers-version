//! Signature checks for `OpenSSL::X509` (certificates, CSRs, CRLs) and the public-only
//! `OpenSSL::PKey::EC` keys certificates carry, with `ring`: ECDSA on P-256/P-384 and
//! Ed25519. (RSA signatures go through `OpenSSL::PKey::RSA#verify`, openssl_pkey.rs.)
//! The certificate parsing and the verification logic are Ruby, in
//! lib/kenny/kenny/openssl_x509*.rb.

use super::*;
use ring::signature;

pub(super) fn init_x509(ossl: *mut Class) {
    let x509 = module_under(ossl, "X509");
    // X509.__kenny_verify(alg, public key bytes, data, signature): true or false, nil when
    // Kenny can't check that algorithm. alg names the key and digest: "ECDSA_P256_SHA256",
    // "ECDSA_P384_SHA384", ..., "ED25519"; the key is the subjectPublicKey's bits (an
    // uncompressed EC point, or the raw Ed25519 key) and an ECDSA signature is DER.
    defs(x509, "__kenny_verify", 4, 4, |_, a, _| {
        let alg = get!(bytes(a[0]));
        let key = get!(bytes(a[1]));
        let data = get!(bytes(a[2]));
        let sig = get!(bytes(a[3]));
        let algo: &'static dyn signature::VerificationAlgorithm = match &alg[..] {
            b"ECDSA_P256_SHA256" => &signature::ECDSA_P256_SHA256_ASN1,
            b"ECDSA_P256_SHA384" => &signature::ECDSA_P256_SHA384_ASN1,
            b"ECDSA_P384_SHA256" => &signature::ECDSA_P384_SHA256_ASN1,
            b"ECDSA_P384_SHA384" => &signature::ECDSA_P384_SHA384_ASN1,
            b"ED25519" => &signature::ED25519,
            _ => return QNIL,
        };
        bool_v(signature::UnparsedPublicKey::new(algo, &key).verify(&data, &sig).is_ok())
    });
}
