# Specialised call paths and inlined blocks, including every case where the guard must
# fall back to a normal call.

# --- Array / Hash / String fast paths ---
a = [1, 2, 3]
p a[0], a[-1], a[5], a[-9], a[1, 2], a[0..1], a[1.9], a.size, a.length, a.empty?, [].empty?, a.first, a.last, [].first
a[1] = :b
a[3] = 4
a[6] = 7
a[-1] = 8
p a, (begin; a[-20] = 1; rescue IndexError; $!.class; end)
a << 9
a.push(10)
p a, a.size
h = { "k" => 1, s: 2 }
p h["k"], h[:s], h[:none], h.size, h.empty?, h.key?("k"), h.include?(:x), h.member?(:s)
h["k"] = 5
h[:new] = 6
d = Hash.new(0)
d[:x] += 1
dp = Hash.new { |hash, k| hash[k] = k.to_s * 2 }
p h.to_a, d.to_a, dp[:ab], dp.to_a, dp.size
key = +"mutable"
h2 = {}
h2[key] = 1
key << "!"
p h2.to_a, h2.keys[0].frozen?
s = +"ab"
s << "cd" << 101
p s, s == "abcde", s == :abcde, "x" == "x", "x".empty?, "".empty?, s.size
fa = [1].freeze
p((fa << 2 rescue $!.class), (begin; fa[0] = 3; rescue => e; e.class; end))
fh = {}.freeze
p((begin; fh[:a] = 1; rescue => e; e.class; end))
p :a == :a, :a == :b, nil.nil?, 1.nil?, Object.new == Object.new, (o = Object.new) == o

# --- subclasses and singletons overriding the specialised methods ---
class MyArray < Array
  def [](i)
    "my[#{i}]"
  end
  def size
    -1
  end
  def <<(x)
    push(x * 100)
  end
end
m = MyArray.new([1, 2])
p m[0], m.size, (m << 3), m.length, m.first
class MyHash < Hash
  def [](k)
    "h[#{k}]"
  end
  def []=(k, v)
    super(k, v * 2)
  end
end
mh = MyHash.new
mh[:a] = 2
p mh[:a], mh.fetch(:a), mh.size
class MyString < String
  def ==(o)
    true
  end
end
p MyString.new("a") == "b", "b" == MyString.new("a")
sa = [1, 2]
def sa.[](i)
  :singleton
end
p sa[0], [1, 2][0]
class Ghost
  def method_missing(name, *args)
    [:mm, name, args]
  end
  def respond_to_missing?(*)
    true
  end
end
g = Ghost.new
p g[1], g.size, g << 1, g.nil?, g.empty?, (g[1] = 2)
class Nilish
  def nil?
    true
  end
  def ==(o)
    :custom
  end
end
p Nilish.new.nil?, Nilish.new == 1, Nilish.new != 1

# --- the same call sites before and after a runtime redefinition ---
def first_of(x)
  x[0]
end
def len_of(x)
  x.size
end
def eq?(x, y)
  x == y
end
p first_of([5, 6]), len_of([5, 6]), eq?(:a, :a), eq?("s", "s")
class Array
  alias_method :orig_aref, :[]
  def [](*args)
    r = orig_aref(*args)
    r.is_a?(Integer) ? r * 10 : r
  end
  alias_method :orig_size, :size
  def size
    orig_size + 1000
  end
end
class Symbol
  def ==(o)
    :patched
  end
end
class String
  alias_method :orig_eq, :==
  def ==(o)
    orig_eq(o) ? :yes : :no
  end
end
p first_of([5, 6]), len_of([5, 6]), eq?(:a, :a), eq?("s", "s"), [7][0], [7].size
class Array
  alias_method :[], :orig_aref
  alias_method :size, :orig_size
end
class String
  alias_method :==, :orig_eq
end
class Symbol
  remove_method :==
end
p first_of([5, 6]), len_of([5, 6]), eq?(:a, :a), eq?("s", "s")

