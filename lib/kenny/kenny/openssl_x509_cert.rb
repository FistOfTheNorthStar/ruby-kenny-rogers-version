# frozen_string_literal: true
# Kenny: OpenSSL::X509::Certificate (ossl_x509cert.c), Request (ossl_x509req.c), Attribute
# (ossl_x509attr.c), CRL and Revoked (ossl_x509crl.c, ossl_x509revoked.c). Each keeps its
# fields and the DER it was read from; changing a field re-encodes, as OpenSSL does.

module Kenny
  module X509Util
    module_function

    # ASN1_buf_print: hex bytes, 15 to a line
    def buf_print(b, indent)
      b.unpack1("H*").scan(/../).each_slice(15).map { |l| " " * indent + l.join(":") }.join(":\n") + "\n"
    end

    # ASN1_bn_print
    def bn_print(name, v, indent)
      s = " " * indent + name
      if v.abs < 1 << 64
        s + " #{v < 0 ? "-" : ""}#{v.abs} (#{v < 0 ? "-" : ""}0x#{v.abs.to_s(16)})\n"
      else
        h = v.abs.to_s(16)
        h = "0" + h if h.size.odd?
        b = [h].pack("H*")
        b = "\x00".b + b if b.getbyte(0) >= 0x80
        s + (v < 0 ? " (Negative)" : "") + "\n" + buf_print(b, indent + 4)
      end
    end

    EC_TEXT = { "1.2.840.10045.3.1.7" => ["prime256v1", "P-256", 256], "1.3.132.0.34" => ["secp384r1", "P-384", 384],
                "1.3.132.0.35" => ["secp521r1", "P-521", 521], "1.3.132.0.10" => ["secp256k1", nil, 256] }.freeze

    # EVP_PKEY_print_public's text for a SubjectPublicKeyInfo
    def key_text(spki, indent)
      oid, params, bits = spki_parts(spki)
      pad = " " * indent
      case oid
      when "1.2.840.113549.1.1.1"
        n, e = Kenny::DER.children(Kenny::DER.unwrap(bits)[1]).map { |_, c| Kenny::DER.int_value(c) }
        nb = [n.to_s(16).rjust((n.bit_length + 7) / 8 * 2, "0")].pack("H*")
        nb = "\x00".b + nb if nb.getbyte(0) >= 0x80
        "#{pad}Public-Key: (#{n.bit_length} bit)\n#{pad}Modulus:\n" + buf_print(nb, indent + 4) + bn_print("Exponent:", e, indent)
      when "1.2.840.10045.2.1"
        curve = EC_TEXT[Kenny::DER.oid_dotted(Kenny::DER.unwrap(params)[1])] || ["unknown", nil, 0]
        s = "#{pad}Public-Key: (#{curve[2]} bit)\n#{pad}pub:\n" + buf_print(bits, indent + 4) + "#{pad}ASN1 OID: #{curve[0]}\n"
        s += "#{pad}NIST CURVE: #{curve[1]}\n" if curve[1]
        s
      when "1.3.101.112", "1.3.101.113"
        "#{pad}#{oid.end_with?("112") ? "ED25519" : "ED448"} Public-Key:\n#{pad}pub:\n" + buf_print(bits, indent + 4)
      else
        "#{pad}Unable to load Public Key\n"
      end
    rescue StandardError
      "#{pad}Unable to load Public Key\n"
    end

    # X509V3_extensions_print
    def exts_text(exts, indent)
      return "" if exts.empty?
      s = +"#{" " * indent}X509v3 extensions:\n"
      exts.each do |e|
        dotted = e.__kenny_dotted
        s << " " * (indent + 4) << ln(dotted) << ": #{e.critical? ? "critical" : ""}\n"
        v = Kenny::ExtPrint.value(dotted, e.value_der, indent + 8) || " " * (indent + 8) + Kenny::ExtPrint.raw(e.value_der)
        s << v.b << "\n"
      end
      s
    end

    # X509_signature_print
    def sig_text(alg_tlv, sig)
      s = +"    Signature Algorithm: #{alg_tlv ? ln(alg_id(alg_tlv)[0]) : "NULL"}"
      return s << "\n" if sig.nil?
      s << "\n    Signature Value:"
      return s << "\n\n" if sig.empty?
      s << "\n" << sig.unpack1("H*").scan(/../).each_slice(18).map { |l| "        " + l.join(":") }.join(":\n") << "\n"
    end

    # ossl_serial_number_print
    def serial_text(v)
      return " (Empty)" if v.nil?
      return " #{v < 0 ? "-" : ""}#{v.abs} (#{v < 0 ? "-" : ""}0x#{v.abs.to_s(16)})" if v >= -(1 << 63) && v < 1 << 63
      h = v.abs.to_s(16)
      h = "0" + h if h.size.odd?
      "\n            #{v < 0 ? " (Negative)" : ""}" + h.scan(/../).join(":")
    end

    def to_bn_int(v)
      case v
      when OpenSSL::BN then v.to_i
      when Integer then v
      else raise TypeError, "Cannot convert into OpenSSL::BN"
      end
    end

    def name_arg(n)
      raise TypeError, "wrong argument type #{Kenny::DER.class_name(n)} (expected OpenSSL/X509/NAME)" unless n.is_a?(OpenSSL::X509::Name)
      n.to_der
    end

    def time_arg(t)
      t = Time.at(Integer(t)) unless t.is_a?(Time)
      Kenny::DER.time(t)
    end

    def time_of(tlv)
      return nil unless tlv
      b, c = Kenny::DER.unwrap(tlv)
      Kenny::DER.time_value(b, c)
    end

    # the Time Ruby code gets (asn1time_to_time: 3.x reads UTCTime years before 69 as 20xx)
    def ruby_time_of(tlv)
      return nil unless tlv
      b, c = Kenny::DER.unwrap(tlv)
      OpenSSL::ASN1.__kenny_time(b, c)
    end

    def ext_arg(e)
      raise TypeError, "wrong argument type #{Kenny::DER.class_name(e)} (expected OpenSSL/X509/EXTENSION)" unless e.is_a?(OpenSSL::X509::Extension)
      e.to_der
    end

    # [Extension DER, ...] of an Extensions SEQUENCE's content
    def exts_of(c)
      Kenny::DER.children(c).map { |_, _, tlv| OpenSSL::X509::Extension.new(tlv) && tlv }
    end

    # PEM text, DER, or first DER then PEM (d2i_X509_bio, then PEM_read_bio_X509)
    def read_der_or_pem(arg, labels, err, what)
      s = input(arg)
      begin
        b, = Kenny::DER.first(s)
        return Kenny::DER.first(s)[2] if b == 0x30 && yield(Kenny::DER.first(s)[2])
      rescue OpenSSL::ASN1::ASN1Error, err
        nil
      end
      pem_blocks(s).each do |label, der|
        next unless labels.include?(label)
        raise err, "PEM_read_bio_#{what}: bad base64 decode" if der.nil?
        return der
      end
      raise err, "PEM_read_bio_#{what}: no start line (Expecting: #{labels[0]})"
    end
  end
