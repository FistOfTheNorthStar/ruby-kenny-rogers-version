# Calls to compiled methods and attributes that skip the runtime once a call site's cache
# knows the receiver's class

class A
  attr_accessor :x, :y
  def initialize(x) = (@x = x)
  def get = x
  def twice = get * 2
  def add(a) = x + a
  def add2(a, b) = x + a + b
  def call_secret = secret
  def call_secret_explicit(o) = o.secret
  private def secret = :hidden
end
class B < A
  def get = x + 100
end
class C < A
  attr_reader :z
  def initialize(x)
    @z = :cz
    super
  end
end

# monomorphic and polymorphic sites, 0-2 arguments
objs = [A.new(1), B.new(2), C.new(3), A.new(4)]
3.times { p objs.map { |o| [o.get, o.twice, o.add(10), o.add2(1, 2), o.x] } }

# private: fine without a receiver, NoMethodError with one, also after warming up
a = A.new(5)
3.times { p a.call_secret }
2.times do
  begin
    a.secret
  rescue NoMethodError => e
    p e.message[/private method 'secret'/]
  end
end
begin
  a.call_secret_explicit(a)
rescue NoMethodError => e
  p e.message[/private method 'secret'/]
end

# redefinition after the sites are warm
k = A.new(7)
r = []
3.times { r << k.twice }
class A
  def get = :changed
end
begin
  r << k.twice
rescue NoMethodError => e
  r << e.class
end
r << A.new(1).get
p r
class A
  def get = x
end
p k.twice

# singleton methods on one object
s = A.new(8)
p s.get
def s.get = :single
p s.get, A.new(9).get

# deep recursion still raises SystemStackError (the direct path checks the stack)
def down(n) = n == 0 ? 0 : 1 + down(n - 1)
p down(10_000)
begin
  down(100_000_000)
rescue SystemStackError => e
  p e.class
end
p down(1000)
f = Fiber.new do
  begin
    down(100_000_000)
  rescue SystemStackError => e
    e.class
  end
end
p f.resume
p down(1000)

# recursion with collections in between
def fib(n) = n < 2 ? n : fib(n - 1) + fib(n - 2)
p fib(24)
def churn(n)
  return 0 if n == 0
  Array.new(50) { Object.new }
  GC.start if n % 100 == 0
  1 + churn(n - 1)
end
p churn(500)

# attributes outside loops: polymorphic readers, writers, frozen, removed ivars
def read_x(o) = o.x
def write_y(o, v) = (o.y = v)
os = [A.new(1), B.new(2), C.new(3)]
3.times { os.each { |o| read_x(o); write_y(o, o.x * 10) } }
p os.map { |o| [read_x(o), o.y] }
fr = A.new(1)
write_y(fr, 2)
fr.freeze
begin
  write_y(fr, 3)
rescue => e
  p e.class
end
p fr.y
rm = A.new(4)
3.times { read_x(rm) }
rm.remove_instance_variable(:@x)
p read_x(rm), rm.instance_variables

# old objects storing young values through a writer outside a loop
olds = Array.new(30) { A.new(0) }
GC.start
GC.start
4.times do |round|
  olds.each_with_index { |o, i| write_y(o, "v#{round}-#{i}") }
  300.times { Object.new }
  GC.start
end
p olds.first(2).map(&:y), olds.all? { |o| o.y.start_with?("v3-") }

# threads (several OS threads: calls go through the runtime)
ts = 4.times.map { |t| Thread.new { sum = 0; 20_000.times { |i| sum += objs[i % 4].add(t) }; sum } }
p ts.map(&:value)
p objs.map(&:twice)

# instance variables of fresh objects
class D
  def initialize
    @a = 1
  end
  def set_b = (@b = 2)
end
d = D.new
p d.instance_variables, d.instance_variable_defined?(:@b), d.inspect.include?("@b")
d.set_b
p d.instance_variables, D.new.instance_variables

# == / != against Symbol literals (inline while Symbol#== is the builtin)
class Any
  def ==(o) = true
end
x = :a
vals = [:a, :b, "a", nil, 1, Any.new]
p vals.map { |v| [v == :a, :a == v, v != :a, :a != v] }
class Symbol
  alias_method :orig_eq, :==
  def ==(o) = :redef
