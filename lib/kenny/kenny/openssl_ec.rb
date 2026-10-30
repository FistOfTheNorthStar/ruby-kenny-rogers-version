# frozen_string_literal: true
# Kenny: a public-only OpenSSL::PKey::EC, what certificates carry (ossl_pkey_ec.c's API for a
# key read from a SubjectPublicKeyInfo): its group, the point, DER/PEM, and ECDSA verification
# on P-256 and P-384 (natively, with ring). Generating keys, private keys and the EC
# arithmetic aren't supported and raise NotImplementedError.

module OpenSSL
  module PKey
    class EC < PKey
      include OpenSSL::Marshal

      NAMED_CURVE = 1
      EXPLICIT_CURVE = 0

      # curve OID => [short name, NIST name, bits]
      CURVES = { "1.2.840.10045.3.1.7" => ["prime256v1", "P-256", 256], "1.3.132.0.34" => ["secp384r1", "P-384", 384],
                 "1.3.132.0.35" => ["secp521r1", "P-521", 521], "1.3.132.0.10" => ["secp256k1", nil, 256] }.freeze
      private_constant :CURVES

      def self.__kenny_unsupported(what)
        raise NotImplementedError, "#{what} is not supported by Kenny (EC public keys from certificates are)"
      end

      def self.generate(*) = __kenny_unsupported("OpenSSL::PKey::EC.generate")

      def self.builtin_curves
        CURVES.values.map { |sn, nist, _| [sn, nist ? "NIST/X9.62/SECG curve over a #{nist[2..]} bit prime field" : "SECG curve over a 256 bit prime field"] }
      end

      def self.__kenny_curve(oid) = CURVES[oid]

      def initialize(arg = nil, pass = nil)
        EC.__kenny_unsupported("OpenSSL::PKey::EC.new without a key") if arg.nil? || arg.is_a?(Group)
        s = OpenSSL::PKey.__kenny_input(arg).b
        der = nil
        Kenny::X509Util.pem_blocks(s).each do |label, d|
          if label == "PUBLIC KEY"
            der = d
            break
          elsif label.include?("PRIVATE KEY")
            EC.__kenny_unsupported("Reading EC private keys")
          end
        end
        der ||= s
        __kenny_load(der) or begin
          EC.__kenny_unsupported("OpenSSL::PKey::EC.new(curve name)") if CURVES.values.any? { |c| c[0] == s || c[1] == s }
          raise ECError, "invalid curve name"
        end
      end

      def initialize_copy(other)
        super
      end

      def oid = "id-ecPublicKey"

      def inspect
        "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} oid=id-ecPublicKey type_name=EC provider=default>"
      end

      def group = Group.__send__(:__kenny_new, @curve)
      def public_key = Point.__send__(:__kenny_new, group, @point)
      def private_key = nil
      def private? = false
      def private_key? = false
      def public? = true
      def public_key? = true
      def check_key = true

      def public_to_der = @spki.dup
      def public_to_pem = Kenny::X509Util.pem("PUBLIC KEY", @spki)
      alias to_der public_to_der

      def to_pem(*) = public_to_pem
      alias export to_pem
      alias to_s to_pem

      def private_to_der(*) = raise(ECError, "i2d_PKCS8PrivateKey_bio: ASN1 lib")
      def private_to_pem(*) = raise(ECError, "PEM_write_bio_PKCS8PrivateKey: ASN1 lib")
      def sign(*) = raise(ECError, "EVP_DigestSign: operation not supported for this keytype")
      def dsa_sign_asn1(*) = raise(ECError, "ECDSA_sign: missing private key")

      def verify(digest, signature, data, options = nil)
        d = Kenny::X509Util.digest_name(digest)
        r = OpenSSL::X509.__kenny_verify("ECDSA_#{@curve[1]&.delete("-")}_#{d}", @point, data.to_s, signature.to_s)
        EC.__kenny_unsupported("ECDSA with #{@curve[0]} and #{d}") if r.nil?
        r
      end

      def dsa_verify_asn1(*) = EC.__kenny_unsupported("OpenSSL::PKey::EC#dsa_verify_asn1")

      def compare?(other)
        raise TypeError, "cannot compare different PKey types" unless other.is_a?(EC)
        public_to_der == other.public_to_der
      end

      def to_text
        "Public-Key: (#{@curve[2]} bit)\npub:\n" + Kenny::X509Util.buf_print(@point, 4) +
          "ASN1 OID: #{@curve[0]}\n" + (@curve[1] ? "NIST CURVE: #{@curve[1]}\n" : "")
      end

      def __kenny_curve_entry = @curve

      private

      def __kenny_load(der)
        oid, params, bits = Kenny::X509Util.spki_parts(der)
        return false unless oid == "1.2.840.10045.2.1" && params && params.getbyte(0) == 6
        curve = CURVES[Kenny::DER.oid_dotted(Kenny::DER.unwrap(params)[1])] or return false
        @curve = curve
        @point = bits
        @spki = Kenny::DER.first(der)[2]
        true
      rescue OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError
        false
      end

      class Group
        def self.__kenny_new(curve)
          g = allocate
          g.instance_variable_set(:@curve, curve)
          g
        end
        private_class_method :__kenny_new

        def initialize(*args)
          name = args[0]
          if args.size == 1 && name.is_a?(String)
            c = CURVES.values.find { |x| x[0] == name || x[1] == name }
            if c
              @curve = c
              return
            end
            raise OpenSSL::PKey::EC::Group::Error, "unknown curve name (#{name})"
          end
          raise NotImplementedError, "OpenSSL::PKey::EC::Group.new(#{args.map(&:class).join(", ")}) is not supported by Kenny"
        end

        def curve_name = @curve[0]
        def degree = @curve[2]
        def asn1_flag = NAMED_CURVE
        def point_conversion_form = :uncompressed

        def ==(other)
          other.is_a?(Group) && curve_name == other.curve_name
        end
        alias eql? ==

        def to_der
          Kenny::DER.tlv(6, Kenny::DER.oid_content(Kenny::OID.lookup(@curve[0])))
        end

        def to_pem = Kenny::X509Util.pem("EC PARAMETERS", to_der)

        class Error < OpenSSL::OpenSSLError; end
      end

      class Point
        def self.__kenny_new(group, bytes)
          pt = allocate
          pt.instance_variable_set(:@group, group)
          pt.instance_variable_set(:@bytes, bytes)
          pt
        end
        private_class_method :__kenny_new

        def initialize(*)
          raise NotImplementedError, "OpenSSL::PKey::EC::Point.new is not supported by Kenny"
        end

        attr_reader :group

        def to_octet_string(form = :uncompressed)
          raise NotImplementedError, "EC point compression is not supported by Kenny" unless form == :uncompressed
          @bytes.dup
        end

        def to_bn(form = :uncompressed) = OpenSSL::BN.new(to_octet_string(form), 2)
        def infinity? = @bytes == "\x00".b

        def ==(other)
          other.is_a?(Point) && to_octet_string == other.to_octet_string
        end
        alias eql? ==

        class Error < OpenSSL::OpenSSLError; end
      end
    end
  end
end
