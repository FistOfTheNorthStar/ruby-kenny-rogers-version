# Marshal (format 4.8): byte-compatible dumps and round trips.
require 'stringio'
require 'date'
require 'set'

def hex(v); Marshal.dump(v).unpack1("H*"); end
def rt(v); Marshal.load(Marshal.dump(v)); end

Point = Struct.new(:x, :y)
class Foo
  attr_reader :a, :b
  def initialize(a = 1, b = "s"); @a = a; @b = b; end
end
class UD
  attr_reader :x
  def initialize(x); @x = x; end
  def _dump(_limit); "#{@x}"; end
  def self._load(s); new(s.to_i); end
end
class UM
  attr_reader :x, :loaded
  def initialize(x); @x = x; end
  def marshal_dump; [@x]; end
  def marshal_load(a); @x = a[0]; @loaded = true; end
end
class MyStr < String; end
class MyArr < Array; end
class MyHash < Hash; end
module Ext; end
class Err < StandardError
  def initialize(msg = "custom"); super; end
end

puts "== hex (matches CRuby)"
[nil, true, false, 0, 1, -1, 122, 123, -123, -124, 255, 256, -256, -257, 65535, 65536, 2**24,
 2**30 - 1, 2**30, -2**30, -2**30 - 1, 2**31, 2**62, 2**64, -2**64, 2**100,
 1.5, 0.0, -0.0, 1.0 / 0, -1.0 / 0, 0.1, 1e100, 1.0e-5, 123456789.123, 1e16, 1.0, 2.5e-300, 100.0,
 0.001, 1.0 / 3, -1.5e-10, 123456.0, 1e15, 12345678901234567890.0, Float::MAX, 5e-324,
 "", "abc", "abc".b, "ü", "x".encode("US-ASCII"), :sym, :ü, :"a b", [:a, :a, :b], [], [1, [2, [3]]],
 {}, {a: 1, "b" => [2]}, Hash.new(5).merge(1 => 2), [1.5, 1.5], [2**40, 2**40, "a"],
 Point.new(1, "x"), Foo.new, UM.new(7), MyStr.new("q"), MyArr.new([1]), String, Kernel, Comparable,
 /ab+c/i, /x/mx, /ü/, 1..2, 1...3, ("a".."b"), (1..), Rational(1, 3), Complex(1, 2), RuntimeError.new("boom"),
 Object.new.extend(Ext), Struct, Point, Set[1, 2], Date.new(2020, 1, 2), DateTime.new(2020, 1, 2, 3, 4, 5, "+01:00"),
 Time.at(1700000000, 123456789, :nsec).utc, Time.at(0).utc].each do |v|
  puts "#{v.class}: #{hex(v)}"
end
s = "dup"; puts hex([s, s])
a = []; a << a; puts hex(a)
f = 1e300; puts hex([f, f])
x = "z"; puts hex([2**40, x, x])
o = Object.new; o.extend(Comparable); o.extend(Enumerable); puts hex(o)
s2 = "x".dup; s2.instance_variable_set(:@foo, 1); puts hex(s2)
a2 = [1]; a2.instance_variable_set(:@foo, 2); puts hex(a2)
puts hex([String, String, :E, "e"])

puts "== round trips"
data = {name: "kenny", list: [1, 2.5, nil, true, :sym, 2**80, -7], nested: {"deep" => [[1], {x: [2]}]}, r: 1..10}
p rt(data) == data, rt(data)
p rt(Point.new(3, [4])), rt(Point.new(3, [4])) == Point.new(3, [4])
f2 = rt(Foo.new(10, ["a", {b: 2}]))
p f2.class, f2.a, f2.b
u = rt(UD.new(42)); p u.class, u.x
m = rt(UM.new([1, "x"])); p m.class, m.x, m.loaded
p rt(MyStr.new("q")).class, rt(MyArr.new([1, 2])).class, rt(MyHash.new.tap { |h| h[1] = 2 }).class
p rt(1..5), rt(1...5), rt("a".."c"), rt(1..), rt(..5), rt(1..5).exclude_end?, rt(1...5).exclude_end?
p rt(Rational(3, 4)), rt(Complex(1, -2)), rt(Rational(-1, 3)) + Rational(1, 3)
e = rt(Err.new("bad")); p e.class, e.message, e.backtrace
p rt(String), rt(Comparable), rt(Struct)
p rt(Set[1, 2, 3]), rt(Set[1, 2]).include?(2)
p rt(Date.new(2021, 3, 4)), rt(DateTime.new(2021, 3, 4, 5, 6, 7, "+02:00"))
t = Time.at(1700000000, 123456789, :nsec).utc
t2 = rt(t); p t2, t2.utc?, t2.nsec, t2 == t
t3 = rt(Time.at(1700000000, 5, :usec).getlocal("+05:30")); p t3, t3.utc_offset, t3.usec
p rt(["ü", "abc".b, "x".encode("US-ASCII")]).map(&:encoding)
p rt(:ü), rt([:a, :b, :a]), rt(/ab+c/ix), rt(/ü/).source
ext = rt(Object.new.extend(Ext)); p ext.singleton_class.include?(Ext), ext.is_a?(Ext)
p rt(Hash.new(5).merge(1 => 2)).then { |h| [h, h[:missing]] }
p rt([0.0, -0.0, 1.0 / 0, -1.0 / 0, 1e300, 5e-324]), rt(Float::NAN).nan?
p rt(2**64), rt(-2**64), rt(2**62), rt(-(2**62)), rt(2**30), rt(-(2**30) - 1)

