# frozen_string_literal: true

# Kenny's OpenSSL: the crypto (digests, HMAC, PBKDF2/HKDF, AES-ECB/CBC/CTR/GCM,
# RSA keys, random bytes) is native Rust (crates/kenny-runtime/src/openssl.rs and
# openssl_pkey.rs); this file adds the Ruby-level conveniences of the openssl gem
# on top. OpenSSL::ASN1 and OpenSSL::X509 (certificates, names, extensions, CSRs, CRLs,
# and a Store that verifies chains as OpenSSL 3.6 does) are Ruby, in kenny/openssl_asn1.rb
# and kenny/openssl_x509*.rb, with signatures checked natively (openssl_x509.rs). EC keys
# (kenny/openssl_ec.rb) and Ed25519 keys (kenny/openssl_ed25519.rb) have their arithmetic in
# openssl_ec.rs. TLS (SSLContext, SSLSocket) runs on rustls
# (kenny/openssl_ssl.rb, openssl_tls.rs) under the gem's own openssl/ssl.rb. DSA and DH
# keys are not provided: their classes exist (gems refer to them when loading), and using
# them raises NotImplementedError.
# (OpenSSL::Digest is a Digest::Class, as in CRuby, whose openssl.so requires "digest")
require "digest"
__kenny_openssl_init

module OpenSSL
  # openssl 4.0 (Ruby 4.0's): an error raised after a failed OpenSSL call keeps the entries of
  # OpenSSL's error queue, and lists them in its detailed message
  if VERSION >= "4"
    class OpenSSLError
      attr_reader :errors

      def detailed_message(...)
        msg = super
        return msg unless errors.is_a?(Array) && !errors.empty?
        "#{msg}\nOpenSSL error queue reported #{errors.size} errors:\n#{errors.join("\n")}"
      end
    end
  end

  class Digest
    def initialize_copy(other)
      __kenny_copy(other)
    end

    # Digest::Instance#new: a reset copy (same algorithm, no data)
    def new = dup.reset

    class MD5 < Digest
      def initialize(data = nil)
        super("MD5", data)
      end
    end

    class SHA1 < Digest
      def initialize(data = nil)
        super("SHA1", data)
      end
    end

    class SHA224 < Digest
      def initialize(data = nil)
        super("SHA224", data)
      end
    end

    class SHA256 < Digest
      def initialize(data = nil)
        super("SHA256", data)
      end
    end

    class SHA384 < Digest
      def initialize(data = nil)
        super("SHA384", data)
      end
    end

    class SHA512 < Digest
      def initialize(data = nil)
        super("SHA512", data)
      end
    end
  end

  class HMAC
    def initialize_copy(other)
      __kenny_copy(other)
    end
  end

  class Cipher
    def initialize_copy(other)
      __kenny_copy(other)
    end

    class AES < Cipher
      def initialize(key_len, mode = "CBC")
        super("aes-#{key_len}-#{mode}")
      end
    end

    class AES128 < Cipher
      def initialize(mode = "CBC")
        super("aes-128-#{mode}")
      end
    end

    class AES192 < Cipher
      def initialize(mode = "CBC")
        super("aes-192-#{mode}")
      end
    end

    class AES256 < Cipher
      def initialize(mode = "CBC")
        super("aes-256-#{mode}")
      end
    end
  end

  module KDF
    def self.pbkdf2_hmac(pass, salt:, iterations:, length:, hash:)
      OpenSSL::PKCS5.pbkdf2_hmac(pass, salt, iterations, length, hash)
    end

    def self.hkdf(ikm, salt:, info:, length:, hash:)
      __hkdf(ikm, salt, info, length, hash)
    end
  end
end

