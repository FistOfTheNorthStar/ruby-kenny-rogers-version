# frozen_string_literal: true

# Kenny's OpenSSL: the crypto (digests, HMAC, PBKDF2/HKDF, AES-ECB/CBC/CTR/GCM,
# random bytes) is native Rust (crates/kenny-runtime/src/openssl.rs); this file
# adds the Ruby-level conveniences of the openssl gem on top. TLS sockets,
# PKey, X509 and BN are not provided.
__kenny_openssl_init

module OpenSSL
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
