# frozen_string_literal: true
# Kenny: OpenSSL::X509::Extension and ExtensionFactory (ossl_x509ext.c). Values print the way
# X509V3_EXT_print does (crypto/x509/v3_*.c); create_ext reads OpenSSL's config syntax for the
# common extensions.

module Kenny
  # X509V3_EXT_print and its per-extension printers
  module ExtPrint
    D = Kenny::DER
    KU_NAMES = ["Digital Signature", "Non Repudiation", "Key Encipherment", "Data Encipherment", "Key Agreement",
                "Certificate Sign", "CRL Sign", "Encipher Only", "Decipher Only"].freeze
    KU_SNAMES = %w[digitalSignature nonRepudiation keyEncipherment dataEncipherment keyAgreement keyCertSign cRLSign
                   encipherOnly decipherOnly].freeze
    NS_NAMES = ["SSL Client", "SSL Server", "S/MIME", "Object Signing", "Unused", "SSL CA", "S/MIME CA",
                "Object Signing CA"].freeze
    NS_SNAMES = %w[client server email objsign reserved sslCA emailCA objCA].freeze
    REASONS = ["Unused", "Key Compromise", "CA Compromise", "Affiliation Changed", "Superseded",
               "Cessation Of Operation", "Certificate Hold", "Privilege Withdrawn", "AA Compromise"].freeze
    IA5_EXTS = %w[2.16.840.1.113730.1.2 2.16.840.1.113730.1.3 2.16.840.1.113730.1.4 2.16.840.1.113730.1.7
                  2.16.840.1.113730.1.8 2.16.840.1.113730.1.12 2.16.840.1.113730.1.13].freeze

    class Fail < StandardError; end

    module_function

    def fail! = raise(Fail)

    def expect(tlv_or_kid, byte)
      fail! unless tlv_or_kid && tlv_or_kid[0] == byte
      tlv_or_kid[1]
    end

    # the content of a TLV that must be the only thing in s, with tag byte b
    def content(s, b)
      t, c = D.first(s)
      fail! unless t == b
      c
    end

    # [[tag byte, content, tlv], ...] of a SEQUENCE TLV
    def seq_items(s)
      D.children(content(s, 0x30))
    end

    def bits(c)
      fail! if c.empty?
      unused = c.getbyte(0)
      fail! if unused > 7
      data = c.byteslice(1..)
      ->(n) { n / 8 < data.bytesize && data.getbyte(n / 8)[7 - n % 8] == 1 }
    end

    def obj_text(c)
      oid = D.oid_dotted(c) or fail!
      Kenny::X509Util.ln(oid)
    end

    def hex(c) = Kenny::X509Util.hex_colon(c)

    # BIO_printf's "%.*s": the bytes up to the first NUL
    def cstr(b) = b.to_s.b.split("\x00", 2)[0].to_s

    # i2s_ASN1_INTEGER: decimal, or hex from 128 bits on
    def int_text(c)
      v = D.int_value(c)
      v.abs.bit_length >= 128 ? (v < 0 ? "-0x" : "0x") + v.abs.to_s(16).upcase : v.to_s
    rescue OpenSSL::ASN1::ASN1Error
      fail!
    end

    def name_oneline(tlv)
      OpenSSL::X509::Name.new(tlv).to_s
    rescue OpenSSL::X509::NameError
      fail!
    end

    def name_print(tlv)
      OpenSSL::X509::Name.new(tlv).to_s(OpenSSL::X509::Name::ONELINE)
    rescue OpenSSL::X509::NameError
      fail!
    end

    # one GeneralName: i2v_GENERAL_NAME's text (r: GENERAL_NAME_print's)
    def gen_name(b, c, tlv, r = false)
      case b
      when 0xa0
        kids = D.children(c)
        oid = D.oid_dotted(expect(kids[0], 6)) or fail!
        label = { "1.3.6.1.4.1.311.20.2.3" => "UPN", "1.3.6.1.5.5.7.8.5" => "XmppAddr", "1.3.6.1.5.5.7.8.7" => "SRVName",
                  "1.3.6.1.5.5.7.8.9" => "SmtpUTF8Mailbox", "1.3.6.1.5.5.7.8.8" => "NAIRealm" }[oid] || Kenny::X509Util.ln(oid)
        inner = kids[1] && kids[1][0] == 0xa0 ? D.children(kids[1][1])[0] : nil
        if inner && [12, 22].include?(inner[0])
          "othername: #{label}:#{inner[1]}"
        else
          "othername: #{label}:<unsupported>"
        end
      when 0x81 then "email:#{c}"
      when 0x82 then "DNS:#{c}"
      when 0x86 then "URI:#{c}"
      when 0xa3 then "X400Name:<unsupported>"
      when 0xa5 then "EdiPartyName:<unsupported>"
      when 0xa4
        n = D.children(c)[0] or fail!
        "DirName:" + (r ? name_print(n[2]) : name_oneline(n[2]))
      when 0x87 then "IP Address:" + Kenny::X509Util.ip_text(c)
      when 0x88 then "Registered ID:" + obj_text(c)
      else fail!
      end
    end

    def gen_names(c, r = false)
      D.children(c).map { |b, cc, tlv| gen_name(b, cc, tlv, r) }
    end

    # X509V3_EXT_val_prn
    def val_prn(list, indent, ml)
      return " " * indent + "<EMPTY>\n" if list.empty?
      if ml
        list.map { |x| " " * indent + x }.join("\n")
      else
        " " * indent + list.join(", ")
      end
    end

    def print_gens(gens, indent)
      gens.map { |g| " " * (indent + 2) + g }.join("\n")
    end

    def print_reasons(name, c, indent)
      bit = bits(c)
      names = REASONS.each_index.select { |i| bit.(i) }.map { |i| REASONS[i] }
      "#{" " * indent}#{name}:\n#{" " * (indent + 2)}" + (names.empty? ? "<EMPTY>\n" : names.join(", ") + "\n")
    end

    def print_distpoint(b, c, indent)
      kid = D.children(c)[0] or fail!
      s = +""
      if kid[0] == 0xa0
        s << "#{" " * indent}Full Name:\n" << print_gens(gen_names(kid[1], true), indent)
      elsif kid[0] == 0xa1
        rdn = OpenSSL::X509::Name.new(D.seq(D.tlv(17, kid[1], true)))
        s << "#{" " * indent}Relative Name:\n#{" " * (indent + 2)}" << rdn.to_s(OpenSSL::X509::Name::ONELINE)
      else
        fail!
      end
      s << "\n"
    end

    def crldp(der, indent)
      seq_items(der).map do |_, dp|
        s = +""
        D.children(dp).each do |b, c|
          case b
          when 0xa0 then s << print_distpoint(b, c, indent)
          when 0x81 then s << print_reasons("Reasons", c, indent)
          when 0xa2 then s << "#{" " * indent}CRL Issuer:\n" << print_gens(gen_names(c, true), indent)
          else fail!
          end
        end
        s
      end.join("\n")
    end

    def idp(der, indent)
      s = +""
      seq_items(der).each do |b, c|
        on = ->() { c.bytesize == 1 && c.getbyte(0) != 0 }
        case b
        when 0xa0 then s << print_distpoint(b, c, indent)
        when 0x81 then s << "#{" " * indent}Only User Certificates\n" if on.()
        when 0x82 then s << "#{" " * indent}Only CA Certificates\n" if on.()
        when 0x83 then s << print_reasons("Only Some Reasons", c, indent)
        when 0x84 then s << "#{" " * indent}Indirect CRL\n" if on.()
        when 0x85 then s << "#{" " * indent}Only Attribute Certificates\n" if on.()
        else fail!
        end
      end
      s.empty? ? "#{" " * indent}<EMPTY>\n" : s
    end

    def certpol(der, indent)
      seq_items(der).map do |_, pi|
        kids = D.children(pi)
        s = "#{" " * indent}Policy: " + obj_text(expect(kids[0], 6))
        if kids[1]
          s += "\n" + D.children(expect(kids[1], 0x30)).map { |_, q| qualifier(q, indent + 2) }.join("\n")
        end
        s
      end.join("\n")
    end

    def qualifier(q, indent)
      kids = D.children(q)
      oid = D.oid_dotted(expect(kids[0], 6))
      case oid
      when "1.3.6.1.5.5.7.2.1" then "#{" " * indent}CPS: #{cstr(kids[1] && kids[1][1])}"
      when "1.3.6.1.5.5.7.2.2"
        s = "#{" " * indent}User Notice:\n"
        un = kids[1] ? D.children(expect(kids[1], 0x30)) : []
        ref = un.find { |k| k[0] == 0x30 }
        text = un.find { |k| k[0] != 0x30 }
        ind = " " * (indent + 2)
        if ref
          org, nums = D.children(ref[1])
          ns = D.children(expect(nums, 0x30)).map { |_, c| int_text(c) }
          s += "#{ind}Organization: #{cstr(org[1])}\n#{ind}Number#{ns.size > 1 ? "s" : ""}: #{ns.join(", ")}"
          s += "\n" if text
        end
        s += "#{ind}Explicit Text: #{cstr(text[1])}" if text
        s
      else "#{" " * (indent + 2)}Unknown Qualifier: " + Kenny::X509Util.ln(oid.to_s)
      end
    end

    def name_constraints(der, indent)
      parts = seq_items(der).map do |b, c|
        trees = D.children(c)
        next "" if trees.empty?
        "#{" " * indent}#{b == 0xa0 ? "Permitted" : "Excluded"}:\n" + trees.map do |_, st|
          gb, gc, gt = D.children(st)[0]
          " " * (indent + 2) + if gb == 0x87
            if gc.bytesize == 8
              "IP:#{gc.byteslice(0, 4).bytes.join(".")}/#{gc.byteslice(4, 4).bytes.join(".")}"
            elsif gc.bytesize == 32
              "IP:#{Kenny::X509Util.ip_text(gc.byteslice(0, 16))}/#{Kenny::X509Util.ip_text(gc.byteslice(16, 16))}"
            else
              "IP Address:<invalid>"
            end
          else
            gen_name(gb, gc, gt, true)
          end
        end.join("\n")
      end
      parts.join("\n")
    end

    # the printed value (X509V3_EXT_print(out, ext, 0, indent)), nil when OpenSSL wouldn't
    # print it (no method, or undecodable)
    def value(oid, der, indent = 0)
      pad = " " * indent
      case oid
      when "2.5.29.19"
        kids = seq_items(der)
        ca = false
        if kids[0] && kids[0][0] == 1
          ca = kids.shift[1].getbyte(0) != 0
        end
        list = ["CA:#{ca ? "TRUE" : "FALSE"}"]
        list << "pathlen:#{int_text(expect(kids[0], 2))}" if kids[0]
        val_prn(list, indent, false)
      when "2.5.29.15", "2.16.840.1.113730.1.1"
        names = oid == "2.5.29.15" ? KU_NAMES : NS_NAMES
        bit = bits(content(der, 3))
        list = names.each_index.select { |i| bit.(i) }.map { |i| names[i] }
        fail! if list.empty?
        val_prn(list, indent, false)
      when "2.5.29.37"
        val_prn(seq_items(der).map { |_, c| obj_text(c) }, indent, false)
      when "2.5.29.14" then pad + hex(content(der, 4))
      when "2.5.29.35"
        list = []
        kids = seq_items(der)
        more = kids.any? { |b,| b != 0x80 }
        kids.each do |b, c|
          case b
          when 0x80 then list << (more ? "keyid:" : "") + hex(c)
          when 0xa1 then list.concat(gen_names(c))
          when 0x82
            # (ASN1_INTEGER keeps the magnitude)
            v = D.int_value(c).abs
            list << "serial:" + hex(v == 0 ? "\x00".b : [v.to_s(16).rjust((v.bit_length + 7) / 8 * 2, "0")].pack("H*"))
          else fail!
          end
        end
        fail! if list.empty?
        val_prn(list, indent, true)
      when "2.5.29.17", "2.5.29.18" then val_prn(gen_names(content(der, 0x30)), indent, false)
      when "1.3.6.1.5.5.7.1.1", "1.3.6.1.5.5.7.1.11"
        list = seq_items(der).map do |_, ad|
          kids = D.children(ad)
          "#{obj_text(expect(kids[0], 6))} - #{gen_name(*(kids[1] || fail!).first(3))}"
        end
        val_prn(list, indent, true)
      when "2.5.29.31", "2.5.29.46" then crldp(der, indent)
      when "2.5.29.28" then idp(der, indent)
      when "2.5.29.32" then certpol(der, indent)
      when "2.5.29.30" then name_constraints(der, indent)
      when "2.5.29.36"
        list = seq_items(der).map do |b, c|
          case b
          when 0x80 then "Require Explicit Policy:#{int_text(c)}"
          when 0x81 then "Inhibit Policy Mapping:#{int_text(c)}"
          else fail!
          end
        end
        val_prn(list, indent, false)
      when "2.5.29.54", "2.5.29.20", "2.5.29.27" then pad + int_text(content(der, 2))
      when "2.5.29.16"
        s = pad.dup
        kids = seq_items(der)
        kids.each_with_index do |(b, c), i|
          t = Kenny::DER.time_value(24, c)
          s << (b == 0x80 ? "Not Before: " : "Not After: ") << Kenny::X509Util.time_text(t)
          s << ", " if i == 0 && kids.size > 1
        end
        s
      when "1.3.6.1.5.5.7.1.24"
        list = seq_items(der).map do |_, c|
          v = D.int_value(c)
          { 5 => "status_request", 17 => "status_request_v2" }[v] || v.to_s
        end
        val_prn(list, indent, false)
      when "1.3.6.1.4.1.11129.2.4.2", "1.3.6.1.4.1.11129.2.4.5" then scts(content(der, 4), indent)
      when "2.5.29.9"
        fail! unless seq_items(der).empty?
        pad + "<Empty Attributes>\n"
      when "1.3.6.1.4.1.11129.2.4.3"
        content(der, 5)
        pad + "NULL"
      when "1.3.6.1.5.5.7.48.1.5" then content(der, 5) && pad
      when *IA5_EXTS then pad + content(der, 22)
      when "2.16.840.1.113730.1.13" then pad + content(der, 22)
      else nil
      end
    rescue Fail, OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError, ArgumentError
      nil
    end

    # BIO_hex_string
    def hex_string(b, indent, width = 16)
      out = +""
      b.bytes.each_with_index do |x, i|
        out << " " * indent if i > 0 && i % width == 0
        out << format("%02X", x)
        out << ":" if i < b.bytesize - 1
        out << "\n" if i < b.bytesize - 1 && (i + 1) % width == 0
      end
      out
    end

    SCT_SIGS = { [4, 3] => "ecdsa-with-SHA256", [4, 1] => "sha256WithRSAEncryption", [5, 3] => "ecdsa-with-SHA384",
                 [6, 3] => "ecdsa-with-SHA512", [5, 1] => "sha384WithRSAEncryption", [6, 1] => "sha512WithRSAEncryption" }.freeze

    # SCT_LIST_print (crypto/ct/ct_prn.c), from the TLS encoding
    def scts(c, indent)
      fail! if c.bytesize < 2
      total = c.unpack1("n")
      fail! unless total == c.bytesize - 2
      pos = 2
      items = []
      while pos < c.bytesize
        len = c.byteslice(pos, 2).unpack1("n")
        sct = c.byteslice(pos + 2, len)
        fail! unless sct && sct.bytesize == len
        pos += 2 + len
        items << sct_text(sct, indent)
      end
      items.join("\n")
    end

    def sct_text(s, indent)
      p4 = " " * (indent + 4)
      out = "#{" " * indent}Signed Certificate Timestamp:\n#{p4}Version   : "
      return out + "unknown\n#{" " * (indent + 16)}" + hex_string(s, indent + 16) if s.getbyte(0) != 0
      log_id = s.byteslice(1, 32)
      ms = s.byteslice(33, 8).unpack1("Q>")
      ext_len = s.byteslice(41, 2).unpack1("n")
      ext = s.byteslice(43, ext_len)
      hash, sig_alg, sig_len = s.byteslice(43 + ext_len, 4).unpack("CCn")
      sig = s.byteslice(47 + ext_len, sig_len)
      fail! unless sig && sig.bytesize == sig_len
      t = Time.at(ms / 1000, ms % 1000, :millisecond).utc
      out + "v1 (0x0)\n#{p4}Log ID    : " + hex_string(log_id, indent + 16) +
        "\n#{p4}Timestamp : " + t.strftime("%b %e %H:%M:%S.") + format("%03d", ms % 1000) + t.strftime(" %Y GMT") +
        "\n#{p4}Extensions: " + (ext_len == 0 ? "none" : hex_string(ext, indent + 16)) +
        "\n#{p4}Signature : " + (SCT_SIGS[[hash, sig_alg]] || format("%02X%02X", hash, sig_alg)) +
        "\n#{p4}            " + hex_string(sig, indent + 16)
    end

    # ASN1_STRING_print: the octets, with unprintable bytes as '.'
    def raw(s)
      s.b.bytes.map { |c| c > 126 || (c < 32 && c != 10 && c != 13) ? 46 : c }.pack("C*")
    end
  end
