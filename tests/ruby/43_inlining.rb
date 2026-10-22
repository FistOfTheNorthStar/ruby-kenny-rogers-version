# Small methods are compiled into their callers (behind a guard on the receiver's class
# and the method serial). Everything must behave as real calls: frames in backtraces,
# redefinition, overriding, visibility, constants.

class Vec
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
  def len2 = @x * @x + @y * @y
  def dot(o) = @x * o.x + @y * o.y
  def scaled(k) = Vec.new(@x * k, @y * k)
  def bad = @x + nil
  def deeper = bad
  def where = caller(0, 2)
  def locs = caller_locations(0, 1).map { |l| [l.lineno, l.label] }
end

v = Vec.new(3, 4)
w = Vec.new(1, 2)
s = 0
i = 0
while i < 10
  s += v.len2 + v.dot(w)
  i += 1
end
p s, v.scaled(2).len2

# errors raised inside inlined code keep the method frames
def run_bad(v)
  v.deeper
rescue TypeError => e
  [e.message, e.backtrace.map { |l| l[/:(\d+):/, 1].to_i }.reject { |l| l == 14 }.first(3)]
end
3.times { run_bad(v) }
p run_bad(v)
def where_from(v) = v.where
p where_from(v).map { |l| l[/:(\d+):/, 1].to_i }
p v.locs.map(&:first)

# redefinition after warm-up
def total(v, w) = v.len2 + v.dot(w)
3.times { total(v, w) }
class Vec
  def len2 = :redefined
end
p total(Vec.new(1, 1), w) rescue p $!.class
class Vec
  def len2 = @x * @x + @y * @y
end
p total(v, w)

# subclass override, singleton methods, other classes with the same method name
class Vec3 < Vec
  def len2 = super + 100
end
p total(Vec3.new(1, 1), w)
o = Vec.new(2, 2)
def o.len2 = -1
p total(o, w)
Fake = Struct.new(:x, :y) do
  def len2 = 7
  def dot(_) = 8
end
p total(Fake.new(1, 1), w)
p (total(nil, w) rescue [$!.class, $!.message])
p (total(5, w) rescue $!.class)

# visibility: private and protected targets aren't called from outside
class Secret
  def pub = hidden + 1
  def hidden = 41
  private :hidden
  def prot = 5
  protected :prot
  def cmp(o) = o.prot
end
def call_hidden(s) = s.hidden
p Secret.new.pub, (call_hidden(Secret.new) rescue $!.class), Secret.new.cmp(Secret.new)

# constants resolve in the method's own scope
K = :top
module M
  K = :m
  class C
    def k = K
  end
end
def get_k(c) = c.k
p get_k(M::C.new)
class D
  K = :d
  def k = K
end
p get_k(D.new)

# receiver-less calls, recursion, method_missing, respond_to
class Shape
  def initialize(w, h)
    @w = w
    @h = h
  end
  def w = @w
  def h = @h
  def area = w * h
  def twice = area + area
end
p Shape.new(3, 4).twice
def fact(n) = n <= 1 ? 1 : n * fact(n - 1)
p fact(20)
class Ghost
  def method_missing(n, *a) = [:ghost, n]
  def respond_to_missing?(*) = true
end
p total(Ghost.new, w)

# methods that modify state, multiple returns through ifs, ivars set on other objects
class Counter
  def initialize = @n = 0
  def bump(k) = @n += k
  def n = @n
  def sign = @n > 0 ? :pos : (@n < 0 ? :neg : :zero)
end
c = Counter.new
5.times { |k| c.bump(k) }
p c.n, c.sign
c.bump(-100)
p c.sign

# C.new with initialize inlined: allocation, arguments, constants in initialize, errors,
# redefined new/initialize/allocate, subclasses, Struct, modules
SCALE = 10
class P2
  attr_reader :a, :b
  def initialize(a, b)
    @a = a * SCALE
    @b = b
  end
end
module Geo
  SCALE = 100
  class Pt
    attr_reader :v
    def initialize(v) = @v = v * SCALE
  end
end
def mk(a, b) = P2.new(a, b)
def mkg(v) = Geo::Pt.new(v)
3.times { mk(1, 2); mkg(1) }
x = mk(1, 2)
p [x.class, x.a, x.b, mkg(3).v, x.instance_variables]
p (mk(nil, 2) rescue [$!.class, $!.backtrace.map { |l| l[/:(\d+):/, 1].to_i }.first(2)])
p (P2.new(1) rescue [$!.class, $!.message])
class P2
  def initialize(a, b)
    @a = :re
    @b = b
  end
end
p mk(1, 2).a
class P2
  def self.new(*a) = :custom_new
end
p mk(1, 2)
class << P2
  remove_method :new
end
p mk(5, 6).a
class Sub2 < P2; end
def mks(a, b) = Sub2.new(a, b)
3.times { mks(1, 2) }
p mks(1, 2).class, mks(1, 2).b
class Q2
  def initialize(x) = @x = x
  def x = @x
end
def mkq(x) = Q2.new(x)
3.times { mkq(1) }
q = mkq(:q)
def q.extra = 1
p mkq(2).x, q.x, q.extra, mkq(3).frozen?, mkq(4).respond_to?(:extra)
class Q2
  def self.allocate = raise("not called by new")
end
p mkq(7).x
class R2
  def initialize(v) = @v = v
  def v = @v
end
def mkr(v) = R2.new(v)
3.times { mkr(1) }
p mkr(1).v
R2.define_method(:initialize) { |v| @v = [:dm, v] }
p mkr(2).v
R2.singleton_class.define_method(:new) { |*a| :sing_new }
p mkr(3)
