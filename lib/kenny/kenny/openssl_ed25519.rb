# frozen_string_literal: true
# Kenny: Ed25519 keys, which OpenSSL gives as plain OpenSSL::PKey::PKey objects (ossl_pkey.c's
# generic API: sign/verify with a nil digest, raw_private_key/raw_public_key, PKCS#8 and SPKI),
# signing natively with ring (crates/kenny-runtime/src/openssl_ec.rs). And the module
# functions that pick a key type: PKey.read, PKey.generate_key, PKey.new_raw_private_key, ...
# X25519, Ed448 and X448 raise NotImplementedError.

module OpenSSL
  module PKey
    class PKey
      def self.new(*args, &blk)
        raise TypeError, "OpenSSL::PKey::PKey can't be instantiated directly" if equal?(PKey)
        super
      end

      def self.__kenny_ed25519(seed, pub)
        k = allocate
        k.instance_variable_set(:@ed_seed, seed&.b&.freeze)
        k.instance_variable_set(:@ed_pub, (pub || OpenSSL::PKey.__kenny_ed25519_public(seed)).b.freeze)
        k
      end

      def oid
        __kenny_ed!
        "ED25519"
      end

      def inspect
        return super unless @ed_pub
        "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} oid=ED25519 type_name=ED25519 provider=default>"
      end

      def raw_private_key
        __kenny_ed!
        raise PKeyError, "EVP_PKEY_get_raw_private_key" unless @ed_seed
        @ed_seed.dup
      end

      def raw_public_key
        __kenny_ed!
        @ed_pub.dup
      end

      def private_to_der(cipher = nil, pass = nil)
        __kenny_ed!
        raise NotImplementedError, "exporting encrypted keys is not supported by Kenny" if cipher
        raise PKeyError, "i2d_PKCS8PrivateKey_bio: ASN1 lib" unless @ed_seed
        D.seq(D.int(0), D.seq(Kenny::PKeyDER.oid(Kenny::PKeyDER::ED25519_OID)), D.tlv(4, D.tlv(4, @ed_seed)))
      end

      def private_to_pem(cipher = nil, pass = nil)
        Kenny::X509Util.pem("PRIVATE KEY", private_to_der(cipher, pass))
      end

      def public_to_der
        __kenny_ed!
        D.seq(D.seq(Kenny::PKeyDER.oid(Kenny::PKeyDER::ED25519_OID)), Kenny::PKeyDER.bits(@ed_pub))
      end

      def public_to_pem = Kenny::X509Util.pem("PUBLIC KEY", public_to_der)

      def sign(digest, data, options = nil)
        __kenny_ed!
        __kenny_no_digest(digest, "EVP_DigestSignInit")
        raise PKeyError, "EVP_DigestSign: not a private key" unless @ed_seed
        OpenSSL::PKey.__kenny_ed25519_sign(@ed_seed, data.to_str)
      end

      def verify(digest, signature, data, options = nil)
        __kenny_ed!
        __kenny_no_digest(digest, "EVP_DigestVerifyInit")
        OpenSSL::X509.__kenny_verify("ED25519", @ed_pub, data.to_str, signature.to_str)
      end

      def sign_raw(*)
        __kenny_ed!
        raise PKeyError, "EVP_PKEY_sign: provider signature failure (ED25519 sign:)"
      end

      def verify_raw(*)
        __kenny_ed!
        raise PKeyError, "EVP_PKEY_verify: provider signature failure (ED25519 verify:)"
      end

      def derive(_peer)
        __kenny_ed!
        raise PKeyError, "EVP_PKEY_derive_init: operation not supported for this keytype"
      end

      def compare?(other)
        __kenny_ed!
        raise TypeError, "cannot compare different PKey types" unless other.is_a?(PKey) && other.instance_variable_get(:@ed_pub)
        @ed_pub == other.raw_public_key
      end

      def to_text
        __kenny_ed!
        b = Kenny::X509Util
        return "ED25519 Public-Key:\npub:\n" + b.buf_print(@ed_pub, 4) unless @ed_seed
        "ED25519 Private-Key:\npriv:\n" + b.buf_print(@ed_seed, 4) + "pub:\n" + b.buf_print(@ed_pub, 4)
      end

      private

      def _dump(_level)
        raise TypeError, "no _dump_data is defined for class #{self.class}"
      end

      D = Kenny::DER
      private_constant :D

      def __kenny_ed!
        raise NotImplementedError, "#{self.class}: this key type is not supported by Kenny" unless @ed_pub
      end

      def __kenny_no_digest(digest, fn)
        return if digest.nil? || digest.to_s.empty?
        raise PKeyError, "#{fn}: invalid digest (Explicit digest not allowed with EdDSA operations)"
      end
    end

    # (the key types OpenSSL knows that Kenny doesn't)
    OTHER_TYPES = %w[X25519 X448 ED448 DSA DH DHX SM2 RSA-PSS].freeze
    private_constant :OTHER_TYPES

    def self.__kenny_type_name(decoded)
      oid = decoded[1]
      case decoded[0]
      when :ed_priv, :ed_pub then "ED25519"
      when :ec_priv, :ec_pub, :ec_params then "id-ecPublicKey"
      else Kenny::OID.sn(oid.to_s) || oid.to_s
      end
    end

    # a key from the decoded form (Kenny::PKeyDER), nil when it's not one Kenny has
    def self.__kenny_key(k)
      case k[0]
      when :ec_priv, :ec_pub, :ec_params
        key = EC.allocate
        key.__send__(:__kenny_from, k)
        key.instance_variable_set(:@provider, true)
        key
      when :ed_priv then PKey.__kenny_ed25519(k[1], nil)
      when :ed_pub then PKey.__kenny_ed25519(nil, k[1])
      when :encrypted then raise NotImplementedError, "encrypted private keys are not supported by Kenny"
      end
    end

    # Kenny reads RSA keys (PKCS#1, PKCS#8, SPKI), EC keys (SEC1, PKCS#8, SPKI, EC PARAMETERS)
    # and Ed25519 keys (PKCS#8, SPKI), in PEM or DER; encrypted keys are not supported.
    def self.read(data, pass = nil)
      s = __kenny_input(data)
      key = RSA.allocate
      return key if key.__send__(:__load, s)
      k = Kenny::PKeyDER.decode_input(s)
      if k
        r = __kenny_key(k)
        return r if r
        name = __kenny_type_name(k)
        raise NotImplementedError, "#{name} keys are not supported by Kenny (RSA, EC and Ed25519 keys are)"
      end
      raise PKeyError, "Could not parse PKey"
    rescue OpenSSL::ASN1::ASN1Error
      raise PKeyError, "Could not parse PKey"
    end

    def self.__kenny_unknown_type(algo, fn)
      if OTHER_TYPES.include?(algo.to_s.upcase)
        raise NotImplementedError, "#{algo.to_s.upcase} keys are not supported by Kenny (RSA, EC and Ed25519 keys are)"
      end
      raise PKeyError, "#{fn}: unsupported (Global default library context, Algorithm (#{algo} : 0), Properties (<null>))"
    end

    # generate_key("RSA", rsa_keygen_bits: ..), ("EC", ec_paramgen_curve: ..), ("ED25519"), or
    # generate_key(EC parameters)
    def self.generate_key(algo, options = {})
      return EC.generate(algo.group) if algo.is_a?(EC)
      opts = (options || {}).to_h { |k, v| [k.to_s, v] }
      case algo.to_s.upcase
      when "RSA"
        RSA.new(Integer(opts.fetch("rsa_keygen_bits", 2048)), Integer(opts.fetch("rsa_keygen_pubexp", 65537)))
      when "EC"
        curve = opts["ec_paramgen_curve"] or raise PKeyError, "EVP_PKEY_keygen: no parameters set"
        EC.generate(__kenny_ec_group(curve))
      when "ED25519"
        PKey.__kenny_ed25519(OpenSSL::Random.random_bytes(32), nil)
      else
        __kenny_unknown_type(algo, "EVP_PKEY_CTX_new")
      end
    end

    def self.generate_parameters(algo, options = {})
      opts = (options || {}).to_h { |k, v| [k.to_s, v] }
      unless algo.to_s.casecmp?("EC")
        __kenny_unknown_type(algo, "EVP_PKEY_CTX_new") unless OTHER_TYPES.include?(algo.to_s.upcase)
        raise NotImplementedError, "OpenSSL::PKey.generate_parameters(#{algo.inspect}) is not supported by Kenny (EC is)"
      end
      curve = opts["ec_paramgen_curve"] or raise PKeyError, "EVP_PKEY_paramgen: no parameters set"
      EC.new(__kenny_ec_group(curve))
    end

    def self.__kenny_ec_group(name)
      c = EC.__kenny_curve_named(name.to_s, nist: true)
      raise PKeyError, "EVP_PKEY_CTX_ctrl_str: invalid curve name (#{name})" unless c
      EC::Group.new(c[0])
    end

    def self.new_raw_private_key(algo, key)
      __kenny_unknown_type(algo, "EVP_PKEY_new_raw_private_key_ex") unless algo.to_s.casecmp?("ED25519")
      key = key.to_str.b
      raise PKeyError, "EVP_PKEY_new_raw_private_key_ex: key setup failed" unless key.bytesize == 32
      PKey.__kenny_ed25519(key, nil)
    end

    def self.new_raw_public_key(algo, key)
      __kenny_unknown_type(algo, "EVP_PKEY_new_raw_public_key_ex") unless algo.to_s.casecmp?("ED25519")
      key = key.to_str.b
      raise PKeyError, "EVP_PKEY_new_raw_public_key_ex: key setup failed" unless key.bytesize == 32
      PKey.__kenny_ed25519(nil, key)
    end
  end
end
