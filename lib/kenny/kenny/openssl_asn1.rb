# frozen_string_literal: true
# Kenny: OpenSSL::ASN1, what the openssl gem's C part (ossl_asn1.c, and openssl 3.x's
# openssl/asn1.rb) provides, in Ruby: the ASN1Data classes, DER/BER encoding with IMPLICIT and
# EXPLICIT tagging and indefinite lengths, decode/decode_all/traverse, and an object identifier
# table with OpenSSL's short and long names (generated from OpenSSL 3.6's objects).
# lib/kenny/openssl.rb requires it; lib/kenny/kenny/openssl_x509.rb builds on it.

module Kenny
  # DER/BER primitives shared by ASN1 and X509 (OpenSSL's ASN1_put_object, ASN1_get_object, ...)
  module DER
    CLASS_BITS = { UNIVERSAL: 0x00, APPLICATION: 0x40, CONTEXT_SPECIFIC: 0x80, PRIVATE: 0xc0 }.freeze
    CLASS_SYMS = [:UNIVERSAL, :APPLICATION, :CONTEXT_SPECIFIC, :PRIVATE].freeze

    module_function

    def length_octets(len)
      return len.chr.b if len < 0x80
      bytes = []
      while len > 0
        bytes.unshift(len & 0xff)
        len >>= 8
      end
      (0x80 | bytes.size).chr.b << bytes.pack("C*")
    end

    # identifier and length octets; len nil: indefinite length
    def header(cls_bits, constructed, tag, len)
      first = cls_bits | (constructed ? 0x20 : 0)
      s = if tag < 31
        (first | tag).chr.b
      else
        t = [tag & 0x7f]
        tag >>= 7
        while tag > 0
          t.unshift(0x80 | (tag & 0x7f))
          tag >>= 7
        end
        (first | 0x1f).chr.b << t.pack("C*")
      end
      len.nil? ? s << "\x80".b : s << length_octets(len)
    end

    # a TLV with a universal tag
    def tlv(tag, body, constructed = false)
      header(0, constructed, tag, body.bytesize) << body
    end

    def seq(*items) = tlv(16, items.join.b, true)
    def set(*items) = tlv(17, items.join.b, true)

    # ASN1_get_object: [flags (0x20 constructed, 0x01 indefinite), tag, class bits, header
    # length, content length]; raises ASN1Error with OpenSSL's reason
    def read_header(s, pos, omax)
      raise OpenSSL::ASN1::ASN1Error, "too small" if omax <= 0
      b = s.getbyte(pos)
      ret = b & 0x20
      xclass = b & 0xc0
      p = pos + 1
      max = omax - 1
      if (b & 0x1f) == 0x1f
        bad_header if max == 0
        tag = 0
        while (c = s.getbyte(p)) & 0x80 != 0
          tag = (tag << 7) | (c & 0x7f)
          p += 1
          max -= 1
          bad_header if max == 0 || tag > (0x7fffffff >> 7)
        end
        tag = (tag << 7) | (s.getbyte(p) & 0x7f)
        p += 1
        max -= 1
        bad_header if max == 0
      else
        tag = b & 0x1f
        bad_header if max == 0
      end
      # asn1_get_length
      bad_header if max < 1
      max -= 1
      c = s.getbyte(p)
      inf = 0
      if c == 0x80
        inf = 1
        len = 0
        p += 1
      else
        p += 1
        if c & 0x80 != 0
          i = c & 0x7f
          bad_header if max < i + 1
          while i > 0 && s.getbyte(p) == 0
            p += 1
            i -= 1
          end
          bad_header if i > 8
          len = 0
          while i > 0
            len = (len << 8) | s.getbyte(p)
            p += 1
            i -= 1
          end
          bad_header if len > 0x7fffffffffffffff
        else
          len = c
        end
      end
      bad_header if inf == 1 && ret == 0
      hlen = p - pos
      raise OpenSSL::ASN1::ASN1Error, "too long" if len > omax - hlen
      [ret | inf, tag, xclass, hlen, len]
    end

    def bad_header
      raise OpenSSL::ASN1::ASN1Error, "header too long"
    end

    # Each TLV of a DER string: [tag byte, content, whole TLV] (definite lengths only; for
    # X509's own parsing, which needs DER)
    def each_tlv(s, pos = 0, stop = s.bytesize)
      while pos < stop
        _, tag, cls, hlen, len = read_header(s, pos, stop - pos)
        first = s.getbyte(pos)
        yield first, s.byteslice(pos + hlen, len), s.byteslice(pos, hlen + len), tag, cls
        pos += hlen + len
      end
    end

    # [tag byte, content, whole TLV] of the first TLV in s (what follows it is ignored, as by d2i)
    def first(s)
      _, _, _, hlen, len = read_header(s, 0, s.bytesize)
      [s.getbyte(0), s.byteslice(hlen, len), s.byteslice(0, hlen + len)]
    end

    def children(s)
      a = []
      each_tlv(s) { |*x| a << x }
      a
    end

    # [tag byte, content] of the single TLV in s
    def unwrap(s)
      _, _, _, hlen, len = read_header(s, 0, s.bytesize)
      raise OpenSSL::ASN1::ASN1Error, "too long" if hlen + len != s.bytesize
      [s.getbyte(0), s.byteslice(hlen, len)]
    end

    def int_content(v)
      return "\x00".b if v == 0
      if v > 0
        h = v.to_s(16)
        h = "0#{h}" if h.size.odd?
        h = "00#{h}" if h[0].to_i(16) >= 8
        [h].pack("H*")
      else
        n = 1
        n += 1 while v < -(1 << (8 * n - 1))
        [((1 << (8 * n)) + v).to_s(16).rjust(2 * n, "0")].pack("H*")
      end
    end

    def int(v) = tlv(2, int_content(v))

    # c2i_ibuf: the integer in content octets (OpenSSL's padding checks)
    def int_value(c)
      raise OpenSSL::ASN1::ASN1Error, "illegal zero content" if c.empty?
      b0 = c.getbyte(0)
      if c.bytesize > 1
        pad = b0 == 0 || (b0 == 0xff && c.byteslice(1..).bytes.any? { |x| x != 0 })
        raise OpenSSL::ASN1::ASN1Error, "illegal padding" if pad && (b0 & 0x80) == (c.getbyte(1) & 0x80)
      end
      v = c.unpack1("H*").to_i(16)
      v -= 1 << (8 * c.bytesize) if b0 >= 0x80
      v
    end

    def oid_content(dotted)
      arcs = dotted.split(".").map(&:to_i)
      out = [arcs[0] * 40 + arcs[1]] + arcs[2..]
      b = []
      out.each do |a|
        t = [a & 0x7f]
        a >>= 7
        while a > 0
          t.unshift(0x80 | (a & 0x7f))
          a >>= 7
        end
        b.concat(t)
      end
      b.pack("C*")
    end

    # the dotted form of OID content octets, nil when they aren't a valid encoding
    def oid_dotted(c)
      return nil if c.empty? || c.getbyte(c.bytesize - 1) & 0x80 != 0
      arcs = []
      v = 0
      start = true
      c.each_byte do |x|
        return nil if start && x == 0x80
        v = (v << 7) | (x & 0x7f)
        if x & 0x80 == 0
          arcs << v
          v = 0
          start = true
        else
          start = false
        end
      end
      first = arcs.shift
      if first < 40 then arcs.unshift(0, first)
      elsif first < 80 then arcs.unshift(1, first - 40)
      else arcs.unshift(2, first - 80)
      end
      arcs.join(".")
    end

    # a2d_ASN1_OBJECT's checks of a dotted OID: nil when valid, else OpenSSL's reason
    def oid_syntax_error(s)
      return "first num too large" unless s.match?(/\A[0-2]/)
      return "invalid separator" unless s.size > 1 && (s[1] == "." || s[1] == " ")
      parts = s.split(/[. ]/, -1)
      return "missing second number" if parts.size < 2 || parts[1].empty? && parts.size == 2
      parts.each { |x| return "invalid digit" unless x.match?(/\A\d+\z/) }
      return "second number too large" if parts[0].to_i < 2 && parts[1].to_i >= 40
      nil
    end

    def time_utc(t)
      t.is_a?(Time) ? t.getutc : Time.at(Integer(t)).utc
    end

    # ASN1_UTCTIME_adj / ASN1_GENERALIZEDTIME_adj
    def time_content(tag, t)
      t = time_utc(t)
      if tag == 23
        raise OpenSSL::ASN1::ASN1Error, "" unless (1950..2049).cover?(t.year)
        format("%02d%02d%02d%02d%02d%02dZ", t.year % 100, t.month, t.day, t.hour, t.min, t.sec).b
      else
        raise OpenSSL::ASN1::ASN1Error, "" unless (0..9999).cover?(t.year)
        format("%04d%02d%02d%02d%02d%02dZ", t.year, t.month, t.day, t.hour, t.min, t.sec).b
      end
    end

    # X509 time: UTCTime through 2049, GeneralizedTime after (ASN1_TIME_adj)
    def time(t)
      t = time_utc(t)
      tag = (1950..2049).cover?(t.year) ? 23 : 24
      tlv(tag, time_content(tag, t))
    end

    # the Time in UTCTime/GeneralizedTime content (ASN1_TIME_to_tm's formats)
    def time_value(tag, c)
      s = c.b
      m = if tag == 23
        s.match(/\A(\d\d)(\d\d)(\d\d)(\d\d)(\d\d)(\d\d)?(Z|[+-]\d{4})\z/)
      else
        s.match(/\A(\d{4})(\d\d)(\d\d)(\d\d)(\d\d)(\d\d)?(?:\.\d+)?(Z|[+-]\d{4})?\z/)
      end
      return nil unless m
      y = m[1].to_i
      y += y < 50 ? 2000 : 1900 if tag == 23
      t = Time.utc(y, m[2].to_i, m[3].to_i, m[4].to_i, m[5].to_i, m[6].to_i)
      z = m[7]
      if z && z != "Z"
        off = z[1, 2].to_i * 3600 + z[3, 2].to_i * 60
        t -= z[0] == "+" ? off : -off
      end
      t
    rescue ArgumentError
      nil
    end
  end

  # OpenSSL's object table (OBJ_txt2obj, OBJ_nid2sn, OBJ_nid2ln), loaded on first use
  module OID
    @by_oid = nil
    @by_name = nil

    def self.load
      return if @by_oid
      by_oid = {}
      by_name = {}
      TABLE.each_line(chomp: true) do |l|
        oid, sn, ln = l.split(" ", 3)
        ln ||= sn
        by_oid[oid] = [sn.freeze, ln.freeze]
        by_name[sn] ||= oid
        by_name[ln] ||= oid
      end
      @by_name = by_name
      @by_oid = by_oid
    end

    # the dotted OID of a short name, long name or dotted string; nil when unknown/invalid
    def self.lookup(txt)
      load
      o = @by_name[txt]
      return o if o
      DER.oid_syntax_error(txt) ? nil : txt.tr(" ", ".")
    end

    # [sn, ln] of a known OID (given by name or dotted)
    def self.names(txt)
      load
      o = @by_name[txt] || txt
      @by_oid[o]
    end

    def self.sn(oid) = names(oid)&.first
    def self.ln(oid) = names(oid)&.last

    # OBJ_obj2txt(no_name = 0): the short name when known, else the dotted form
    def self.name_of(oid)
      load
      @by_oid[oid]&.first || oid
    end

    def self.register(oid, sn, ln)
      load
      raise OpenSSL::ASN1::ASN1Error, "oid exists" if @by_name[sn] || @by_name[ln]
      if (e = DER.oid_syntax_error(oid))
        raise OpenSSL::ASN1::ASN1Error, e
      end
      @by_oid[oid] = [sn.dup.freeze, ln.dup.freeze]
      @by_name[sn] = oid
      @by_name[ln] = oid
      true
    end
  end
end

