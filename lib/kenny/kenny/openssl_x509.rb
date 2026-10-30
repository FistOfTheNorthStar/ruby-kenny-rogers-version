# frozen_string_literal: true
# Kenny: OpenSSL::X509, what the openssl gem's C part provides (ossl_x509cert.c, ossl_x509name.c,
# ossl_x509ext.c, ossl_x509store.c, ossl_x509req.c, ossl_x509crl.c, ...), in Ruby on top of
# kenny/openssl_asn1.rb: certificates, names, extensions (with OpenSSL's printed forms),
# ExtensionFactory's config syntax for the common extensions, CSRs, CRLs, and a Store whose
# verification follows OpenSSL 3.6's x509_vfy.c (chain building, trust, the verify callback,
# its error codes and strings). Signatures are checked natively (OpenSSL::X509.__kenny_verify,
# crates/kenny-runtime/src/openssl_x509.rs). The stdlib's openssl/x509.rb is loaded on top, as
# in CRuby.

module Kenny
  module X509Util
    module_function

    def pem(label, der)
      +"-----BEGIN #{label}-----\n" << [der].pack("m0").scan(/.{1,64}/).join("\n") << "\n-----END #{label}-----\n"
    end

    # the PEM blocks in s: [[label, der], ...]
    def pem_blocks(s)
      out = []
      s = s.b
      pos = 0
      while (i = s.index("-----BEGIN ", pos))
        j = s.index("-----", i + 11) or break
        label = s.byteslice(i + 11, j - i - 11)
        body_start = j + 5
        k = s.index("-----END #{label}-----", body_start) or break
        body = s.byteslice(body_start, k - body_start)
        out << [label, body.include?(":") ? nil : body.unpack1("m")]
        pos = k + 14 + label.bytesize
      end
      out
    end

    def input(arg)
      arg = arg.read if arg.is_a?(IO)
      arg = arg.to_der if !arg.is_a?(String) && arg.respond_to?(:to_der)
      s = String.try_convert(arg) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(arg)} into String"
      s.b
    end

    def hex_colon(b, lower = false)
      h = b.unpack1("H*")
      h = h.upcase unless lower
      h.scan(/../).join(":")
    end

    # "Jan  1 00:00:00 2020 GMT" (ASN1_TIME_print)
    def time_text(t)
      return "Bad time value" unless t
      t.utc.strftime("%b %e %H:%M:%S %Y GMT")
    end

    # OpenSSL's ossl_ipaddress_to_asc
    def ip_text(b)
      case b.bytesize
      when 4 then b.bytes.join(".")
      when 16 then b.unpack("n8").map { |x| x.to_s(16).upcase }.join(":")
      else "<invalid length=#{b.bytesize}>"
      end
    end

    # a2i_IPADDRESS: the bytes of a dotted IPv4 or an IPv6 address, nil when invalid
    def ip_bytes(s)
      if s.include?(":")
        return nil unless s.match?(/\A[0-9a-fA-F:.]+\z/)
        head, tail = s.split("::", -1)
        return nil if s.scan("::").size > 1
        parse = ->(part) {
          return [] if part.nil? || part.empty?
          groups = part.split(":", -1)
          words = []
          groups.each_with_index do |g, i|
            if g.include?(".") && i == groups.size - 1
              v4 = ip_bytes(g) or return nil
              words.concat(v4.unpack("n2"))
            else
              return nil unless g.match?(/\A\h{1,4}\z/)
              words << g.to_i(16)
            end
          end
          words
        }
        a = parse.(head) or return nil
        if tail.nil?
          return nil unless a.size == 8
          a.pack("n8")
        else
          b = parse.(tail) or return nil
          return nil if a.size + b.size > 7
          (a + [0] * (8 - a.size - b.size) + b).pack("n8")
        end
      else
        parts = s.split(".", -1)
        return nil unless parts.size == 4 && parts.all? { |x| x.match?(/\A\d{1,3}\z/) && x.to_i < 256 }
        parts.map(&:to_i).pack("C4")
      end
    end

    def ln(oid) = Kenny::OID.ln(oid) || oid
    def sn(oid) = Kenny::OID.sn(oid) || oid

    # (signature algorithm OID => [digest name, key type OID]) as OBJ_find_sigid_algs
    SIG_ALGS = {
      "1.2.840.113549.1.1.4" => ["MD5", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.5" => ["SHA1", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.14" => ["SHA224", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.11" => ["SHA256", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.12" => ["SHA384", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.13" => ["SHA512", "1.2.840.113549.1.1.1"],
      "1.2.840.113549.1.1.10" => [nil, "1.2.840.113549.1.1.10"],
      "1.2.840.10045.4.1" => ["SHA1", "1.2.840.10045.2.1"],
      "1.2.840.10045.4.3.1" => ["SHA224", "1.2.840.10045.2.1"],
      "1.2.840.10045.4.3.2" => ["SHA256", "1.2.840.10045.2.1"],
      "1.2.840.10045.4.3.3" => ["SHA384", "1.2.840.10045.2.1"],
      "1.2.840.10045.4.3.4" => ["SHA512", "1.2.840.10045.2.1"],
      "1.3.101.112" => [nil, "1.3.101.112"],
      "1.3.101.113" => [nil, "1.3.101.113"],
      "1.2.840.10040.4.3" => ["SHA1", "1.2.840.10040.4.1"],
      "2.16.840.1.101.3.4.3.2" => ["SHA256", "1.2.840.10040.4.1"],
    }.freeze

    RSA_SIG = { "MD5" => "1.2.840.113549.1.1.4", "SHA1" => "1.2.840.113549.1.1.5", "SHA224" => "1.2.840.113549.1.1.14",
                "SHA256" => "1.2.840.113549.1.1.11", "SHA384" => "1.2.840.113549.1.1.12", "SHA512" => "1.2.840.113549.1.1.13" }.freeze

    def digest_name(d)
      n = d.is_a?(OpenSSL::Digest) ? d.name : d.is_a?(Class) ? d.new.name : d.to_s
      n = n.upcase.delete("-")
      n = "SHA1" if n == "SHA"
      unless RSA_SIG.key?(n)
        # (OpenSSL::Digest's error for a name it doesn't know)
        OpenSSL::Digest.new(d.to_s)
        raise NotImplementedError, "#{d} signatures are not supported by Kenny"
      end
      n
    end

    # [algorithm OID, parameters TLV or nil] of an AlgorithmIdentifier
    def alg_id(tlv)
      _, c = Kenny::DER.unwrap(tlv)
      kids = Kenny::DER.children(c)
      oid = Kenny::DER.oid_dotted(kids[0][1])
      [oid, kids[1] && kids[1][2]]
    end

    def alg_tlv(oid, null_params = true)
      Kenny::DER.seq(Kenny::DER.tlv(6, Kenny::DER.oid_content(oid)), null_params ? "\x05\x00".b : "".b)
    end

    # [algorithm OID, parameters TLV, subjectPublicKey bits] of a SubjectPublicKeyInfo
    def spki_parts(spki)
      _, c = Kenny::DER.unwrap(spki)
      kids = Kenny::DER.children(c)
      oid, params = alg_id(kids[0][2])
      [oid, params, kids[1][1].byteslice(1..)]
    end

    # the PKey object of a SubjectPublicKeyInfo (X509_get_pubkey)
    def pkey(spki, err)
      oid, = spki_parts(spki)
      case oid
      when "1.2.840.113549.1.1.1" then OpenSSL::PKey::RSA.new(spki)
      when "1.2.840.10045.2.1" then OpenSSL::PKey::EC.new(spki)
      when "1.3.101.112" then OpenSSL::PKey.read(spki)
      else raise NotImplementedError, "#{Kenny::OID.ln(oid.to_s) || oid} public keys are not supported by Kenny (RSA, EC and Ed25519 keys are)"
      end
    rescue OpenSSL::PKey::PKeyError, OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError
      raise err, "decode error"
    end

    def key_spki(key)
      raise TypeError, "wrong argument type #{Kenny::DER.class_name(key)} (expected OpenSSL/EVP_PKEY)" unless key.is_a?(OpenSSL::PKey::PKey)
      key.public_to_der.b
    end

    # signs tbs with key; [signature algorithm TLV, signature]
    def sign(key, digest, tbs_builder)
      raise TypeError, "wrong argument type #{Kenny::DER.class_name(key)} (expected OpenSSL/EVP_PKEY)" unless key.is_a?(OpenSSL::PKey::PKey)
      if key.is_a?(OpenSSL::PKey::EC)
        raise ArgumentError, "private key is needed" unless key.private?
        d = digest_name(digest)
        alg = alg_tlv(EC_SIG.fetch(d) { raise NotImplementedError, "ECDSA with #{d} is not supported by Kenny" }, false)
        tbs = tbs_builder.call(alg)
        return [alg, tbs, key.sign(d, tbs)]
      end
      if key.instance_of?(OpenSSL::PKey::PKey) && key.oid == "ED25519"
        alg = alg_tlv("1.3.101.112", false)
        tbs = tbs_builder.call(alg)
        return [alg, tbs, key.sign(nil, tbs)]
      end
      unless key.is_a?(OpenSSL::PKey::RSA)
        raise NotImplementedError, "signing with #{key.class} keys is not supported by Kenny (RSA, EC and Ed25519 keys are)"
      end
      raise ArgumentError, "private key is needed" unless key.private?
      d = digest_name(digest)
      alg = alg_tlv(RSA_SIG.fetch(d))
      tbs = tbs_builder.call(alg)
      [alg, tbs, key.sign(d, tbs)]
    end

    EC_SIG = { "SHA1" => "1.2.840.10045.4.1", "SHA224" => "1.2.840.10045.4.3.1", "SHA256" => "1.2.840.10045.4.3.2",
               "SHA384" => "1.2.840.10045.4.3.3", "SHA512" => "1.2.840.10045.4.3.4" }.freeze

    EC_CURVES = { "1.2.840.10045.3.1.7" => "P256", "1.3.132.0.34" => "P384" }.freeze

    # X509_verify and friends: whether sig is a signature over data by the key of spki, raising
    # err when the algorithm isn't known or doesn't fit the key
    def verify(spki, sig_alg_tlv, data, sig, err)
      oid, params = alg_id(sig_alg_tlv)
      digest, key_type = SIG_ALGS[oid] || raise(err, "unknown signature algorithm")
      key_oid, key_params, bits = spki_parts(spki)
      ok_type = key_oid == key_type || (key_type == "1.2.840.113549.1.1.10" && key_oid == "1.2.840.113549.1.1.1")
      raise err, "wrong public key type" unless ok_type
      case key_type
      when "1.2.840.113549.1.1.1"
        key = OpenSSL::PKey::RSA.new(spki)
        key.verify(digest, sig, data)
      when "1.2.840.113549.1.1.10"
        key = OpenSSL::PKey::RSA.new(Kenny::DER.seq(alg_tlv("1.2.840.113549.1.1.1"), Kenny::DER.tlv(3, "\x00".b + bits)))
        hash, mgf, salt = pss_params(params)
        key.verify_pss(hash, sig, data, salt_length: salt, mgf1_hash: mgf)
      when "1.2.840.10045.2.1"
        curve = key_params && EC_CURVES[Kenny::DER.oid_dotted(Kenny::DER.unwrap(key_params)[1])]
        r = curve && OpenSSL::X509.__kenny_verify("ECDSA_#{curve}_#{digest}", bits, data, sig)
        if r.nil?
          # (P-521, secp256k1, or a digest ring doesn't pair with the curve)
          key = OpenSSL::PKey::EC.new(spki)
          r = begin
            key.verify(digest, sig, data)
          rescue OpenSSL::PKey::PKeyError
            false
          end
        end
        r
      when "1.3.101.112"
        OpenSSL::X509.__kenny_verify("ED25519", bits, data, sig)
      else
        raise NotImplementedError, "#{Kenny::OID.ln(oid) || oid} signatures are not supported by Kenny"
      end
    rescue OpenSSL::PKey::PKeyError, OpenSSL::ASN1::ASN1Error
      raise err, "decode error"
    end

    HASH_OIDS = { "1.3.14.3.2.26" => "SHA1", "2.16.840.1.101.3.4.2.4" => "SHA224", "2.16.840.1.101.3.4.2.1" => "SHA256",
                  "2.16.840.1.101.3.4.2.2" => "SHA384", "2.16.840.1.101.3.4.2.3" => "SHA512" }.freeze

    # RSASSA-PSS-params: [hash, MGF1 hash, salt length] (defaults SHA-1 and 20)
    def pss_params(params)
      hash = "SHA1"
      mgf = "SHA1"
      salt = 20
      if params && params.getbyte(0) == 0x30
        Kenny::DER.children(Kenny::DER.unwrap(params)[1]).each do |b, c|
          case b
          when 0xa0 then hash = HASH_OIDS.fetch(alg_id(Kenny::DER.children(c)[0][2])[0])
          when 0xa1
            _, mp = alg_id(Kenny::DER.children(c)[0][2])
            mgf = HASH_OIDS.fetch(alg_id(mp)[0])
          when 0xa2 then salt = Kenny::DER.int_value(Kenny::DER.children(c)[0][1])
          end
        end
      end
      [hash, mgf, salt]
    end
  end
end

module OpenSSL
  module X509
    class CertificateError < OpenSSLError; end
    class CRLError < OpenSSLError; end
    class StoreError < OpenSSLError; end
    class NameError < OpenSSLError; end
    class ExtensionError < OpenSSLError; end
    class RequestError < OpenSSLError; end
    class AttributeError < OpenSSLError; end
    class RevokedError < OpenSSLError; end

    # where the CA certificates are (OpenSSL's compiled-in defaults name OPENSSLDIR; Kenny
    # names the system's: macOS and Alpine, Debian/Ubuntu, RHEL/Fedora, SUSE)
    DEFAULT_CERT_FILES = ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt",
                          "/etc/ssl/ca-bundle.pem", "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
                          "/usr/local/etc/openssl/cert.pem", "/opt/homebrew/etc/openssl@3/cert.pem"].freeze
    private_constant :DEFAULT_CERT_FILES
    DEFAULT_CERT_AREA = "/etc/ssl"
    DEFAULT_CERT_FILE = DEFAULT_CERT_FILES.find { |f| File.file?(f) } || "/etc/ssl/cert.pem"
    DEFAULT_CERT_DIR = "/etc/ssl/certs"
    DEFAULT_PRIVATE_DIR = "/etc/ssl/private"
    DEFAULT_CERT_FILE_ENV = "SSL_CERT_FILE"
    DEFAULT_CERT_DIR_ENV = "SSL_CERT_DIR"

    {
      V_OK: 0, V_ERR_UNSPECIFIED: 1, V_ERR_UNABLE_TO_GET_ISSUER_CERT: 2, V_ERR_UNABLE_TO_GET_CRL: 3,
      V_ERR_UNABLE_TO_DECRYPT_CERT_SIGNATURE: 4, V_ERR_UNABLE_TO_DECRYPT_CRL_SIGNATURE: 5,
      V_ERR_UNABLE_TO_DECODE_ISSUER_PUBLIC_KEY: 6, V_ERR_CERT_SIGNATURE_FAILURE: 7, V_ERR_CRL_SIGNATURE_FAILURE: 8,
      V_ERR_CERT_NOT_YET_VALID: 9, V_ERR_CERT_HAS_EXPIRED: 10, V_ERR_CRL_NOT_YET_VALID: 11, V_ERR_CRL_HAS_EXPIRED: 12,
      V_ERR_ERROR_IN_CERT_NOT_BEFORE_FIELD: 13, V_ERR_ERROR_IN_CERT_NOT_AFTER_FIELD: 14,
      V_ERR_ERROR_IN_CRL_LAST_UPDATE_FIELD: 15, V_ERR_ERROR_IN_CRL_NEXT_UPDATE_FIELD: 16, V_ERR_OUT_OF_MEM: 17,
      V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT: 18, V_ERR_SELF_SIGNED_CERT_IN_CHAIN: 19,
      V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY: 20, V_ERR_UNABLE_TO_VERIFY_LEAF_SIGNATURE: 21,
      V_ERR_CERT_CHAIN_TOO_LONG: 22, V_ERR_CERT_REVOKED: 23, V_ERR_PATH_LENGTH_EXCEEDED: 25, V_ERR_INVALID_PURPOSE: 26,
      V_ERR_CERT_UNTRUSTED: 27, V_ERR_CERT_REJECTED: 28, V_ERR_SUBJECT_ISSUER_MISMATCH: 29, V_ERR_AKID_SKID_MISMATCH: 30,
      V_ERR_AKID_ISSUER_SERIAL_MISMATCH: 31, V_ERR_KEYUSAGE_NO_CERTSIGN: 32, V_ERR_UNABLE_TO_GET_CRL_ISSUER: 33,
      V_ERR_UNHANDLED_CRITICAL_EXTENSION: 34, V_ERR_KEYUSAGE_NO_CRL_SIGN: 35, V_ERR_UNHANDLED_CRITICAL_CRL_EXTENSION: 36,
      V_ERR_INVALID_NON_CA: 37, V_ERR_PROXY_PATH_LENGTH_EXCEEDED: 38, V_ERR_KEYUSAGE_NO_DIGITAL_SIGNATURE: 39,
      V_ERR_PROXY_CERTIFICATES_NOT_ALLOWED: 40, V_ERR_INVALID_EXTENSION: 41, V_ERR_INVALID_POLICY_EXTENSION: 42,
      V_ERR_NO_EXPLICIT_POLICY: 43, V_ERR_DIFFERENT_CRL_SCOPE: 44, V_ERR_UNSUPPORTED_EXTENSION_FEATURE: 45,
      V_ERR_UNNESTED_RESOURCE: 46, V_ERR_PERMITTED_VIOLATION: 47, V_ERR_EXCLUDED_VIOLATION: 48, V_ERR_SUBTREE_MINMAX: 49,
      V_ERR_APPLICATION_VERIFICATION: 50, V_ERR_UNSUPPORTED_CONSTRAINT_TYPE: 51,
      V_ERR_UNSUPPORTED_CONSTRAINT_SYNTAX: 52, V_ERR_UNSUPPORTED_NAME_SYNTAX: 53, V_ERR_CRL_PATH_VALIDATION_ERROR: 54,
      V_ERR_PATH_LOOP: 55, V_ERR_SUITE_B_INVALID_VERSION: 56, V_ERR_SUITE_B_INVALID_ALGORITHM: 57,
      V_ERR_SUITE_B_INVALID_CURVE: 58, V_ERR_SUITE_B_INVALID_SIGNATURE_ALGORITHM: 59, V_ERR_SUITE_B_LOS_NOT_ALLOWED: 60,
      V_ERR_SUITE_B_CANNOT_SIGN_P_384_WITH_P_256: 61, V_ERR_HOSTNAME_MISMATCH: 62, V_ERR_EMAIL_MISMATCH: 63,
      V_ERR_IP_ADDRESS_MISMATCH: 64, V_ERR_DANE_NO_MATCH: 65, V_ERR_EE_KEY_TOO_SMALL: 66, V_ERR_CA_KEY_TOO_SMALL: 67,
      V_ERR_CA_MD_TOO_WEAK: 68, V_ERR_INVALID_CALL: 69, V_ERR_STORE_LOOKUP: 70, V_ERR_NO_VALID_SCTS: 71,
      V_ERR_PROXY_SUBJECT_NAME_VIOLATION: 72, V_ERR_OCSP_VERIFY_NEEDED: 73, V_ERR_OCSP_VERIFY_FAILED: 74,
      V_ERR_OCSP_CERT_UNKNOWN: 75, V_ERR_INVALID_CA: 79,
      V_FLAG_USE_CHECK_TIME: 0x2, V_FLAG_CRL_CHECK: 0x4, V_FLAG_CRL_CHECK_ALL: 0x8, V_FLAG_IGNORE_CRITICAL: 0x10,
      V_FLAG_X509_STRICT: 0x20, V_FLAG_ALLOW_PROXY_CERTS: 0x40, V_FLAG_POLICY_CHECK: 0x80, V_FLAG_EXPLICIT_POLICY: 0x100,
      V_FLAG_INHIBIT_ANY: 0x200, V_FLAG_INHIBIT_MAP: 0x400, V_FLAG_NOTIFY_POLICY: 0x800,
      V_FLAG_EXTENDED_CRL_SUPPORT: 0x1000, V_FLAG_USE_DELTAS: 0x2000, V_FLAG_CHECK_SS_SIGNATURE: 0x4000,
      V_FLAG_TRUSTED_FIRST: 0x8000, V_FLAG_SUITEB_128_LOS_ONLY: 0x10000, V_FLAG_SUITEB_192_LOS: 0x20000,
      V_FLAG_SUITEB_128_LOS: 0x30000, V_FLAG_PARTIAL_CHAIN: 0x80000, V_FLAG_NO_ALT_CHAINS: 0x100000,
      V_FLAG_NO_CHECK_TIME: 0x200000,
      PURPOSE_SSL_CLIENT: 1, PURPOSE_SSL_SERVER: 2, PURPOSE_NS_SSL_SERVER: 3, PURPOSE_SMIME_SIGN: 4,
      PURPOSE_SMIME_ENCRYPT: 5, PURPOSE_CRL_SIGN: 6, PURPOSE_ANY: 7, PURPOSE_OCSP_HELPER: 8, PURPOSE_TIMESTAMP_SIGN: 9,
      TRUST_COMPAT: 1, TRUST_SSL_CLIENT: 2, TRUST_SSL_SERVER: 3, TRUST_EMAIL: 4, TRUST_OBJECT_SIGN: 5,
      TRUST_OCSP_SIGN: 6, TRUST_OCSP_REQUEST: 7, TRUST_TSA: 8
    }.each { |k, v| const_set(k, v) }

    # X509_verify_cert_error_string (OpenSSL 3.6's wording)
    VERIFY_ERROR_STRINGS = [
      "ok", "unspecified certificate verification error", "unable to get issuer certificate",
      "unable to get certificate CRL", "unable to decrypt certificate's signature", "unable to decrypt CRL's signature",
      "unable to decode issuer public key", "certificate signature failure", "CRL signature failure",
      "certificate is not yet valid or the system clock is incorrect", "certificate has expired", "CRL is not yet valid",
      "CRL has expired", "format error in certificate's notBefore field", "format error in certificate's notAfter field",
      "format error in CRL's lastUpdate field", "format error in CRL's nextUpdate field", "out of memory",
      "self-signed certificate", "self-signed certificate in certificate chain", "unable to get local issuer certificate",
      "unable to verify the first certificate", "certificate chain too long", "certificate revoked",
      "issuer certificate doesn't have a public key", "path length constraint exceeded", "unsuitable certificate purpose",
      "certificate not trusted", "certificate rejected", "subject issuer mismatch",
      "authority and subject key identifier mismatch", "authority and issuer serial number mismatch",
      "key usage does not include certificate signing", "unable to get CRL issuer certificate",
      "unhandled critical extension", "key usage does not include CRL signing", "unhandled critical CRL extension",
      "invalid non-CA certificate (has CA markings)", "proxy path length constraint exceeded",
      "key usage does not include digital signature", "proxy certificates not allowed, please set the appropriate flag",
      "invalid or inconsistent certificate extension", "invalid or inconsistent certificate policy extension",
      "no explicit policy", "different CRL scope", "unsupported extension feature",
      "RFC 3779 resource not subset of parent's resources", "permitted subtree violation", "excluded subtree violation",
      "name constraints minimum and maximum not supported", "application verification failure",
      "unsupported name constraint type", "unsupported or invalid name constraint syntax",
      "unsupported or invalid name syntax", "CRL path validation error", "path loop",
      "Suite B: certificate version invalid", "Suite B: invalid public key algorithm", "Suite B: invalid ECC curve",
      "Suite B: invalid signature algorithm", "Suite B: curve not allowed for this LOS",
      "Suite B: cannot sign P-384 with P-256", "hostname mismatch", "email address mismatch", "IP address mismatch",
      "no matching DANE TLSA records", "EE certificate key too weak", "CA certificate key too weak",
      "CA signature digest algorithm too weak", "invalid certificate verification context",
      "issuer certificate lookup error", "Certificate Transparency required, but no valid SCTs found",
      "proxy subject name violation", "OCSP verification needed", "OCSP verification failed", "OCSP unknown cert",
      "Cannot find certificate signature algorithm", "subject signature algorithm and issuer public key algorithm mismatch",
      "cert info signature and signature algorithm mismatch", "invalid CA certificate",
      "Path length invalid for non-CA cert", "Path length given without key usage keyCertSign",
      "Key usage keyCertSign invalid for non-CA cert", "Issuer name empty", "Subject name empty",
      "Missing Authority Key Identifier", "Missing Subject Key Identifier", "Empty Subject Alternative Name extension",
      "Subject empty and Subject Alt Name extension not critical", "Basic Constraints of CA cert not marked critical",
      "Authority Key Identifier marked critical", "Subject Key Identifier marked critical",
      "CA cert does not include key usage extension", "Using cert extension requires at least X509v3",
      "Certificate public key has explicit ECC parameters", "Raw public key untrusted, no trusted keys configured",
      "OCSP response(s) invalid", "OCSP response signature verification failure",
      "OCSP response not yet valid (contains a date in the future)", "OCSP response has expired",
      "no OCSP response available for certificate"
    ].freeze
    private_constant :VERIFY_ERROR_STRINGS

    def self.__kenny_error_string(code)
      (code >= 0 && VERIFY_ERROR_STRINGS[code]) || "unknown certificate verification error"
    end
  end
end

require "kenny/openssl_x509_name"
require "kenny/openssl_x509_ext"
require "kenny/openssl_x509_cert"
require "kenny/openssl_x509_store"
require "kenny/openssl_ec"
require "kenny/openssl_ed25519"
