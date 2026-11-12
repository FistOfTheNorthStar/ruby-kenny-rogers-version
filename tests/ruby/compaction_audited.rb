# Natives audited to hold object references only where the GC sees them (stackmap.rs
# `audited_natives`): a compaction inside their blocks goes ahead (GC.compact isn't nil),
# and what they hold afterwards is still right. Pages emptied by a compaction are made
# inaccessible (Kenny's KENNY_GC_COMPACT_POISON), so a stale reference would crash.
ENV["KENNY_GC_COMPACT_POISON"] = "1"
require "digest"

def junk = Array.new(200) { |i| "junk #{i}" * 3 }
def compacts = !GC.compact.nil?

$ok = []
def check(name)
  done = false
  r = yield(-> { junk; done ||= compacts; nil })
  $ok << [name, done, r]
end

words = %w[apple banana cherry date elder fig grape]
mk = -> { words.map { |w| w + "!" } }

check("Array#include?") { |gc| a = mk.(); o = Object.new; o.define_singleton_method(:==) { |x| gc.(); false }; a << o; [a.include?("zzz"), a.first] }
class Eq
  def initialize(s, gc) = (@s, @gc = s, gc)
  def ==(o) = (@gc.(); o == @s)
  def to_s = @s
end
check("Array#delete") { |gc| a = words.map { |w| Eq.new(w + "?", gc) }; [a.delete("fig?").to_s, a.delete("nope") { gc.(); "none" }, a.size, a.last.to_s] }
check("Array#find") { |gc| mk.().find { |x| gc.(); x.start_with?("c") } }
check("Array#inject") { |gc| mk.().inject { |acc, x| gc.(); acc + x[0] } }
check("Array#sort_by!") { |gc| a = mk.(); a.sort_by! { |x| gc.(); -x.size }; a }
check("Array#map!") { |gc| a = mk.(); a.map! { |x| gc.(); x.upcase }; a }
check("Array#select!") { |gc| a = mk.(); a.select! { |x| gc.(); x.size > 5 }; a }
check("Array#reject") { |gc| mk.().reject { |x| gc.(); x.include?("e") } }
check("Array#filter_map") { |gc| mk.().filter_map { |x| gc.(); x * 2 if x.size == 4 + 1 } }
check("Array#any? (pattern)") { |gc| p = Object.new; p.define_singleton_method(:===) { |x| gc.(); x == "date!" }; [mk.().any?(p), mk.().all? { |x| gc.(); x.end_with?("!") }] }
check("Hash#[] (default proc)") { |gc| h = Hash.new { |hh, k| gc.(); hh[k] = k * 2 }; [h["ab"], h["ab"], h.size] }
check("Hash#==") { |gc| v = Object.new; v.define_singleton_method(:==) { |x| gc.(); true }; [{ a: v, b: "x" * 3 } == { a: 1, b: "xxx" }] }
check("Hash#transform_keys") { |gc| { "k1" => 1, "k2" => 2 }.transform_keys { |k| gc.(); k + "!" } }
check("Hash#select") { |gc| { "a" => "x", "b" => "yy" }.select { |k, v| gc.(); v.size > 1 } }
check("Range#each") { |gc| s = +""; ("aa".."ad").each { |x| gc.(); s << x }; s }
check("Range#to_a") { |gc| gc.(); o = Struct.new(:n) { include Comparable; def succ = (GC.compact; self.class.new(n + 1)); def <=>(x) = n <=> x.n }; (o.new(1)..o.new(3)).to_a.map(&:n) }
check("Kernel#Array") { |gc| o = Object.new; o.define_singleton_method(:to_a) { gc.(); ["t" + "o_a"] }; Array(o) }
check("BasicObject#!=") { |gc| o = Object.new; o.define_singleton_method(:==) { |x| gc.(); x == "s" * 2 }; [o != "ss", o != "q"] }
check("Exception#message") { |gc| e = Class.new(StandardError) { define_method(:to_s) { gc.(); "m" * 3 } }.new; e.message }
check("String#match") { |gc| re = Object.new; m = "hello".match(/l+/) { |md| gc.(); md[0] + "!" }; m }
check("Enumerable#to_a") { |gc| e = Object.new; e.extend(Enumerable); e.define_singleton_method(:each) { |&b| %w[p q].each { |x| gc.(); b.(x + "!") } }; e.to_a }
check("String#tr!") { |gc| s = +"hello"; s.tr!("l", "L"); gc.(); s }
check("Struct.new") { |gc| k = Struct.new(:a, :b) { gc.(); def sum = a + b }; k.new("x", "y").sum }
check("Enumerable#map") { |gc| words.each_slice(2).map { |x| gc.(); x.join("-") } }
check("Enumerable#group_by") { |gc| words.each_entry.group_by { |w| gc.(); w.size } }
check("Enumerable#each_with_object") { |gc| words.each_entry.each_with_object([]) { |w, acc| gc.(); acc << w[0, 2] } }
check("Enumerable#sort_by") { |gc| words.each_entry.sort_by { |w| gc.(); w.reverse } }
check("Enumerable#any? (pattern)") { |gc| p = Object.new; p.define_singleton_method(:===) { |x| gc.(); x == "fig" }; words.each_entry.any?(p) }
check("Fiber[]") { |gc| Fiber[:k] = "v" * 2; gc.(); Fiber[:k] + Fiber.current.class.name }
check("Digest#hexdigest") { |gc| d = Digest::SHA256.new; d << "abc"; gc.(); d.hexdigest[0, 16] }
class ToS
  def initialize(s, gc) = (@s, @gc = s, gc)
  def to_s = (@gc.(); @s + "!")
  def inspect = (@gc.(); "<#{@s}>")
