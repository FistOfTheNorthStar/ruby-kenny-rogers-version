# attr_reader / attr_writer through the call site's cached ivar slot

class P
  attr_accessor :a, :b
  def initialize(a, b)
    @a = a
    @b = b
  end
end
class Q < P
  attr_accessor :c
  def initialize(a, b)
    @c = :c   # Q's own ivar order differs from P's
    @b = b
    @a = a
  end
end
class R
  attr_accessor :b, :a
  def initialize
    @b = :rb
  end
end

# one call site, several receiver classes (more than the cache holds)
class S1 < P; end
class S2 < P; end
class S3 < P; end
objs = [P.new(1, 2), Q.new(3, 4), R.new, S1.new(5, 6), S2.new(7, 8), S3.new(9, 10)]
3.times { p objs.map { |o| [o.a, o.b] } }

# a reader before its ivar exists in any object, then after
class Late
  attr_accessor :x
end
l = Late.new
r = []
3.times { |i| r << l.x; l.x = i if i == 1 }
p r, Late.new.x

# writes that grow the object, and to a frozen object
w = Late.new
3.times { |i| w.x = i * 10 }
p w.x
w.freeze
begin
  w.x = 1
rescue => e
  p e.class
end
p w.x

# private / protected attributes: not with an explicit receiver, fine without
class Priv
  def initialize
    @s = :secret
    @t = :prot
  end
  def peek
    [s, t, self.s]
  end
  def other(o)
    o.t
  end
  private attr_reader :s
  protected attr_reader :t
end
pv = Priv.new
3.times { p pv.peek }
p pv.other(Priv.new)
2.times do
  begin
    pv.s
  rescue NoMethodError => e
    p e.message[/private method 's'/]
  end
end

# redefining the method after the cache is warm, and a singleton method on one object
k = P.new(:x, :y)
v = []
3.times { v << k.a }
class P
  def a = :redefined
end
3.times { v << k.a }
def k.b = :singleton
v << k.b << P.new(1, 2).b
p v
class P
  remove_method :a
  attr_reader :a
end
p k.a

# remove_instance_variable leaves the slot empty: the reader says nil
z = P.new(1, 2)
2.times { z.b }
z.remove_instance_variable(:@b)
p z.b, z.instance_variables

# compound assignment through the attribute (read and write at one site each)
acc = P.new(0, 0.0)
1000.times { |i| acc.a += i; acc.b += 0.5 }
p acc.a, acc.b

# old objects storing young ones (write barrier), with collections in between
keep = Array.new(50) { P.new(nil, nil) }
GC.start
GC.start
4.times do |round|
  keep.each_with_index { |o, i| o.a = "s#{round}-#{i}"; o.b = [round, i] }
  200.times { Object.new }
  GC.start
end
p keep.first(2).map { |o| [o.a, o.b] }, keep.all? { |o| o.a.start_with?("s3-") && o.b[0] == 3 }

# several threads on shared objects
shared = Array.new(8) { P.new(0, 0) }
ts = 4.times.map do |t|
  Thread.new do
    20_000.times { |i| o = shared[i % 8]; o.b = t; o.a = o.a.to_i }
  end
end
ts.each(&:join)
p shared.map(&:b).all? { |x| (0..3).include?(x) }

# string eval (compiled at runtime) uses the same cache
P.class_eval "def sum_ab = a + b"
p P.new(20, 22).sum_ab, eval("q = Q.new(1, 2); [q.a, q.b, q.c]")

# singleton classes of short-lived objects (freed by the GC; a new one may reuse the address)
# for classes with different ivar layouts, read at one site inside a loop
module Tag; end
class L1
  attr_reader :v
  def initialize(v) = (@v = v)
end
class L2
  attr_reader :v
  def initialize(v)
    @pad = 0
    @v = v
  end
end
bad = 0
200.times do |i|
  o = (i.even? ? L1 : L2).new(i)
  o.singleton_class if i % 3 == 0
  o.extend(Tag) if i % 5 == 0
  j = 0
  while j < 3
    bad += 1 unless o.v == i
    j += 1
  end
  GC.start if i % 20 == 0
end
p bad

# attribute reads and writes inside a loop (compiled inline there)
pts = [P.new(1, 2), Q.new(3, 4), P.new(5, 6)]
i = 0
while i < 3000
  pt = pts[i % 3]
  pt.a = pt.a + pt.b
  i += 1
end
p pts.map(&:a)
k2 = P.new(0, 0)
i = 0
out = []
while i < 4
  out << k2.a
  k2.a = i
  P.class_eval { def a = :late } if i == 1
  i += 1
end
p out
