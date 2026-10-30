# frozen_string_literal: true
# Kenny: OpenSSL::X509::Name (ossl_x509name.c): entries, DER, OpenSSL's text forms
# (X509_NAME_oneline, X509_NAME_print_ex with its escaping) and X509_NAME_cmp/X509_NAME_hash on
# the canonical encoding.

module Kenny
  # X509_NAME_print_ex's escaping (crypto/asn1/a_strex.c)
  module NamePrint
    # charmap.h: 1 RFC 2253 escape, 2 control, 8 quote-able, 0x20 escape when first, 0x40 when last
    CHAR_TYPE = [
      1026, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
      2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
      120, 0, 1, 40, 0, 0, 0, 16, 16, 16, 0, 25, 25, 16, 16, 16,
      16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 9, 9, 16, 9, 16,
      0, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
      16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 0, 1, 0, 0, 0,
      0, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
      16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 0, 0, 0, 0, 2
    ].freeze
    # bytes per character of each string type (-1: dump)
    TAG2NBYTE = [-1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 0, -1, -1, -1, -1, -1, 1, 1, 1, -1, 1, 1, 1, -1, 1, -1, 4, -1, 2].freeze
    ESC_2253 = 1
    ESC_CTRL = 2
    ESC_MSB = 4
    ESC_QUOTE = 8
    UTF8_CONVERT = 0x10
    IGNORE_TYPE = 0x20
    SHOW_TYPE = 0x40
    DUMP_ALL = 0x80
    DUMP_UNKNOWN = 0x100
    DUMP_DER = 0x200
    ESC_2254 = 0x400
    ESC_FLAGS = ESC_2253 | ESC_2254 | ESC_QUOTE | ESC_CTRL | ESC_MSB
    BS_ESC = 0x61
    CONVUTF8 = 8

    class Bad < StandardError; end

    module_function

    def esc_char(out, c, flags, quotes)
      raise Bad if c > 0xffffffff
      if c > 0xffff
        out << format("\\W%08X", c)
        return
      end
      if c > 0xff
        out << format("\\U%04X", c)
        return
      end
      chflgs = c > 0x7f ? flags & ESC_MSB : CHAR_TYPE[c] & flags
      if chflgs & BS_ESC != 0
        if chflgs & ESC_QUOTE != 0
          quotes[0] = true
          out << c.chr
        else
          out << "\\" << c.chr
        end
      elsif chflgs & (ESC_CTRL | ESC_MSB | ESC_2254) != 0
        out << format("\\%02X", c)
      elsif c == 92 && flags & ESC_FLAGS != 0
        out << "\\\\"
      else
        out << c.chr
      end
    end

    def utf8_bytes(c)
      [c].pack("U").b.bytes
    rescue RangeError
      raise Bad
    end

    # do_buf
    def buf(data, type, flags, quotes)
      out = "".b
      width = type & 7
      raise Bad if (width == 4 && data.bytesize % 4 != 0) || (width == 2 && data.bytesize.odd?)
      chars =
        case width
        when 4 then data.unpack("N*")
        when 2 then data.unpack("n*")
        when 1 then data.bytes
        when 0
          s = data.dup.force_encoding(Encoding::UTF_8)
          raise Bad unless s.valid_encoding?
          s.codepoints
        else raise Bad
        end
      n = chars.size
      chars.each_with_index do |c, i|
        orflags = 0
        orflags = 0x20 if i == 0 && flags & ESC_2253 != 0
        orflags = 0x40 if i == n - 1 && flags & ESC_2253 != 0
        if type & CONVUTF8 != 0
          utf8_bytes(c).each { |b| esc_char(out, b, flags | orflags, quotes) }
        else
          esc_char(out, c, flags | orflags, quotes)
        end
      end
      out
    end

    # do_print_ex
    def print_ex(value, type, lflags)
      flags = lflags & ESC_FLAGS
      t =
        if lflags & DUMP_ALL != 0 then -1
        elsif lflags & IGNORE_TYPE != 0 then 1
        else
          w = type > 0 && type < 31 ? TAG2NBYTE[type] : -1
          w == -1 && lflags & DUMP_UNKNOWN == 0 ? 1 : w
        end
      if t == -1
        data = lflags & DUMP_DER != 0 ? Kenny::DER.tlv(type, value) : value
        return "#" + data.unpack1("H*").upcase
      end
      if lflags & UTF8_CONVERT != 0
        t = t == 0 ? 1 : t | CONVUTF8
      end
      quotes = [false]
      s = buf(value, t, flags, quotes)
      quotes[0] ? "\"#{s}\"".b : s
    end
  end
end