end

module OpenSSL
  module X509
    class Certificate
      U = Kenny::X509Util
      D = Kenny::DER
      private_constant :U, :D

      def self.load(data)
        s = Kenny::X509Util.input(data)
        # (DER first: it may contain bytes that look like PEM)
        certs = []
        pos = 0
        while pos < s.bytesize
          begin
            _, _, _, hlen, len = Kenny::DER.read_header(s, pos, s.bytesize - pos)
            certs << new(s.byteslice(pos, hlen + len))
            pos += hlen + len
          rescue OpenSSL::ASN1::ASN1Error, CertificateError
            break
          end
        end
        return certs unless certs.empty?
        Kenny::X509Util.pem_blocks(s).each do |label, der|
          next unless label == "CERTIFICATE" || label == "X509 CERTIFICATE"
          raise CertificateError, "ASN1 lib" if der.nil?
          begin
            certs << new(der)
          rescue CertificateError
            raise CertificateError, "ASN1 lib"
          end
        end
        raise CertificateError, "Could not detect format of certificate data!" if certs.empty?
        certs
      end

      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..1)" if args.size > 1
        @version = 0
        @serial = nil
        @tbs_alg = nil
        @issuer = Name.new.to_der
        @subject = @issuer
        @not_before = @not_after = nil
        @spki = nil
        @uids = []
        @exts = []
        @sig_alg = nil
        @sig = nil
        @der = nil
        return if args.empty?
        der = U.read_der_or_pem(args[0], ["CERTIFICATE", "X509 CERTIFICATE"], CertificateError, "X509") { |d| __kenny_parse(d) }
        __kenny_parse(der) or raise CertificateError, "PEM_read_bio_X509: ASN1 lib"
      end

      def initialize_copy(other)
        super
        @exts = @exts.dup
      end

      def version = @version

      def version=(v)
        v = Integer(v)
        raise CertificateError, "version must be >= 0!" if v < 0
        __kenny_modified
        @version = v
      end

      def serial = OpenSSL::BN.new(@serial || 0)

      def serial=(v)
        v = U.to_bn_int(v)
        __kenny_modified
        @serial = v
      end

      def subject = Name.new(@subject)
      def issuer = Name.new(@issuer)

      def subject=(n)
        d = U.name_arg(n)
        __kenny_modified
        @subject = d
      end

      def issuer=(n)
        d = U.name_arg(n)
        __kenny_modified
        @issuer = d
      end

      def not_before = __kenny_time(@not_before)
      def not_after = __kenny_time(@not_after)

      def not_before=(t)
        v = U.time_arg(t)
        __kenny_modified
        @not_before = v
      end

      def not_after=(t)
        v = U.time_arg(t)
        __kenny_modified
        @not_after = v
      end

      # (OpenSSL 3 keeps the key it was given, private parts and all, until the certificate
      # is read back from DER)
      def public_key
        return @pkey.dup if @pkey
        raise CertificateError, "decode error" unless @spki
        U.pkey(@spki, CertificateError)
      end

      def public_key=(key)
        d = U.key_spki(key)
        __kenny_modified
        @spki = d
        @pkey = key.dup
      end

      def signature_algorithm
        unless @sig_alg
          raise OpenSSLError, "OBJ_obj2txt" if OpenSSL::VERSION >= "4"
          return +"NULL"
        end
        +U.ln(U.alg_id(@sig_alg)[0])
      end

      def extensions = @exts.map { |d| Extension.new(d) }

      def extensions=(exts)
        ders = Array(exts).map { |e| U.ext_arg(e) }
        __kenny_modified
        @exts = ders
      end

      def add_extension(ext)
        d = U.ext_arg(ext)
        __kenny_modified
        @exts << d
        ext
      end

      def sign(key, digest)
        raise CertificateError, "internal error" unless @spki
        alg, tbs, sig = U.sign(key, digest, ->(a) { @tbs_alg = a; __kenny_tbs_build })
        @sig_alg = alg
        @sig = sig
        @der = D.seq(tbs, alg, D.tlv(3, "\x00".b + sig))
        self
      end

      def verify(key)
        spki = U.key_spki(key)
        raise CertificateError, "unknown signature algorithm" unless @sig_alg && @sig
        U.verify(spki, @sig_alg, __kenny_tbs, @sig, CertificateError)
      end

      def check_private_key(key)
        spki = U.key_spki(key)
        return false unless @spki && key.private?
        spki == @spki
      end

      def to_der
        @der || D.seq(__kenny_tbs, @sig_alg, D.tlv(3, "\x00".b + (@sig || "".b)))
      rescue OpenSSL::ASN1::ASN1Error
        raise CertificateError, "illegal zero content"
      end

      def to_pem
        U.pem("CERTIFICATE", to_der)
      rescue CertificateError
        raise CertificateError, "ASN1 lib"
      end
      alias to_s to_pem

      def tbs_bytes = __kenny_tbs

      def ==(other)
        other.is_a?(Certificate) && to_der == other.to_der
      end

      def inspect
        "#<#{self.class}: subject=#{subject.inspect}, issuer=#{issuer.inspect}, serial=#{serial.inspect}, " \
          "not_before=#{(not_before.inspect rescue "(error)")}, not_after=#{(not_after.inspect rescue "(error)")}>"
      end

      # X509_print
      def to_text
        s = +"Certificate:\n    Data:\n"
        s << (@version <= 2 ? "        Version: #{@version + 1} (0x#{@version.to_s(16)})\n" : "        Version: Unknown (#{@version})\n")
        s << "        Serial Number:" << U.serial_text(@serial) << "\n"
        s << "    " << U.sig_text(@tbs_alg, nil)
        s << "        Issuer: " << Name.new(@issuer).to_s(Name::COMPAT) << "\n"
        s << "        Validity\n"
        s << "            Not Before: " << U.time_text(U.time_of(@not_before)) << "\n"
        s << "            Not After : " << U.time_text(U.time_of(@not_after)) << "\n"
        s << "        Subject: " << Name.new(@subject).to_s(Name::COMPAT) << "\n"
        s << "        Subject Public Key Info:\n"
        if @spki
          s << "            Public Key Algorithm: " << U.ln(U.spki_parts(@spki)[0]) << "\n" << U.key_text(@spki, 16)
        else
          s << "            Public Key Algorithm: NULL\n            Unable to load Public Key\n"
        end
        s << U.exts_text(extensions, 8)
        s << U.sig_text(@sig_alg, @sig || "".b)
        s
      end

      # internals for the Store and ExtensionFactory
      def __kenny_spki = @spki
      def __kenny_subject_der = @subject
      def __kenny_issuer_der = @issuer
      def __kenny_sig_alg = @sig_alg
      def __kenny_sig = @sig
      def __kenny_tbs_alg = @tbs_alg
      def __kenny_serial_i = @serial
      def __kenny_not_before = U.time_of(@not_before)
      def __kenny_not_after = U.time_of(@not_after)
      def __kenny_ext_ders = @exts

      private

      def __kenny_time(tlv)
        return U.ruby_time_of(tlv) if tlv
        raise OpenSSL::ASN1::ASN1Error, "ASN1_TIME_to_tm" if OpenSSL::VERSION >= "4"
        nil
      end

      def __kenny_modified
        @der = nil
        @tbs = nil
      end

      def __kenny_tbs
        @tbs ||= __kenny_tbs_build
      end

      def __kenny_tbs_build
        raise OpenSSL::ASN1::ASN1Error, "illegal zero content" unless @tbs_alg && @spki
        ver = @version == 0 ? "".b : D.header(0x80, true, 0, D.int(@version).bytesize) + D.int(@version)
        exts = "".b
        unless @exts.empty?
          body = D.seq(*@exts)
          exts = D.header(0x80, true, 3, body.bytesize) + body
        end
        # (an unset time is an empty ASN1_TIME, which encodes as this odd TLV)
        unset = "\x3f\x80\x00\x00".b
        D.seq(ver, D.int(@serial || 0), @tbs_alg, @issuer, D.seq(@not_before || unset, @not_after || unset), @subject, @spki, *@uids, exts)
      end

      # d2i_X509: true when der is a certificate (fields set), false otherwise
      def __kenny_parse(der)
        b, c = D.unwrap(der)
        return false unless b == 0x30
        top = D.children(c)
        return false unless top.size == 3 && top[0][0] == 0x30 && top[1][0] == 0x30 && top[2][0] == 3
        tbs = D.children(top[0][1])
        version = 0
        if tbs[0] && tbs[0][0] == 0xa0
          v = D.children(tbs.shift[1])
          return false unless v.size == 1 && v[0][0] == 2
          version = D.int_value(v[0][1])
        end
        serial, alg, issuer, validity, subject, spki = tbs.shift(6)
        return false unless spki && serial[0] == 2 && alg[0] == 0x30 && issuer[0] == 0x30 && validity[0] == 0x30 &&
                            subject[0] == 0x30 && spki[0] == 0x30
        times = D.children(validity[1])
        return false unless times.size == 2 && times.all? { |t, tc| (t == 23 || t == 24) && D.time_value(t, tc) }
        Name.new(issuer[2])
        Name.new(subject[2])
        uids = []
        exts = []
        tbs.each do |t, tc, tlv|
          case t
          when 0x81, 0x82, 0xa1, 0xa2 then uids << tlv
          when 0xa3
            inner = D.children(tc)
            return false unless inner.size == 1 && inner[0][0] == 0x30
            exts = U.exts_of(inner[0][1])
          else return false
          end
        end
        bits = top[2][1]
        return false if bits.empty?
        @version = version
        @serial = D.int_value(serial[1])
        @tbs_alg = alg[2]
        @issuer = issuer[2]
        @not_before, @not_after = times.map { |x| x[2] }
        @subject = subject[2]
        @spki = spki[2]
        @uids = uids
        @exts = exts
        @sig_alg = top[1][2]
        @sig = bits.byteslice(1..)
        @tbs = top[0][2]
        @der = D.seq(*top.map { |x| x[2] })
        true
      rescue OpenSSL::ASN1::ASN1Error, NameError, ExtensionError
        false
      end
    end

    class Attribute
      D = Kenny::DER
      private_constant :D

      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 1..2)" unless (1..2).cover?(args.size)
        @oid = nil
        @value = nil
        if args.size == 1
          der = Kenny::X509Util.input(args[0])
          begin
            b, c = D.first(der)
            kids = D.children(c)
            raise AttributeError, "nested asn1 error" unless b == 0x30 && kids.size == 2 && kids[0][0] == 6 && kids[1][0] == 0x31
            @oid = D.oid_dotted(kids[0][1]) or raise AttributeError, "nested asn1 error"
            @value = kids[1][2]
          rescue OpenSSL::ASN1::ASN1Error
            raise AttributeError, "nested asn1 error"
          end
        else
          self.oid = args[0]
          self.value = args[1]
        end
      end

      def oid=(oid)
        s = String.try_convert(oid) or raise TypeError, "no implicit conversion of #{D.class_name(oid)} into String"
        @oid = Kenny::OID.lookup(s) or raise AttributeError, "OBJ_txt2obj: unknown object name"
      end

      def oid = +(Kenny::OID.sn(@oid) || @oid)

      # (d2i_ASN1_SET_ANY of the value's DER)
      def value=(v)
        der = v.respond_to?(:to_der) ? v.to_der : v
        der = String.try_convert(der) or raise TypeError, "no implicit conversion of #{D.class_name(der)} into String"
        b, = begin
          D.first(der.b)
        rescue OpenSSL::ASN1::ASN1Error
          nil
        end
        raise AttributeError, "attribute value must be ASN1::Set: nested asn1 error" unless b == 0x31
        @value = D.first(der.b)[2]
      end

      def value = OpenSSL::ASN1.decode(@value)

      def to_der
        D.seq(D.tlv(6, D.oid_content(@oid)), @value)
      end

      def ==(other)
        other.is_a?(Attribute) && to_der == other.to_der
      end
    end

    class Request
      U = Kenny::X509Util
      D = Kenny::DER
      private_constant :U, :D

      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..1)" if args.size > 1
        @version = 0
        @subject = Name.new.to_der
        @spki = nil
        @attrs = []
        @sig_alg = nil
        @sig = nil
        @der = nil
        return if args.empty?
        der = U.read_der_or_pem(args[0], ["CERTIFICATE REQUEST", "NEW CERTIFICATE REQUEST"], RequestError, "X509_REQ") { |d| __kenny_parse(d) }
        __kenny_parse(der) or raise RequestError, "PEM_read_bio_X509_REQ: ASN1 lib"
      end

      def initialize_copy(other)
        super
        @attrs = @attrs.dup
      end

      def version = @version

      def version=(v)
        v = Integer(v)
        raise RequestError, "version must be >= 0!" if v < 0
        @der = nil
        @version = v
      end

      def subject = Name.new(@subject)

      def subject=(n)
        d = U.name_arg(n)
        @der = nil
        @subject = d
      end

      def public_key
        raise RequestError, "decode error" unless @spki
        U.pkey(@spki, RequestError)
      end

      def public_key=(key)
        d = U.key_spki(key)
        @der = nil
        @spki = d
      end

      def signature_algorithm
        unless @sig_alg
          raise OpenSSLError, "OBJ_obj2txt" if OpenSSL::VERSION >= "4"
          return +"NULL"
        end
        +U.ln(U.alg_id(@sig_alg)[0])
      end

      def attributes = @attrs.map { |d| Attribute.new(d) }

      def attributes=(attrs)
        ders = Array(attrs).map do |a|
          raise TypeError, "wrong argument type #{D.class_name(a)} (expected OpenSSL/X509/ATTRIBUTE)" unless a.is_a?(Attribute)
          a.to_der
        end
        @der = nil
        @attrs = ders
      end

      def add_attribute(attr)
        raise TypeError, "wrong argument type #{D.class_name(attr)} (expected OpenSSL/X509/ATTRIBUTE)" unless attr.is_a?(Attribute)
        @der = nil
        @attrs << attr.to_der
        attr
      end

      def sign(key, digest)
        alg, info, sig = U.sign(key, digest, ->(_) { __kenny_info })
        @sig_alg = alg
        @sig = sig
        @der = D.seq(info, alg, D.tlv(3, "\x00".b + sig))
        self
      end

      def verify(key)
        spki = U.key_spki(key)
        raise RequestError, "unknown signature algorithm" unless @sig_alg && @sig
        U.verify(spki, @sig_alg, __kenny_info, @sig, RequestError)
      end

      def to_der
        @der || D.seq(__kenny_info, @sig_alg || raise(RequestError, "illegal zero content"), D.tlv(3, "\x00".b + (@sig || "".b)))
      end

      def to_pem
        U.pem("CERTIFICATE REQUEST", to_der)
      end
      alias to_s to_pem

      def to_text
        s = +"Certificate Request:\n    Data:\n"
        s << "        Version: #{@version + 1} (0x#{@version.to_s(16)})\n"
        s << "        Subject: " << Name.new(@subject).to_s(Name::COMPAT) << "\n"
        s << "        Subject Public Key Info:\n"
        if @spki
          s << "            Public Key Algorithm: " << U.ln(U.spki_parts(@spki)[0]) << "\n" << U.key_text(@spki, 16)
        else
          s << "            Public Key Algorithm: NULL\n            Unable to load Public Key\n"
        end
        s << "        Attributes:\n"
        if @attrs.empty?
          s << "            (none)\n"
        else
          attributes.each do |a|
            s << "            " << U.ln(Kenny::OID.lookup(a.oid)) << " :"
            a.value.value.each { |v| s << (v.value.is_a?(String) ? v.value : v.to_der.unpack1("H*")) }
            s << "\n"
          end
        end
        s << "        Requested Extensions:\n" if false
        s << U.sig_text(@sig_alg, @sig || "".b)
        s
      end

      def __kenny_spki = @spki

      private

      def __kenny_info
        raise RequestError, "illegal zero content" unless @spki
        attrs = @attrs.sort.join.b
        D.seq(D.int(@version), @subject, @spki, D.header(0x80, true, 0, attrs.bytesize) + attrs)
      end

      def __kenny_parse(der)
        b, c = D.unwrap(der)
        return false unless b == 0x30
        top = D.children(c)
        return false unless top.size == 3 && top[0][0] == 0x30 && top[1][0] == 0x30 && top[2][0] == 3
        ver, subj, spki, attrs = D.children(top[0][1])
        return false unless ver && ver[0] == 2 && subj && subj[0] == 0x30 && spki && spki[0] == 0x30
        Name.new(subj[2])
        @version = D.int_value(ver[1])
        @subject = subj[2]
        @spki = spki[2]
        @attrs = attrs && attrs[0] == 0xa0 ? D.children(attrs[1]).map { |x| x[2] } : []
        @sig_alg = top[1][2]
        @sig = top[2][1].byteslice(1..)
        @der = D.seq(*top.map { |x| x[2] })
        true
      rescue OpenSSL::ASN1::ASN1Error, NameError
        false
      end
    end

    class Revoked
      U = Kenny::X509Util
      D = Kenny::DER
      private_constant :U, :D

      def initialize(*args)
        @serial = 0
        @time = nil
        @exts = []
        return if args.empty?
        der = U.input(args[0])
        b, c = D.first(der)
        kids = D.children(c)
        raise RevokedError, "nested asn1 error" unless b == 0x30 && kids.size >= 2 && kids[0][0] == 2
        @serial = D.int_value(kids[0][1])
        @time = kids[1][2]
        @exts = kids[2] ? U.exts_of(kids[2][1]) : []
      rescue OpenSSL::ASN1::ASN1Error
        raise RevokedError, "nested asn1 error"
      end

      def serial = OpenSSL::BN.new(@serial)
      def serial=(v)
        @serial = U.to_bn_int(v)
      end

      def time = U.ruby_time_of(@time)
      def time=(t)
        @time = U.time_arg(t)
      end

      def extensions = @exts.map { |d| Extension.new(d) }
      def extensions=(exts)
        @exts = Array(exts).map { |e| U.ext_arg(e) }
      end

      def add_extension(ext)
        @exts << U.ext_arg(ext)
        ext
      end

      def to_der
        raise RevokedError, "illegal zero content" unless @time
        D.seq(D.int(@serial), @time, @exts.empty? ? "".b : D.seq(*@exts))
      end

      def __kenny_serial_i = @serial
    end

    class CRL
      U = Kenny::X509Util
      D = Kenny::DER
      private_constant :U, :D

      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..1)" if args.size > 1
        @version = 0
        @tbs_alg = nil
        @issuer = Name.new.to_der
        @last = @next = nil
        @revoked = []
        @exts = []
        @sig_alg = @sig = @der = nil
        return if args.empty?
        der = U.read_der_or_pem(args[0], ["X509 CRL"], CRLError, "X509_CRL") { |d| __kenny_parse(d) }
        __kenny_parse(der) or raise CRLError, "PEM_read_bio_X509_CRL: ASN1 lib"
      end

      def initialize_copy(other)
        super
        @revoked = @revoked.dup
        @exts = @exts.dup
      end

      def version = @version

      def version=(v)
        v = Integer(v)
        raise CRLError, "version must be >= 0!" if v < 0
        @der = nil
        @version = v
      end

      def signature_algorithm
        unless @sig_alg
          raise OpenSSLError, "OBJ_obj2txt" if OpenSSL::VERSION >= "4"
          return +"NULL"
        end
        +U.ln(U.alg_id(@sig_alg)[0])
      end

      def issuer = Name.new(@issuer)

      def issuer=(n)
        d = U.name_arg(n)
        @der = nil
        @issuer = d
      end

      def last_update = U.ruby_time_of(@last)
      def next_update = U.ruby_time_of(@next)
      def __kenny_last_update = U.time_of(@last)
      def __kenny_next_update = U.time_of(@next)

      def last_update=(t)
        v = U.time_arg(t)
        @der = nil
        @last = v
      end

      def next_update=(t)
        v = U.time_arg(t)
        @der = nil
        @next = v
      end

      def revoked = @revoked.map { |d| Revoked.new(d) }

      def revoked=(list)
        ders = Array(list).map do |r|
          raise TypeError, "wrong argument type #{D.class_name(r)} (expected OpenSSL/X509/REVOKED)" unless r.is_a?(Revoked)
          r.to_der
        end
        @der = nil
        @revoked = ders
      end

      def add_revoked(r)
        raise TypeError, "wrong argument type #{D.class_name(r)} (expected OpenSSL/X509/REVOKED)" unless r.is_a?(Revoked)
        @der = nil
        @revoked << r.to_der
        r
      end

      def extensions = @exts.map { |d| Extension.new(d) }

      def extensions=(exts)
        ders = Array(exts).map { |e| U.ext_arg(e) }
        @der = nil
        @exts = ders
      end

      def add_extension(ext)
        d = U.ext_arg(ext)
        @der = nil
        @exts << d
        ext
      end

      def sign(key, digest)
        alg, tbs, sig = U.sign(key, digest, ->(a) { @tbs_alg = a; __kenny_tbs })
        @sig_alg = alg
        @sig = sig
        @der = D.seq(tbs, alg, D.tlv(3, "\x00".b + sig))
        self
      end

      def verify(key)
        spki = U.key_spki(key)
        raise CRLError, "unknown signature algorithm" unless @sig_alg && @sig
        U.verify(spki, @sig_alg, __kenny_tbs, @sig, CRLError)
      end

      def to_der
        @der || D.seq(__kenny_tbs, @sig_alg || raise(CRLError, "illegal zero content"), D.tlv(3, "\x00".b + (@sig || "".b)))
      end

      def to_pem = U.pem("X509 CRL", to_der)
      alias to_s to_pem

      def to_text = raise(NotImplementedError, "OpenSSL::X509::CRL#to_text is not supported by Kenny")

      def ==(other)
        other.is_a?(CRL) && to_der == other.to_der
      end

      def __kenny_issuer_der = @issuer
      def __kenny_tbs_bytes = __kenny_tbs
      def __kenny_sig_alg = @sig_alg
      def __kenny_sig = @sig
      def __kenny_revoked_serials = @revoked.map { |d| Revoked.new(d).__kenny_serial_i }

      private

      def __kenny_tbs
        @tbs ||= nil
        return @tbs if @der && @tbs
        raise CRLError, "illegal zero content" unless @tbs_alg && @last
        ver = @version == 0 ? "".b : D.int(@version)
        exts = "".b
        unless @exts.empty?
          body = D.seq(*@exts)
          exts = D.header(0x80, true, 0, body.bytesize) + body
        end
        D.seq(ver, @tbs_alg, @issuer, @last, @next || "".b, @revoked.empty? ? "".b : D.seq(*@revoked), exts)
      end

      def __kenny_parse(der)
        b, c = D.unwrap(der)
        return false unless b == 0x30
        top = D.children(c)
        return false unless top.size == 3 && top[0][0] == 0x30 && top[1][0] == 0x30 && top[2][0] == 3
        tbs = D.children(top[0][1])
        version = tbs[0] && tbs[0][0] == 2 ? D.int_value(tbs.shift[1]) : 0
        alg, issuer, last = tbs.shift(3)
        return false unless last && alg[0] == 0x30 && issuer[0] == 0x30 && (last[0] == 23 || last[0] == 24)
        nxt = tbs[0] && (tbs[0][0] == 23 || tbs[0][0] == 24) ? tbs.shift[2] : nil
        revoked = tbs[0] && tbs[0][0] == 0x30 ? D.children(tbs.shift[1]).map { |x| x[2] } : []
        exts = tbs[0] && tbs[0][0] == 0xa0 ? U.exts_of(D.children(tbs.shift[1])[0][1]) : []
        return false unless tbs.empty?
        @version = version
        @tbs_alg = alg[2]
        @issuer = issuer[2]
        @last = last[2]
        @next = nxt
        @revoked = revoked
        @exts = exts
        @sig_alg = top[1][2]
        @sig = top[2][1].byteslice(1..)
        @tbs = top[0][2]
        @der = D.seq(*top.map { |x| x[2] })
        true
      rescue OpenSSL::ASN1::ASN1Error, NameError, ExtensionError
        false
      end
    end
  end
end