puts "== identity and cycles"
s = "shared"; arr = rt([s, s, [s]]); p arr, arr[0].equal?(arr[1]), arr[0].equal?(arr[2][0])
cyc = []; cyc << cyc << 1; c2 = rt(cyc); p c2[0].equal?(c2), c2[1]
h = {}; h[:self] = h; h2 = rt(h); p h2[:self].equal?(h2)
class Node
  attr_accessor :val, :next_node
  def initialize(v); @val = v; end
end
n1 = Node.new(1); n2 = Node.new(2); n1.next_node = n2; n2.next_node = n1
r1 = rt(n1); p r1.val, r1.next_node.val, r1.next_node.next_node.equal?(r1)
st = Point.new(nil, nil); st.x = st; st2 = rt(st); p st2.x.equal?(st2)
fl = 2.5; p rt([fl, fl]).then { |x| x[0].equal?(x[1]) }

puts "== IO and options"
io = StringIO.new
Marshal.dump({a: [1, 2]}, io)
Marshal.dump("second", io)
io.rewind
p Marshal.load(io), Marshal.load(io)
seen = []
p Marshal.load(Marshal.dump([1, "s", [:k]]), proc { |x| seen << x; x })
p seen
p Marshal.load(Marshal.dump(["a", :b]), ->(x) { x.is_a?(String) ? x.upcase : x })
fz = Marshal.load(Marshal.dump(["a", ["b"], {k: "v"}]), freeze: true)
p fz.frozen?, fz[0].frozen?, fz[1].frozen?, fz[2].frozen?, fz[2][:k].frozen?
p Marshal.restore(Marshal.dump(:ok)), Marshal::MAJOR_VERSION, Marshal::MINOR_VERSION
p Marshal.dump(1, -1), Marshal.dump([[1]], 3)
p Marshal.dump("x").encoding

puts "== errors"
[proc {}, method(:puts), Thread.current, STDOUT, Class.new, Class.new.new,
 Object.new.singleton_class, Hash.new {}, Struct.new(:a).new(1), (o = Object.new; def o.x; 1; end; o)].each do |v|
  Marshal.dump(v)
  puts "dumped #{v.class}"
rescue TypeError => e
  puts "TypeError: #{e.message.sub(/0x\h+/, '0x...')}"
end
begin
  Marshal.dump([[[1]]], 3)
rescue ArgumentError => e
  puts "ArgumentError: #{e.message}"
end
["", "\x04\x08", "\x04\x09", "\x03\x00", "\x04\x08o:\x08Zzz\x00", "\x04\x08c\x08Zzz", "\x04\x08X",
 "\x04\x08[\x07i\x06", "\x04\x08@\x06", "\x04\x08c\x0bKernel", "\x04\x08m\x0bString", "\x04\x08o:\x0bKernel\x00",
 "\x04\x08U:\x0bObject\x00", "\x04\x08u:\x0bObject\x00", "\x04\x08o:\rFoo::Bar\x00", "\x04\x08o:\x0dPoint::Q\x00"].each do |s|
  p Marshal.load(s)
rescue TypeError, ArgumentError => e
  puts "#{e.class}: #{e.message}"
end

puts "== ActiveSupport-style prepend"
module MarshalWithAutoloading
  def load(source, proc = nil)
    super(source, proc)
  rescue ArgumentError, NameError => exc
    if exc.message.match(%r|undefined class/module (.+?)(?:::)?\z|)
      puts "autoloading #{$1}"
      Object.const_set($1, Class.new { attr_reader :a })
      source.rewind if source.respond_to?(:rewind)
      retry
    else
      raise exc
    end
  end
end
Marshal.singleton_class.prepend(MarshalWithAutoloading)
p Marshal.load("\x04\bo:\x08Zzz\x06:\a@ai\x06").a
p Marshal.load(Marshal.dump([1, "two", :three]))