# --- inlined block loops ---
sum = 0
10.times { |i| sum += i }
p sum, 3.times { }, 0.times { raise "never" }, -2.times { raise "never" }
p [1, 2, 3].map { |x| x * 2 }, [1, 2, 3].collect { _1 }, [1, 2, 3, 4].select(&:even?), [1, 2, 3, 4].reject { |x| x.odd? }
p [1, 2, 3].map { |x| next 0 if x == 2; x }, [1, 2, 3].each { |x| break x * 10 if x == 2 }, (1..9).each { |i| break i if i * i > 10 }
p [[1, 2], [3, 4]].map { |a, b| a + b }, [[1, [2, 3]]].map { |a, (b, c)| [a, b, c] }, [1, [2]].map { |a, b| [a, b] }
p [5, 6].each_with_index.map { |x, i| x * i }, [5, 6].each_with_index { |x, i| print x, i }, [3, 4].each_index { |i| print i }
puts
p 1.upto(4) { |i| print i }, 4.downto(1) { |i| print i }, 1.upto(3.5) { |i| print i }, 3.downto(0.5) { |i| print i }
puts
p (1..4).each { |i| print i }, (1...4).each { |i| print i }, (4..1).each { |i| print i }, ("a".."c").each { |c| print c }
puts
p [1, nil, 3].any? { |x| x.nil? }, [1, 2].all? { |x| x > 0 }, [].any? { true }, [].all? { false }, [1].all? { |x| x > 5 }
arr = [1, 2, 3]
arr.each { |e| arr << e * 10 if arr.size < 6 }
p arr
arr2 = [1, 2, 3, 4]
p arr2.map { |e| arr2.pop if e == 1; e }
x = 10
[1].each { |y| x = y; z = 5 }
p x, defined?(z)
3.times { |i| w ||= i; print w }
puts
r = []
3.times do |i|
  j = i * 2
  r << -> { j }
end
p r.map(&:call)
def ret_from_block
  [1, 2, 3].each { |x| return x * 100 if x == 2 }
  :none
end
p ret_from_block
def yielder
  out = []
  [1, 2].each { |x| out << yield(x) }
  out
end
p yielder { |v| v * 3 }
begin
  [1, 2].each { |x| raise ArgumentError, "boom #{x}" if x == 2 }
rescue ArgumentError => e
  p e.message
end
p [1, 2, 3].each_with_index.to_a, 3.times.map { |i| i * 2 }, [4, 5].map.with_index { |x, i| x + i }
class Countdown
  def times
    yield :custom
    :countdown
  end
  def each
    yield 1
    yield 2
    :each_done
  end
  def map
    [:mine]
  end
end
c = Countdown.new
p c.times { |v| p v }, c.each { |v| print v }, c.map { |v| v }
puts
class MyArray
  def each
    yield :own_each
    self
  end
end
p MyArray.new([1, 2]).each { |e| p e }.class, MyArray.new([1]).map { |e| e }
class Integer
  alias_method :orig_times, :times
  def times
    yield :patched_times
    self
  end
end
p 2.times { |i| p i }
class Integer
  alias_method :times, :orig_times
end
p 2.times { |i| print i }
puts
o = [1, 2]
def o.each
  yield :singleton_each
end
o.each { |v| p v }
p((nil.each { } rescue $!.class), (5.each { } rescue $!.class))
nums = [3, 1, 2]
p nums.sort_by { |n| -n }, nums.sort, %w[b a c].sort_by { |w| w }, [2.5, 1, 3.0].sort, [[2, :b], [1, :a]].sort_by(&:first)
p (1..Float::INFINITY).each { |i| break i if i > 3 }, (1..).each { |i| break i * 2 if i > 2 }
i = 0
while i < 3
  [1, 2].each { |e| next if e == 1; i += e }
end
p i

# methods made private can't be called with a receiver, fast path or not
class Array; private :size, :each, :map; end
class Hash; private :[]; end
pa = [1, 2]
p((pa.size rescue $!.class), (pa.each { |x| p x } rescue $!.class), (pa.map { |x| x } rescue $!.class), ({ a: 1 }[:a] rescue $!.class))
class Array; public :size, :each, :map; end
class Hash; public :[]; end
p pa.size, pa.map { |x| x }, { a: 1 }[:a]

# refinements: call sites after `using` see the refined methods
module Refined
  refine Array do
    def [](i)
      :refined_aref
    end
    def each
      yield :refined_each
      self
    end
    def size
      :refined_size
    end
  end
  refine Integer do
    def times
      yield :refined_times
    end
  end
  refine Float do
    def +(o)
      :refined_plus
    end
    def <(o)
      :refined_lt
    end
  end
end
using Refined
p [1, 2][0], [1, 2].size, [9].each { |v| p v }.size, 3.times { |v| p v }, [1].map { |v| v + 1 }
p 1.5 + 1.0, 1.5 < 2.0, 2.5 * 2, 1 + 2