end
check("IO#write") { |gc| r, w = IO.pipe; n = w.write(ToS.new("a" * 2, gc), "-", ToS.new("b", gc)); w << ToS.new("c", gc); w.print(ToS.new("d", gc)); w.close; [n, r.read] }
check("IO#puts") { |gc| r, w = IO.pipe; w.puts([ToS.new("x", gc), [ToS.new("y", gc)]], ToS.new("z", gc)); w.close; r.read }
check("Kernel#p") { |gc| $stdout.flush; v = p(ToS.new("q", gc)); [v.to_s] }
class HashKey
  def initialize(k, gc) = (@k, @gc = k, gc)
  attr_reader :k
  def hash = (@gc.(); @k.hash)
  def eql?(o) = (@gc.(); o.is_a?(HashKey) && o.k == @k)
end
check("Hash#delete") { |gc| h = { HashKey.new("a" * 2, gc) => "v1", HashKey.new("b", gc) => "v2" }; [h.delete(HashKey.new("aa", gc)), h.delete(HashKey.new("zz", gc)) { |k| gc.(); k.k + "?" }, h.size] }
check("Array#hash") { |gc| a = [HashKey.new("e" * 2, gc), "f"]; a.hash == [HashKey.new("ee", gc), "f"].hash }
check("Array#flat_map") { |gc| mk.().first(3).flat_map { |x| gc.(); [x[0], x[-2]] } }
check("Array#uniq") { |gc| [mk.().uniq { |x| gc.(); x.size }, [HashKey.new("u", gc), HashKey.new("u", gc), "w" * 2].uniq.size] }
check("Array#each_with_object") { |gc| mk.().each_with_object(+"") { |x, acc| gc.(); acc << x[0] } }
class Ver
  include Comparable
  def initialize(n, gc) = (@n, @gc = n, gc)
  attr_reader :n
  def <=>(o) = (@gc.(); n <=> o.n)
end
check("Comparable#<") { |gc| a, b = Ver.new(1, gc), Ver.new(2, gc); [a < b, a >= b, a == Ver.new(1, gc), b.between?(a, b)] }
check("String#gsub") { |gc| [("a-b-c" * 2).gsub(/-(.)/) { gc.(); $1.upcase + "+" }, "hello".gsub("l") { gc.(); "L" * 2 }, "abc".gsub(/[ab]/, Hash.new { |_, k| gc.(); k.upcase * 2 })] }
check("String#sub!") { |gc| s = +"x1y2"; s.sub!(/\d/) { |d| gc.(); (d.to_i * 7).to_s }; t = +"x1y2"; t.gsub!(/\d/) { |d| gc.(); "<#{d}>" }; [s, t] }
check("Kernel#format") { |gc| [format("%s-%p|%5s", ToS.new("f", gc), ToS.new("g", gc), ToS.new("h", gc)), "%s/%s" % [ToS.new("i", gc), ToS.new("j" * 2, gc)]] }
check("Hash#delete_if") { |gc| h = { "a" => 1, "bb" => 2, "ccc" => 3 }; h.delete_if { |k, _| gc.(); k.size == 2 }; g = h.dup; g.select! { |k, v| gc.(); v > 1 }; [h, g] }
check("Array#reverse_each") { |gc| out = []; mk.().reverse_each { |x| gc.(); out << x[0, 2] }; out }
check("Enumerable#sort") { |gc| e = Object.new; e.extend(Enumerable); e.define_singleton_method(:each) { |&b| %w[m k z].each { |x| b.(x * 2) } }; [e.sort { |x, y| gc.(); y <=> x }, e.min { |x, y| gc.(); x <=> y }] }
check("Enumerable#find") { |gc| e = Object.new; e.extend(Enumerable); e.define_singleton_method(:each) { |&b| %w[n o p].each { |x| b.(x * 2) } }; [e.find { |x| gc.(); x > "nn" }, e.detect(-> { gc.(); "none" }) { |x| gc.(); x == "q" }] }
check("Enumerable#find_index") { |gc| e = Object.new; e.extend(Enumerable); e.define_singleton_method(:each) { |&b| %w[n o p].each { |x| gc.(); b.(x * 2) } }; [e.find_index("o" * 2), e.find_index { |x| gc.(); x == "pp" }] }
check("Hash#merge!") { |gc| h = { "a" => "1", "b" => "2" }; h.merge!({ "b" => "3" * 2, "c" => "4" }) { |k, o, n| gc.(); k + o + n }; [h, h.merge({ "a" => "5" }) { |_, o, n| gc.(); o + n }] }
class Single
  def initialize(s, gc) = (gc.(); @s = s * 2)
  attr_reader :s
  class << self
    alias_method :build, :new
  end