module OpenSSL
  module ASN1
    class ASN1Error < OpenSSLError; end

    # (openssl 4.0 builds the same classes in C; its objects set their instance variables in
    # another order, which `inspect` shows)
    KENNY_V4 = OpenSSL::VERSION >= "4"
    private_constant :KENNY_V4

    UNIVERSAL_TAG_NAME = [
      "EOC", "BOOLEAN", "INTEGER", "BIT_STRING", "OCTET_STRING", "NULL", "OBJECT", "OBJECT_DESCRIPTOR",
      "EXTERNAL", "REAL", "ENUMERATED", "EMBEDDED_PDV", "UTF8STRING", "RELATIVE_OID", nil, nil,
      "SEQUENCE", "SET", "NUMERICSTRING", "PRINTABLESTRING", "T61STRING", "VIDEOTEXSTRING", "IA5STRING",
      "UTCTIME", "GENERALIZEDTIME", "GRAPHICSTRING", "ISO64STRING", "GENERALSTRING", "UNIVERSALSTRING",
      "CHARACTER_STRING", "BMPSTRING"
    ].map { |n| n && (KENNY_V4 ? n.dup.freeze : n.dup) }
    UNIVERSAL_TAG_NAME.freeze if KENNY_V4
    UNIVERSAL_TAG_NAME.each_with_index { |n, i| const_set(n, i) if n }

    class ASN1Data
      attr_accessor :value, :tag, :tag_class, :indefinite_length
      alias infinite_length indefinite_length
      alias infinite_length= indefinite_length=

      def initialize(value, tag, tag_class)
        raise ASN1Error, "invalid tag class" unless tag_class.is_a?(Symbol)
        @tag = tag
        @value = value
        @tag_class = tag_class
        @indefinite_length = false
      end

      def to_der
        if @value.is_a?(Array)
          __kenny_cons_der
        else
          raise ASN1Error, "indefinite length form cannot be used with primitive encoding" if @indefinite_length
          __kenny_prim_der
        end
      end

      private

      # (ossl_asn1_initialize: a tag must be given with any argument after the value)
      def __kenny_tagged_init(value, args)
        raise ArgumentError, "wrong number of arguments (given #{args.size + 1}, expected 1..4)" if args.size > 3
        tag, tagging, tag_class = args
        default = __kenny_default_tag
        if default.nil? || !args.empty?
          raise ASN1Error, "must specify tag number" if tag.nil?
          raise ASN1Error, "invalid tagging method" if !tagging.nil? && !tagging.is_a?(Symbol)
          tag_class = tagging.nil? ? :UNIVERSAL : :CONTEXT_SPECIFIC if tag_class.nil?
          raise ASN1Error, "invalid tag class" unless tag_class.is_a?(Symbol)
        else
          tag = default
          tagging = nil
          tag_class = :UNIVERSAL
        end
        @tag = tag
        @value = value
        @tagging = tagging
        @tag_class = tag_class
        @indefinite_length = false
        @unused_bits = 0 if default == 3
      end

      def __kenny_default_tag
        k = self.class
        while k
          t = TAGS[k]
          return t if t
          k = k.superclass
        end
        nil
      end

      def __kenny_cls_bits
        Kenny::DER::CLASS_BITS.fetch(@tag_class.nil? ? :UNIVERSAL : @tag_class) { raise ASN1Error, "invalid tag class" }
      end

      # to_der_internal
      def __kenny_wrap(constructed, indef, body)
        cls = __kenny_cls_bits
        raise ASN1Error, "tag number not specified" if @tag.nil?
        tag = Integer(@tag)
        if instance_variable_defined?(:@tagging) && @tagging == :EXPLICIT
          dt = __kenny_default_tag or raise ASN1Error, "explicit tagging of unknown tag"
          inner = Kenny::DER.header(0, constructed, dt, indef ? nil : body.bytesize) << body
          inner << "\0\0".b if indef
          out = Kenny::DER.header(cls, true, tag, indef ? nil : inner.bytesize) << inner
          out << "\0\0".b if indef
          out
        else
          out = Kenny::DER.header(cls, constructed, tag, indef ? nil : body.bytesize) << body
          out << "\0\0".b if indef
          out
        end
      end

      def __kenny_str(v)
        String.try_convert(v) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(v)} into String"
      end

      def __kenny_prim_der
        t = __kenny_default_tag
        v = @value
        body =
          case t
          when nil, 4, 12, 18, 19, 20, 21, 22, 25, 26, 27, 28, 30 then __kenny_str(v).b
          when 1
            raise TypeError, "Can't convert nil into Boolean" if v.nil?
            v ? "\xff".b : "\x00".b
          when 2, 10
            raise TypeError, "Can't convert nil into Integer" if v.nil?
            raise TypeError, "Cannot convert into OpenSSL::BN" unless v.is_a?(::Integer) || v.is_a?(OpenSSL::BN)
            Kenny::DER.int_content(v.to_i)
          when 3
            bits = Integer(@unused_bits)
            raise ASN1Error, "unused_bits for a bitstring value must be in the range 0 to 7" unless (0..7).cover?(bits)
            s = __kenny_str(v).b
            s.setbyte(-1, s.getbyte(-1) & (0xff << bits) & 0xff) if !s.empty? && bits > 0
            bits.chr.b << s
          when 5
            raise ASN1Error, "nil expected" unless v.nil?
            "".b
          when 6
            s = __kenny_str(v)
            oid = Kenny::OID.lookup(s)
            raise ASN1Error, "invalid OBJECT ID #{s}: #{Kenny::DER.oid_syntax_error(s)}" unless oid
            Kenny::DER.oid_content(oid)
          when 23, 24 then Kenny::DER.time_content(t, v)
          else raise ASN1Error, "unsupported ASN.1 type"
          end
        __kenny_wrap(false, false, body)
      end

      def __kenny_cons_der
        indef = @indefinite_length ? true : false
        ary = @value.is_a?(Array) ? @value : Array(@value)
        body = "".b
        ary.each_with_index do |item, i|
          if indef && item.is_a?(EndOfContent)
            raise ASN1Error, "illegal EOC octets in value" if i != ary.size - 1
            break
          end
          item = item.to_der if item.respond_to?(:to_der)
          body << __kenny_str(item).b
        end
        __kenny_wrap(true, indef, body)
      end
    end

    if KENNY_V4
      class Primitive < ASN1Data
        attr_accessor :tagging
        undef_method :indefinite_length=
        undef_method :infinite_length=

        def initialize(value, *args) = __kenny_tagged_init(value, args)

        def to_der = __kenny_prim_der
      end

      class Constructive < ASN1Data
        include Enumerable
        attr_accessor :tagging

        def initialize(value, *args) = __kenny_tagged_init(value, args)

        def each(&blk)
          @value.each(&blk)
          self
        end

        def to_der = __kenny_cons_der
      end
    else
      module TaggedASN1Data
        attr_accessor :tagging

        def initialize(value, tag = nil, tagging = nil, tag_class = nil)
          tag ||= __send__(:__kenny_default_tag)
          raise ASN1Error, "must specify tag number" unless tag
          raise ASN1Error, "invalid tagging method" if tagging && !tagging.is_a?(Symbol)
          tag_class ||= tagging ? :CONTEXT_SPECIFIC : :UNIVERSAL
          raise ASN1Error, "invalid tag class" unless tag_class.is_a?(Symbol)
          @tagging = tagging
          super(value, tag, tag_class)
        end
      end

      class Primitive < ASN1Data
        include TaggedASN1Data
        undef_method :indefinite_length=
        undef_method :infinite_length=

        def to_der = __kenny_prim_der
      end

      class Constructive < ASN1Data
        include TaggedASN1Data
        include Enumerable

        def each(&blk)
          @value.each(&blk)
          self
        end

        def to_der = __kenny_cons_der
      end
    end

    class Boolean < Primitive; end
    class Integer < Primitive; end
    class Enumerated < Primitive; end

    class BitString < Primitive
      attr_accessor :unused_bits

      unless KENNY_V4
        def initialize(*)
          super
          @unused_bits = 0
        end
      end
    end

    class OctetString < Primitive; end
    class UTF8String < Primitive; end
    class NumericString < Primitive; end
    class PrintableString < Primitive; end
    class T61String < Primitive; end
    class VideotexString < Primitive; end
    class IA5String < Primitive; end
    class GraphicString < Primitive; end
    class ISO64String < Primitive; end
    class GeneralString < Primitive; end
    class UniversalString < Primitive; end
    class BMPString < Primitive; end
    class Null < Primitive; end
    class UTCTime < Primitive; end
    class GeneralizedTime < Primitive; end
    class Sequence < Constructive; end
    class Set < Constructive; end

    class ObjectId < Primitive
      def self.register(oid, sn, ln)
        Kenny::OID.register(__kenny_cstr(oid), __kenny_cstr(sn), __kenny_cstr(ln))
      end

      def self.__kenny_cstr(v)
        s = String.try_convert(v) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(v)} into String"
        raise ArgumentError, "string contains null byte" if s.include?("\0")
        s
      end

      def sn
        n = Kenny::OID.names(ObjectId.__kenny_cstr(@value))
        n && n[0].dup
      end
      alias short_name sn

      def ln
        n = Kenny::OID.names(ObjectId.__kenny_cstr(@value))
        n && n[1].dup
      end
      alias long_name ln

      def oid
        s = ObjectId.__kenny_cstr(@value)
        o = Kenny::OID.lookup(s) or raise ASN1Error, "invalid OBJECT ID #{s}: #{Kenny::DER.oid_syntax_error(s)}"
        o.dup.force_encoding(Encoding::US_ASCII)
      end

      def ==(other)
        return false unless other.is_a?(ObjectId)
        oid == other.oid
      end
    end

    class EndOfContent < ASN1Data
      def initialize
        if KENNY_V4
          @tag = 0
          @value = +""
          @tagging = nil
          @tag_class = :UNIVERSAL
          @indefinite_length = false
        else
          super(+"", 0, :UNIVERSAL)
        end
      end

      def to_der = "\0\0".b
    end

    TAGS = {
      EndOfContent => 0, Boolean => 1, Integer => 2, BitString => 3, OctetString => 4, Null => 5,
      ObjectId => 6, Enumerated => 10, UTF8String => 12, Sequence => 16, Set => 17, NumericString => 18,
      PrintableString => 19, T61String => 20, VideotexString => 21, IA5String => 22, UTCTime => 23,
      GeneralizedTime => 24, GraphicString => 25, ISO64String => 26, GeneralString => 27,
      UniversalString => 28, BMPString => 30
    }.freeze
    private_constant :TAGS
    # (openssl 3.x's; 4.0 keeps the map private)
    CLASS_TAG_MAP = TAGS unless KENNY_V4

    CLASSES = TAGS.invert.freeze
    private_constant :CLASSES

    # OpenSSL::ASN1::Integer(1) and the other constructor functions
    %w[Boolean Integer Enumerated BitString OctetString UTF8String NumericString PrintableString T61String
       VideotexString IA5String GraphicString ISO64String GeneralString UniversalString BMPString Null
       ObjectId UTCTime GeneralizedTime Sequence Set EndOfContent].each do |n|
      k = const_get(n)
      define_method(n) { |*args| k.new(*args) }
      module_function n
    end

    module_function

    unless KENNY_V4
      def take_default_tag(klass)
        TAGS[klass] || (klass.superclass && take_default_tag(klass.superclass))
      end
    end

    def decode(obj)
      s = ASN1.__kenny_der_arg(obj)
      st = [0]
      v = ASN1.__kenny_decode0(s, 0, s.bytesize, 0, nil, st)
      ASN1.__kenny_sanity(s.bytesize, st[0], st[0])
      v
    end

    def decode_all(obj)
      s = ASN1.__kenny_der_arg(obj)
      ary = []
      pos = 0
      while pos < s.bytesize
        st = [0]
        ary << ASN1.__kenny_decode0(s, pos, s.bytesize - pos, 0, nil, st)
        pos += st[0]
      end
      ASN1.__kenny_sanity(s.bytesize, pos, pos)
      ary
    end

    def traverse(obj, &blk)
      s = ASN1.__kenny_der_arg(obj)
      st = [0]
      ASN1.__kenny_decode0(s, 0, s.bytesize, 0, blk, st)
      ASN1.__kenny_sanity(s.bytesize, st[0], st[0])
      nil
    end

    def self.__kenny_der_arg(obj)
      obj = obj.to_der if obj.respond_to?(:to_der)
      s = String.try_convert(obj) or raise TypeError, "no implicit conversion of #{Kenny::DER.class_name(obj)} into String"
      s.b
    end

    def self.__kenny_sanity(len, read, offset)
      return if len == 0 || (read == len && offset == len)
      raise ASN1Error, "Type mismatch. Total bytes read: #{read} Bytes available: #{len} Offset: #{offset}"
    end

    # ossl_asn1_decode0: the value at pos (at most `length` bytes); st[0] += bytes read
    def self.__kenny_decode0(s, pos, length, depth, yld, st)
      j, tag, xclass, hlen, len = Kenny::DER.read_header(s, pos, length)
      tc = Kenny::DER::CLASS_SYMS[xclass >> 6]
      yld&.call([depth, pos, hlen, len, j & 0x20 != 0, tc, tag])
      if j & 0x20 != 0
        indefinite = j == 0x21
        ary = []
        available = indefinite ? length - hlen : len
        p = pos + hlen
        found_eoc = false
        while available > 0
          inner = [0]
          v = ASN1.__kenny_decode0(s, p, available, depth + 1, yld, inner)
          available -= inner[0]
          p += inner[0]
          if indefinite && v.tag == EOC && v.tag_class == :UNIVERSAL
            found_eoc = true
            break
          end
          ary << v
        end
        raise ASN1Error, "EOC missing in indefinite length encoding" if indefinite && !found_eoc && KENNY_V4
        read = p - pos
        obj =
          if tc == :UNIVERSAL
            k = tag == SEQUENCE ? Sequence : tag == SET ? Set : Constructive
            k.new(ary, tag, nil, tc)
          else
            ASN1Data.new(ary, tag, tc)
          end
        obj.instance_variable_set(:@indefinite_length, indefinite)
        if len != 0 && read != hlen + len
          raise ASN1Error, "Type mismatch. Bytes read: #{read} Bytes available: #{hlen + len}"
        end
        st[0] += read
        obj
      else
        raise ASN1Error, "indefinite length for primitive value" if (j & 1) == 1 && len == 0
        c = s.byteslice(pos + hlen, len)
        unused = 0
        value =
          if tc == :UNIVERSAL && tag < 31
            case tag
            when 0
              raise ASN1Error, "" unless hlen + len == 2 && s.getbyte(pos) == 0 && s.getbyte(pos + 1) == 0
              +""
            when 1
              raise ASN1Error, "invalid length for BOOLEAN" if hlen + len != 3
              raise ASN1Error, "invalid BOOLEAN" if s.getbyte(pos) != 1 || s.getbyte(pos + 1) != 1
              c.getbyte(0) != 0
            when 2, 10 then OpenSSL::BN.new(Kenny::DER.int_value(c))
            when 3
              raise ASN1Error, "d2i_ASN1_BIT_STRING: string too short" if c.empty?
              unused = c.getbyte(0)
              raise ASN1Error, "d2i_ASN1_BIT_STRING: invalid bit string bits left" if unused > 7
              v = c.byteslice(1..)
              v.setbyte(-1, v.getbyte(-1) & (0xff << unused) & 0xff) if !v.empty? && unused > 0
              v
            when 5
              raise ASN1Error, "null is wrong length" unless c.empty?
              nil
            when 6
              o = Kenny::DER.oid_dotted(c) or raise ASN1Error, KENNY_V4 ? "d2i_ASN1_OBJECT: invalid object encoding" : "invalid object encoding"
              Kenny::OID.name_of(o).dup
            when 23, 24 then ASN1.__kenny_time(tag, c)
            else c
            end
          else
            c
          end
        k = tc == :UNIVERSAL && CLASSES[tag]
        obj =
          if k == EndOfContent
            EndOfContent.new
          elsif k
            o = k.new(value, tag, nil, tc)
            o.instance_variable_set(:@unused_bits, unused) if tag == 3
            o
          else
            ASN1Data.new(value, tag, tc)
          end
        st[0] += hlen + len
        obj
      end
    end

    def self.__kenny_time(tag, c)
      raise ASN1Error, tag == 23 ? "utctime is too short" : "generalizedtime is too short" if c.bytesize < (tag == 23 ? 13 : 15)
      if KENNY_V4
        Kenny::DER.time_value(tag, c) or raise ASN1Error, "ASN1_TIME_to_tm"
      else
        # (openssl 3.x reads the fields with sscanf, and UTCTime years before 69 as 20xx)
        n = tag == 23 ? 2 : 4
        m = c.match(/\A(\d{#{n}})(\d\d)(\d\d)(\d\d)(\d\d)(\d\d)?/)
        raise TypeError, "bad #{tag == 23 ? "UTCTIME" : "GENERALIZEDTIME"} format: \"#{c}\"" unless m
        y = m[1].to_i
        y += y < 69 ? 2000 : 1900 if tag == 23
        Time.utc(y, m[2].to_i, m[3].to_i, m[4].to_i, m[5].to_i, m[6].to_i)
      end
    end

  end
end

module Kenny
  module DER
    def self.class_name(v)
      case v
      when nil then "nil"
      when true then "true"
      when false then "false"
      else v.class.name
      end
    end
  end
end

module Kenny
  module OID
    # OID, short name, long name (when it differs) of the objects OpenSSL 3.6 knows
    # (its obj_mac.h)
    TABLE = <<~OIDS
      0.3.4401.5.3.1.9.1 CAMELLIA-128-ECB camellia-128-ecb
      0.3.4401.5.3.1.9.3 CAMELLIA-128-OFB camellia-128-ofb
      0.3.4401.5.3.1.9.4 CAMELLIA-128-CFB camellia-128-cfb
      0.3.4401.5.3.1.9.6 CAMELLIA-128-GCM camellia-128-gcm
      0.3.4401.5.3.1.9.7 CAMELLIA-128-CCM camellia-128-ccm
      0.3.4401.5.3.1.9.9 CAMELLIA-128-CTR camellia-128-ctr
      0.3.4401.5.3.1.9.10 CAMELLIA-128-CMAC camellia-128-cmac
      0.3.4401.5.3.1.9.21 CAMELLIA-192-ECB camellia-192-ecb
      0.3.4401.5.3.1.9.23 CAMELLIA-192-OFB camellia-192-ofb
      0.3.4401.5.3.1.9.24 CAMELLIA-192-CFB camellia-192-cfb
      0.3.4401.5.3.1.9.26 CAMELLIA-192-GCM camellia-192-gcm
      0.3.4401.5.3.1.9.27 CAMELLIA-192-CCM camellia-192-ccm
      0.3.4401.5.3.1.9.29 CAMELLIA-192-CTR camellia-192-ctr
      0.3.4401.5.3.1.9.30 CAMELLIA-192-CMAC camellia-192-cmac
      0.3.4401.5.3.1.9.41 CAMELLIA-256-ECB camellia-256-ecb
      0.3.4401.5.3.1.9.43 CAMELLIA-256-OFB camellia-256-ofb
      0.3.4401.5.3.1.9.44 CAMELLIA-256-CFB camellia-256-cfb
      0.3.4401.5.3.1.9.46 CAMELLIA-256-GCM camellia-256-gcm
      0.3.4401.5.3.1.9.47 CAMELLIA-256-CCM camellia-256-ccm
      0.3.4401.5.3.1.9.49 CAMELLIA-256-CTR camellia-256-ctr
      0.3.4401.5.3.1.9.50 CAMELLIA-256-CMAC camellia-256-cmac
      0.4 itu-t-identified-organization
      0.4.0 etsi
      0.4.0.1733 electronic-signature-standard
      0.4.0.1733.2 ess-attributes
      0.4.0.1733.2.1 id-aa-ets-mimeType
      0.4.0.1733.2.2 id-aa-ets-longTermValidation
      0.4.0.1733.2.3 id-aa-ets-SignaturePolicyDocument
      0.4.0.1733.2.4 id-aa-ets-archiveTimestampV3
      0.4.0.1733.2.5 id-aa-ATSHashIndex
      0.4.0.19122 cades
      0.4.0.19122.1 cades-attributes
      0.4.0.19122.1.1 id-aa-ets-signerAttrV2
      0.4.0.19122.1.3 id-aa-ets-sigPolicyStore
      0.4.0.19122.1.4 id-aa-ATSHashIndex-v2
      0.4.0.19122.1.5 id-aa-ATSHashIndex-v3
      0.4.0.19122.1.6 signedAssertion
      0.9 data
      0.9.2342 pss
      0.9.2342.19200300 ucl
      0.9.2342.19200300.100 pilot
      0.9.2342.19200300.100.1 pilotAttributeType
      0.9.2342.19200300.100.1.1 UID userId
      0.9.2342.19200300.100.1.2 textEncodedORAddress
      0.9.2342.19200300.100.1.3 mail rfc822Mailbox
      0.9.2342.19200300.100.1.4 info
      0.9.2342.19200300.100.1.5 favouriteDrink
      0.9.2342.19200300.100.1.6 roomNumber
      0.9.2342.19200300.100.1.7 photo
      0.9.2342.19200300.100.1.8 userClass
      0.9.2342.19200300.100.1.9 host
      0.9.2342.19200300.100.1.10 manager
      0.9.2342.19200300.100.1.11 documentIdentifier
      0.9.2342.19200300.100.1.12 documentTitle
      0.9.2342.19200300.100.1.13 documentVersion
      0.9.2342.19200300.100.1.14 documentAuthor
      0.9.2342.19200300.100.1.15 documentLocation
      0.9.2342.19200300.100.1.20 homeTelephoneNumber
      0.9.2342.19200300.100.1.21 secretary
      0.9.2342.19200300.100.1.22 otherMailbox
      0.9.2342.19200300.100.1.23 lastModifiedTime
      0.9.2342.19200300.100.1.24 lastModifiedBy
      0.9.2342.19200300.100.1.25 DC domainComponent
      0.9.2342.19200300.100.1.26 aRecord
      0.9.2342.19200300.100.1.27 pilotAttributeType27
      0.9.2342.19200300.100.1.28 mXRecord
      0.9.2342.19200300.100.1.29 nSRecord
      0.9.2342.19200300.100.1.30 sOARecord
      0.9.2342.19200300.100.1.31 cNAMERecord
      0.9.2342.19200300.100.1.37 associatedDomain
      0.9.2342.19200300.100.1.38 associatedName
      0.9.2342.19200300.100.1.39 homePostalAddress
      0.9.2342.19200300.100.1.40 personalTitle
      0.9.2342.19200300.100.1.41 mobileTelephoneNumber
      0.9.2342.19200300.100.1.42 pagerTelephoneNumber
      0.9.2342.19200300.100.1.43 friendlyCountryName
      0.9.2342.19200300.100.1.44 uid uniqueIdentifier
      0.9.2342.19200300.100.1.45 organizationalStatus
      0.9.2342.19200300.100.1.46 janetMailbox
      0.9.2342.19200300.100.1.47 mailPreferenceOption
      0.9.2342.19200300.100.1.48 buildingName
      0.9.2342.19200300.100.1.49 dSAQuality
      0.9.2342.19200300.100.1.50 singleLevelQuality
      0.9.2342.19200300.100.1.51 subtreeMinimumQuality
      0.9.2342.19200300.100.1.52 subtreeMaximumQuality
      0.9.2342.19200300.100.1.53 personalSignature
      0.9.2342.19200300.100.1.54 dITRedirect
      0.9.2342.19200300.100.1.55 audio
      0.9.2342.19200300.100.1.56 documentPublisher
      0.9.2342.19200300.100.3 pilotAttributeSyntax
      0.9.2342.19200300.100.3.4 iA5StringSyntax
      0.9.2342.19200300.100.3.5 caseIgnoreIA5StringSyntax
      0.9.2342.19200300.100.4 pilotObjectClass
      0.9.2342.19200300.100.4.3 pilotObject
      0.9.2342.19200300.100.4.4 pilotPerson
      0.9.2342.19200300.100.4.5 account
      0.9.2342.19200300.100.4.6 document
      0.9.2342.19200300.100.4.7 room
      0.9.2342.19200300.100.4.9 documentSeries
      0.9.2342.19200300.100.4.13 domain Domain
      0.9.2342.19200300.100.4.14 rFC822localPart
      0.9.2342.19200300.100.4.15 dNSDomain
      0.9.2342.19200300.100.4.17 domainRelatedObject
      0.9.2342.19200300.100.4.18 friendlyCountry
      0.9.2342.19200300.100.4.19 simpleSecurityObject
      0.9.2342.19200300.100.4.20 pilotOrganization
      0.9.2342.19200300.100.4.21 pilotDSA
      0.9.2342.19200300.100.4.22 qualityLabelledData
      0.9.2342.19200300.100.10 pilotGroups
      1.0.9797.3.4 GMAC gmac
      1.0.10118.3.0.55 whirlpool
      1.2 member-body ISO Member Body
      1.2.156 ISO-CN ISO CN Member Body
      1.2.156.10197 oscca
      1.2.156.10197.1 sm-scheme
      1.2.156.10197.1.104.1 SM4-ECB sm4-ecb
      1.2.156.10197.1.104.2 SM4-CBC sm4-cbc
      1.2.156.10197.1.104.3 SM4-OFB sm4-ofb
      1.2.156.10197.1.104.4 SM4-CFB sm4-cfb
      1.2.156.10197.1.104.5 SM4-CFB1 sm4-cfb1
      1.2.156.10197.1.104.6 SM4-CFB8 sm4-cfb8
      1.2.156.10197.1.104.7 SM4-CTR sm4-ctr
      1.2.156.10197.1.104.8 SM4-GCM sm4-gcm
      1.2.156.10197.1.104.9 SM4-CCM sm4-ccm
      1.2.156.10197.1.104.10 SM4-XTS sm4-xts
      1.2.156.10197.1.301 SM2 sm2
      1.2.156.10197.1.401 SM3 sm3
      1.2.156.10197.1.401.3.1 hmacWithSM3
      1.2.156.10197.1.501 SM2-SM3 SM2-with-SM3
      1.2.156.10197.1.504 RSA-SM3 sm3WithRSAEncryption
      1.2.392.200011.61.1.1.1.2 CAMELLIA-128-CBC camellia-128-cbc
      1.2.392.200011.61.1.1.1.3 CAMELLIA-192-CBC camellia-192-cbc
      1.2.392.200011.61.1.1.1.4 CAMELLIA-256-CBC camellia-256-cbc
      1.2.392.200011.61.1.1.3.2 id-camellia128-wrap
      1.2.392.200011.61.1.1.3.3 id-camellia192-wrap
      1.2.392.200011.61.1.1.3.4 id-camellia256-wrap
      1.2.410.200004 KISA kisa
      1.2.410.200004.1.3 SEED-ECB seed-ecb
      1.2.410.200004.1.4 SEED-CBC seed-cbc
      1.2.410.200004.1.5 SEED-CFB seed-cfb
      1.2.410.200004.1.6 SEED-OFB seed-ofb
      1.2.410.200046.1.1.1 ARIA-128-ECB aria-128-ecb
      1.2.410.200046.1.1.2 ARIA-128-CBC aria-128-cbc
      1.2.410.200046.1.1.3 ARIA-128-CFB aria-128-cfb
      1.2.410.200046.1.1.4 ARIA-128-OFB aria-128-ofb
      1.2.410.200046.1.1.5 ARIA-128-CTR aria-128-ctr
      1.2.410.200046.1.1.6 ARIA-192-ECB aria-192-ecb
      1.2.410.200046.1.1.7 ARIA-192-CBC aria-192-cbc
      1.2.410.200046.1.1.8 ARIA-192-CFB aria-192-cfb
      1.2.410.200046.1.1.9 ARIA-192-OFB aria-192-ofb
      1.2.410.200046.1.1.10 ARIA-192-CTR aria-192-ctr
      1.2.410.200046.1.1.11 ARIA-256-ECB aria-256-ecb
      1.2.410.200046.1.1.12 ARIA-256-CBC aria-256-cbc
      1.2.410.200046.1.1.13 ARIA-256-CFB aria-256-cfb
      1.2.410.200046.1.1.14 ARIA-256-OFB aria-256-ofb
      1.2.410.200046.1.1.15 ARIA-256-CTR aria-256-ctr
      1.2.410.200046.1.1.34 ARIA-128-GCM aria-128-gcm
      1.2.410.200046.1.1.35 ARIA-192-GCM aria-192-gcm
      1.2.410.200046.1.1.36 ARIA-256-GCM aria-256-gcm
      1.2.410.200046.1.1.37 ARIA-128-CCM aria-128-ccm
      1.2.410.200046.1.1.38 ARIA-192-CCM aria-192-ccm
      1.2.410.200046.1.1.39 ARIA-256-CCM aria-256-ccm
      1.2.643.2.2 cryptopro
      1.2.643.2.2.3 id-GostR3411-94-with-GostR3410-2001 GOST R 34.11-94 with GOST R 34.10-2001
      1.2.643.2.2.4 id-GostR3411-94-with-GostR3410-94 GOST R 34.11-94 with GOST R 34.10-94
      1.2.643.2.2.9 md_gost94 GOST R 34.11-94
      1.2.643.2.2.10 id-HMACGostR3411-94 HMAC GOST 34.11-94
      1.2.643.2.2.14.0 id-Gost28147-89-None-KeyMeshing
      1.2.643.2.2.14.1 id-Gost28147-89-CryptoPro-KeyMeshing
      1.2.643.2.2.19 gost2001 GOST R 34.10-2001
      1.2.643.2.2.20 gost94 GOST R 34.10-94
      1.2.643.2.2.20.1 id-GostR3410-94-a
      1.2.643.2.2.20.2 id-GostR3410-94-aBis
      1.2.643.2.2.20.3 id-GostR3410-94-b
      1.2.643.2.2.20.4 id-GostR3410-94-bBis
      1.2.643.2.2.21 gost89 GOST 28147-89
      1.2.643.2.2.22 gost-mac GOST 28147-89 MAC
      1.2.643.2.2.23 prf-gostr3411-94 GOST R 34.11-94 PRF
      1.2.643.2.2.30.0 id-GostR3411-94-TestParamSet
      1.2.643.2.2.30.1 id-GostR3411-94-CryptoProParamSet
      1.2.643.2.2.31.0 id-Gost28147-89-TestParamSet
      1.2.643.2.2.31.1 id-Gost28147-89-CryptoPro-A-ParamSet
      1.2.643.2.2.31.2 id-Gost28147-89-CryptoPro-B-ParamSet
      1.2.643.2.2.31.3 id-Gost28147-89-CryptoPro-C-ParamSet
      1.2.643.2.2.31.4 id-Gost28147-89-CryptoPro-D-ParamSet
      1.2.643.2.2.31.5 id-Gost28147-89-CryptoPro-Oscar-1-1-ParamSet
      1.2.643.2.2.31.6 id-Gost28147-89-CryptoPro-Oscar-1-0-ParamSet
      1.2.643.2.2.31.7 id-Gost28147-89-CryptoPro-RIC-1-ParamSet
      1.2.643.2.2.32.0 id-GostR3410-94-TestParamSet
      1.2.643.2.2.32.2 id-GostR3410-94-CryptoPro-A-ParamSet
      1.2.643.2.2.32.3 id-GostR3410-94-CryptoPro-B-ParamSet
      1.2.643.2.2.32.4 id-GostR3410-94-CryptoPro-C-ParamSet
      1.2.643.2.2.32.5 id-GostR3410-94-CryptoPro-D-ParamSet
      1.2.643.2.2.33.1 id-GostR3410-94-CryptoPro-XchA-ParamSet
      1.2.643.2.2.33.2 id-GostR3410-94-CryptoPro-XchB-ParamSet
      1.2.643.2.2.33.3 id-GostR3410-94-CryptoPro-XchC-ParamSet
      1.2.643.2.2.35.0 id-GostR3410-2001-TestParamSet
      1.2.643.2.2.35.1 id-GostR3410-2001-CryptoPro-A-ParamSet
      1.2.643.2.2.35.2 id-GostR3410-2001-CryptoPro-B-ParamSet
      1.2.643.2.2.35.3 id-GostR3410-2001-CryptoPro-C-ParamSet
      1.2.643.2.2.36.0 id-GostR3410-2001-CryptoPro-XchA-ParamSet
      1.2.643.2.2.36.1 id-GostR3410-2001-CryptoPro-XchB-ParamSet
      1.2.643.2.2.98 id-GostR3410-2001DH GOST R 34.10-2001 DH
      1.2.643.2.2.99 id-GostR3410-94DH GOST R 34.10-94 DH
      1.2.643.2.9 cryptocom
      1.2.643.2.9.1.3.3 id-GostR3411-94-with-GostR3410-94-cc GOST R 34.11-94 with GOST R 34.10-94 Cryptocom
      1.2.643.2.9.1.3.4 id-GostR3411-94-with-GostR3410-2001-cc GOST R 34.11-94 with GOST R 34.10-2001 Cryptocom
      1.2.643.2.9.1.5.3 gost94cc GOST 34.10-94 Cryptocom
      1.2.643.2.9.1.5.4 gost2001cc GOST 34.10-2001 Cryptocom
      1.2.643.2.9.1.6.1 id-Gost28147-89-cc GOST 28147-89 Cryptocom ParamSet
      1.2.643.2.9.1.8.1 id-GostR3410-2001-ParamSet-cc GOST R 3410-2001 Parameter Set Cryptocom
      1.2.643.3.131.1.1 INN
      1.2.643.7.1 id-tc26
      1.2.643.7.1.1 id-tc26-algorithms
      1.2.643.7.1.1.1 id-tc26-sign
      1.2.643.7.1.1.1.1 gost2012_256 GOST R 34.10-2012 with 256 bit modulus
      1.2.643.7.1.1.1.2 gost2012_512 GOST R 34.10-2012 with 512 bit modulus
      1.2.643.7.1.1.2 id-tc26-digest
      1.2.643.7.1.1.2.2 md_gost12_256 GOST R 34.11-2012 with 256 bit hash
      1.2.643.7.1.1.2.3 md_gost12_512 GOST R 34.11-2012 with 512 bit hash
      1.2.643.7.1.1.3 id-tc26-signwithdigest
      1.2.643.7.1.1.3.2 id-tc26-signwithdigest-gost3410-2012-256 GOST R 34.10-2012 with GOST R 34.11-2012 (256 bit)
      1.2.643.7.1.1.3.3 id-tc26-signwithdigest-gost3410-2012-512 GOST R 34.10-2012 with GOST R 34.11-2012 (512 bit)
      1.2.643.7.1.1.4 id-tc26-mac
      1.2.643.7.1.1.4.1 id-tc26-hmac-gost-3411-2012-256 HMAC GOST 34.11-2012 256 bit
      1.2.643.7.1.1.4.2 id-tc26-hmac-gost-3411-2012-512 HMAC GOST 34.11-2012 512 bit
      1.2.643.7.1.1.5 id-tc26-cipher
      1.2.643.7.1.1.5.1 id-tc26-cipher-gostr3412-2015-magma
      1.2.643.7.1.1.5.1.1 magma-ctr-acpkm
      1.2.643.7.1.1.5.1.2 magma-ctr-acpkm-omac
      1.2.643.7.1.1.5.2 id-tc26-cipher-gostr3412-2015-kuznyechik
      1.2.643.7.1.1.5.2.1 kuznyechik-ctr-acpkm
      1.2.643.7.1.1.5.2.2 kuznyechik-ctr-acpkm-omac
      1.2.643.7.1.1.6 id-tc26-agreement
      1.2.643.7.1.1.6.1 id-tc26-agreement-gost-3410-2012-256
      1.2.643.7.1.1.6.2 id-tc26-agreement-gost-3410-2012-512
      1.2.643.7.1.1.7 id-tc26-wrap
      1.2.643.7.1.1.7.1 id-tc26-wrap-gostr3412-2015-magma
      1.2.643.7.1.1.7.1.1 magma-kexp15
      1.2.643.7.1.1.7.2 id-tc26-wrap-gostr3412-2015-kuznyechik
      1.2.643.7.1.1.7.2.1 kuznyechik-kexp15
      1.2.643.7.1.2 id-tc26-constants
      1.2.643.7.1.2.1 id-tc26-sign-constants
      1.2.643.7.1.2.1.1 id-tc26-gost-3410-2012-256-constants
      1.2.643.7.1.2.1.1.1 id-tc26-gost-3410-2012-256-paramSetA GOST R 34.10-2012 (256 bit) ParamSet A
      1.2.643.7.1.2.1.1.2 id-tc26-gost-3410-2012-256-paramSetB GOST R 34.10-2012 (256 bit) ParamSet B
      1.2.643.7.1.2.1.1.3 id-tc26-gost-3410-2012-256-paramSetC GOST R 34.10-2012 (256 bit) ParamSet C
      1.2.643.7.1.2.1.1.4 id-tc26-gost-3410-2012-256-paramSetD GOST R 34.10-2012 (256 bit) ParamSet D
      1.2.643.7.1.2.1.2 id-tc26-gost-3410-2012-512-constants
      1.2.643.7.1.2.1.2.0 id-tc26-gost-3410-2012-512-paramSetTest GOST R 34.10-2012 (512 bit) testing parameter set
      1.2.643.7.1.2.1.2.1 id-tc26-gost-3410-2012-512-paramSetA GOST R 34.10-2012 (512 bit) ParamSet A
      1.2.643.7.1.2.1.2.2 id-tc26-gost-3410-2012-512-paramSetB GOST R 34.10-2012 (512 bit) ParamSet B
      1.2.643.7.1.2.1.2.3 id-tc26-gost-3410-2012-512-paramSetC GOST R 34.10-2012 (512 bit) ParamSet C
      1.2.643.7.1.2.2 id-tc26-digest-constants
      1.2.643.7.1.2.5 id-tc26-cipher-constants
      1.2.643.7.1.2.5.1 id-tc26-gost-28147-constants
      1.2.643.7.1.2.5.1.1 id-tc26-gost-28147-param-Z GOST 28147-89 TC26 parameter set
      1.2.643.100.1 OGRN
      1.2.643.100.3 SNILS
      1.2.643.100.5 OGRNIP
      1.2.643.100.111 subjectSignTool Signing Tool of Subject
      1.2.643.100.112 issuerSignTool Signing Tool of Issuer
      1.2.643.100.113 classSignTool Class of Signing Tool
      1.2.643.100.113.1 classSignToolKC1 Class of Signing Tool KC1
      1.2.643.100.113.2 classSignToolKC2 Class of Signing Tool KC2
      1.2.643.100.113.3 classSignToolKC3 Class of Signing Tool KC3
      1.2.643.100.113.4 classSignToolKB1 Class of Signing Tool KB1
      1.2.643.100.113.5 classSignToolKB2 Class of Signing Tool KB2
      1.2.643.100.113.6 classSignToolKA1 Class of Signing Tool KA1
      1.2.804 ISO-UA
      1.2.804.2.1.1.1 ua-pki
      1.2.804.2.1.1.1.1.1.1 dstu28147 DSTU Gost 28147-2009
      1.2.804.2.1.1.1.1.1.1.2 dstu28147-ofb DSTU Gost 28147-2009 OFB mode
      1.2.804.2.1.1.1.1.1.1.3 dstu28147-cfb DSTU Gost 28147-2009 CFB mode
      1.2.804.2.1.1.1.1.1.1.5 dstu28147-wrap DSTU Gost 28147-2009 key wrap
      1.2.804.2.1.1.1.1.1.2 hmacWithDstu34311 HMAC DSTU Gost 34311-95
      1.2.804.2.1.1.1.1.2.1 dstu34311 DSTU Gost 34311-95
      1.2.804.2.1.1.1.1.3.1.1 dstu4145le DSTU 4145-2002 little endian
      1.2.804.2.1.1.1.1.3.1.1.1.1 dstu4145be DSTU 4145-2002 big endian
      1.2.804.2.1.1.1.1.3.1.1.2.0 uacurve0 DSTU curve 0
      1.2.804.2.1.1.1.1.3.1.1.2.1 uacurve1 DSTU curve 1
      1.2.804.2.1.1.1.1.3.1.1.2.2 uacurve2 DSTU curve 2
      1.2.804.2.1.1.1.1.3.1.1.2.3 uacurve3 DSTU curve 3
      1.2.804.2.1.1.1.1.3.1.1.2.4 uacurve4 DSTU curve 4
      1.2.804.2.1.1.1.1.3.1.1.2.5 uacurve5 DSTU curve 5
      1.2.804.2.1.1.1.1.3.1.1.2.6 uacurve6 DSTU curve 6
      1.2.804.2.1.1.1.1.3.1.1.2.7 uacurve7 DSTU curve 7
      1.2.804.2.1.1.1.1.3.1.1.2.8 uacurve8 DSTU curve 8
      1.2.804.2.1.1.1.1.3.1.1.2.9 uacurve9 DSTU curve 9
      1.2.840 ISO-US ISO US Member Body
      1.2.840.10040 X9-57 X9.57
      1.2.840.10040.2.1 holdInstructionNone Hold Instruction None
      1.2.840.10040.2.2 holdInstructionCallIssuer Hold Instruction Call Issuer
      1.2.840.10040.2.3 holdInstructionReject Hold Instruction Reject
      1.2.840.10040.4 X9cm X9.57 CM ?
      1.2.840.10040.4.1 DSA dsaEncryption
      1.2.840.10040.4.3 DSA-SHA1 dsaWithSHA1
      1.2.840.10045 ansi-X9-62 ANSI X9.62
      1.2.840.10045.1.1 prime-field
      1.2.840.10045.1.2 characteristic-two-field
      1.2.840.10045.1.2.3 id-characteristic-two-basis
      1.2.840.10045.1.2.3.1 onBasis
      1.2.840.10045.1.2.3.2 tpBasis
      1.2.840.10045.1.2.3.3 ppBasis
      1.2.840.10045.2.1 id-ecPublicKey
      1.2.840.10045.3.0.1 c2pnb163v1
      1.2.840.10045.3.0.2 c2pnb163v2
      1.2.840.10045.3.0.3 c2pnb163v3
      1.2.840.10045.3.0.4 c2pnb176v1
      1.2.840.10045.3.0.5 c2tnb191v1
      1.2.840.10045.3.0.6 c2tnb191v2
      1.2.840.10045.3.0.7 c2tnb191v3
      1.2.840.10045.3.0.8 c2onb191v4
      1.2.840.10045.3.0.9 c2onb191v5
      1.2.840.10045.3.0.10 c2pnb208w1
      1.2.840.10045.3.0.11 c2tnb239v1
      1.2.840.10045.3.0.12 c2tnb239v2
      1.2.840.10045.3.0.13 c2tnb239v3
      1.2.840.10045.3.0.14 c2onb239v4
      1.2.840.10045.3.0.15 c2onb239v5
      1.2.840.10045.3.0.16 c2pnb272w1
      1.2.840.10045.3.0.17 c2pnb304w1
      1.2.840.10045.3.0.18 c2tnb359v1
      1.2.840.10045.3.0.19 c2pnb368w1
      1.2.840.10045.3.0.20 c2tnb431r1
      1.2.840.10045.3.1.1 prime192v1
      1.2.840.10045.3.1.2 prime192v2
      1.2.840.10045.3.1.3 prime192v3
      1.2.840.10045.3.1.4 prime239v1
      1.2.840.10045.3.1.5 prime239v2
      1.2.840.10045.3.1.6 prime239v3
      1.2.840.10045.3.1.7 prime256v1
      1.2.840.10045.4.1 ecdsa-with-SHA1
      1.2.840.10045.4.2 ecdsa-with-Recommended
      1.2.840.10045.4.3 ecdsa-with-Specified
      1.2.840.10045.4.3.1 ecdsa-with-SHA224
      1.2.840.10045.4.3.2 ecdsa-with-SHA256
      1.2.840.10045.4.3.3 ecdsa-with-SHA384
      1.2.840.10045.4.3.4 ecdsa-with-SHA512
      1.2.840.10046.2.1 dhpublicnumber X9.42 DH
      1.2.840.113533.7.66.10 CAST5-CBC cast5-cbc
      1.2.840.113533.7.66.12 pbeWithMD5AndCast5CBC
      1.2.840.113533.7.66.13 id-PasswordBasedMAC password based MAC
      1.2.840.113533.7.66.30 id-DHBasedMac Diffie-Hellman based MAC
      1.2.840.113549 rsadsi RSA Data Security, Inc.
      1.2.840.113549.1 pkcs RSA Data Security, Inc. PKCS
      1.2.840.113549.1.1 pkcs1
      1.2.840.113549.1.1.1 rsaEncryption
      1.2.840.113549.1.1.2 RSA-MD2 md2WithRSAEncryption
      1.2.840.113549.1.1.3 RSA-MD4 md4WithRSAEncryption
      1.2.840.113549.1.1.4 RSA-MD5 md5WithRSAEncryption
      1.2.840.113549.1.1.5 RSA-SHA1 sha1WithRSAEncryption
      1.2.840.113549.1.1.6 rsaOAEPEncryptionSET
      1.2.840.113549.1.1.7 RSAES-OAEP rsaesOaep
      1.2.840.113549.1.1.8 MGF1 mgf1
      1.2.840.113549.1.1.9 PSPECIFIED pSpecified
      1.2.840.113549.1.1.10 RSASSA-PSS rsassaPss
      1.2.840.113549.1.1.11 RSA-SHA256 sha256WithRSAEncryption
      1.2.840.113549.1.1.12 RSA-SHA384 sha384WithRSAEncryption
      1.2.840.113549.1.1.13 RSA-SHA512 sha512WithRSAEncryption
      1.2.840.113549.1.1.14 RSA-SHA224 sha224WithRSAEncryption
      1.2.840.113549.1.1.15 RSA-SHA512/224 sha512-224WithRSAEncryption
      1.2.840.113549.1.1.16 RSA-SHA512/256 sha512-256WithRSAEncryption
      1.2.840.113549.1.3 pkcs3
      1.2.840.113549.1.3.1 dhKeyAgreement
      1.2.840.113549.1.5 pkcs5
      1.2.840.113549.1.5.1 PBE-MD2-DES pbeWithMD2AndDES-CBC
      1.2.840.113549.1.5.3 PBE-MD5-DES pbeWithMD5AndDES-CBC
      1.2.840.113549.1.5.4 PBE-MD2-RC2-64 pbeWithMD2AndRC2-CBC
      1.2.840.113549.1.5.6 PBE-MD5-RC2-64 pbeWithMD5AndRC2-CBC
      1.2.840.113549.1.5.10 PBE-SHA1-DES pbeWithSHA1AndDES-CBC
      1.2.840.113549.1.5.11 PBE-SHA1-RC2-64 pbeWithSHA1AndRC2-CBC
      1.2.840.113549.1.5.12 PBKDF2
      1.2.840.113549.1.5.13 PBES2
      1.2.840.113549.1.5.14 PBMAC1
      1.2.840.113549.1.7 pkcs7
      1.2.840.113549.1.7.1 pkcs7-data
      1.2.840.113549.1.7.2 pkcs7-signedData
      1.2.840.113549.1.7.3 pkcs7-envelopedData
      1.2.840.113549.1.7.4 pkcs7-signedAndEnvelopedData
      1.2.840.113549.1.7.5 pkcs7-digestData
      1.2.840.113549.1.7.6 pkcs7-encryptedData
      1.2.840.113549.1.9 pkcs9
      1.2.840.113549.1.9.1 emailAddress
      1.2.840.113549.1.9.2 unstructuredName
      1.2.840.113549.1.9.3 contentType
      1.2.840.113549.1.9.4 messageDigest
      1.2.840.113549.1.9.5 signingTime
      1.2.840.113549.1.9.6 countersignature
      1.2.840.113549.1.9.7 challengePassword
      1.2.840.113549.1.9.8 unstructuredAddress
      1.2.840.113549.1.9.9 extendedCertificateAttributes
      1.2.840.113549.1.9.14 extReq Extension Request
      1.2.840.113549.1.9.15 SMIME-CAPS S/MIME Capabilities
      1.2.840.113549.1.9.16 SMIME S/MIME
      1.2.840.113549.1.9.16.0 id-smime-mod
      1.2.840.113549.1.9.16.0.1 id-smime-mod-cms
      1.2.840.113549.1.9.16.0.2 id-smime-mod-ess
      1.2.840.113549.1.9.16.0.3 id-smime-mod-oid
      1.2.840.113549.1.9.16.0.4 id-smime-mod-msg-v3
      1.2.840.113549.1.9.16.0.5 id-smime-mod-ets-eSignature-88
      1.2.840.113549.1.9.16.0.6 id-smime-mod-ets-eSignature-97
      1.2.840.113549.1.9.16.0.7 id-smime-mod-ets-eSigPolicy-88
      1.2.840.113549.1.9.16.0.8 id-smime-mod-ets-eSigPolicy-97
      1.2.840.113549.1.9.16.1 id-smime-ct
      1.2.840.113549.1.9.16.1.1 id-smime-ct-receipt
      1.2.840.113549.1.9.16.1.2 id-smime-ct-authData
      1.2.840.113549.1.9.16.1.3 id-smime-ct-publishCert
      1.2.840.113549.1.9.16.1.4 id-smime-ct-TSTInfo
      1.2.840.113549.1.9.16.1.5 id-smime-ct-TDTInfo
      1.2.840.113549.1.9.16.1.6 id-smime-ct-contentInfo
      1.2.840.113549.1.9.16.1.7 id-smime-ct-DVCSRequestData
      1.2.840.113549.1.9.16.1.8 id-smime-ct-DVCSResponseData
      1.2.840.113549.1.9.16.1.9 id-smime-ct-compressedData
      1.2.840.113549.1.9.16.1.19 id-smime-ct-contentCollection
      1.2.840.113549.1.9.16.1.23 id-smime-ct-authEnvelopedData
      1.2.840.113549.1.9.16.1.24 id-ct-routeOriginAuthz
      1.2.840.113549.1.9.16.1.26 id-ct-rpkiManifest
      1.2.840.113549.1.9.16.1.27 id-ct-asciiTextWithCRLF
      1.2.840.113549.1.9.16.1.28 id-ct-xml
      1.2.840.113549.1.9.16.1.35 id-ct-rpkiGhostbusters
      1.2.840.113549.1.9.16.1.36 id-ct-resourceTaggedAttest
      1.2.840.113549.1.9.16.1.47 id-ct-geofeedCSVwithCRLF
      1.2.840.113549.1.9.16.1.48 id-ct-signedChecklist
      1.2.840.113549.1.9.16.1.49 id-ct-ASPA
      1.2.840.113549.1.9.16.1.50 id-ct-signedTAL
      1.2.840.113549.1.9.16.1.51 id-ct-rpkiSignedPrefixList
      1.2.840.113549.1.9.16.2 id-smime-aa
      1.2.840.113549.1.9.16.2.1 id-smime-aa-receiptRequest
      1.2.840.113549.1.9.16.2.2 id-smime-aa-securityLabel
      1.2.840.113549.1.9.16.2.3 id-smime-aa-mlExpandHistory
      1.2.840.113549.1.9.16.2.4 id-smime-aa-contentHint
      1.2.840.113549.1.9.16.2.5 id-smime-aa-msgSigDigest
      1.2.840.113549.1.9.16.2.6 id-smime-aa-encapContentType
      1.2.840.113549.1.9.16.2.7 id-smime-aa-contentIdentifier
      1.2.840.113549.1.9.16.2.8 id-smime-aa-macValue
      1.2.840.113549.1.9.16.2.9 id-smime-aa-equivalentLabels
      1.2.840.113549.1.9.16.2.10 id-smime-aa-contentReference
      1.2.840.113549.1.9.16.2.11 id-smime-aa-encrypKeyPref
      1.2.840.113549.1.9.16.2.12 id-smime-aa-signingCertificate
      1.2.840.113549.1.9.16.2.13 id-smime-aa-smimeEncryptCerts
      1.2.840.113549.1.9.16.2.14 id-smime-aa-timeStampToken
      1.2.840.113549.1.9.16.2.15 id-smime-aa-ets-sigPolicyId
      1.2.840.113549.1.9.16.2.16 id-smime-aa-ets-commitmentType
      1.2.840.113549.1.9.16.2.17 id-smime-aa-ets-signerLocation
      1.2.840.113549.1.9.16.2.18 id-smime-aa-ets-signerAttr
      1.2.840.113549.1.9.16.2.19 id-smime-aa-ets-otherSigCert
      1.2.840.113549.1.9.16.2.20 id-smime-aa-ets-contentTimestamp
      1.2.840.113549.1.9.16.2.21 id-smime-aa-ets-CertificateRefs
      1.2.840.113549.1.9.16.2.22 id-smime-aa-ets-RevocationRefs
      1.2.840.113549.1.9.16.2.23 id-smime-aa-ets-certValues
      1.2.840.113549.1.9.16.2.24 id-smime-aa-ets-revocationValues
      1.2.840.113549.1.9.16.2.25 id-smime-aa-ets-escTimeStamp
      1.2.840.113549.1.9.16.2.26 id-smime-aa-ets-certCRLTimestamp
      1.2.840.113549.1.9.16.2.27 id-smime-aa-ets-archiveTimeStamp
      1.2.840.113549.1.9.16.2.28 id-smime-aa-signatureType
      1.2.840.113549.1.9.16.2.29 id-smime-aa-dvcs-dvc
      1.2.840.113549.1.9.16.2.44 id-aa-ets-attrCertificateRefs
      1.2.840.113549.1.9.16.2.45 id-aa-ets-attrRevocationRefs
      1.2.840.113549.1.9.16.2.47 id-smime-aa-signingCertificateV2
      1.2.840.113549.1.9.16.2.48 id-aa-ets-archiveTimestampV2
      1.2.840.113549.1.9.16.3 id-smime-alg
      1.2.840.113549.1.9.16.3.1 id-smime-alg-ESDHwith3DES
      1.2.840.113549.1.9.16.3.2 id-smime-alg-ESDHwithRC2
      1.2.840.113549.1.9.16.3.3 id-smime-alg-3DESwrap
      1.2.840.113549.1.9.16.3.4 id-smime-alg-RC2wrap
      1.2.840.113549.1.9.16.3.5 id-smime-alg-ESDH
      1.2.840.113549.1.9.16.3.6 id-smime-alg-CMS3DESwrap
      1.2.840.113549.1.9.16.3.7 id-smime-alg-CMSRC2wrap
      1.2.840.113549.1.9.16.3.8 ZLIB zlib compression
      1.2.840.113549.1.9.16.3.9 id-alg-PWRI-KEK
      1.2.840.113549.1.9.16.3.28 id-alg-hkdf-with-sha256 HKDF-SHA256
      1.2.840.113549.1.9.16.3.29 id-alg-hkdf-with-sha384 HKDF-SHA384
      1.2.840.113549.1.9.16.3.30 id-alg-hkdf-with-sha512 HKDF-SHA512
      1.2.840.113549.1.9.16.4 id-smime-cd
      1.2.840.113549.1.9.16.4.1 id-smime-cd-ldap
      1.2.840.113549.1.9.16.5 id-smime-spq
      1.2.840.113549.1.9.16.5.1 id-smime-spq-ets-sqt-uri
      1.2.840.113549.1.9.16.5.2 id-smime-spq-ets-sqt-unotice
      1.2.840.113549.1.9.16.6 id-smime-cti
      1.2.840.113549.1.9.16.6.1 id-smime-cti-ets-proofOfOrigin
      1.2.840.113549.1.9.16.6.2 id-smime-cti-ets-proofOfReceipt
      1.2.840.113549.1.9.16.6.3 id-smime-cti-ets-proofOfDelivery
      1.2.840.113549.1.9.16.6.4 id-smime-cti-ets-proofOfSender
      1.2.840.113549.1.9.16.6.5 id-smime-cti-ets-proofOfApproval
      1.2.840.113549.1.9.16.6.6 id-smime-cti-ets-proofOfCreation
      1.2.840.113549.1.9.16.13 id-smime-ori
      1.2.840.113549.1.9.16.13.3 id-smime-ori-kem
      1.2.840.113549.1.9.20 friendlyName
      1.2.840.113549.1.9.21 localKeyID
      1.2.840.113549.1.9.22.1 x509Certificate
      1.2.840.113549.1.9.22.2 sdsiCertificate
      1.2.840.113549.1.9.23.1 x509Crl
      1.2.840.113549.1.9.52 id-aa-CMSAlgorithmProtection
      1.2.840.113549.1.12.1.1 PBE-SHA1-RC4-128 pbeWithSHA1And128BitRC4
      1.2.840.113549.1.12.1.2 PBE-SHA1-RC4-40 pbeWithSHA1And40BitRC4
      1.2.840.113549.1.12.1.3 PBE-SHA1-3DES pbeWithSHA1And3-KeyTripleDES-CBC
      1.2.840.113549.1.12.1.4 PBE-SHA1-2DES pbeWithSHA1And2-KeyTripleDES-CBC
      1.2.840.113549.1.12.1.5 PBE-SHA1-RC2-128 pbeWithSHA1And128BitRC2-CBC
      1.2.840.113549.1.12.1.6 PBE-SHA1-RC2-40 pbeWithSHA1And40BitRC2-CBC
      1.2.840.113549.1.12.10.1.1 keyBag
      1.2.840.113549.1.12.10.1.2 pkcs8ShroudedKeyBag
      1.2.840.113549.1.12.10.1.3 certBag
      1.2.840.113549.1.12.10.1.4 crlBag
      1.2.840.113549.1.12.10.1.5 secretBag
      1.2.840.113549.1.12.10.1.6 safeContentsBag
      1.2.840.113549.2.2 MD2 md2
      1.2.840.113549.2.4 MD4 md4
      1.2.840.113549.2.5 MD5 md5
      1.2.840.113549.2.6 hmacWithMD5
      1.2.840.113549.2.7 hmacWithSHA1
      1.2.840.113549.2.8 hmacWithSHA224
      1.2.840.113549.2.9 hmacWithSHA256
      1.2.840.113549.2.10 hmacWithSHA384
      1.2.840.113549.2.11 hmacWithSHA512
      1.2.840.113549.2.12 hmacWithSHA512-224
      1.2.840.113549.2.13 hmacWithSHA512-256
      1.2.840.113549.3.2 RC2-CBC rc2-cbc
      1.2.840.113549.3.4 RC4 rc4
      1.2.840.113549.3.7 DES-EDE3-CBC des-ede3-cbc
      1.2.840.113549.3.8 RC5-CBC rc5-cbc
      1.2.840.113549.3.10 DES-CDMF des-cdmf
      1.3 ORG org
      1.3.6 DOD dod
      1.3.6.1 IANA iana
      1.3.6.1.1 directory Directory
      1.3.6.1.2 mgmt Management
      1.3.6.1.3 experimental Experimental
      1.3.6.1.4 private Private
      1.3.6.1.4.1 enterprises Enterprises
      1.3.6.1.4.1.188.7.1.1.2 IDEA-CBC idea-cbc
      1.3.6.1.4.1.311.2.1.14 msExtReq Microsoft Extension Request
      1.3.6.1.4.1.311.2.1.21 msCodeInd Microsoft Individual Code Signing
      1.3.6.1.4.1.311.2.1.22 msCodeCom Microsoft Commercial Code Signing
      1.3.6.1.4.1.311.10.3.1 msCTLSign Microsoft Trust List Signing
      1.3.6.1.4.1.311.10.3.3 msSGC Microsoft Server Gated Crypto
      1.3.6.1.4.1.311.10.3.4 msEFS Microsoft Encrypted File System
      1.3.6.1.4.1.311.17.1 CSPName Microsoft CSP Name
      1.3.6.1.4.1.311.17.2 LocalKeySet Microsoft Local Key set
      1.3.6.1.4.1.311.20.2.2 msSmartcardLogin Microsoft Smartcard Login
      1.3.6.1.4.1.311.20.2.3 msUPN Microsoft User Principal Name
      1.3.6.1.4.1.311.21.7 ms-cert-templ Microsoft certificate template
      1.3.6.1.4.1.311.21.10 ms-app-policies Microsoft Application Policies Extension
      1.3.6.1.4.1.311.25.2 ms-ntds-sec-ext Microsoft NTDS CA Extension
      1.3.6.1.4.1.311.25.2.1 ms-ntds-obj-sid Microsoft NTDS AD objectSid
      1.3.6.1.4.1.311.60.2.1.1 jurisdictionL jurisdictionLocalityName
      1.3.6.1.4.1.311.60.2.1.2 jurisdictionST jurisdictionStateOrProvinceName
      1.3.6.1.4.1.311.60.2.1.3 jurisdictionC jurisdictionCountryName
      1.3.6.1.4.1.1466.344 dcobject dcObject
      1.3.6.1.4.1.1722.12.2.1 BLAKE2BMAC blake2bmac
      1.3.6.1.4.1.1722.12.2.1.16 BLAKE2b512 blake2b512
      1.3.6.1.4.1.1722.12.2.2 BLAKE2SMAC blake2smac
      1.3.6.1.4.1.1722.12.2.2.8 BLAKE2s256 blake2s256
      1.3.6.1.4.1.3029.1.2 BF-CBC bf-cbc
      1.3.6.1.4.1.11129.2.4.2 ct_precert_scts CT Precertificate SCTs
      1.3.6.1.4.1.11129.2.4.3 ct_precert_poison CT Precertificate Poison
      1.3.6.1.4.1.11129.2.4.4 ct_precert_signer CT Precertificate Signer
      1.3.6.1.4.1.11129.2.4.5 ct_cert_scts CT Certificate SCTs
      1.3.6.1.4.1.11591.4.11 id-scrypt scrypt
      1.3.6.1.4.1.45605.1 id-kp-wisun-fan-device Wi-SUN Alliance Field Area Network (FAN)
      1.3.6.1.5 security Security
      1.3.6.1.5.2.3 id-pkinit
      1.3.6.1.5.2.3.4 pkInitClientAuth PKINIT Client Auth
      1.3.6.1.5.2.3.5 pkInitKDC Signing KDC Response
      1.3.6.1.5.5.7 PKIX
      1.3.6.1.5.5.7.0 id-pkix-mod
      1.3.6.1.5.5.7.0.1 id-pkix1-explicit-88
      1.3.6.1.5.5.7.0.2 id-pkix1-implicit-88
      1.3.6.1.5.5.7.0.3 id-pkix1-explicit-93
      1.3.6.1.5.5.7.0.4 id-pkix1-implicit-93
      1.3.6.1.5.5.7.0.5 id-mod-crmf
      1.3.6.1.5.5.7.0.6 id-mod-cmc
      1.3.6.1.5.5.7.0.7 id-mod-kea-profile-88
      1.3.6.1.5.5.7.0.8 id-mod-kea-profile-93
      1.3.6.1.5.5.7.0.9 id-mod-cmp
      1.3.6.1.5.5.7.0.10 id-mod-qualified-cert-88
      1.3.6.1.5.5.7.0.11 id-mod-qualified-cert-93
      1.3.6.1.5.5.7.0.12 id-mod-attribute-cert
      1.3.6.1.5.5.7.0.13 id-mod-timestamp-protocol
      1.3.6.1.5.5.7.0.14 id-mod-ocsp
      1.3.6.1.5.5.7.0.15 id-mod-dvcs
      1.3.6.1.5.5.7.0.16 id-mod-cmp2000
      1.3.6.1.5.5.7.0.50 id-mod-cmp2000-02
      1.3.6.1.5.5.7.0.99 id-mod-cmp2021-88
      1.3.6.1.5.5.7.0.100 id-mod-cmp2021-02
      1.3.6.1.5.5.7.1 id-pe
      1.3.6.1.5.5.7.1.1 authorityInfoAccess Authority Information Access
      1.3.6.1.5.5.7.1.2 biometricInfo Biometric Info
      1.3.6.1.5.5.7.1.3 qcStatements
      1.3.6.1.5.5.7.1.4 ac-auditIdentity X509v3 Audit Identity
      1.3.6.1.5.5.7.1.5 ac-targeting
      1.3.6.1.5.5.7.1.6 aaControls
      1.3.6.1.5.5.7.1.7 sbgp-ipAddrBlock
      1.3.6.1.5.5.7.1.8 sbgp-autonomousSysNum
      1.3.6.1.5.5.7.1.9 sbgp-routerIdentifier
      1.3.6.1.5.5.7.1.10 ac-proxying
      1.3.6.1.5.5.7.1.11 subjectInfoAccess Subject Information Access
      1.3.6.1.5.5.7.1.14 proxyCertInfo Proxy Certificate Information
      1.3.6.1.5.5.7.1.24 tlsfeature TLS Feature
      1.3.6.1.5.5.7.1.28 sbgp-ipAddrBlockv2
      1.3.6.1.5.5.7.1.29 sbgp-autonomousSysNumv2
      1.3.6.1.5.5.7.2 id-qt
      1.3.6.1.5.5.7.2.1 id-qt-cps Policy Qualifier CPS
      1.3.6.1.5.5.7.2.2 id-qt-unotice Policy Qualifier User Notice
      1.3.6.1.5.5.7.2.3 textNotice
      1.3.6.1.5.5.7.3 id-kp
      1.3.6.1.5.5.7.3.1 serverAuth TLS Web Server Authentication
      1.3.6.1.5.5.7.3.2 clientAuth TLS Web Client Authentication
      1.3.6.1.5.5.7.3.3 codeSigning Code Signing
      1.3.6.1.5.5.7.3.4 emailProtection E-mail Protection
      1.3.6.1.5.5.7.3.5 ipsecEndSystem IPSec End System
      1.3.6.1.5.5.7.3.6 ipsecTunnel IPSec Tunnel
      1.3.6.1.5.5.7.3.7 ipsecUser IPSec User
      1.3.6.1.5.5.7.3.8 timeStamping Time Stamping
      1.3.6.1.5.5.7.3.9 OCSPSigning OCSP Signing
      1.3.6.1.5.5.7.3.10 DVCS dvcs
      1.3.6.1.5.5.7.3.17 ipsecIKE ipsec Internet Key Exchange
      1.3.6.1.5.5.7.3.18 capwapAC Ctrl/provision WAP Access
      1.3.6.1.5.5.7.3.19 capwapWTP Ctrl/Provision WAP Termination
      1.3.6.1.5.5.7.3.21 secureShellClient SSH Client
      1.3.6.1.5.5.7.3.22 secureShellServer SSH Server
      1.3.6.1.5.5.7.3.23 sendRouter Send Router
      1.3.6.1.5.5.7.3.24 sendProxiedRouter Send Proxied Router
      1.3.6.1.5.5.7.3.25 sendOwner Send Owner
      1.3.6.1.5.5.7.3.26 sendProxiedOwner Send Proxied Owner
      1.3.6.1.5.5.7.3.27 cmcCA CMC Certificate Authority
      1.3.6.1.5.5.7.3.28 cmcRA CMC Registration Authority
      1.3.6.1.5.5.7.3.29 cmcArchive CMC Archive Server
      1.3.6.1.5.5.7.3.30 id-kp-bgpsec-router BGPsec Router
      1.3.6.1.5.5.7.3.31 id-kp-BrandIndicatorforMessageIdentification Brand Indicator for Message Identification
      1.3.6.1.5.5.7.3.32 cmKGA Certificate Management Key Generation Authority
      1.3.6.1.5.5.7.4 id-it
      1.3.6.1.5.5.7.4.1 id-it-caProtEncCert
      1.3.6.1.5.5.7.4.2 id-it-signKeyPairTypes
      1.3.6.1.5.5.7.4.3 id-it-encKeyPairTypes
      1.3.6.1.5.5.7.4.4 id-it-preferredSymmAlg
      1.3.6.1.5.5.7.4.5 id-it-caKeyUpdateInfo
      1.3.6.1.5.5.7.4.6 id-it-currentCRL
      1.3.6.1.5.5.7.4.7 id-it-unsupportedOIDs
      1.3.6.1.5.5.7.4.8 id-it-subscriptionRequest
      1.3.6.1.5.5.7.4.9 id-it-subscriptionResponse
      1.3.6.1.5.5.7.4.10 id-it-keyPairParamReq
      1.3.6.1.5.5.7.4.11 id-it-keyPairParamRep
      1.3.6.1.5.5.7.4.12 id-it-revPassphrase
      1.3.6.1.5.5.7.4.13 id-it-implicitConfirm
      1.3.6.1.5.5.7.4.14 id-it-confirmWaitTime
      1.3.6.1.5.5.7.4.15 id-it-origPKIMessage
      1.3.6.1.5.5.7.4.16 id-it-suppLangTags
      1.3.6.1.5.5.7.4.17 id-it-caCerts
      1.3.6.1.5.5.7.4.18 id-it-rootCaKeyUpdate
      1.3.6.1.5.5.7.4.19 id-it-certReqTemplate
      1.3.6.1.5.5.7.4.20 id-it-rootCaCert
      1.3.6.1.5.5.7.4.21 id-it-certProfile
      1.3.6.1.5.5.7.4.22 id-it-crlStatusList
      1.3.6.1.5.5.7.4.23 id-it-crls
      1.3.6.1.5.5.7.5 id-pkip
      1.3.6.1.5.5.7.5.1 id-regCtrl
      1.3.6.1.5.5.7.5.1.1 id-regCtrl-regToken
      1.3.6.1.5.5.7.5.1.2 id-regCtrl-authenticator
      1.3.6.1.5.5.7.5.1.3 id-regCtrl-pkiPublicationInfo
      1.3.6.1.5.5.7.5.1.4 id-regCtrl-pkiArchiveOptions
      1.3.6.1.5.5.7.5.1.5 id-regCtrl-oldCertID
      1.3.6.1.5.5.7.5.1.6 id-regCtrl-protocolEncrKey
      1.3.6.1.5.5.7.5.1.7 id-regCtrl-altCertTemplate
      1.3.6.1.5.5.7.5.1.11 id-regCtrl-algId
      1.3.6.1.5.5.7.5.1.12 id-regCtrl-rsaKeyLen
      1.3.6.1.5.5.7.5.2 id-regInfo
      1.3.6.1.5.5.7.5.2.1 id-regInfo-utf8Pairs
      1.3.6.1.5.5.7.5.2.2 id-regInfo-certReq
      1.3.6.1.5.5.7.6 id-alg
      1.3.6.1.5.5.7.6.1 id-alg-des40
      1.3.6.1.5.5.7.6.2 id-alg-noSignature
      1.3.6.1.5.5.7.6.3 id-alg-dh-sig-hmac-sha1
      1.3.6.1.5.5.7.6.4 id-alg-dh-pop
      1.3.6.1.5.5.7.7 id-cmc
      1.3.6.1.5.5.7.7.1 id-cmc-statusInfo
      1.3.6.1.5.5.7.7.2 id-cmc-identification
      1.3.6.1.5.5.7.7.3 id-cmc-identityProof
      1.3.6.1.5.5.7.7.4 id-cmc-dataReturn
      1.3.6.1.5.5.7.7.5 id-cmc-transactionId
      1.3.6.1.5.5.7.7.6 id-cmc-senderNonce
      1.3.6.1.5.5.7.7.7 id-cmc-recipientNonce
      1.3.6.1.5.5.7.7.8 id-cmc-addExtensions
      1.3.6.1.5.5.7.7.9 id-cmc-encryptedPOP
      1.3.6.1.5.5.7.7.10 id-cmc-decryptedPOP
      1.3.6.1.5.5.7.7.11 id-cmc-lraPOPWitness
      1.3.6.1.5.5.7.7.15 id-cmc-getCert
      1.3.6.1.5.5.7.7.16 id-cmc-getCRL
      1.3.6.1.5.5.7.7.17 id-cmc-revokeRequest
      1.3.6.1.5.5.7.7.18 id-cmc-regInfo
      1.3.6.1.5.5.7.7.19 id-cmc-responseInfo
      1.3.6.1.5.5.7.7.21 id-cmc-queryPending
      1.3.6.1.5.5.7.7.22 id-cmc-popLinkRandom
      1.3.6.1.5.5.7.7.23 id-cmc-popLinkWitness
      1.3.6.1.5.5.7.7.24 id-cmc-confirmCertAcceptance
      1.3.6.1.5.5.7.8 id-on
      1.3.6.1.5.5.7.8.1 id-on-personalData
      1.3.6.1.5.5.7.8.3 id-on-permanentIdentifier Permanent Identifier
      1.3.6.1.5.5.7.8.4 id-on-hardwareModuleName Hardware Module Name
      1.3.6.1.5.5.7.8.5 id-on-xmppAddr XmppAddr
      1.3.6.1.5.5.7.8.7 id-on-dnsSRV SRVName
      1.3.6.1.5.5.7.8.8 id-on-NAIRealm NAIRealm
      1.3.6.1.5.5.7.8.9 id-on-SmtpUTF8Mailbox Smtp UTF8 Mailbox
      1.3.6.1.5.5.7.9 id-pda
      1.3.6.1.5.5.7.9.1 id-pda-dateOfBirth
      1.3.6.1.5.5.7.9.2 id-pda-placeOfBirth
      1.3.6.1.5.5.7.9.3 id-pda-gender
      1.3.6.1.5.5.7.9.4 id-pda-countryOfCitizenship
      1.3.6.1.5.5.7.9.5 id-pda-countryOfResidence
      1.3.6.1.5.5.7.10 id-aca
      1.3.6.1.5.5.7.10.1 id-aca-authenticationInfo
      1.3.6.1.5.5.7.10.2 id-aca-accessIdentity
      1.3.6.1.5.5.7.10.3 id-aca-chargingIdentity
      1.3.6.1.5.5.7.10.4 id-aca-group
      1.3.6.1.5.5.7.10.5 id-aca-role
      1.3.6.1.5.5.7.10.6 id-aca-encAttrs
      1.3.6.1.5.5.7.11 id-qcs
      1.3.6.1.5.5.7.11.1 id-qcs-pkixQCSyntax-v1
      1.3.6.1.5.5.7.12 id-cct
      1.3.6.1.5.5.7.12.1 id-cct-crs
      1.3.6.1.5.5.7.12.2 id-cct-PKIData
      1.3.6.1.5.5.7.12.3 id-cct-PKIResponse
      1.3.6.1.5.5.7.14 id-cp
      1.3.6.1.5.5.7.14.2 ipAddr-asNumber
      1.3.6.1.5.5.7.14.3 ipAddr-asNumberv2
      1.3.6.1.5.5.7.21 id-ppl
      1.3.6.1.5.5.7.21.0 id-ppl-anyLanguage Any language
      1.3.6.1.5.5.7.21.1 id-ppl-inheritAll Inherit all
      1.3.6.1.5.5.7.21.2 id-ppl-independent Independent
      1.3.6.1.5.5.7.48 id-ad
      1.3.6.1.5.5.7.48.1 OCSP
      1.3.6.1.5.5.7.48.1.1 basicOCSPResponse Basic OCSP Response
      1.3.6.1.5.5.7.48.1.2 Nonce OCSP Nonce
      1.3.6.1.5.5.7.48.1.3 CrlID OCSP CRL ID
      1.3.6.1.5.5.7.48.1.4 acceptableResponses Acceptable OCSP Responses
      1.3.6.1.5.5.7.48.1.5 noCheck OCSP No Check
      1.3.6.1.5.5.7.48.1.6 archiveCutoff OCSP Archive Cutoff
      1.3.6.1.5.5.7.48.1.7 serviceLocator OCSP Service Locator
      1.3.6.1.5.5.7.48.1.8 extendedStatus Extended OCSP Status
      1.3.6.1.5.5.7.48.1.9 valid
      1.3.6.1.5.5.7.48.1.10 path
      1.3.6.1.5.5.7.48.1.11 trustRoot Trust Root
      1.3.6.1.5.5.7.48.2 caIssuers CA Issuers
      1.3.6.1.5.5.7.48.3 ad_timestamping AD Time Stamping
      1.3.6.1.5.5.7.48.4 AD_DVCS ad dvcs
      1.3.6.1.5.5.7.48.5 caRepository CA Repository
      1.3.6.1.5.5.7.48.10 rpkiManifest RPKI Manifest
      1.3.6.1.5.5.7.48.11 signedObject Signed Object
      1.3.6.1.5.5.7.48.13 rpkiNotify RPKI Notify
      1.3.6.1.5.5.8.1.1 HMAC-MD5 hmac-md5
      1.3.6.1.5.5.8.1.2 HMAC-SHA1 hmac-sha1
      1.3.6.1.6 snmpv2 SNMPv2
      1.3.6.1.7 Mail
      1.3.6.1.7.1 mime-mhs MIME MHS
      1.3.6.1.7.1.1 mime-mhs-headings
      1.3.6.1.7.1.1.1 id-hex-partial-message
      1.3.6.1.7.1.1.2 id-hex-multipart-message
      1.3.6.1.7.1.2 mime-mhs-bodies
      1.3.14.3.2 algorithm
      1.3.14.3.2.3 RSA-NP-MD5 md5WithRSA
      1.3.14.3.2.6 DES-ECB des-ecb
      1.3.14.3.2.7 DES-CBC des-cbc
      1.3.14.3.2.8 DES-OFB des-ofb
      1.3.14.3.2.9 DES-CFB des-cfb
      1.3.14.3.2.11 rsaSignature
      1.3.14.3.2.12 DSA-old dsaEncryption-old
      1.3.14.3.2.13 DSA-SHA dsaWithSHA
      1.3.14.3.2.15 RSA-SHA shaWithRSAEncryption
      1.3.14.3.2.17 DES-EDE des-ede
      1.3.14.3.2.18 SHA sha
      1.3.14.3.2.26 SHA1 sha1
      1.3.14.3.2.27 DSA-SHA1-old dsaWithSHA1-old
      1.3.14.3.2.29 RSA-SHA1-2 sha1WithRSA
      1.3.36.3.2.1 RIPEMD160 ripemd160
      1.3.36.3.3.1.2 RSA-RIPEMD160 ripemd160WithRSA
      1.3.36.3.3.2.8.1.1.1 brainpoolP160r1
      1.3.36.3.3.2.8.1.1.2 brainpoolP160t1
      1.3.36.3.3.2.8.1.1.3 brainpoolP192r1
      1.3.36.3.3.2.8.1.1.4 brainpoolP192t1
      1.3.36.3.3.2.8.1.1.5 brainpoolP224r1
      1.3.36.3.3.2.8.1.1.6 brainpoolP224t1
      1.3.36.3.3.2.8.1.1.7 brainpoolP256r1
      1.3.36.3.3.2.8.1.1.8 brainpoolP256t1
      1.3.36.3.3.2.8.1.1.9 brainpoolP320r1
      1.3.36.3.3.2.8.1.1.10 brainpoolP320t1
      1.3.36.3.3.2.8.1.1.11 brainpoolP384r1
      1.3.36.3.3.2.8.1.1.12 brainpoolP384t1
      1.3.36.3.3.2.8.1.1.13 brainpoolP512r1
      1.3.36.3.3.2.8.1.1.14 brainpoolP512t1
      1.3.36.8.3.3 x509ExtAdmission Professional Information or basis for Admission
      1.3.101.1.4.1 SXNetID Strong Extranet ID
      1.3.101.110 X25519
      1.3.101.111 X448
      1.3.101.112 ED25519
      1.3.101.113 ED448
      1.3.111 ieee
      1.3.111.2.1619 ieee-siswg IEEE Security in Storage Working Group
      1.3.111.2.1619.0.1.1 AES-128-XTS aes-128-xts
      1.3.111.2.1619.0.1.2 AES-256-XTS aes-256-xts
      1.3.132 certicom-arc
      1.3.132.0.1 sect163k1
      1.3.132.0.2 sect163r1
      1.3.132.0.3 sect239k1
      1.3.132.0.4 sect113r1
      1.3.132.0.5 sect113r2
      1.3.132.0.6 secp112r1
      1.3.132.0.7 secp112r2
      1.3.132.0.8 secp160r1
      1.3.132.0.9 secp160k1
      1.3.132.0.10 secp256k1
      1.3.132.0.15 sect163r2
      1.3.132.0.16 sect283k1
      1.3.132.0.17 sect283r1
      1.3.132.0.22 sect131r1
      1.3.132.0.23 sect131r2
      1.3.132.0.24 sect193r1
      1.3.132.0.25 sect193r2
      1.3.132.0.26 sect233k1
      1.3.132.0.27 sect233r1
      1.3.132.0.28 secp128r1
      1.3.132.0.29 secp128r2
      1.3.132.0.30 secp160r2
      1.3.132.0.31 secp192k1
      1.3.132.0.32 secp224k1
      1.3.132.0.33 secp224r1
      1.3.132.0.34 secp384r1
      1.3.132.0.35 secp521r1
      1.3.132.0.36 sect409k1
      1.3.132.0.37 sect409r1
      1.3.132.0.38 sect571k1
      1.3.132.0.39 sect571r1
      1.3.132.1.11.0 dhSinglePass-stdDH-sha224kdf-scheme
      1.3.132.1.11.1 dhSinglePass-stdDH-sha256kdf-scheme
      1.3.132.1.11.2 dhSinglePass-stdDH-sha384kdf-scheme
      1.3.132.1.11.3 dhSinglePass-stdDH-sha512kdf-scheme
      1.3.132.1.14.0 dhSinglePass-cofactorDH-sha224kdf-scheme
      1.3.132.1.14.1 dhSinglePass-cofactorDH-sha256kdf-scheme
      1.3.132.1.14.2 dhSinglePass-cofactorDH-sha384kdf-scheme
      1.3.132.1.14.3 dhSinglePass-cofactorDH-sha512kdf-scheme
      1.3.133.16.840.63.0.2 dhSinglePass-stdDH-sha1kdf-scheme
      1.3.133.16.840.63.0.3 dhSinglePass-cofactorDH-sha1kdf-scheme
      2.5 X500 directory services (X.500)
      2.5.1.5 selected-attribute-types Selected Attribute Types
      2.5.1.5.55 clearance
      2.5.4 X509
      2.5.4.3 CN commonName
      2.5.4.4 SN surname
      2.5.4.5 serialNumber
      2.5.4.6 C countryName
      2.5.4.7 L localityName
      2.5.4.8 ST stateOrProvinceName
      2.5.4.9 street streetAddress
      2.5.4.10 O organizationName
      2.5.4.11 OU organizationalUnitName
      2.5.4.12 title
      2.5.4.13 description
      2.5.4.14 searchGuide
      2.5.4.15 businessCategory
      2.5.4.16 postalAddress
      2.5.4.17 postalCode
      2.5.4.18 postOfficeBox
      2.5.4.19 physicalDeliveryOfficeName
      2.5.4.20 telephoneNumber
      2.5.4.21 telexNumber
      2.5.4.22 teletexTerminalIdentifier
      2.5.4.23 facsimileTelephoneNumber
      2.5.4.24 x121Address
      2.5.4.25 internationaliSDNNumber
      2.5.4.26 registeredAddress
      2.5.4.27 destinationIndicator
      2.5.4.28 preferredDeliveryMethod
      2.5.4.29 presentationAddress
      2.5.4.30 supportedApplicationContext
      2.5.4.31 member
      2.5.4.32 owner
      2.5.4.33 roleOccupant
      2.5.4.34 seeAlso
      2.5.4.35 userPassword
      2.5.4.36 userCertificate
      2.5.4.37 cACertificate
      2.5.4.38 authorityRevocationList
      2.5.4.39 certificateRevocationList
      2.5.4.40 crossCertificatePair
      2.5.4.41 name
      2.5.4.42 GN givenName
      2.5.4.43 initials
      2.5.4.44 generationQualifier
      2.5.4.45 x500UniqueIdentifier
      2.5.4.46 dnQualifier
      2.5.4.47 enhancedSearchGuide
      2.5.4.48 protocolInformation
      2.5.4.49 distinguishedName
      2.5.4.50 uniqueMember
      2.5.4.51 houseIdentifier
      2.5.4.52 supportedAlgorithms
      2.5.4.53 deltaRevocationList
      2.5.4.54 dmdName
      2.5.4.65 pseudonym
      2.5.4.72 role
      2.5.4.97 organizationIdentifier
      2.5.4.98 c3 countryCode3c
      2.5.4.99 n3 countryCode3n
      2.5.4.100 dnsName
      2.5.8 X500algorithms directory services - algorithms
      2.5.8.1.1 RSA rsa
      2.5.8.3.100 RSA-MDC2 mdc2WithRSA
      2.5.8.3.101 MDC2 mdc2
      2.5.29 id-ce
      2.5.29.9 subjectDirectoryAttributes X509v3 Subject Directory Attributes
      2.5.29.14 subjectKeyIdentifier X509v3 Subject Key Identifier
      2.5.29.15 keyUsage X509v3 Key Usage
      2.5.29.16 privateKeyUsagePeriod X509v3 Private Key Usage Period
      2.5.29.17 subjectAltName X509v3 Subject Alternative Name
      2.5.29.18 issuerAltName X509v3 Issuer Alternative Name
      2.5.29.19 basicConstraints X509v3 Basic Constraints
      2.5.29.20 crlNumber X509v3 CRL Number
      2.5.29.21 CRLReason X509v3 CRL Reason Code
      2.5.29.23 holdInstructionCode Hold Instruction Code
      2.5.29.24 invalidityDate Invalidity Date
      2.5.29.27 deltaCRL X509v3 Delta CRL Indicator
      2.5.29.28 issuingDistributionPoint X509v3 Issuing Distribution Point
      2.5.29.29 certificateIssuer X509v3 Certificate Issuer
      2.5.29.30 nameConstraints X509v3 Name Constraints
      2.5.29.31 crlDistributionPoints X509v3 CRL Distribution Points
      2.5.29.32 certificatePolicies X509v3 Certificate Policies
      2.5.29.32.0 anyPolicy X509v3 Any Policy
      2.5.29.33 policyMappings X509v3 Policy Mappings
      2.5.29.35 authorityKeyIdentifier X509v3 Authority Key Identifier
      2.5.29.36 policyConstraints X509v3 Policy Constraints
      2.5.29.37 extendedKeyUsage X509v3 Extended Key Usage
      2.5.29.37.0 anyExtendedKeyUsage Any Extended Key Usage
      2.5.29.38 authorityAttributeIdentifier X509v3 Authority Attribute Identifier
      2.5.29.39 roleSpecCertIdentifier X509v3 Role Specification Certificate Identifier
      2.5.29.41 basicAttConstraints X509v3 Basic Attribute Certificate Constraints
      2.5.29.42 delegatedNameConstraints X509v3 Delegated Name Constraints
      2.5.29.43 timeSpecification X509v3 Time Specification
      2.5.29.46 freshestCRL X509v3 Freshest CRL
      2.5.29.48 attributeDescriptor X509v3 Attribute Descriptor
      2.5.29.49 userNotice X509v3 User Notice
      2.5.29.50 sOAIdentifier X509v3 Source of Authority Identifier
      2.5.29.52 acceptableCertPolicies X509v3 Acceptable Certification Policies
      2.5.29.54 inhibitAnyPolicy X509v3 Inhibit Any Policy
      2.5.29.55 targetInformation X509v3 AC Targeting
      2.5.29.56 noRevAvail X509v3 No Revocation Available
      2.5.29.57 acceptablePrivPolicies X509v3 Acceptable Privilege Policies
      2.5.29.61 indirectIssuer X509v3 Indirect Issuer
      2.5.29.62 noAssertion X509v3 No Assertion
      2.5.29.63 aAissuingDistributionPoint X509v3 Attribute Authority Issuing Distribution Point
      2.5.29.64 issuedOnBehalfOf X509v3 Issued On Behalf Of
      2.5.29.65 singleUse X509v3 Single Use
      2.5.29.66 groupAC X509v3 Group Attribute Certificate
      2.5.29.67 allowedAttributeAssignments X509v3 Allowed Attribute Assignments
      2.5.29.68 attributeMappings X509v3 Attribute Mappings
      2.5.29.69 holderNameConstraints X509v3 Holder Name Constraints
      2.5.29.70 authorizationValidation X509v3 Authorization Validation
      2.5.29.71 protRestrict X509v3 Protocol Restriction
      2.5.29.72 subjectAltPublicKeyInfo X509v3 Subject Alternative Public Key Info
      2.5.29.73 altSignatureAlgorithm X509v3 Alternative Signature Algorithm
      2.5.29.74 altSignatureValue X509v3 Alternative Signature Value
      2.5.29.75 associatedInformation X509v3 Associated Information
      2.16.840.1.101.3.4.1.1 AES-128-ECB aes-128-ecb
      2.16.840.1.101.3.4.1.2 AES-128-CBC aes-128-cbc
      2.16.840.1.101.3.4.1.3 AES-128-OFB aes-128-ofb
      2.16.840.1.101.3.4.1.4 AES-128-CFB aes-128-cfb
      2.16.840.1.101.3.4.1.5 id-aes128-wrap
      2.16.840.1.101.3.4.1.6 id-aes128-GCM aes-128-gcm
      2.16.840.1.101.3.4.1.7 id-aes128-CCM aes-128-ccm
      2.16.840.1.101.3.4.1.8 id-aes128-wrap-pad
      2.16.840.1.101.3.4.1.21 AES-192-ECB aes-192-ecb
      2.16.840.1.101.3.4.1.22 AES-192-CBC aes-192-cbc
      2.16.840.1.101.3.4.1.23 AES-192-OFB aes-192-ofb
      2.16.840.1.101.3.4.1.24 AES-192-CFB aes-192-cfb
      2.16.840.1.101.3.4.1.25 id-aes192-wrap
      2.16.840.1.101.3.4.1.26 id-aes192-GCM aes-192-gcm
      2.16.840.1.101.3.4.1.27 id-aes192-CCM aes-192-ccm
      2.16.840.1.101.3.4.1.28 id-aes192-wrap-pad
      2.16.840.1.101.3.4.1.41 AES-256-ECB aes-256-ecb
      2.16.840.1.101.3.4.1.42 AES-256-CBC aes-256-cbc
      2.16.840.1.101.3.4.1.43 AES-256-OFB aes-256-ofb
      2.16.840.1.101.3.4.1.44 AES-256-CFB aes-256-cfb
      2.16.840.1.101.3.4.1.45 id-aes256-wrap
      2.16.840.1.101.3.4.1.46 id-aes256-GCM aes-256-gcm
      2.16.840.1.101.3.4.1.47 id-aes256-CCM aes-256-ccm
      2.16.840.1.101.3.4.1.48 id-aes256-wrap-pad
      2.16.840.1.101.3.4.2.1 SHA256 sha256
      2.16.840.1.101.3.4.2.2 SHA384 sha384
      2.16.840.1.101.3.4.2.3 SHA512 sha512
      2.16.840.1.101.3.4.2.4 SHA224 sha224
      2.16.840.1.101.3.4.2.5 SHA512-224 sha512-224
      2.16.840.1.101.3.4.2.6 SHA512-256 sha512-256
      2.16.840.1.101.3.4.2.7 SHA3-224 sha3-224
      2.16.840.1.101.3.4.2.8 SHA3-256 sha3-256
      2.16.840.1.101.3.4.2.9 SHA3-384 sha3-384
      2.16.840.1.101.3.4.2.10 SHA3-512 sha3-512
      2.16.840.1.101.3.4.2.11 SHAKE128 shake128
      2.16.840.1.101.3.4.2.12 SHAKE256 shake256
      2.16.840.1.101.3.4.2.13 id-hmacWithSHA3-224 hmac-sha3-224
      2.16.840.1.101.3.4.2.14 id-hmacWithSHA3-256 hmac-sha3-256
      2.16.840.1.101.3.4.2.15 id-hmacWithSHA3-384 hmac-sha3-384
      2.16.840.1.101.3.4.2.16 id-hmacWithSHA3-512 hmac-sha3-512
      2.16.840.1.101.3.4.2.19 KMAC128 kmac128
      2.16.840.1.101.3.4.2.20 KMAC256 kmac256
      2.16.840.1.101.3.4.3.1 dsa_with_SHA224
      2.16.840.1.101.3.4.3.2 dsa_with_SHA256
      2.16.840.1.101.3.4.3.3 id-dsa-with-sha384 dsa_with_SHA384
      2.16.840.1.101.3.4.3.4 id-dsa-with-sha512 dsa_with_SHA512
      2.16.840.1.101.3.4.3.5 id-dsa-with-sha3-224 dsa_with_SHA3-224
      2.16.840.1.101.3.4.3.6 id-dsa-with-sha3-256 dsa_with_SHA3-256
      2.16.840.1.101.3.4.3.7 id-dsa-with-sha3-384 dsa_with_SHA3-384
      2.16.840.1.101.3.4.3.8 id-dsa-with-sha3-512 dsa_with_SHA3-512
      2.16.840.1.101.3.4.3.9 id-ecdsa-with-sha3-224 ecdsa_with_SHA3-224
      2.16.840.1.101.3.4.3.10 id-ecdsa-with-sha3-256 ecdsa_with_SHA3-256
      2.16.840.1.101.3.4.3.11 id-ecdsa-with-sha3-384 ecdsa_with_SHA3-384
      2.16.840.1.101.3.4.3.12 id-ecdsa-with-sha3-512 ecdsa_with_SHA3-512
      2.16.840.1.101.3.4.3.13 id-rsassa-pkcs1-v1_5-with-sha3-224 RSA-SHA3-224
      2.16.840.1.101.3.4.3.14 id-rsassa-pkcs1-v1_5-with-sha3-256 RSA-SHA3-256
      2.16.840.1.101.3.4.3.15 id-rsassa-pkcs1-v1_5-with-sha3-384 RSA-SHA3-384
      2.16.840.1.101.3.4.3.16 id-rsassa-pkcs1-v1_5-with-sha3-512 RSA-SHA3-512
      2.16.840.1.101.3.4.3.17 id-ml-dsa-44 ML-DSA-44
      2.16.840.1.101.3.4.3.18 id-ml-dsa-65 ML-DSA-65
      2.16.840.1.101.3.4.3.19 id-ml-dsa-87 ML-DSA-87
      2.16.840.1.101.3.4.3.20 id-slh-dsa-sha2-128s SLH-DSA-SHA2-128s
      2.16.840.1.101.3.4.3.21 id-slh-dsa-sha2-128f SLH-DSA-SHA2-128f
      2.16.840.1.101.3.4.3.22 id-slh-dsa-sha2-192s SLH-DSA-SHA2-192s
      2.16.840.1.101.3.4.3.23 id-slh-dsa-sha2-192f SLH-DSA-SHA2-192f
      2.16.840.1.101.3.4.3.24 id-slh-dsa-sha2-256s SLH-DSA-SHA2-256s
      2.16.840.1.101.3.4.3.25 id-slh-dsa-sha2-256f SLH-DSA-SHA2-256f
      2.16.840.1.101.3.4.3.26 id-slh-dsa-shake-128s SLH-DSA-SHAKE-128s
      2.16.840.1.101.3.4.3.27 id-slh-dsa-shake-128f SLH-DSA-SHAKE-128f
      2.16.840.1.101.3.4.3.28 id-slh-dsa-shake-192s SLH-DSA-SHAKE-192s
      2.16.840.1.101.3.4.3.29 id-slh-dsa-shake-192f SLH-DSA-SHAKE-192f
      2.16.840.1.101.3.4.3.30 id-slh-dsa-shake-256s SLH-DSA-SHAKE-256s
      2.16.840.1.101.3.4.3.31 id-slh-dsa-shake-256f SLH-DSA-SHAKE-256f
      2.16.840.1.101.3.4.3.32 id-hash-ml-dsa-44-with-sha512 HASH-ML-DSA-44-WITH-SHA512
      2.16.840.1.101.3.4.3.33 id-hash-ml-dsa-65-with-sha512 HASH-ML-DSA-65-WITH-SHA512
      2.16.840.1.101.3.4.3.34 id-hash-ml-dsa-87-with-sha512 HASH-ML-DSA-87-WITH-SHA512
      2.16.840.1.101.3.4.3.35 id-hash-slh-dsa-sha2-128s-with-sha256 SLH-DSA-SHA2-128s-WITH-SHA256
      2.16.840.1.101.3.4.3.36 id-hash-slh-dsa-sha2-128f-with-sha256 SLH-DSA-SHA2-128f-WITH-SHA256
      2.16.840.1.101.3.4.3.37 id-hash-slh-dsa-sha2-192s-with-sha512 SLH-DSA-SHA2-192s-WITH-SHA512
      2.16.840.1.101.3.4.3.38 id-hash-slh-dsa-sha2-192f-with-sha512 SLH-DSA-SHA2-192f-WITH-SHA512
      2.16.840.1.101.3.4.3.39 id-hash-slh-dsa-sha2-256s-with-sha512 SLH-DSA-SHA2-256s-WITH-SHA512
      2.16.840.1.101.3.4.3.40 id-hash-slh-dsa-sha2-256f-with-sha512 SLH-DSA-SHA2-256f-WITH-SHA512
      2.16.840.1.101.3.4.3.41 id-hash-slh-dsa-shake-128s-with-shake128 SLH-DSA-SHAKE-128s-WITH-SHAKE128
      2.16.840.1.101.3.4.3.42 id-hash-slh-dsa-shake-128f-with-shake128 SLH-DSA-SHAKE-128f-WITH-SHAKE128
      2.16.840.1.101.3.4.3.43 id-hash-slh-dsa-shake-192s-with-shake256 SLH-DSA-SHAKE-192s-WITH-SHAKE256
      2.16.840.1.101.3.4.3.44 id-hash-slh-dsa-shake-192f-with-shake256 SLH-DSA-SHAKE-192f-WITH-SHAKE256
      2.16.840.1.101.3.4.3.45 id-hash-slh-dsa-shake-256s-with-shake256 SLH-DSA-SHAKE-256s-WITH-SHAKE256
      2.16.840.1.101.3.4.3.46 id-hash-slh-dsa-shake-256f-with-shake256 SLH-DSA-SHAKE-256f-WITH-SHAKE256
      2.16.840.1.101.3.4.4.1 id-alg-ml-kem-512 ML-KEM-512
      2.16.840.1.101.3.4.4.2 id-alg-ml-kem-768 ML-KEM-768
      2.16.840.1.101.3.4.4.3 id-alg-ml-kem-1024 ML-KEM-1024
      2.16.840.1.113730 Netscape Netscape Communications Corp.
      2.16.840.1.113730.1 nsCertExt Netscape Certificate Extension
      2.16.840.1.113730.1.1 nsCertType Netscape Cert Type
      2.16.840.1.113730.1.2 nsBaseUrl Netscape Base Url
      2.16.840.1.113730.1.3 nsRevocationUrl Netscape Revocation Url
      2.16.840.1.113730.1.4 nsCaRevocationUrl Netscape CA Revocation Url
      2.16.840.1.113730.1.7 nsRenewalUrl Netscape Renewal Url
      2.16.840.1.113730.1.8 nsCaPolicyUrl Netscape CA Policy Url
      2.16.840.1.113730.1.12 nsSslServerName Netscape SSL Server Name
      2.16.840.1.113730.1.13 nsComment Netscape Comment
      2.16.840.1.113730.2 nsDataType Netscape Data Type
      2.16.840.1.113730.2.5 nsCertSequence Netscape Certificate Sequence
      2.16.840.1.113730.4.1 nsSGC Netscape Server Gated Crypto
      2.16.840.1.113894 oracle-organization Oracle organization
      2.16.840.1.113894.746875.1.1 oracle-jdk-trustedkeyusage Trusted key usage (Oracle)
      2.23 international-organizations International Organizations
      2.23.42 id-set Secure Electronic Transactions
      2.23.42.0 set-ctype content types
      2.23.42.0.0 setct-PANData
      2.23.42.0.1 setct-PANToken
      2.23.42.0.2 setct-PANOnly
      2.23.42.0.3 setct-OIData
      2.23.42.0.4 setct-PI
      2.23.42.0.5 setct-PIData
      2.23.42.0.6 setct-PIDataUnsigned
      2.23.42.0.7 setct-HODInput
      2.23.42.0.8 setct-AuthResBaggage
      2.23.42.0.9 setct-AuthRevReqBaggage
      2.23.42.0.10 setct-AuthRevResBaggage
      2.23.42.0.11 setct-CapTokenSeq
      2.23.42.0.12 setct-PInitResData
      2.23.42.0.13 setct-PI-TBS
      2.23.42.0.14 setct-PResData
      2.23.42.0.16 setct-AuthReqTBS
      2.23.42.0.17 setct-AuthResTBS
      2.23.42.0.18 setct-AuthResTBSX
      2.23.42.0.19 setct-AuthTokenTBS
      2.23.42.0.20 setct-CapTokenData
      2.23.42.0.21 setct-CapTokenTBS
      2.23.42.0.22 setct-AcqCardCodeMsg
      2.23.42.0.23 setct-AuthRevReqTBS
      2.23.42.0.24 setct-AuthRevResData
      2.23.42.0.25 setct-AuthRevResTBS
      2.23.42.0.26 setct-CapReqTBS
      2.23.42.0.27 setct-CapReqTBSX
      2.23.42.0.28 setct-CapResData
      2.23.42.0.29 setct-CapRevReqTBS
      2.23.42.0.30 setct-CapRevReqTBSX
      2.23.42.0.31 setct-CapRevResData
      2.23.42.0.32 setct-CredReqTBS
      2.23.42.0.33 setct-CredReqTBSX
      2.23.42.0.34 setct-CredResData
      2.23.42.0.35 setct-CredRevReqTBS
      2.23.42.0.36 setct-CredRevReqTBSX
      2.23.42.0.37 setct-CredRevResData
      2.23.42.0.38 setct-PCertReqData
      2.23.42.0.39 setct-PCertResTBS
      2.23.42.0.40 setct-BatchAdminReqData
      2.23.42.0.41 setct-BatchAdminResData
      2.23.42.0.42 setct-CardCInitResTBS
      2.23.42.0.43 setct-MeAqCInitResTBS
      2.23.42.0.44 setct-RegFormResTBS
      2.23.42.0.45 setct-CertReqData
      2.23.42.0.46 setct-CertReqTBS
      2.23.42.0.47 setct-CertResData
      2.23.42.0.48 setct-CertInqReqTBS
      2.23.42.0.49 setct-ErrorTBS
      2.23.42.0.50 setct-PIDualSignedTBE
      2.23.42.0.51 setct-PIUnsignedTBE
      2.23.42.0.52 setct-AuthReqTBE
      2.23.42.0.53 setct-AuthResTBE
      2.23.42.0.54 setct-AuthResTBEX
      2.23.42.0.55 setct-AuthTokenTBE
      2.23.42.0.56 setct-CapTokenTBE
      2.23.42.0.57 setct-CapTokenTBEX
      2.23.42.0.58 setct-AcqCardCodeMsgTBE
      2.23.42.0.59 setct-AuthRevReqTBE
      2.23.42.0.60 setct-AuthRevResTBE
      2.23.42.0.61 setct-AuthRevResTBEB
      2.23.42.0.62 setct-CapReqTBE
      2.23.42.0.63 setct-CapReqTBEX
      2.23.42.0.64 setct-CapResTBE
      2.23.42.0.65 setct-CapRevReqTBE
      2.23.42.0.66 setct-CapRevReqTBEX
      2.23.42.0.67 setct-CapRevResTBE
      2.23.42.0.68 setct-CredReqTBE
      2.23.42.0.69 setct-CredReqTBEX
      2.23.42.0.70 setct-CredResTBE
      2.23.42.0.71 setct-CredRevReqTBE
      2.23.42.0.72 setct-CredRevReqTBEX
      2.23.42.0.73 setct-CredRevResTBE
      2.23.42.0.74 setct-BatchAdminReqTBE
      2.23.42.0.75 setct-BatchAdminResTBE
      2.23.42.0.76 setct-RegFormReqTBE
      2.23.42.0.77 setct-CertReqTBE
      2.23.42.0.78 setct-CertReqTBEX
      2.23.42.0.79 setct-CertResTBE
      2.23.42.0.80 setct-CRLNotificationTBS
      2.23.42.0.81 setct-CRLNotificationResTBS
      2.23.42.0.82 setct-BCIDistributionTBS
      2.23.42.1 set-msgExt message extensions
      2.23.42.1.1 setext-genCrypt generic cryptogram
      2.23.42.1.3 setext-miAuth merchant initiated auth
      2.23.42.1.4 setext-pinSecure
      2.23.42.1.5 setext-pinAny
      2.23.42.1.7 setext-track2
      2.23.42.1.8 setext-cv additional verification
      2.23.42.3 set-attr
      2.23.42.3.0 setAttr-Cert
      2.23.42.3.0.0 set-rootKeyThumb
      2.23.42.3.0.1 set-addPolicy
      2.23.42.3.1 setAttr-PGWYcap payment gateway capabilities
      2.23.42.3.2 setAttr-TokenType
      2.23.42.3.2.1 setAttr-Token-EMV
      2.23.42.3.2.2 setAttr-Token-B0Prime
      2.23.42.3.3 setAttr-IssCap issuer capabilities
      2.23.42.3.3.3 setAttr-IssCap-CVM
      2.23.42.3.3.3.1 setAttr-GenCryptgrm generate cryptogram
      2.23.42.3.3.4 setAttr-IssCap-T2
      2.23.42.3.3.4.1 setAttr-T2Enc encrypted track 2
      2.23.42.3.3.4.2 setAttr-T2cleartxt cleartext track 2
      2.23.42.3.3.5 setAttr-IssCap-Sig
      2.23.42.3.3.5.1 setAttr-TokICCsig ICC or token signature
      2.23.42.3.3.5.2 setAttr-SecDevSig secure device signature
      2.23.42.5 set-policy
      2.23.42.5.0 set-policy-root
      2.23.42.7 set-certExt certificate extensions
      2.23.42.7.0 setCext-hashedRoot
      2.23.42.7.1 setCext-certType
      2.23.42.7.2 setCext-merchData
      2.23.42.7.3 setCext-cCertRequired
      2.23.42.7.4 setCext-tunneling
      2.23.42.7.5 setCext-setExt
      2.23.42.7.6 setCext-setQualf
      2.23.42.7.7 setCext-PGWYcapabilities
      2.23.42.7.8 setCext-TokenIdentifier
      2.23.42.7.9 setCext-Track2Data
      2.23.42.7.10 setCext-TokenType
      2.23.42.7.11 setCext-IssuerCapabilities
      2.23.42.8 set-brand
      2.23.42.8.1 set-brand-IATA-ATA
      2.23.42.8.4 set-brand-Visa
      2.23.42.8.5 set-brand-MasterCard
      2.23.42.8.30 set-brand-Diners
      2.23.42.8.34 set-brand-AmericanExpress
      2.23.42.8.35 set-brand-JCB
      2.23.42.8.6011 set-brand-Novus
      2.23.43 wap
      2.23.43.1 wap-wsg
      2.23.43.1.4.1 wap-wsg-idm-ecid-wtls1
      2.23.43.1.4.3 wap-wsg-idm-ecid-wtls3
      2.23.43.1.4.4 wap-wsg-idm-ecid-wtls4
      2.23.43.1.4.5 wap-wsg-idm-ecid-wtls5
      2.23.43.1.4.6 wap-wsg-idm-ecid-wtls6
      2.23.43.1.4.7 wap-wsg-idm-ecid-wtls7
      2.23.43.1.4.8 wap-wsg-idm-ecid-wtls8
      2.23.43.1.4.9 wap-wsg-idm-ecid-wtls9
      2.23.43.1.4.10 wap-wsg-idm-ecid-wtls10
      2.23.43.1.4.11 wap-wsg-idm-ecid-wtls11
      2.23.43.1.4.12 wap-wsg-idm-ecid-wtls12
      2.23.133 tcg Trusted Computing Group
      2.23.133.1 tcg-tcpaSpecVersion
      2.23.133.2 tcg-attribute Trusted Computing Group Attributes
      2.23.133.2.1 tcg-at-tpmManufacturer TPM Manufacturer
      2.23.133.2.2 tcg-at-tpmModel TPM Model
      2.23.133.2.3 tcg-at-tpmVersion TPM Version
      2.23.133.2.10 tcg-at-securityQualities Security Qualities
      2.23.133.2.11 tcg-at-tpmProtectionProfile TPM Protection Profile
      2.23.133.2.12 tcg-at-tpmSecurityTarget TPM Security Target
      2.23.133.2.13 tcg-at-tbbProtectionProfile TBB Protection Profile
      2.23.133.2.14 tcg-at-tbbSecurityTarget TBB Security Target
      2.23.133.2.15 tcg-at-tpmIdLabel TPM ID Label
      2.23.133.2.16 tcg-at-tpmSpecification TPM Specification
      2.23.133.2.17 tcg-at-tcgPlatformSpecification TPM Platform Specification
      2.23.133.2.18 tcg-at-tpmSecurityAssertions TPM Security Assertions
      2.23.133.2.19 tcg-at-tbbSecurityAssertions TBB Security Assertions
      2.23.133.2.23 tcg-at-tcgCredentialSpecification TCG Credential Specification
      2.23.133.2.25 tcg-at-tcgCredentialType TCG Credential Type
      2.23.133.2.26 tcg-at-previousPlatformCertificates TCG Previous Platform Certificates
      2.23.133.2.27 tcg-at-tbbSecurityAssertions-v3 TCG TBB Security Assertions V3
      2.23.133.2.28 tcg-at-cryptographicAnchors TCG Cryptographic Anchors
      2.23.133.3 tcg-protocol Trusted Computing Group Protocols
      2.23.133.3.1 tcg-prt-tpmIdProtocol TCG TPM Protocol
      2.23.133.4 tcg-algorithm Trusted Computing Group Algorithms
      2.23.133.4.1 tcg-algorithm-null TCG NULL Algorithm
      2.23.133.5 tcg-platformClass Trusted Computing Group Platform Classes
      2.23.133.5.1 tcg-common Trusted Computing Group Common
      2.23.133.5.1.1 tcg-at-platformManufacturerStr TCG Platform Manufacturer String
      2.23.133.5.1.2 tcg-at-platformManufacturerId TCG Platform Manufacturer ID
      2.23.133.5.1.3 tcg-at-platformConfigUri TCG Platform Configuration URI
      2.23.133.5.1.4 tcg-at-platformModel TCG Platform Model
      2.23.133.5.1.5 tcg-at-platformVersion TCG Platform Version
      2.23.133.5.1.6 tcg-at-platformSerial TCG Platform Serial Number
      2.23.133.5.1.7 tcg-at-platformConfiguration TCG Platform Configuration
      2.23.133.5.1.7.1 tcg-at-platformConfiguration-v1 Platform Configuration Version 1
      2.23.133.5.1.7.2 tcg-at-platformConfiguration-v2 Platform Configuration Version 2
      2.23.133.5.1.7.3 tcg-at-platformConfiguration-v3 Platform Configuration Version 3
      2.23.133.5.1.7.4 tcg-at-platformConfigUri-v3 Platform Configuration URI Version 3
      2.23.133.5.1.8 tcg-at-platformIdentifier TCG Platform Identifier
      2.23.133.6 tcg-ce Trusted Computing Group Certificate Extensions
      2.23.133.6.2 tcg-ce-relevantCredentials Relevant Credentials
      2.23.133.6.3 tcg-ce-relevantManifests Relevant Manifests
      2.23.133.6.4 tcg-ce-virtualPlatformAttestationService Virtual Platform Attestation Service
      2.23.133.6.5 tcg-ce-migrationControllerAttestationService Migration Controller Attestation Service
      2.23.133.6.6 tcg-ce-migrationControllerRegistrationService Migration Controller Registration Service
      2.23.133.6.7 tcg-ce-virtualPlatformBackupService Virtual Platform Backup Service
      2.23.133.8 tcg-kp Trusted Computing Group Key Purposes
      2.23.133.8.1 tcg-kp-EKCertificate Endorsement Key Certificate
      2.23.133.8.2 tcg-kp-PlatformAttributeCertificate Platform Attribute Certificate
      2.23.133.8.3 tcg-kp-AIKCertificate Attestation Identity Key Certificate
      2.23.133.8.4 tcg-kp-PlatformKeyCertificate Platform Key Certificate
      2.23.133.8.5 tcg-kp-DeltaPlatformAttributeCertificate Delta Platform Attribute Certificate
      2.23.133.8.6 tcg-kp-DeltaPlatformKeyCertificate Delta Platform Key Certificate
      2.23.133.8.7 tcg-kp-AdditionalPlatformAttributeCertificate Additional Platform Attribute Certificate
      2.23.133.8.8 tcg-kp-AdditionalPlatformKeyCertificate Additional Platform Key Certificate
      2.23.133.11 tcg-ca Trusted Computing Group Certificate Policies
      2.23.133.11.4 tcg-cap-verifiedPlatformCertificate TCG Verified Platform Certificate CA Policy
      2.23.133.17 tcg-address Trusted Computing Group Address Formats
      2.23.133.17.1 tcg-address-ethernetmac Ethernet MAC Address
      2.23.133.17.2 tcg-address-wlanmac WLAN MAC Address
      2.23.133.17.3 tcg-address-bluetoothmac Bluetooth MAC Address
      2.23.133.18 tcg-registry Trusted Computing Group Registry
      2.23.133.18.3 tcg-registry-componentClass TCG Component Class
      2.23.133.18.3.1 tcg-registry-componentClass-tcg Trusted Computed Group Registry
      2.23.133.18.3.2 tcg-registry-componentClass-ietf Internet Engineering Task Force Registry
      2.23.133.18.3.3 tcg-registry-componentClass-dmtf Distributed Management Task Force Registry
      2.23.133.18.3.4 tcg-registry-componentClass-pcie PCIE Component Class
      2.23.133.18.3.5 tcg-registry-componentClass-disk Disk Component Class
      2.23.133.19 tcg-traits Trusted Computing Group Traits
      2.23.133.19.1 tcg-tr-ID TCG Trait Identifiers
      2.23.133.19.1.1 tcg-tr-ID-Boolean Boolean Trait
      2.23.133.19.1.2 tcg-tr-ID-CertificateIdentifier Certificate Identifier Trait
      2.23.133.19.1.3 tcg-tr-ID-CommonCriteria Common Criteria Trait
      2.23.133.19.1.4 tcg-tr-ID-componentClass Component Class Trait
      2.23.133.19.1.5 tcg-tr-ID-componentIdentifierV11 Component Identifier V1.1 Trait
      2.23.133.19.1.6 tcg-tr-ID-FIPSLevel FIPS Level Trait
      2.23.133.19.1.7 tcg-tr-ID-ISO9000Level ISO 9000 Level Trait
      2.23.133.19.1.8 tcg-tr-ID-networkMAC Network MAC Trait
      2.23.133.19.1.9 tcg-tr-ID-OID Object Identifier Trait
      2.23.133.19.1.10 tcg-tr-ID-PEN Private Enterprise Number Trait
      2.23.133.19.1.11 tcg-tr-ID-platformFirmwareCapabilities Platform Firmware Capabilities Trait
      2.23.133.19.1.12 tcg-tr-ID-platformFirmwareSignatureVerification Platform Firmware Signature Verification Trait
      2.23.133.19.1.13 tcg-tr-ID-platformFirmwareUpdateCompliance Platform Firmware Update Compliance Trait
      2.23.133.19.1.14 tcg-tr-ID-platformHardwareCapabilities Platform Hardware Capabilities Trait
      2.23.133.19.1.15 tcg-tr-ID-RTM Root of Trust for Measurement Trait
      2.23.133.19.1.16 tcg-tr-ID-status Attribute Status Trait
      2.23.133.19.1.17 tcg-tr-ID-URI Uniform Resource Identifier Trait
      2.23.133.19.1.18 tcg-tr-ID-UTF8String UTF8String Trait
      2.23.133.19.1.19 tcg-tr-ID-IA5String IA5String Trait
      2.23.133.19.1.20 tcg-tr-ID-PEMCertString PEM-Encoded Certificate String Trait
      2.23.133.19.1.21 tcg-tr-ID-PublicKey Public Key Trait
      2.23.133.19.2 tcg-tr-category TCG Trait Categories
      2.23.133.19.2.1 tcg-tr-cat-platformManufacturer Platform Manufacturer Trait Category
      2.23.133.19.2.2 tcg-tr-cat-platformModel Platform Model Trait Category
      2.23.133.19.2.3 tcg-tr-cat-platformVersion Platform Version Trait Category
      2.23.133.19.2.4 tcg-tr-cat-platformSerial Platform Serial Trait Category
      2.23.133.19.2.5 tcg-tr-cat-platformManufacturerIdentifier Platform Manufacturer Identifier Trait Category
      2.23.133.19.2.6 tcg-tr-cat-platformOwnership Platform Ownership Trait Category
      2.23.133.19.2.7 tcg-tr-cat-componentClass Component Class Trait Category
      2.23.133.19.2.8 tcg-tr-cat-componentManufacturer Component Manufacturer Trait Category
      2.23.133.19.2.9 tcg-tr-cat-componentModel Component Model Trait Category
      2.23.133.19.2.10 tcg-tr-cat-componentSerial Component Serial Trait Category
      2.23.133.19.2.11 tcg-tr-cat-componentStatus Component Status Trait Category
      2.23.133.19.2.12 tcg-tr-cat-componentLocation Component Location Trait Category
      2.23.133.19.2.13 tcg-tr-cat-componentRevision Component Revision Trait Category
      2.23.133.19.2.14 tcg-tr-cat-componentFieldReplaceable Component Field Replaceable Trait Category
      2.23.133.19.2.15 tcg-tr-cat-EKCertificate EK Certificate Trait Category
      2.23.133.19.2.16 tcg-tr-cat-IAKCertificate IAK Certificate Trait Category
      2.23.133.19.2.17 tcg-tr-cat-IDevIDCertificate IDevID Certificate Trait Category
      2.23.133.19.2.18 tcg-tr-cat-DICECertificate DICE Certificate Trait Category
      2.23.133.19.2.19 tcg-tr-cat-SPDMCertificate SPDM Certificate Trait Category
      2.23.133.19.2.20 tcg-tr-cat-PEMCertificate PEM Certificate Trait Category
      2.23.133.19.2.21 tcg-tr-cat-PlatformCertificate Platform Certificate Trait Category
      2.23.133.19.2.22 tcg-tr-cat-DeltaPlatformCertificate Delta Platform Certificate Trait Category
      2.23.133.19.2.23 tcg-tr-cat-RebasePlatformCertificate Rebase Platform Certificate Trait Category
      2.23.133.19.2.24 tcg-tr-cat-genericCertificate Generic Certificate Trait Category
      2.23.133.19.2.25 tcg-tr-cat-CommonCriteria Common Criteria Trait Category
      2.23.133.19.2.26 tcg-tr-cat-componentIdentifierV11 Component Identifier V1.1 Trait Category
      2.23.133.19.2.27 tcg-tr-cat-FIPSLevel FIPS Level Trait Category
      2.23.133.19.2.28 tcg-tr-cat-ISO9000 ISO 9000 Trait Category
      2.23.133.19.2.29 tcg-tr-cat-networkMAC Network MAC Trait Category
      2.23.133.19.2.30 tcg-tr-cat-attestationProtocol Attestation Protocol Trait Category
      2.23.133.19.2.31 tcg-tr-cat-PEN Private Enterprise Number Trait Category
      2.23.133.19.2.32 tcg-tr-cat-platformFirmwareCapabilities Platform Firmware Capabilities Trait Category
      2.23.133.19.2.33 tcg-tr-cat-platformHardwareCapabilities Platform Hardware Capabilities Trait Category
      2.23.133.19.2.34 tcg-tr-cat-platformFirmwareSignatureVerification Platform Firmware Signature Verification Trait Category
      2.23.133.19.2.35 tcg-tr-cat-platformFirmwareUpdateCompliance Platform Firmware Update Compliance Trait Category
      2.23.133.19.2.36 tcg-tr-cat-RTM Root of Trust of Measurement Trait Category
      2.23.133.19.2.37 tcg-tr-cat-PublicKey Public Key Trait Category
      2.23.133.19.3 tcg-tr-registry TCG Trait Registries
    OIDS
  end
end
