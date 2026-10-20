# Core-library extras: pack/unpack, Base64, Kernel/Math/Array/Hash/String/... gaps.
require "base64"

puts "-- pack"
p [1, 2, 255].pack("C*")
p [-1, 65535].pack("s<S>")
p [1, 2].pack("NV"), [1, 2].pack("nv")
p [2**64 - 1, -2].pack("Qq")
p [1, 2].pack("s_l_").bytesize, [1].pack("j").bytesize, [1].pack("L!").bytesize
p ["abc"].pack("a5"), ["abc"].pack("A5"), ["abc"].pack("Z*"), ["abcdef"].pack("a3")
p ["4142"].pack("H*"), ["1424"].pack("h*"), ["0100000101"].pack("B*"), ["1000001"].pack("b*")
p [104, 233, 0x1F600].pack("U*")
p [300, 0].pack("w*")
p [1.5].pack("e"), [1.5].pack("G"), [0.1].pack("d").bytesize
p [1, 2].pack("CxC"), [1, 2, 3].pack("CCXC"), [1].pack("C@3")
p ["hello world"].pack("m"), ["hello world"].pack("m0"), ["x" * 50].pack("m30")
p ["café = 1\tok"].pack("M")
p ["abc"].pack("u")
buf = +"head:"
[65, 66].pack("C*", buffer: buf)
p buf
begin
  [1].pack("C2")
rescue ArgumentError => e
  p e.message
end
begin
  ["a"].pack("C")
rescue TypeError => e
  p e.message
end

puts "-- unpack"
p "\x01\x02\x03\x04".unpack("C*"), "\x01\x02\x03\x04".unpack("S<*"), "\x01\x02\x03\x04".unpack("N")
p "\xff\xff".unpack("s"), "\xff\xfe".unpack("n"), "\xff\xff\xff\xff\xff\xff\xff\xff".unpack("Qq")
p "abc  \0".unpack("A*"), "ab\0cd".unpack("Z*Z*"), "abcdef".unpack("a2 x2 a*"), "abc".unpack("@1a")
p "AB".unpack("H*"), "AB".unpack("h*"), "AB".unpack("B*"), "AB".unpack("b*"), "AB".unpack("B4")
p "héllo".unpack("U*"), "\x82\x2c\x00".unpack("w*")
p "aGVsbG8gd29ybGQ=\n".unpack1("m"), "aGk=".unpack1("m0")
p "=3D caf=C3=A9 =\nok".unpack1("M")
p "#86)C\n".unpack("u")
p "\x00\x00\xc0\x3f".unpack1("e"), [3.25, -1.0].pack("E*").unpack("E*")
p "\x01\x02\x03".unpack("C", offset: 2), "\x01".unpack("S")
p "hello".unpack1("a*").encoding, [1].pack("C").encoding, [97].pack("U").encoding
begin
  "!!".unpack1("m0")
rescue ArgumentError => e
  p e.message
end

puts "-- base64"
p Base64.strict_encode64("hello world")
p Base64.encode64("x" * 60)
p Base64.decode64("aGVsbG8gd29ybGQ=")
p Base64.urlsafe_encode64("\xff\xfe\xfd>?".b)
p Base64.urlsafe_encode64("ab", padding: false)
p Base64.urlsafe_decode64("__79Pj8")
p Base64.strict_decode64(Base64.strict_encode64("round trip é")).force_encoding("UTF-8")

puts "-- inspect/encoding"
p "\0\x01\e\x7f", "\a\b\v\f", "\xff\xfe".b, "café\xff"
p "abc".b.encoding, "abc".encoding, (+"abc").force_encoding("BINARY").encoding
p Encoding::US_ASCII, Encoding.default_external, Encoding.find("utf-8")

puts "-- Kernel"
Kernel.puts "via Kernel.puts"
p Kernel.format("%05.1f", 3.14159), Kernel.Integer("0x1f"), Kernel.Float("2.5")
begin
  Kernel.raise ArgumentError, "kernel raise"
rescue ArgumentError => e
  p e.message
end
o = Object.new
p o <=> o, (o <=> Object.new), 1 <=> 2
p Hash(nil), Hash([]), Hash({ a: 1 }), Hash([[1, 2]].to_h)
p local_variables.is_a?(Array), global_variables.include?(:$stdout)
class Pt
  def initialize; @x = 1; @y = 2; @z = 3; end
