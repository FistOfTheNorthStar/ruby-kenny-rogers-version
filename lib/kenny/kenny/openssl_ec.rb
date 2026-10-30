# frozen_string_literal: true
# Kenny: OpenSSL::PKey::EC (ossl_pkey_ec.c's API) on P-256, P-384, P-521 and secp256k1: key
# generation, SEC1/PKCS#8/SPKI keys in DER and PEM, ECDSA (sign/verify, dsa_sign_asn1), ECDH
# (derive, dh_compute_key), and Group/Point with point arithmetic. The curve arithmetic is
# native (crates/kenny-runtime/src/openssl_ec.rs, RustCrypto's curve crates); keys are bytes
# here: the private scalar (the field's size, big-endian) and the public point's SEC1 octets.
# Other curves, explicit curve parameters and encrypted keys raise NotImplementedError.

module Kenny
  # The keys DER and PEM text hold, as OpenSSL's decoders see them (ossl_pkey_read_generic):
  #   [:ec_priv, curve, scalar, point or nil], [:ec_pub, curve, point], [:ec_params, curve],
  #   [:ed_priv, seed], [:ed_pub, key], [:other, algorithm OID], [:encrypted]
  # PEM text: the first private key, else the first public key, else the first parameters.
  module PKeyDER
    module_function

    EC_OID = "1.2.840.10045.2.1"
    ED25519_OID = "1.3.101.112"

    def decode_input(s)
      s = s.b
      blocks = Kenny::X509Util.pem_blocks(s)
      return decode(s) if blocks.empty?
      found = blocks.filter_map do |label, der|
        case label
        when "EC PRIVATE KEY" then der ? sec1(der) : [:encrypted]
        when "PRIVATE KEY" then der && pkcs8(der)
        when "ENCRYPTED PRIVATE KEY" then [:encrypted]
        when "PUBLIC KEY" then der && spki(der)
        when "EC PARAMETERS" then der && params(der)
        when "RSA PRIVATE KEY", "RSA PUBLIC KEY" then [:other, "1.2.840.113549.1.1.1"]
        end
      rescue OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError
        nil
      end
      found.find { |k| k[0] == :encrypted || k[0].end_with?("priv") } || found.find { |k| k[0].end_with?("pub") } ||
        found.find { |k| k[0] == :ec_params } || found.first
    end

    def decode(der)
      %i[pkcs8 sec1 spki params].each do |m|
        k = begin
          __send__(m, der)
        rescue OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError
          nil
        end
        return k if k
      end
      nil
    end

    def curve_of(oid_content)
      oid = Kenny::DER.oid_dotted(oid_content)
      OpenSSL::PKey::EC.__kenny_curve(oid) or
        OpenSSL::PKey::EC.__kenny_unsupported_curve(Kenny::OID.sn(oid) || oid)
    end

    # ECPrivateKey (RFC 5915); curve: the PKCS#8 AlgorithmIdentifier's, when it's inside one
    def sec1(der, curve = nil)
      t, c = Kenny::DER.unwrap(der)
      return nil unless t == 0x30
      kids = Kenny::DER.children(c)
      return nil unless kids.size >= 2 && kids[0][0] == 2 && kids[0][1] == "\x01".b && kids[1][0] == 4
      point = nil
      kids[2..].each do |b, kc|
        case b
        when 0xa0
          pt, pc = Kenny::DER.unwrap(kc)
          raise NotImplementedError, "EC keys with explicit curve parameters are not supported by Kenny" unless pt == 6
          curve = curve_of(pc)
        when 0xa1
          bt, bc = Kenny::DER.unwrap(kc)
          point = bc.byteslice(1..) if bt == 3
        end
      end
      curve ? [:ec_priv, curve, kids[1][1], point] : nil
    end

    def pkcs8(der)
      t, c = Kenny::DER.unwrap(der)
      return nil unless t == 0x30
      kids = Kenny::DER.children(c)
      return nil unless kids.size >= 3 && kids[0][0] == 2 && kids[1][0] == 0x30 && kids[2][0] == 4
      oid, params = Kenny::X509Util.alg_id(kids[1][2])
      case oid
      when EC_OID
        raise NotImplementedError, "EC keys with explicit curve parameters are not supported by Kenny" unless params&.getbyte(0) == 6
        sec1(kids[2][1], curve_of(Kenny::DER.unwrap(params)[1]))
      when ED25519_OID
        ot, seed = Kenny::DER.unwrap(kids[2][1])
        ot == 4 && seed.bytesize == 32 ? [:ed_priv, seed] : nil
      else
        [:other, oid]
      end
    end

    def spki(der)
      oid, params, bits = Kenny::X509Util.spki_parts(der)
      case oid
      when EC_OID
        raise NotImplementedError, "EC keys with explicit curve parameters are not supported by Kenny" unless params&.getbyte(0) == 6
        [:ec_pub, curve_of(Kenny::DER.unwrap(params)[1]), bits]
      when ED25519_OID
        bits.bytesize == 32 ? [:ed_pub, bits] : nil
      else
        [:other, oid]
      end
    end

    def params(der)
      t, c = Kenny::DER.unwrap(der)
      t == 6 ? [:ec_params, curve_of(c)] : nil
    end

    def ctx(n, body) = (0xa0 | n).chr.b << Kenny::DER.length_octets(body.bytesize) << body
    def bits(b) = Kenny::DER.tlv(3, "\x00".b + b)
    def oid(dotted) = Kenny::DER.tlv(6, Kenny::DER.oid_content(dotted))
  end
