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

$ok.each { |name, done, r| puts "#{name}: #{done} #{r.inspect}" }
