# frozen_string_literal: true

# Kenny's OpenSSL: the crypto (digests, HMAC, PBKDF2/HKDF, AES-ECB/CBC/CTR/GCM,
# random bytes) is native Rust (crates/kenny-runtime/src/openssl.rs); this file
# adds the Ruby-level conveniences of the openssl gem on top. TLS sockets,
# PKey, X509 and BN are not provided.
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
  # No TLS sockets in Kenny; the classes exist so `io.is_a?(OpenSSL::SSL::SSLSocket)` checks
  # (nio4r's selector, Net::HTTP) work.
  module SSL
    class SSLError < OpenSSL::OpenSSLError; end unless const_defined?(:SSLError, false)

    class SSLSocket
      def initialize(*)
        raise NotImplementedError, "TLS sockets (OpenSSL::SSL::SSLSocket) are not supported by Kenny"
      end
    end

    class SSLContext
      def initialize(*)
        raise NotImplementedError, "TLS (OpenSSL::SSL::SSLContext) is not supported by Kenny"
      end
    end unless const_defined?(:SSLContext, false)
  end
end
