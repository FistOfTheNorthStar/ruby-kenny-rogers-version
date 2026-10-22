# a[i] and a[i] = v are done inline for plain Arrays with Integer indexes (while Array#[]
# and Array#[]= are the builtin ones); everything else must behave as the normal call.

def get(a, i) = a[i]
def set(a, i, v) = (a[i] = v)
def bump(a, i) = a[i] += 1

a = [10, 20, 30]
p get(a, 0), get(a, -1), get(a, -3), get(a, -4), get(a, 3), get(a, 100), get(a, 1.9)
p get(a, 0..1), get(a, -2..), (get(a, "x") rescue [$!.class, $!.message])
p get([], 0), get([], -1)
p set(a, 0, :a), set(a, -1, :c), set(a, 5, :f), a
p (set(a, -10, 1) rescue [$!.class, $!.message])
b = [0, 0]
5.times { bump(b, 1) }
p b
p (bump([nil], 0) rescue $!.class)

# frozen
f = [1, 2].freeze
p get(f, 0), (set(f, 0, 9) rescue [$!.class, $!.message]), f

# subclass, singleton method, other classes with []
class MyAry < Array
  def [](i) = [:my, super]
  def []=(i, v)
    super(i, v * 2)
  end
end
m = MyAry.new([1, 2, 3])
p get(m, 1), set(m, 0, 5), m.to_a
s = [1, 2, 3]
def s.[](i) = :single
p get(s, 0), get([7], 0)
h = { 0 => :zero }
p get(h, 0), set(h, 1, :one), h, get("hello", 1)
p (get(1..5, 0) rescue $!.class)
p get(5, 0), get(5, 1)
Pt = Struct.new(:x, :y)
p get(Pt.new(3, 4), 1), set(Pt.new(3, 4), 0, 9)

# old arrays storing new objects (write barrier) with collections in between
keep = Array.new(1000) { [] }
GC.start
3.times do |r|
  1000.times { |i| set(keep, i, "s#{r}-#{i}" * 2) }
  GC.start
  junk = Array.new(2000) { |i| [i, i.to_s] }
  GC.start
end
p keep.first, keep.last, keep.map(&:size).sum
ints = Array.new(1000, 0)
GC.start
1000.times { |i| set(ints, i, i * 1.5) }
p ints.sum

# loops: a histogram, a sieve, swaps
counts = Array.new(8, 0)
sums = Array.new(8, 0.0)
i = 0
while i < 1000
  counts[i % 8] += 1
  sums[i % 8] += i * 0.5
  i += 1
end
p counts, sums
sieve = Array.new(50, true)
sieve[0] = sieve[1] = false
(2..7).each { |k| (k * k).step(49, k) { |j| sieve[j] = false } if sieve[k] }
p sieve.each_index.select { |k| sieve[k] }
xs = [5, 3, 9, 1]
j = 0
while j < xs.size - 1
  xs[j], xs[j + 1] = xs[j + 1], xs[j] if xs[j] > xs[j + 1]
  j += 1
end
p xs

# Array#[] redefined (after the sites are warm), then restored
class Array
  alias_method :orig_aref, :[]
  alias_method :orig_aset, :[]=
  def [](*a) = [:redef, *a]
  def []=(*a)
    orig_aset(0, :set_redef)
  end
end
t = [1, 2]
p get(t, 0), set(t, 1, 5), t
class Array
  alias_method :[], :orig_aref
  alias_method :[]=, :orig_aset
end
p get(t, 0), set(t, 1, 6), t

# threads sharing arrays
shared = Array.new(4, 0)
ths = 4.times.map { |k| Thread.new { 1000.times { |n| set(shared, k, get(shared, k) + 1) } } }
ths.each(&:join)
p shared