end

module OpenSSL
  module X509
    class Extension
      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 1..3)" unless (1..3).cover?(args.size)
        @oid = "2.5.29.0"
        @value = "".b
        @critical = false
        if args.size == 1
          der = Kenny::X509Util.input(args[0])
          __kenny_parse(der)
        else
          self.oid = args[0]
          self.value = args[1]
          self.critical = args[2] if args.size > 2
        end
      end

      def initialize_copy(other)
        super
        @value = @value.dup
      end

      def oid=(oid)
        s = String.try_convert(oid) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(oid)} into String"
        @oid = Kenny::OID.lookup(s) or raise ExtensionError, "OBJ_txt2obj: unknown object name"
      end

      def value=(data)
        data = data.to_der if data.respond_to?(:to_der)
        s = String.try_convert(data) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(data)} into String"
        @value = s.b
      end

      def critical=(flag)
        @critical = flag ? true : false
      end

      def oid
        +(Kenny::OID.sn(@oid) || @oid)
      end

      def value
        (Kenny::ExtPrint.value(@oid, @value) || Kenny::ExtPrint.raw(@value)).b
      end

      def value_der = @value.dup
      def critical? = @critical

      def to_der
        D.seq(D.tlv(6, D.oid_content(@oid)), @critical ? "\x01\x01\xff".b : "".b, D.tlv(4, @value))
      end

      def __kenny_dotted = @oid

      D = Kenny::DER
      private_constant :D

      private

      def __kenny_parse(der)
        b, c = D.first(der)
        raise ExtensionError, "nested asn1 error (Type=X509_EXTENSION)" unless b == 0x30
        kids = D.children(c)
        oid = kids[0] && kids[0][0] == 6 && D.oid_dotted(kids[0][1])
        raise ExtensionError, "nested asn1 error (Type=X509_EXTENSION)" unless oid
        crit = false
        if kids[1] && kids[1][0] == 1
          crit = kids[1][1].getbyte(0) != 0
          kids.delete_at(1)
        end
        raise ExtensionError, "nested asn1 error (Type=X509_EXTENSION)" unless kids.size == 2 && kids[1][0] == 4
        @oid = oid
        @critical = crit
        @value = kids[1][1]
      rescue OpenSSL::ASN1::ASN1Error
        raise ExtensionError, "nested asn1 error (Type=X509_EXTENSION)"
      end
    end

    class ExtensionFactory
      attr_reader :issuer_certificate, :subject_certificate, :subject_request, :crl, :config

      def initialize(issuer_cert = nil, subject_cert = nil, subject_req = nil, crl = nil)
        self.issuer_certificate = issuer_cert if issuer_cert
        self.subject_certificate = subject_cert if subject_cert
        self.subject_request = subject_req if subject_req
        self.crl = crl if crl
      end

      def issuer_certificate=(cert)
        __kenny_check(cert, Certificate, "OpenSSL/X509")
        @issuer_certificate = cert
      end

      def subject_certificate=(cert)
        __kenny_check(cert, Certificate, "OpenSSL/X509")
        @subject_certificate = cert
      end

      def subject_request=(req)
        __kenny_check(req, Request, "OpenSSL/X509/REQ")
        @subject_request = req
      end

      def crl=(crl)
        __kenny_check(crl, CRL, "OpenSSL/X509/CRL")
        @crl = crl
      end

      def config=(config)
        @config = config
      end

      def create_ext(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 2..3)" unless (2..3).cover?(args.size)
        oid, value, critical = args
        oid = String.try_convert(oid) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(oid)} into String"
        value = String.try_convert(value) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(value)} into String"
        valstr = (critical ? "critical," : "") + value
        v = valstr.dup
        crit = false
        # v3_check_critical
        if v.start_with?("critical,")
          crit = true
          v = v.byteslice(9..).sub(/\A\s+/, "")
        end
        ext = __kenny_build(oid, v, crit)
        raise ExtensionError, "#{oid} = #{valstr}: error in extension (name=#{oid}, value=#{v})" unless ext
        ext
      end

      private

      def __kenny_check(obj, cls, name)
        raise TypeError, "wrong argument type #{Kenny::DER.class_name(obj)} (expected #{name})" unless obj.is_a?(cls)
      end

      D = Kenny::DER
      private_constant :D

      # X509V3_parse_list: "a:b, c" => [["a", "b"], ["c", nil]]
      def __kenny_list(s)
        s.split(",").map do |item|
          n, val = item.split(":", 2)
          [n.strip, val&.strip]
        end
      end

      def __kenny_bool(s)
        return true if %w[TRUE true Y y YES yes].include?(s)
        return false if %w[FALSE false N n NO no].include?(s)
        nil
      end

      def __kenny_subject_spki
        if @subject_certificate
          @subject_certificate.__send__(:__kenny_spki)
        elsif @subject_request
          @subject_request.__send__(:__kenny_spki)
        end
      end

      # v2i_GENERAL_NAME
      def __kenny_gen(name, val, ia5only = false)
        return nil if val.nil?
        case name
        when "email"
          if val == "copy"
            subj = @subject_certificate&.subject || @subject_request&.subject or return nil
            emails = subj.__kenny_entries.select { |o,| o == "1.2.840.113549.1.9.1" }.map { |e| D.header(0x80, false, 1, e[1].bytesize) + e[1] }
            return emails.join.b
          end
          D.header(0x80, false, 1, val.bytesize) + val.b
        when "DNS" then D.header(0x80, false, 2, val.bytesize) + val.b
        when "URI" then D.header(0x80, false, 6, val.bytesize) + val.b
        when "RID"
          oid = Kenny::OID.lookup(val) or return nil
          c = D.oid_content(oid)
          D.header(0x80, false, 8, c.bytesize) + c
        when "IP"
          ip = Kenny::X509Util.ip_bytes(val) or return nil
          D.header(0x80, false, 7, ip.bytesize) + ip
        when "issuer"
          return nil unless val == "copy" && @issuer_certificate
          ext = @issuer_certificate.extensions.find { |e| e.__kenny_dotted == "2.5.29.17" } or return "".b
          D.unwrap(ext.value_der)[1]
        else nil
        end
      end

      def __kenny_build(oid, v, crit)
        if v.start_with?("DER:")
          dotted = Kenny::OID.lookup(oid) or return nil
          hex = v.byteslice(4..).delete(":")
          return nil unless hex.match?(/\A(\h\h)*\z/)
          return Extension.new(dotted, [hex].pack("H*"), crit)
        end
        dotted = Kenny::OID.names(oid) && Kenny::OID.lookup(oid)
        return nil unless dotted && !oid.match?(/\A[\d.]+\z/)
        der = __kenny_value(dotted, v) or return nil
        Extension.new(dotted, der, crit)
      rescue OpenSSL::OpenSSLError, ArgumentError
        nil
      end

      def __kenny_value(dotted, v)
        # (X509V3_parse_list has nothing to give for an empty value)
        return nil if v.strip.empty? && !Kenny::ExtPrint::IA5_EXTS.include?(dotted)
        case dotted
        when "2.5.29.19"
          ca = false
          path = nil
          __kenny_list(v).each do |n, val|
            case n
            when "CA" then (ca = __kenny_bool(val)).nil? and return nil
            when "pathlen" then path = Integer(val, 10) rescue (return nil)
            else return nil
            end
          end
          D.seq(ca ? "\x01\x01\xff".b : "".b, path ? D.int(path) : "".b)
        when "2.5.29.15", "2.16.840.1.113730.1.1"
          snames, lnames = dotted == "2.5.29.15" ? [Kenny::ExtPrint::KU_SNAMES, Kenny::ExtPrint::KU_NAMES] : [Kenny::ExtPrint::NS_SNAMES, Kenny::ExtPrint::NS_NAMES]
          set = __kenny_list(v).map do |n, _|
            snames.index(n) || lnames.index(n) or return nil
          end
          bits = [0] * 2
          set.each { |i| bits[i / 8] |= 0x80 >> (i % 8) }
          bits.pop while bits.size > 0 && bits.last == 0
          unused = bits.empty? ? 0 : (0..7).find { |i| bits.last[i] == 1 }
          D.tlv(3, unused.chr.b + bits.pack("C*"))
        when "2.5.29.37"
          oids = __kenny_list(v).map do |n, val|
            return nil if val
            o = Kenny::OID.lookup(n) or return nil
            D.tlv(6, D.oid_content(o))
          end
          D.seq(*oids)
        when "2.5.29.14"
          if v == "hash"
            spki = __kenny_subject_spki or return nil
            D.tlv(4, OpenSSL::Digest::SHA1.digest(Kenny::X509Util.spki_parts(spki)[2]))
          else
            hex = v.delete(":")
            return nil unless hex.match?(/\A(\h\h)+\z/)
            D.tlv(4, [hex].pack("H*"))
          end
        when "2.5.29.35" then __kenny_akid(v)
        when "2.5.29.17", "2.5.29.18"
          gens = __kenny_list(v).map { |n, val| __kenny_gen(n, val) or return nil }
          D.seq(*gens)
        when "1.3.6.1.5.5.7.1.1", "1.3.6.1.5.5.7.1.11"
          ads = __kenny_list(v).map do |n, val|
            meth, gname = n.split(";", 2)
            return nil unless gname
            o = Kenny::OID.lookup(meth) or return nil
            g = __kenny_gen(gname, val) or return nil
            D.seq(D.tlv(6, D.oid_content(o)), g)
          end
          D.seq(*ads)
        when "2.5.29.31", "2.5.29.46"
          dps = __kenny_list(v).map do |n, val|
            g = __kenny_gen(n, val) or return nil
            inner = D.header(0x80, true, 0, g.bytesize) + g
            D.seq(D.header(0x80, true, 0, inner.bytesize) + inner)
          end
          D.seq(*dps)
        when "2.5.29.54" then D.int(Integer(v, 10))
        when *Kenny::ExtPrint::IA5_EXTS then D.tlv(22, v.b)
        else nil
        end
      end

      # v2i_AUTHORITY_KEYID (OpenSSL 3.x)
      def __kenny_akid(v)
        keyid = 0
        issuer = 0
        __kenny_list(v).each do |n, val|
          lvl = val.nil? ? 1 : val == "always" ? 2 : (return nil)
          case n
          when "keyid" then keyid = lvl
          when "issuer" then issuer = lvl
          when "none" then nil
          else return nil
          end
        end
        ic = @issuer_certificate or return nil
        same = ic.equal?(@subject_certificate)
        ss = same
        ikeyid = nil
        if keyid == 2 || (keyid == 1 && !ss)
          skid = ic.extensions.find { |e| e.__kenny_dotted == "2.5.29.14" }
          if skid && !(same && !ss)
            ikeyid = D.unwrap(skid.value_der)[1]
            ikeyid = nil if ikeyid.empty?
          end
          return nil if keyid == 2 && ikeyid.nil?
        end
        parts = []
        parts << D.header(0x80, false, 0, ikeyid.bytesize) + ikeyid if ikeyid
        if issuer == 2 || (issuer == 1 && !ss && ikeyid.nil?)
          name = ic.issuer.to_der
          dir = D.header(0x80, true, 4, name.bytesize) + name
          parts << D.header(0x80, true, 1, dir.bytesize) + dir
          serial = D.int_content(ic.serial.to_i)
          parts << D.header(0x80, false, 2, serial.bytesize) + serial
        end
        D.seq(*parts)
      end
    end
  end
end