end
p [x == :a, :a == x, x != :a, :b == :b]
class Symbol
  alias_method :==, :orig_eq
end
p [x == :a, :a == x, x != :a, x == :b]

# an old closure environment and old hashes storing young values repeatedly between
# minor collections (the write barrier records each old object once per collection)
held = nil
h = {}
GC.start
GC.start
lst = []
5.times do |round|
  2000.times do |i|
    held = "s#{round}-#{i}"
    h[:k] = [held]
    lst << held if i % 500 == 0
    Object.new
  end
  GC.start(full_mark: false) rescue GC.start
end
p held, h[:k], lst.size, lst.last

# nil? / is_a? / kind_of? (answered without a call while they are the builtin methods)
module Mx; end
class Pa; end
class Pb < Pa; include Mx; end
class Pn < Pa
  def nil? = :custom
  def is_a?(c) = :custom
end
class Bo < BasicObject; end
vals = [nil, 1, 2.5, :s, true, false, "str", [1], Pa.new, Pb.new, Pn.new]
3.times do
  p vals.map { |v| [v.nil?, v.is_a?(Pa), v.kind_of?(Mx), v.is_a?(Integer), v.is_a?(Comparable), v.is_a?(Object)] }
end
3.times do
  begin
    Bo.new.nil?
  rescue NoMethodError => e
    p e.class
  end
end
def chk(v) = [v.nil?, v.is_a?(Pa)]
sp = Pa.new
3.times { p chk(sp) }
def sp.nil? = :single
p chk(sp)
class Integer
  def nil? = :int_nil
end
p [1.nil?, chk(5), nil.nil?]
class Integer
  remove_method :nil?
end
p [1.nil?, chk(5)]
class NilClass
  def is_a?(c) = :nil_is_a
end
p [nil.is_a?(Pa), chk(nil)]
class NilClass
  remove_method :is_a?
end
p [nil.is_a?(NilClass), chk(nil)]
module Kernel
  alias_method :orig_kind_of?, :kind_of?
  def kind_of?(c) = :kernel_kind_of
end
p [1.kind_of?(Integer), Pa.new.kind_of?(Pa)]
module Kernel
  alias_method :kind_of?, :orig_kind_of?
end
p [1.kind_of?(Integer), Pa.new.kind_of?(Pa)]

# is_a? answers remembered per call site, then include/extend/reopen changing them
module Late; end
class Qa; end
class Qb < Qa; end
def late?(o) = [o.is_a?(Late), o.is_a?(Qa), o.kind_of?(Comparable)]
qs = [Qa.new, Qb.new]
3.times { p qs.map { |q| late?(q) } }
class Qb
  include Late
end
p qs.map { |q| late?(q) }
q = Qa.new
3.times { late?(q) }
q.extend(Late)
p late?(q), late?(Qa.new)
k = Class.new(Qa)
3.times { p late?(k.new) }
consts = [Qa, Qb, Late, Integer]
p consts.map { |c| qs.map { |o| o.is_a?(c) } }

# attribute names read without a receiver in methods (inline for declared names), on
# nil/false/integer self, subclasses, private readers, removed ivars, singletons
class NilClass
  def get_v = v
  def v = :nil_v
end
class FalseClass
  def get_v = v
  def v = :false_v
end
class Integer
  def get_v = v
  def v = :int_v
end
p [nil.get_v, false.get_v, 3.get_v]
class Ra
  attr_reader :v
  attr_accessor :u
  def initialize(v) = (@v = v; @u = v * 2)
  def get_v = v
  def both = [v, u]
  private attr_reader :pv
  def get_pv = pv
end
class Rb < Ra; end
class Rc < Ra
  def v = :overridden
end
os = [Ra.new(1), Rb.new(2), Rc.new(3), Ra.new(4)]
3.times { p os.map { |o| [o.get_v, o.both] } }
pr = Ra.new(9)
pr.instance_variable_set(:@pv, :private_v)
p pr.get_pv
rm = Ra.new(5)
3.times { rm.get_v }
rm.remove_instance_variable(:@v)
p rm.get_v
sg = Ra.new(6)
3.times { sg.get_v }
def sg.v = :single_v
p sg.get_v, Ra.new(7).get_v
class Ra
  def v = :redefined
end
p os.map(&:get_v)