end
check("Class#new (alias)") { |gc| Single.build("q", gc).s }
class Bag < Array
  def initialize(n, gc) = (gc.(); super(n) { |i| gc.(); "b#{i}" })
end
class Oops < StandardError
  def initialize(m, gc) = (gc.(); super(m * 2))
end
check("Array.new") { |gc| [Bag.new(2, gc), Array.new(2) { |i| gc.(); "a#{i}" }, (raise Oops.new("x", gc) rescue $!.message), Oops.exception("y", gc).message] }
check("Symbol#match") { |gc| :hello.match(/l+/) { |m| gc.(); m[0] + "!" } }
require "json"
class J
  def initialize(s, gc) = (@s, @gc = s, gc)
  def to_json(*) = (@gc.(); "\"#{@s}\"")
end
check("JSON.generate") { |gc| [JSON.generate({ "a" => [J.new("x" * 2, gc), "y"], "b" => { "c" => J.new("z", gc) } }), JSON::State.new.generate([J.new("w", gc), 1]), [J.new("v", gc)].to_json] }
check("Hash#transform_values") { |gc| h = { "a" => "x", "b" => "y" * 2 }; g = h.dup; g.transform_values! { |v| gc.(); v * 2 }; [h.transform_values { |v| gc.(); v.upcase }, g, h.any? { |k, v| gc.(); v.size > 1 }] }
check("Array#drop_while") { |gc| mk.().drop_while { |x| gc.(); x < "c" } }
require "random/formatter"
class Fmt
  include Random::Formatter
  def initialize(gc) = @gc = gc
  def bytes(n) = (@gc.(); ("\x01\xab" * n)[0, n])
end
check("Random::Formatter") { |gc| f = Fmt.new(gc); [f.hex(4), f.base64(3), f.random_number(10).between?(0, 9), f.alphanumeric(3).size] }
class Made
  def initialize(s, gc) = (gc.(); @s = s * 2)
  attr_reader :s
  private_class_method :new
  def self.make(s, gc) = new(s, gc)
end
class Open < Made
  public_class_method :new
end
check("Class#new (visibility copies)") { |gc| [Made.make("m", gc).s, Open.new("o", gc).s] }
check("Array#each_with_index") { |gc| out = []; mk.().each_with_index { |x, i| gc.(); out << "#{i}#{x}" }; out }
class Shown
  def initialize(s, gc) = (@s, @gc = s, gc)
  def inspect = (@gc.(); "<#{@s * 2}>")
end
class Holder
  def initialize(a, b) = (@a, @b = a, b)
end
check("Hash#inspect") { |gc| h = { "k" => Shown.new("v", gc), Shown.new("q", gc) => [Shown.new("w", gc), "s"] }; [h.inspect, h.to_s] }
check("Kernel#inspect") { |gc| Holder.new(Shown.new("a", gc), ["b" * 2, Shown.new("c", gc)]).inspect.sub(/0x\h+/, "X") }
class Coerced
  def initialize(k, gc) = (@k, @gc = k, gc)
  def coerce(n) = (@gc.(); ["<#{n}>", @k])
end
check("Integer#+ (coerce)") { |gc| [1 + Coerced.new("ab" * 2, gc), 2.5 * Coerced.new(2, gc), Rational(1, 2) + Coerced.new("r", gc)] }
check("Array#index") { |gc| a = mk.(); [a.index { |x| gc.(); x.start_with?("d") }, a.rindex { |x| gc.(); x.size == 4 }, a.find_index("fig!")] }
class Year < Numeric
  def initialize(gc) = @gc = gc
  def to_i = (@gc.(); 2024)
end
check("Time.utc (Numeric#to_int)") { |gc| [Time.utc(Year.new(gc), "mar" + "", "1" * 2, 2, 3, 4).to_a.first(6), Time.new(Year.new(gc), 3, 1, 0, 0, 0, "+09:00").utc_offset] }

$ok.each { |name, done, r| puts "#{name}: #{done} #{r.inspect}" }