module OpenSSL
  module X509
    class Name
      include Comparable

      DEFAULT_OBJECT_TYPE = OpenSSL::ASN1::UTF8STRING
      OBJECT_TYPE_TEMPLATE = Hash.new(OpenSSL::ASN1::UTF8STRING)
      OBJECT_TYPE_TEMPLATE.update("C" => OpenSSL::ASN1::PRINTABLESTRING, "countryName" => OpenSSL::ASN1::PRINTABLESTRING,
                                  "serialNumber" => OpenSSL::ASN1::PRINTABLESTRING, "dnQualifier" => OpenSSL::ASN1::PRINTABLESTRING,
                                  "DC" => OpenSSL::ASN1::IA5STRING, "domainComponent" => OpenSSL::ASN1::IA5STRING,
                                  "emailAddress" => OpenSSL::ASN1::IA5STRING)
      COMPAT = 0
      RFC2253 = 17892119
      ONELINE = 8520479
      MULTILINE = 44302342

      # the string types an entry may have (ASN1_PRINTABLE)
      ENTRY_TYPES = [3, 7, 8, 9, 11, 12, 13, 14, 15, 18, 19, 20, 22, 29, 30].freeze
      private_constant :ENTRY_TYPES

      def initialize(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..2)" if args.size > 2
        @entries = []
        @der = nil
        return if args.empty?
        arg, template = args
        if (ary = Array.try_convert(arg))
          template = OBJECT_TYPE_TEMPLATE if template.nil?
          ary.each do |e|
            raise TypeError, "wrong argument type #{Kenny::DER.class_name(e)} (expected Array)" unless e.is_a?(Array)
            type = e[2]
            type = template[e[0]] if type.nil?
            type = DEFAULT_OBJECT_TYPE if type.nil?
            add_entry(e[0], e[1], type)
          end
        else
          der = arg.respond_to?(:to_der) ? arg.to_der : arg
          der = String.try_convert(der) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(der)} into String"
          __kenny_parse(der.b)
        end
      end

      def initialize_copy(other)
        super
        @entries = other.instance_variable_get(:@entries).map(&:dup)
        @der = other.instance_variable_get(:@der)
        @canon = nil
      end

      def add_entry(oid, value, type = nil, loc: -1, set: 0)
        oid = String.try_convert(oid) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(oid)} into String"
        v = String.try_convert(value) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(value)} into String"
        type = OBJECT_TYPE_TEMPLATE[oid] if type.nil?
        type = Integer(type)
        dotted = Kenny::OID.lookup(oid) or raise NameError, "X509_NAME_add_entry_by_txt: invalid field name (name=#{oid})"
        v = v.b
        if type & 0x1000 != 0
          v, type = Name.__kenny_mbstring(dotted, v, type)
        end
        unless ENTRY_TYPES.include?(type)
          raise NameError, type == 1 || type == 6 ? "X509_NAME_add_entry_by_txt: ASN1 lib" : "X509_NAME_add_entry_by_txt: nested asn1 error (Field=value, Type=X509_NAME_ENTRY)"
        end
        __kenny_insert([dotted, v, type, 0], Integer(loc), Integer(set))
        self
      end

      def to_s(*args)
        raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..1)" if args.size > 1
        flags = args[0]
        return __kenny_oneline if flags.nil?
        __kenny_print(Integer(flags))
      end

      def to_utf8
        __kenny_print(RFC2253 & ~Kenny::NamePrint::ESC_MSB).force_encoding(Encoding::UTF_8)
      end

      def inspect
        "#<#{self.class} #{to_utf8}>"
      end

      def to_a
        @entries.map { |oid, v, t, _| [Kenny::X509Util.sn(oid).dup, v.dup, t] }
      end

      def cmp(other)
        return nil unless other.is_a?(Name)
        a = __kenny_canon
        b = other.__kenny_canon
        return -1 unless a && b
        r = a.bytesize <=> b.bytesize
        r = a <=> b if r == 0
        r
      end
      alias <=> cmp

      def eql?(other)
        other.is_a?(Name) && cmp(other) == 0
      end

      # X509_NAME_hash: the first four bytes of the canonical encoding's SHA-1, little-endian
      def hash
        c = __kenny_canon
        c ? OpenSSL::Digest::SHA1.digest(c).unpack1("V") : 0
      end

      def hash_old
        OpenSSL::Digest::MD5.digest(to_der).unpack1("V")
      end

      def to_der
        @der ||= __kenny_encode.freeze
        @der.dup
      end

      # the entries with their RDN (set) numbers
      def __kenny_entries = @entries

      # X509_NAME's canonical encoding (x509_name_canon): RDN SETs of entries whose strings are
      # UTF-8, trimmed, with runs of spaces collapsed and ASCII lowercased
      def __kenny_canon
        return @canon if defined?(@canon) && !@canon.nil?
        sets = __kenny_rdns.map do |rdn|
          items = rdn.map do |oid, v, t|
            if [12, 30, 28, 19, 20, 22, 26].include?(t)
              s = Name.__kenny_to_utf8(v, t) or return(@canon = false)
              s = s.b.sub(/\A[ \t\n\v\f\r]+/, "").sub(/[ \t\n\v\f\r]+\z/, "")
              s = s.gsub(/[ \t\n\v\f\r]+/, " ").gsub(/[A-Z]/) { |c| c.downcase }
              t = 12
              v = s
            end
            Kenny::DER.seq(Kenny::DER.tlv(6, Kenny::DER.oid_content(oid)), Name.__kenny_value_tlv(t, v))
          end
          Kenny::DER.tlv(17, items.sort.join.b, true)
        end
        @canon = sets.join.b.freeze
      end

      # an entry's value as DER (a BIT STRING gets its unused-bits octet, as i2c_ASN1_BIT_STRING)
      def self.__kenny_value_tlv(t, v)
        return Kenny::DER.tlv(t, v) unless t == 3
        v = v.sub(/\x00+\z/n, "")
        unused = v.empty? ? 0 : (0..7).find { |i| v.getbyte(v.bytesize - 1)[i] == 1 }
        Kenny::DER.tlv(3, unused.chr.b + v)
      end

      # (ASN1_STRING_TABLE: [min, max] characters, and the type MBSTRING input becomes)
      MB_TABLE = { "2.5.4.3" => [1, 64], "2.5.4.6" => [2, 2, 19], "2.5.4.7" => [1, 128], "2.5.4.8" => [1, 128],
                   "2.5.4.10" => [1, 64], "2.5.4.11" => [1, 64], "1.2.840.113549.1.9.1" => [1, 128, 22],
                   "2.5.4.5" => [1, 64, 19], "2.5.4.46" => [-1, -1, 19], "0.9.2342.19200300.100.1.25" => [1, -1, 22],
                   "2.5.4.42" => [1, 32768], "2.5.4.4" => [1, 32768], "2.5.4.43" => [1, 32768], "2.5.4.41" => [1, 32768],
                   "2.5.4.12" => [1, 64] }.freeze
      private_constant :MB_TABLE

      # ASN1_mbstring_ncopy: [content, type] of MBSTRING_* input for the attribute oid
      def self.__kenny_mbstring(oid, v, type)
        e = ->(m) { raise NameError, "X509_NAME_add_entry_by_txt: #{m}" }
        chars =
          case type
          when 0x1000
            u = v.dup.force_encoding(Encoding::UTF_8)
            e.("invalid utf8string") unless u.valid_encoding?
            u.codepoints
          when 0x1001 then v.bytes
          when 0x1002
            e.("invalid bmpstring length") if v.bytesize.odd?
            v.unpack("n*")
          when 0x1004
            e.("invalid universalstring length") if v.bytesize % 4 != 0
            v.unpack("N*")
          else e.("unknown format")
          end
        min, max, out = MB_TABLE.fetch(oid, [-1, -1])
        e.("string too short (minsize=#{min})") if min > 0 && chars.size < min
        e.("string too long (maxsize=#{max})") if max > 0 && chars.size > max
        out ||= 12
        if out == 19
          e.("illegal characters") unless chars.all? { |c| c < 128 && c.chr.match?(%r{[A-Za-z0-9 '()+,\-./:=?]}) }
        elsif out == 22
          e.("illegal characters") unless chars.all? { |c| c < 128 }
        end
        [chars.pack("U*").b, out]
      rescue RangeError
        e.("illegal characters")
      end

      # the UTF-8 of a string value (ASN1_STRING_to_UTF8), nil when it isn't valid
      def self.__kenny_to_utf8(v, t)
        case t
        when 30
          return nil if v.bytesize.odd?
          v.unpack("n*").pack("U*").b
        when 28
          return nil if v.bytesize % 4 != 0
          v.unpack("N*").pack("U*").b
        when 12 then v.dup.force_encoding(Encoding::UTF_8).valid_encoding? ? v : nil
        else v.bytes.pack("U*").b
        end
      rescue ArgumentError, RangeError
        nil
      end

      private

      def __kenny_rdns
        rdns = []
        prev = nil
        @entries.each do |e|
          if prev == e[3]
            rdns.last << e
          else
            rdns << [e]
          end
          prev = e[3]
        end
        rdns
      end

      def __kenny_encode
        sets = __kenny_rdns.map do |rdn|
          items = rdn.map { |oid, v, t| Kenny::DER.seq(Kenny::DER.tlv(6, Kenny::DER.oid_content(oid)), Name.__kenny_value_tlv(t, v)) }
          Kenny::DER.tlv(17, items.sort.join.b, true)
        end
        Kenny::DER.seq(*sets)
      end

      # X509_NAME_add_entry
      def __kenny_insert(e, loc, set)
        n = @entries.size
        loc = n if loc > n || loc < 0
        inc = set == 0
        if set == -1
          if loc == 0
            set = 0
            inc = true
          else
            set = @entries[loc - 1][3]
          end
        elsif loc >= n
          set = loc != 0 ? @entries[loc - 1][3] + 1 : 0
        else
          set = @entries[loc][3]
        end
        e[3] = set
        @entries.insert(loc, e)
        ((loc + 1)...@entries.size).each { |i| @entries[i][3] += 1 } if inc
        @der = nil
        @canon = nil
      end

      def __kenny_parse(der)
        @entries = []
        b, c, tlv = Kenny::DER.first(der)
        raise NameError, "nested asn1 error" unless b == 0x30
        set = 0
        Kenny::DER.each_tlv(c) do |b0, sc|
          raise NameError, "nested asn1 error" unless b0 == 0x31
          Kenny::DER.each_tlv(sc) do |b1, ec|
            raise NameError, "nested asn1 error" unless b1 == 0x30
            kids = Kenny::DER.children(ec)
            raise NameError, "nested asn1 error" unless kids.size == 2 && kids[0][0] == 6 && ENTRY_TYPES.include?(kids[1][0])
            oid = Kenny::DER.oid_dotted(kids[0][1]) or raise NameError, "nested asn1 error"
            val = kids[1][1]
            val = val.byteslice(1..) if kids[1][0] == 3
            @entries << [oid, val, kids[1][0], set]
          end
          set += 1
        end
        # (X509_NAME keeps the encoding it was read from)
        @der = tlv.freeze
      rescue OpenSSL::ASN1::ASN1Error
        raise NameError, "nested asn1 error"
      end

      # X509_NAME_oneline
      def __kenny_oneline
        out = "".b
        prev = nil
        @entries.each do |oid, v, t, set|
          out << (prev == set ? "+" : "/")
          prev = set
          out << Kenny::X509Util.sn(oid) << "="
          q = v.bytes
          keep = [true] * 4
          if t == 27 && q.size % 4 == 0
            nz = [false] * 4
            q.each_with_index { |b, j| nz[j & 3] = true if b != 0 }
            keep = nz[0] || nz[1] || nz[2] ? [true] * 4 : [false, false, false, true]
          end
          q.each_with_index do |b, j|
            next unless keep[j & 3]
            if b < 32 || b > 126
              out << format("\\x%02X", b)
            elsif b == 47 || b == 43
              out << "\\" << b.chr
            else
              out << b.chr
            end
          end
        end
        out
      end

      # X509_NAME_print_ex
      def __kenny_print(flags)
        return __kenny_compat if flags == 0
        sep_dn, sep_mv =
          case flags & (0xf << 16)
          when 4 << 16 then ["\n", " + "]
          when 1 << 16 then [",", "+"]
          when 2 << 16 then [", ", " + "]
          when 3 << 16 then ["; ", " + "]
          else raise NameError, "X509_NAME_print_ex"
          end
        sep_eq = flags & (1 << 23) != 0 ? " = " : "="
        fn_opt = flags & (3 << 21)
        list = flags & (1 << 20) != 0 ? @entries.reverse : @entries
        out = "".b
        prev = nil
        list.each do |oid, v, t, set|
          out << (prev == set ? sep_mv : sep_dn) unless prev.nil?
          prev = set
          known = !Kenny::OID.names(oid).nil?
          if fn_opt != 3 << 21
            if fn_opt == 2 << 21 || !known
              name = oid
              width = 0
            elsif fn_opt == 0
              name = Kenny::X509Util.sn(oid)
              width = 10
            else
              name = Kenny::X509Util.ln(oid)
              width = 25
            end
            out << name
            out << " " * (width - name.bytesize) if name.bytesize < width && flags & (1 << 25) != 0
            out << sep_eq
          end
          orflags = !known && flags & (1 << 24) != 0 ? Kenny::NamePrint::DUMP_ALL : 0
          out << Kenny::NamePrint.print_ex(v, t, flags | orflags)
        end
        out
      rescue Kenny::NamePrint::Bad
        raise NameError, "X509_NAME_print_ex"
      end

      # X509_NAME_print: the oneline form with "/" before an attribute turned into ", "
      def __kenny_compat
        b = __kenny_oneline
        return b if b.empty?
        s = b.byteslice(1..)
        out = "".b
        c = 0
        i = 0
        n = s.bytesize
        up = ->(x) { x && x >= 65 && x <= 90 }
        loop do
          ch = s.getbyte(i)
          if ch.nil? || (ch == 47 && up.(s.getbyte(i + 1)) && (s.getbyte(i + 2) == 61 || (up.(s.getbyte(i + 2)) && s.getbyte(i + 3) == 61)))
            out << s.byteslice(c, i - c)
            c = i + 1
            out << ", " unless ch.nil?
          end
          break if i >= n
          i += 1
        end
        out
      end
    end
  end
end