module OpenSSL
  class BNError < OpenSSLError; end

  # Arbitrary-precision integers, kept as a Ruby Integer.
  class BN
    include Comparable

    def initialize(value, base = 10)
      @v =
        case value
        when BN then value.to_i
        when Integer then value
        else
          s = String.try_convert(value) or raise TypeError, "no implicit conversion of #{value.class} into String"
          case base
          when 10 then Integer(s, 10)
          when 16 then s.start_with?("-") ? -Integer(s[1..], 16) : Integer(s, 16)
          when 2 then s.unpack1("H*").then { |h| h.empty? ? 0 : h.to_i(16) }
          else raise ArgumentError, "invalid radix #{base}"
          end
        end
    rescue ArgumentError => e
      raise if e.message.start_with?("invalid radix")
      raise BNError, "invalid number"
    end

    def to_i = @v
    alias to_int to_i
    def to_bn = self
    def inspect = Kernel.instance_method(:to_s).bind_call(self)

    def to_s(base = 10)
      case base
      when 10 then @v.to_s
      when 16
        h = @v.abs.to_s(16).upcase
        h = "0#{h}" if h.size.odd? && @v != 0
        @v.negative? ? "-#{h}" : h
      when 2
        h = @v.abs.to_s(16)
        @v.zero? ? "".b : [h.size.odd? ? "0#{h}" : h].pack("H*")
      else raise ArgumentError, "invalid radix #{base}"
      end
    end

    def num_bits = @v.abs.bit_length
    def num_bytes = (num_bits + 7) / 8
    def zero? = @v.zero?
    def one? = @v == 1
    def odd? = @v.odd?
    def negative? = @v.negative?
    def bit_set?(n) = @v[n] == 1

    def <=>(other) = @v <=> (other.is_a?(BN) ? other.to_i : other)
    def cmp(other) = self <=> other
    def ucmp(other) = @v.abs <=> BN.new(other).to_i.abs
    def ==(other) = (other.is_a?(BN) || other.is_a?(Integer)) && @v == other.to_i
    alias === ==
    def eql?(other) = other.is_a?(BN) && @v == other.to_i
    def hash = @v.hash ^ BN.hash
    def coerce(other) = [other, @v]

    def +(other) = BN.new(@v + other.to_i)
    def -(other) = BN.new(@v - other.to_i)
    def *(other) = BN.new(@v * other.to_i)
    def %(other) = BN.new(@v.remainder(other.to_i))

    # [quotient, remainder], truncating like BN_div
    def /(other)
      d = other.to_i
      q = @v.abs / d.abs
      q = -q if @v.negative? ^ d.negative?
      [BN.new(q), BN.new(@v - q * d)]
    end

    def **(other) = BN.new(@v**other.to_i)
    def -@ = BN.new(-@v)
    def +@ = self
    def <<(n) = BN.new(@v << n)
    def >>(n) = BN.new(@v >> n)
    def mod_exp(e, m) = BN.new(@v.pow(e.to_i, m.to_i))
    def mod_inverse(m) = BN.new(@v.pow(-1, m.to_i))
    def gcd(other) = BN.new(@v.gcd(other.to_i))
  end

  # Marshal.dump/load of keys through their DER
  module Marshal
    def self.included(base)
      base.extend(ClassMethods)
    end

    module ClassMethods
      def _load(string) = new(string)
    end

    def _dump(_level) = to_der
  end

  module PKey
    if RUBY_VERSION >= "4"
      # (the openssl gem 4.0: other names of PKeyError)
      ECError = DSAError = DHError = PKeyError
    else
      class ECError < PKeyError; end
      class DSAError < PKeyError; end
      class DHError < PKeyError; end
    end

    # PEM or DER text: a String, an IO (a File), or anything with #to_der
    def self.__kenny_input(data)
      data = data.read if data.is_a?(IO)
      data = data.to_der if !data.is_a?(String) && data.respond_to?(:to_der)
      String.try_convert(data) or raise TypeError, "no implicit conversion of #{data.class} into String"
    end

    # (PKey.read, PKey.generate_key and friends: kenny/openssl_ed25519.rb)

    class PKey
      def oid = raise(NotImplementedError)

      def inspect
        "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} oid=#{oid}>"
      end
    end

    class RSA < PKey
      include OpenSSL::Marshal

      def self.generate(size, exp = 65537) = new(size, exp)

      # RSA.new: an empty key; RSA.new(bits, e = 65537): a new key; RSA.new(pem_or_der, pass = nil)
      def initialize(arg = nil, pass = nil)
        return if arg.nil?
        return __generate(arg, pass.nil? ? 65537 : Integer(pass)) if arg.is_a?(Integer)
        s = OpenSSL::PKey.__kenny_input(arg)
        return if __load(s)
        k = begin
          Kenny::PKeyDER.decode_input(s)
        rescue OpenSSL::ASN1::ASN1Error, NotImplementedError
          nil
        end
        raise RSAError, "incorrect pkey type: #{OpenSSL::PKey.__kenny_type_name(k)}" if k && k[0] != :encrypted
        raise RSAError, "Neither PUB key nor PRIV key"
      end

      def initialize_copy(other)
        __copy(other)
      end

      def oid = "rsaEncryption"

      def inspect
        "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} oid=rsaEncryption type_name=RSA provider=default>"
      end

      def public_key = RSA.allocate.__send__(:__public_of, self)

      def export(cipher = nil, pass = nil)
        __kenny_no_cipher(cipher)
        __pem(0)
      end
      alias to_pem export
      alias to_s export

      def to_der = __der(0)

      def private_to_pem(cipher = nil, pass = nil)
        __kenny_no_cipher(cipher)
        raise PKeyError, "PEM_write_bio_PKCS8PrivateKey: ASN1 lib" unless private?
        __pem(1)
      end

      def private_to_der(cipher = nil, pass = nil)
        __kenny_no_cipher(cipher)
        raise PKeyError, "i2d_PKCS8PrivateKey_bio: ASN1 lib" unless private?
        __der(1)
      end

      def public_to_pem = __pem(2)
      def public_to_der = __der(2)

      def sign(digest, data, options = nil)
        return sign_pss(digest, data, **__kenny_pss_opts(digest, options)) if __kenny_pss?(options)
        __sign(digest, data)
      end

      def verify(digest, signature, data, options = nil)
        return verify_pss(digest, signature, data, **__kenny_pss_opts(digest, options)) if __kenny_pss?(options)
        __verify(digest, signature, data)
      end

      def sign_pss(digest, data, salt_length:, mgf1_hash:)
        __kenny_same_mgf1(digest, mgf1_hash)
        __sign_pss(digest, data, salt_length)
      end

      def verify_pss(digest, signature, data, salt_length:, mgf1_hash:)
        __kenny_same_mgf1(digest, mgf1_hash)
        __verify_pss(digest, signature, data, salt_length)
      end

      %w[n e d p q dmp1 dmq1 iqmp].each_with_index do |name, i|
        define_method(name) do
          b = __parts[i]
          b && BN.new(b, 2)
        end
      end

      def params
        %w[n e d p q dmp1 dmq1 iqmp].zip(__parts).to_h { |k, b| [k, BN.new(b || "", 2)] }
      end

      def compare?(other)
        raise TypeError, "cannot compare different PKey types" unless other.is_a?(RSA)
        __public_eq(other)
      end

      %w[set_key set_factors set_crt_params].each do |m|
        define_method(m) { |*| raise PKeyError, "rsa##{m}= is incompatible with OpenSSL 3.0" }
      end

      private

      def __kenny_no_cipher(cipher)
        raise NotImplementedError, "exporting encrypted keys is not supported by Kenny" if cipher
      end

      def __kenny_pss?(options)
        options && options.to_h { |k, v| [k.to_s, v.to_s] }["rsa_padding_mode"] == "pss"
      end

      def __kenny_pss_opts(digest, options)
        o = options.to_h { |k, v| [k.to_s, v] }
        salt = o.fetch("rsa_pss_saltlen", -1)
        salt = { "digest" => :digest, "max" => :max, "auto" => :auto }.fetch(salt.to_s) { Integer(salt) } unless salt.is_a?(Integer)
        { salt_length: salt, mgf1_hash: o.fetch("rsa_mgf1_md", digest) }
      end

      def __kenny_same_mgf1(digest, mgf1)
        name = ->(d) { (d.respond_to?(:name) && !d.is_a?(String) ? d.name : d.to_s).upcase.delete("-") }
        return if name.(digest) == name.(mgf1)
        raise NotImplementedError, "PSS with an MGF1 digest other than the signature's is not supported by Kenny"
      end
    end

    # DSA and DH keys aren't implemented; the classes exist for the gems that name them. (EC
    # keys are in kenny/openssl_ec.rb, Ed25519 keys in kenny/openssl_ed25519.rb.)
    { DSA: DSAError, DH: DHError }.each do |name, _err|
      const_set(name, Class.new(PKey) {
        define_method(:initialize) do |*|
          raise NotImplementedError, "OpenSSL::PKey::#{name} keys are not supported by Kenny (RSA, EC and Ed25519 keys are)"
        end
        define_singleton_method(:generate) do |*|
          raise NotImplementedError, "OpenSSL::PKey::#{name} keys are not supported by Kenny (RSA, EC and Ed25519 keys are)"
        end
      })
    end
  end
end

class Integer
  def to_bn = OpenSSL::BN.new(self)
end

# OpenSSL::ASN1 and OpenSSL::X509 (in Ruby, signatures checked natively), then the stdlib's
# openssl/x509.rb on top, as the gem does
require "kenny/openssl_asn1"
require "kenny/openssl_x509"
require "openssl/x509"

# TLS: openssl.so's half (lib/kenny/kenny/openssl_ssl.rb), then the openssl gem's own Ruby half
require "kenny/openssl_ssl"
require "openssl/ssl"