end

module OpenSSL
  module PKey
    class EC < PKey
      include OpenSSL::Marshal

      NAMED_CURVE = 1
      EXPLICIT_CURVE = 0

      # curve OID => [short name, NIST name, bits, field bytes, order]
      CURVES = {
        "1.2.840.10045.3.1.7" => ["prime256v1", "P-256", 256, 32,
                                  0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551],
        "1.3.132.0.34" => ["secp384r1", "P-384", 384, 48,
                           0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFC7634D81F4372DDF581A0DB248B0A77AECEC196ACCC52973],
        "1.3.132.0.35" => ["secp521r1", "P-521", 521, 66,
                           0x01FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFA51868783BF2F966B7FCC0148F709A5D03BB5C9B8899C47AEBB6FB71E91386409],
        "1.3.132.0.10" => ["secp256k1", nil, 256, 32,
                           0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141],
      }.freeze
      private_constant :CURVES

      # (curves OpenSSL has that Kenny doesn't: NotImplementedError rather than "unknown curve")
      OTHER_CURVES = %w[secp112r1 secp112r2 secp128r1 secp128r2 secp160k1 secp160r1 secp160r2 secp192k1 secp224k1
                        secp224r1 prime192v1 prime192v2 prime192v3 prime239v1 prime239v2 prime239v3 sect163k1
                        sect163r2 sect233k1 sect233r1 sect283k1 sect283r1 sect409k1 sect409r1 sect571k1 sect571r1
                        brainpoolP160r1 brainpoolP192r1 brainpoolP224r1 brainpoolP256r1 brainpoolP320r1
                        brainpoolP384r1 brainpoolP512r1 SM2].freeze
      private_constant :OTHER_CURVES

      def self.__kenny_unsupported_curve(name)
        raise NotImplementedError, "EC keys on #{name} are not supported by Kenny (prime256v1, secp384r1, secp521r1 and secp256k1 are)"
      end

      def self.__kenny_curve(oid) = CURVES[oid]

      # the curve of a short name (or a NIST one, "P-256", with nist: as EVP_PKEY_CTX's
      # ec_paramgen_curve takes); nil when it isn't one
      def self.__kenny_curve_named(name, nist: false)
        c = CURVES.values.find { |x| x[0] == name || (nist && x[1] == name) }
        return c if c
        __kenny_unsupported_curve(name) if OTHER_CURVES.include?(name)
        nil
      end

      def self.__kenny_immutable
        raise PKeyError, "pkeys are immutable on OpenSSL 3.0"
      end

      # (Kenny's curves, in OpenSSL's order, with its descriptions)
      def self.builtin_curves
        [["secp256k1", "SECG curve over a 256 bit prime field"], ["secp384r1", "NIST/SECG curve over a 384 bit prime field"],
         ["secp521r1", "NIST/SECG curve over a 521 bit prime field"], ["prime256v1", "X9.62/SECG curve over a 256 bit prime field"]]
      end

      # EC.generate(curve name or Group): a new key
      def self.generate(arg)
        curve = arg.is_a?(Group) ? arg.__kenny_curve : __kenny_curve_named(String(arg))
        raise ECError, "invalid curve name" unless curve
        d, point = __kenny_generate(curve[0])
        key = allocate
        key.__send__(:__kenny_init, curve, d, point)
        key
      end

      # EC.new(curve name or Group): no key yet (only the group); EC.new(PEM or DER, pass)
      def initialize(arg = nil, pass = nil)
        if arg.nil?
          if RUBY_VERSION >= "4"
            raise ArgumentError, "OpenSSL::PKey::EC.new cannot be called without arguments; pkeys are immutable with OpenSSL 3.0"
          end
          return
        end
        if arg.is_a?(Group)
          @curve = arg.__kenny_curve
          return
        end
        s = OpenSSL::PKey.__kenny_input(arg).b
        k = Kenny::PKeyDER.decode_input(s)
        if k.nil?
          c = EC.__kenny_curve_named(s)
          raise ECError, "invalid curve name" unless c
          @curve = c
          return
        end
        __kenny_from(k) or raise ECError, "incorrect pkey type: #{OpenSSL::PKey.__kenny_type_name(k)}"
        @provider = true
      end

      def oid = "id-ecPublicKey"

      def inspect
        "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} oid=id-ecPublicKey type_name=EC#{@provider ? " provider=default" : ""}>"
      end

      def group = @curve && Group.__send__(:__kenny_new, @curve)
      def public_key = @point && Point.__send__(:__kenny_new, group, @point)
      def private_key = @priv && OpenSSL::BN.new(@priv, 2)
      def private? = !@priv.nil?
      def public? = !@point.nil?
      alias private_key? private?
      alias public_key? public?

      %i[group= private_key= public_key= generate_key! generate_key].each do |m|
        define_method(m) { |*| EC.__kenny_immutable }
      end

      def check_key
        raise ECError, "EVP_PKEY_public_check: point at infinity" unless @point
        raise ECError, "EVP_PKEY_check: invalid private key" if @priv && EC.__kenny_public(@curve[0], @priv) != @point
        true
      end

      def public_to_der
        raise PKeyError, "i2d_PUBKEY_bio: ASN1 lib" unless @point
        D.seq(D.seq(P.oid(P::EC_OID), P.oid(Kenny::OID.lookup(@curve[0]))), P.bits(@point))
      end

      def public_to_pem = Kenny::X509Util.pem("PUBLIC KEY", public_to_der)

      # (traditional SEC1, with the curve and the public key, as OpenSSL writes it)
      def to_der
        return public_to_der unless @priv
        D.seq(D.int(1), D.tlv(4, @priv), P.ctx(0, P.oid(Kenny::OID.lookup(@curve[0]))), P.ctx(1, P.bits(@point)))
      end

      def export(cipher = nil, pass = nil)
        __kenny_no_cipher(cipher)
        @priv ? Kenny::X509Util.pem("EC PRIVATE KEY", to_der) : public_to_pem
      end
      alias to_pem export
      alias to_s export

      def private_to_der(cipher = nil, pass = nil)
        __kenny_no_cipher(cipher)
        raise PKeyError, "i2d_PKCS8PrivateKey_bio: ASN1 lib" unless @priv
        alg = D.seq(P.oid(P::EC_OID), P.oid(Kenny::OID.lookup(@curve[0])))
        D.seq(D.int(0), alg, D.tlv(4, D.seq(D.int(1), D.tlv(4, @priv), P.ctx(1, P.bits(@point)))))
      end

      def private_to_pem(cipher = nil, pass = nil)
        raise PKeyError, "PEM_write_bio_PKCS8PrivateKey: ASN1 lib" unless @priv
        Kenny::X509Util.pem("PRIVATE KEY", private_to_der(cipher, pass))
      end

      # sign(digest, data): ECDSA over the digest of data (SHA-256 when digest is nil), DER
      def sign(digest, data, options = nil)
        raise ArgumentError, "private key is needed" unless @priv
        EC.__kenny_sign(@curve[0], @priv, __kenny_hash(digest, data))
      end

      def verify(digest, signature, data, options = nil)
        sig = signature.to_str.b
        raise PKeyError, "EVP_DigestVerify" unless __kenny_sig_der?(sig) && @point
        EC.__kenny_verify(@curve[0], @point, __kenny_hash(digest, data), sig)
      end

      # (ECDSA over a digest the caller computed)
      def dsa_sign_asn1(data)
        raise ECError, "EVP_PKEY_sign: provider signature failure (ECDSA sign:)" unless @priv
        EC.__kenny_sign(@curve[0], @priv, data.to_str.b)
      end

      def dsa_verify_asn1(data, signature)
        sig = signature.to_str.b
        raise ECError, "EVP_PKEY_verify: provider signature failure (ECDSA verify:)" unless __kenny_sig_der?(sig) && @point
        EC.__kenny_verify(@curve[0], @point, data.to_str.b, sig)
      end

      def sign_raw(_digest, data, _options = nil) = dsa_sign_asn1(data)

      def verify_raw(_digest, signature, data, _options = nil)
        dsa_verify_asn1(data, signature)
      rescue ECError
        raise PKeyError, "EVP_PKEY_verify"
      end

      # ECDH: the shared secret with peer (an EC key on the same curve)
      def derive(peer)
        raise TypeError, "wrong argument type #{peer.class} (expected OpenSSL/EVP_PKEY)" unless peer.is_a?(PKey)
        raise PKeyError, "EVP_PKEY_derive: missing private key" unless @priv
        raise PKeyError, "EVP_PKEY_derive_set_peer_ex: different key types" unless peer.is_a?(EC)
        c = peer.__kenny_curve_entry
        raise PKeyError, "EVP_PKEY_derive_set_peer: mismatching domain parameters" unless c == @curve
        EC.__kenny_dh(@curve[0], @priv, peer.public_key.to_octet_string(:uncompressed))
      end

      def dh_compute_key(pubkey)
        raise TypeError, "wrong argument type #{pubkey.class} (expected OpenSSL/EC_POINT)" unless pubkey.is_a?(Point)
        raise PKeyError, "EVP_PKEY_derive: missing private key" unless @priv
        raise PKeyError, "Could not parse PKey" unless pubkey.group.__kenny_curve == @curve && !pubkey.infinity?
        EC.__kenny_dh(@curve[0], @priv, pubkey.to_octet_string(:uncompressed))
      end

      def compare?(other)
        raise TypeError, "cannot compare different PKey types" unless other.is_a?(EC)
        @curve == other.__kenny_curve_entry && public_key == other.public_key
      end

      def to_text
        b = Kenny::X509Util
        s = +""
        s << "Private-Key: (#{@curve[2]} bit)\npriv:\n" << b.buf_print(@priv, 4) if @priv
        s << "Public-Key: (#{@curve[2]} bit)\n" unless @priv
        s << "pub:\n" << b.buf_print(@point, 4) if @point
        s << "ASN1 OID: #{@curve[0]}\n" << (@curve[1] ? "NIST CURVE: #{@curve[1]}\n" : "")
      end

      def __kenny_curve_entry = @curve

      private

      D = Kenny::DER
      P = Kenny::PKeyDER
      private_constant :D, :P

      def __kenny_init(curve, d, point)
        @curve = curve
        @priv = d.b.freeze
        @point = point.b.freeze
      end

      # from a decoded key (Kenny::PKeyDER); false when it isn't an EC one
      def __kenny_from(k)
        case k[0]
        when :ec_priv
          _, curve, d, point = k
          d = d.rjust(curve[3], "\x00") if d.bytesize < curve[3]
          derived = EC.__kenny_public(curve[0], d) if d.bytesize == curve[3]
          raise ECError, "invalid private key" unless derived
          if point && EC.__kenny_point(curve[0], point, false) != derived
            raise ECError, "EVP_PKEY_check: private key and public key do not match"
          end
          __kenny_init(curve, d, derived)
        when :ec_pub
          _, curve, point = k
          pt = Point.__send__(:__kenny_decode, curve, point)
          raise ECError, "invalid public key" unless pt && pt != "\x00".b
          @curve = curve
          @point = pt.freeze
        when :ec_params then @curve = k[1]
        when :encrypted then raise NotImplementedError, "encrypted private keys are not supported by Kenny"
        else return false
        end
        true
      end

      def __kenny_no_cipher(cipher)
        raise NotImplementedError, "exporting encrypted keys is not supported by Kenny" if cipher
      end

      def __kenny_hash(digest, data)
        name = digest.nil? ? "SHA256" : digest.is_a?(OpenSSL::Digest) ? digest.name : digest.to_s
        OpenSSL::Digest.new(name).digest(data.to_str)
      end

      # whether sig is the DER of an ECDSA-Sig-Value (SEQUENCE of two INTEGERs), as d2i reads it
      def __kenny_sig_der?(sig)
        t, c = D.unwrap(sig)
        return false unless t == 0x30
        kids = D.children(c)
        return false unless kids.size == 2 && kids.all? { |k| k[0] == 2 }
        kids.each { |k| D.int_value(k[1]) }
        true
      rescue OpenSSL::ASN1::ASN1Error
        false
      end

      public

      class Group
        def self.__kenny_new(curve)
          g = allocate
          g.instance_variable_set(:@curve, curve)
          g
        end
        private_class_method :__kenny_new

        # Group.new(curve name), Group.new(group), Group.new(DER or PEM of the curve's OID)
        def initialize(*args)
          arg = args[0]
          if args.size == 1 && arg.is_a?(Group)
            @curve = arg.__kenny_curve
            @form = arg.point_conversion_form
            @asn1_flag = arg.asn1_flag
            return
          end
          if args.size == 1 && (arg.is_a?(String) || arg.respond_to?(:to_str))
            s = arg.to_str
            @curve = EC.__kenny_curve_named(s)
            return if @curve
            k = begin
              Kenny::PKeyDER.decode_input(s)
            rescue OpenSSL::ASN1::ASN1Error
              nil
            end
            if k && k[0] == :ec_params
              @curve = k[1]
              return
            end
            raise Error, "unknown curve name (#{s})"
          end
          raise NotImplementedError, "OpenSSL::PKey::EC::Group.new(#{args.map(&:class).join(", ")}) is not supported by Kenny (named curves are)"
        end

        def __kenny_curve = @curve
        def curve_name = @curve[0]
        def degree = @curve[2]
        def order = OpenSSL::BN.new(@curve[4])
        def cofactor = OpenSSL::BN.new(1)
        def seed = nil
        def asn1_flag = @asn1_flag || NAMED_CURVE
        def point_conversion_form = @form || :uncompressed

        def generator
          @generator ||= Point.__send__(:__kenny_new, self, EC.__kenny_mul(@curve[0], "".b, [1].pack("C").rjust(@curve[3], "\x00")))
        end

        def asn1_flag=(flag)
          raise NotImplementedError, "EC groups with explicit curve parameters are not supported by Kenny" unless flag & NAMED_CURVE != 0
          @asn1_flag = flag
        end

        def point_conversion_form=(form)
          raise ArgumentError, "unsupported point conversion form: #{form}" unless %i[compressed uncompressed hybrid].include?(form)
          @form = form
        end

        def inspect = "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")}>"

        def ==(other)
          other.is_a?(Group) && @curve == other.__kenny_curve
        end
        alias eql? ==

        def to_der
          Kenny::DER.tlv(6, Kenny::DER.oid_content(Kenny::OID.lookup(@curve[0])))
        end

        def to_pem = Kenny::X509Util.pem("EC PARAMETERS", to_der)

        def to_text
          "ASN1 OID: #{@curve[0]}\n" + (@curve[1] ? "NIST CURVE: #{@curve[1]}\n" : "")
        end

        class Error < OpenSSL::OpenSSLError; end
      end

      class Point
        def self.__kenny_new(group, bytes)
          pt = allocate
          pt.instance_variable_set(:@group, group)
          pt.instance_variable_set(:@bytes, bytes.b.freeze)
          pt
        end
        private_class_method :__kenny_new

        # SEC1 octets (uncompressed, compressed or hybrid) as the uncompressed octets of the
        # point, "\x00" for infinity; nil when they're not a point; :encoding when they're
        # not SEC1 octets at all
        def self.__kenny_decode(curve, s)
          s = s.b
          n = curve[3]
          tag = s.getbyte(0)
          ok_len = case tag
            when 0 then 1
            when 2, 3 then n + 1
            when 4, 6, 7 then 2 * n + 1
          end
          return :encoding unless ok_len == s.bytesize
          if tag >= 6
            return nil unless (tag & 1) == (s.getbyte(-1) & 1)
            s = "\x04".b + s.byteslice(1..)
          end
          EC.__kenny_point(curve[0], s, false)
        end
        private_class_method :__kenny_decode

        # Point.new(group): infinity; Point.new(point); Point.new(group, octets or BN)
        def initialize(*args)
          case args.size
          when 1
            a = args[0]
            if a.is_a?(Point)
              @group = a.group
              @bytes = a.__kenny_bytes
            elsif a.is_a?(Group)
              @group = a
              @bytes = "\x00".b.freeze
            else
              raise TypeError, "wrong argument type #{a.class} (expected OpenSSL/EC_GROUP)"
            end
          when 2
            g, v = args
            raise TypeError, "wrong argument type #{g.class} (expected OpenSSL/EC_GROUP)" unless g.is_a?(Group)
            s = v.is_a?(OpenSSL::BN) ? v.to_s(2) : v.to_str
            b = Point.__send__(:__kenny_decode, g.__kenny_curve, s)
            raise Error, "EC_POINT_oct2point: invalid encoding" if b == :encoding
            raise Error, "EC_POINT_oct2point: point is not on curve" unless b
            @group = g
            @bytes = b.freeze
          else
            raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 1..2)"
          end
        end

        attr_reader :group

        def __kenny_bytes = @bytes

        def inspect
          "#<#{self.class}:0x#{object_id.to_s(16).rjust(16, "0")} @group=#{@group.inspect}>"
        end

        def to_octet_string(form = @group.point_conversion_form)
          return @bytes if infinity?
          case form
          when :uncompressed then @bytes.dup
          when :compressed then EC.__kenny_point(__kenny_curve[0], @bytes, true)
          when :hybrid then (6 | (@bytes.getbyte(-1) & 1)).chr.b + @bytes.byteslice(1..)
          else raise ArgumentError, "unsupported point conversion form: #{form}"
          end
        end

        def to_bn(form = @group.point_conversion_form) = OpenSSL::BN.new(to_octet_string(form), 2)
        def infinity? = @bytes == "\x00".b
        def on_curve? = true
        def make_affine! = self

        def invert!
          @bytes = __kenny_mul(__kenny_curve[4] - 1, @bytes).freeze
          self
        end

        def set_to_infinity!
          @bytes = "\x00".b.freeze
          self
        end

        def add(other)
          raise TypeError, "wrong argument type #{other.class} (expected OpenSSL/EC_POINT)" unless other.is_a?(Point)
          Point.__send__(:__kenny_new, @group, EC.__kenny_add(__kenny_curve[0], @bytes, other.__kenny_bytes))
        end

        # mul(bn): bn * self; mul(bn1, bn2): bn1 * self + bn2 * G
        def mul(*args)
          raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 1..3)" unless (1..3).cover?(args.size)
          if args[0].is_a?(Array)
            raise NotImplementedError, (RUBY_VERSION >= "4" ? "OpenSSL::PKey::EC::Point#mul with arrays is no longer supported" : "calling #mul with arrays is notsupported by this OpenSSL version")
          end
          r = __kenny_mul(args[0], @bytes)
          r = EC.__kenny_add(__kenny_curve[0], r, __kenny_mul(args[1], "".b)) if args[1]
          Point.__send__(:__kenny_new, @group, r)
        end

        def ==(other)
          other.is_a?(Point) && @group == other.group && @bytes == other.__kenny_bytes
        end
        alias eql? ==

        private

        def __kenny_curve = @group.__kenny_curve

        # k * point ("" for the generator), k an Integer or BN reduced mod the order
        def __kenny_mul(k, point)
          c = __kenny_curve
          k = Integer(k.is_a?(OpenSSL::BN) ? k.to_i : k) % c[4]
          return "\x00".b if k.zero? || point == "\x00".b
          EC.__kenny_mul(c[0], point, [k.to_s(16).rjust(2 * c[3], "0")].pack("H*"))
        end

        public

        class Error < OpenSSL::OpenSSLError; end
      end
    end
  end
end