end
pt = Pt.new
p pt.remove_instance_variable(:@y), pt.instance_variables, pt.instance_variable_get(:@y)
begin
  pt.remove_instance_variable(:@nope)
rescue NameError => e
  p e.message
end
def o.solo; 42; end
p o.singleton_method(:solo).call, o.public_method(:solo).call, o.public_methods.include?(:solo)
p gem("json"), __method__.inspect
bo = BasicObject.new
p bo.__send__(:==, bo), bo.instance_eval { 7 }, bo.instance_exec(3) { |x| x * 2 }

puts "-- Math"
include Math
p sqrt(16), cbrt(27.0).round(6), hypot(3, 4)
p Math.acosh(1.0), Math.asinh(0.0), Math.atanh(0.5).round(10)
p Math.erf(0.0), Math.erfc(0.0), Math.gamma(5), Math.gamma(0.5).round(10)
p Math.lgamma(3.0), Math.frexp(1234.0), Math.ldexp(0.6025390625, 11)
begin
  Math.atanh(2)
rescue Math::DomainError => e
  p e.message
end

puts "-- Array"
p [1, 3, 5, 7, 9].bsearch_index { |x| x >= 5 }, [1, 3, 5].bsearch_index { |x| 4 <=> x }
p [[1, :a], [2, :b]].rassoc(:b), [[1, :a]].rassoc(:z)
p [1, 2, 3].repeated_combination(2).to_a
p [0, 1].repeated_permutation(2).to_a, [1].repeated_combination(0).to_a
a = [1, 2, 3, 4]
p a.rotate!, a.rotate!(-2), a
p Array.try_convert([1]), Array.try_convert("x")
p [1, "a", 2.0, :b].grep(Numeric), %w[apple bob cat].grep(/a/) { |s| s.upcase }, [1, 2, 3].grep_v(2)
p [3, 1, 4, 1, 5, 9, 2, 6].chunk(&:odd?).to_a
p [1, 2, 6, 7, 8, 10].slice_before { |x| x % 5 == 0 }.to_a, [1, 2, 3, 4].slice_after(&:even?).to_a
p %w[a b # c d # e].slice_before("#").to_a, [[1], [2, 3]].each.to_a, [[1]].each_with_index.to_a

puts "-- Enumerable/Enumerator"
p (1..6).chunk { |i| i.even? ? :_separator : true }.to_a
p (1..10).grep(3..5), (1..4).grep_v(2..3) { |x| x * 10 }
p (1..3).chain([4, 5]).to_a, ([1].each + [2, 3]).to_a
p Enumerator.produce(1) { |x| x * 2 }.take(6)
p Enumerator.produce([0, 1]) { |a, b| [b, a + b] }.take(5).map(&:first)
p Enumerator.produce(3) { |x| x.zero? ? raise(StopIteration) : x - 1 }.to_a
e = [1, 2].each
p e.next_values, e.peek_values
p [1, 2, 3].each.each_with_object([]) { |x, acc| acc << x * x }

puts "-- numbers"
p 1.5.prev_float < 1.5, 1.0.prev_float.next_float, 0.0.prev_float < 0
p 7.0.quo(2), 1.5.angle, -1.5.arg, 2.phase, -2.angle
p 0.75.numerator, 0.75.denominator, 3.numerator, 3.denominator
p 0b1010.allbits?(0b1000), 0b1010.anybits?(0b0101), 0b1010.nobits?(0b0101), 12.gcdlcm(18)
p 3.abs2, -2.5.abs2, 4.real?, 4.imag, -3.polar, 5.rect

puts "-- Hash"
h = { a: 1, b: 2 }
p({ a: 1 } <= h, { a: 1 } < h, h < h, h >= { b: 2 }, h > { c: 3 })
p({ a: 1, b: nil }.compact!, { a: 1 }.compact!)
p h.compare_by_identity?, {}.compare_by_identity.compare_by_identity?
d = {}
d.default_proc = proc { |hh, k| hh[k] = k.to_s * 2 }
p d[:ab], d
p({ a: [1, 2], b: 3 }.flatten, { a: [1, [2]] }.flatten(2))
p h.index(2), h.rassoc(1), h.rassoc(9)
p h.rehash.equal?(h), %i[a b c].map(&h)
p Hash.try_convert(h).equal?(h), Hash.try_convert([1])

puts "-- Module"
module Mx; end
class Base; include Mx; Y = 1; end
class Sub < Base; @@cv = 1; X = 5; end
p Sub <=> Base, Base <=> Sub, Base <=> String, Mx <=> Base, Base <=> Base
p Sub.class_variables, Sub.remove_class_variable(:@@cv), Sub.class_variables
p Sub.send(:remove_const, :X), Sub.const_defined?(:X)
p Sub.singleton_class.singleton_class?, Sub.singleton_class?
p Base.protected_method_defined?(:to_s)
p Module.nesting, Base.deprecate_constant(:Y).equal?(Base), Base.const_source_location(:X)

puts "-- nil/true/Proc"
p nil ^ 1, nil ^ nil, nil =~ /x/, NilClass === nil, true === true, false === nil
pr = proc { |x| x }
p pr.hash == pr.hash, pr.dup.call(4), pr.clone.call(5), pr.to_s.start_with?("#<Proc:")

puts "-- Range"
p (0..100).bsearch { |x| x * x >= 50 }, (0.0..10.0).bsearch { |x| x >= 2.5 }, (1..).bsearch { |x| x > 1000 }
p (1..10).minmax, (1.0..2.0).step(0.25).to_a, ("a".."e").minmax

puts "-- Regexp/MatchData"
p /abc/.encoding, /café/.encoding, /a/.fixed_encoding?, Regexp.try_convert(/x/), Regexp.try_convert("x")
m1 = "hello".match(/l+/)
m2 = "hello".match(/l+/)
p m1 == m2, m1.eql?(m2), m1.hash == m2.hash, m1 == "hello".match(/h/)

puts "-- String"
s = "héllo wörld"
p s.byteslice(0, 3), s.byteslice(1..2), s.byteslice(100), "abc".chr, "été".chr
p "hé!".codepoints, "ab".each_codepoint.to_a
p "éa\u{1F1EB}\u{1F1F7}\r\n".grapheme_clusters, "ab".each_grapheme_cluster.to_a
t = +"hello"
p t.delete!("l"), t, t.delete!("z")
t = +"prefix-body"
p t.delete_prefix!("prefix-"), t.delete_prefix!("x"), t.delete_suffix!("dy"), t
t = +"az"
p t.succ!, t.next!, t
t = +"hello"
p t.tr!("el", "ip"), t, t.tr!("z", "q")
p "aabbccdd".tr_s("abc", "x"), "hello".tr_s("l", "r"), "aaabbb".tr_s("a-b", "AB")
t = +"aaab"
p t.tr_s!("a", "x"), t
t = +"abc"
p t.setbyte(0, 65), t
t = +"hello world"
p t.slice!(0, 6), t, t.slice!(/o/), t, t.slice!("rl"), t, t.slice!(1..), t, t.slice!(9)
p "café".unicode_normalized?, (+"x").unicode_normalize!, (+"x").scrub!, (+"x").encode!("UTF-8")
p "\"a\\nb\\u00e9\"".undump, "\"\\x41\\101\"".undump, "café\n".dump, "café\n".dump.undump
p String.try_convert("s"), String.try_convert(1)

puts "-- Struct"
P3 = Struct.new(:x, :y, :z)
pt3 = P3.new(1, { k: [10, 20] }, nil)
p pt3.dig(:y, :k, 1), pt3.dig(:z, :a), pt3.dig(0), pt3.size, pt3.length
p pt3.each_pair.to_a, pt3.select { |v| v.is_a?(Integer) }, pt3.filter(&:nil?), pt3.values_at(0, 2)

puts "-- Symbol"
p :abc === :abc, :Abc.casecmp(:aBC), :Abc.casecmp?(:aBC), :a.casecmp("a")
p :abc.encoding, :"é".encoding, :az.next, :hello.slice(1, 3), :HeLLo.swapcase
p Symbol.all_symbols.include?(:hello)

puts "-- Thread"
th = Thread.current
p th.report_on_exception, th.abort_on_exception, th.pending_interrupt?
th[:k] = 1
p th.fetch(:k), th.fetch(:missing, :dflt), th.fetch(:missing) { |key| "blk #{key}" }
begin
  th.fetch(:missing)
rescue KeyError => e
  p e.message
end
th.thread_variable_set(:tv, 2)
p th.thread_variables
p Thread.handle_interrupt(Object => :never) { :handled }, Thread.report_on_exception, Thread.abort_on_exception
t2 = Thread.new { sleep 10 }
Thread.kill(t2)
t2.join
p t2.alive?

puts "-- Fiber"
f = Fiber.new { Fiber.yield 1; 2 }
p f.inspect[/\((\w+)\)/, 1]
f.resume
p f.to_s[/\((\w+)\)/, 1]
f.resume
p f.inspect[/\((\w+)\)/, 1]

puts "-- Time"
t = Time.utc(2000, 1, 2, 3, 4, 5.5)
p t, t.to_s, t.year, t.month, t.day, t.hour, t.min, t.sec, t.usec, t.nsec, t.subsec.to_f
p t.sunday?, t.monday?, t.wday, t.yday, t.utc?, t.gmt?, t.utc_offset, t.zone, t.dst?
p t.to_a, t.asctime, t.ctime, t.iso8601, t.iso8601(3), t.strftime("%Y-%m-%dT%H:%M:%S.%3N%:z")
p t + 0.25, t - 0.5, (t + 86400).day, t - Time.utc(2000, 1, 2), t.succ.sec
p t.round, t.floor, t.ceil, Time.utc(2000, 1, 1, 0, 0, 0.123456789).round(3).nsec
p t == Time.utc(2000, 1, 2, 3, 4, 5.5), t.eql?(t + 0), t.hash == (t + 0).hash, t <=> t + 1
p Time.gm(2000, "feb", 29).to_s, Time.utc(1999, 12, 31, 23, 59, 59).to_i
off = Time.new(2024, 5, 6, 7, 8, 9, "+09:00")
p off, off.utc_offset, off.zone, off.hour, off.getutc.to_s, off.dup.utc.utc?, off.utc?
p Time.new(2024, 1, 1, 0, 0, 0, "-05:30").strftime("%z %:z %::z"), Time.new(2024, 1, 1, in: "Z").utc?
p Time.at(0).utc.to_s, Time.at(1.5, in: "UTC").inspect, Time.at(0, 500, :millisecond, in: "+01:00").inspect
p Time.at(946684800, 123456789, :nsec).utc.nsec, Time.at(Time.at(5)).to_i
p Time.utc(2000, 1, 1).getlocal("+02:00").to_s, Time.utc(2000, 1, 1).localtime("-01:00").hour
p Time.new("2001-02-03 04:05:06.75 +00:00").inspect, Time.utc(*Time.utc(2001, 2, 3, 4, 5, 6).to_a).to_s
lt = Time.local(2020, 6, 15, 12)
p lt.year, lt.month, lt.day, lt.hour, Time.mktime(2020, 6, 15, 12) == lt
p (Time.now - Time.now).abs < 1, Time.now.is_a?(Comparable)
begin
  Time.utc(2000, 13)
rescue ArgumentError => e
  p e.message
end

puts "-- Random/SecureRandom"
require "securerandom"
r1 = Random.new(42)
r2 = Random.new(42)
p r1.rand(100) == r2.rand(100), r1.seed, r1.bytes(5).bytesize, r1.bytes(2).encoding
p (1..50).all? { (0...7).cover?(r1.rand(7)) }, (1..50).all? { (2..4).cover?(r1.rand(2..4)) }, r1.rand.between?(0, 1)
p Random.rand(1.5).between?(0, 1.5), Random.urandom(4).bytesize, Random.new_seed.is_a?(Integer)
p SecureRandom.hex(4).match?(/\A\h{8}\z/), SecureRandom.uuid.match?(/\A\h{8}-\h{4}-4\h{3}-[89ab]\h{3}-\h{12}\z/)
p SecureRandom.base64(6).size, SecureRandom.urlsafe_base64(9).match?(/\A[\w-]{12}\z/), SecureRandom.random_number(10).between?(0, 9)
p SecureRandom.alphanumeric(12).match?(/\A[A-Za-z0-9]{12}\z/), SecureRandom.random_bytes(3).bytesize
begin
  r1.rand(-1)
rescue ArgumentError => e
  p e.message
end
p [[1]].each.with_object([]).to_a
