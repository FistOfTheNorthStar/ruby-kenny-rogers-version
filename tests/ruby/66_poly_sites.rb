# Call sites whose receivers are of several classes: inlined and static calls keep a guard
# per class (up to four, then the runtime), compiled code checks every way of a call site's
# inline cache, attributes are read inline for each class, and is_a? is answered inline in
# loops. Each change below (redefinition, singleton method, include/prepend, visibility,
# method_missing) must be seen at once by sites that already ran hot.

class Shape
  attr_reader :name
  def initialize(name) = @name = name
  def big? = area > 100
  def label = "#{name}:#{area}"
end
class Rect < Shape
  attr_accessor :w, :h
  def initialize(w, h)
    super(:rect)
    @w = w
    @h = h
  end
  def area = w * h
end
class Square < Rect
  def initialize(s) = super(s, s)
end
class Circle < Shape
  attr_reader :r
  def initialize(r)
    super(:circle)
    @r = r
  end
  def area = 3 * r * r
end
class Tri < Shape
  def initialize(b, h)
    super(:tri)
    @b = b
    @h = h
  end
  def area = @b * @h / 2
end
class Pent < Shape
  def initialize = super(:pent)
  def area = 500
end
class Hex < Rect
  def initialize = super(2, 3)
end

def survey(shapes)
  big = 0
  names = Hash.new(0)
  rects = 0
  shapes.each do |s|
    big += 1 if s.big?
    names[s.name] += 1
    rects += 1 if s.is_a?(Rect) && !s.nil?
    s.w = s.w if s.is_a?(Rect)
  end
  [big, names.sort.to_h, rects]
end

three = [Rect.new(20, 10), Square.new(5), Circle.new(7)] * 4
six = three + [Tri.new(30, 10), Pent.new, Hex.new] * 3
50.times { survey(three) }
p survey(three)
50.times { survey(six) }
p survey(six), survey(three)
p six.first(6).map(&:label)

# redefining a method the hot sites reach (in the subclass only, then in the base)
class Square
  def area = 1000
end
p survey(three), survey(six)
class Shape
  def big? = :never
end
p survey(three).first, three.map(&:big?).uniq
class Shape
  def big? = area > 100
end
p survey(three)

# an attribute reader replaced by a method in one subclass
class Circle
  def r = 100
end
p survey(three)
class Circle
  attr_reader :r
end
p survey(three)

# a singleton method on one receiver
odd = Rect.new(1, 1)
list = three + [odd]
20.times { survey(list) }
def odd.area = 999
def odd.name = :odd
p survey(list)

# include / prepend after warmup
module Huge
  def area = 10_000
end
module Tiny
  def area = 0
end
20.times { survey(six) }
class Tri
  include Huge # Tri's own area still wins
end
p survey(six)
class Tri
  prepend Tiny
end
p survey(six)
class Pent
  prepend Tiny
end
p survey(six)

# method_missing for a class without the method
class Ghost
  def method_missing(m, *a)
    m == :area ? 7 : (m == :name ? :ghost : super)
  end
  def respond_to_missing?(m, p = false) = %i[area name].include?(m) || super
  define_method(:big?, proc { area > 100 })
end
mixed = three + [Ghost.new]
5.times { mixed.map(&:big?) }
p mixed.map(&:big?), mixed.map(&:name)

# visibility: a public method made private after the call sites are hot
class Visi
  attr_reader :v
  def initialize(v) = @v = v
  def peek = v
end
class Visi2 < Visi; end
class Visi3 < Visi; end
vs = [Visi.new(1), Visi2.new(2), Visi3.new(3)]
read = ->(xs) { xs.map { |x| x.v } }
peek = ->(xs) { xs.map { |x| x.peek } }
20.times { read.(vs); peek.(vs) }
p read.(vs), peek.(vs)
class Visi2
  private :v
end
p peek.(vs) # a receiver-less call still reaches the private reader
begin
  read.(vs)
rescue NoMethodError => e
  p e.message[/private method 'v'|private method `v'/] ? :private : e.message
end
class Visi2
  public :v
end
p read.(vs)

# protected
class Prot
  def initialize(x) = @x = x
  def same?(o) = o.x == x
  protected
  def x = @x
end
class Prot2 < Prot; end
ps = [Prot.new(1), Prot2.new(1), Prot.new(2)]
p ps.map { |a| ps.count { |b| a.same?(b) } }
begin
  ps[0].x
rescue NoMethodError => e
  p :protected
end

# backtraces from inside methods that are inlined into hot loops
class Calc
  def initialize(d) = @d = d
  def ratio(n) = n / @d
  def fetch_d = @d
end
class Calc2 < Calc; end
cs = [Calc.new(2), Calc2.new(5)]
30.times { cs.each { |c| c.ratio(10) } }
cs << Calc2.new(0)
begin
  cs.each { |c| c.ratio(10) }
rescue ZeroDivisionError => e
  p e.message
  p e.backtrace.map { |l| l[/\d+:in .*/] }.reject { |l| l =~ /Integer#|Array#/ }.first(3)
end
cs << Calc.new(nil)
begin
  cs.each { |c| c.ratio(10) if c.fetch_d }
rescue ZeroDivisionError => e
  p e.backtrace.map { |l| l[/\d+:in .*/] }.reject { |l| l =~ /Integer#|Array#/ }.first(3)
end

# is_a? in a loop: a module, a non-class argument, then is_a? redefined on one class
module Marked; end
class Square
  include Marked
end
p three.map { |s| [s.is_a?(Marked), s.is_a?(Shape), s.kind_of?(Circle)] }.uniq
begin
  three.each { |s| s.is_a?(:nope) }
rescue TypeError => e
  p e.class
end
class Circle
  def is_a?(k) = :custom
end
p three.map { |s| s.is_a?(Rect) }.uniq

# more classes than the guards hold
klasses = (1..9).map { |i| Class.new(Rect) { define_method(:tag) { i } } }
objs = klasses.map { |k| k.new(i = klasses.index(k) + 1, 2) }
3.times { objs.each { |o| o.big? } }
p objs.map { |o| [o.w, o.area, o.big?, o.tag] }

# a call in a loop to a method defined in two or three classes: each body inlined behind its
# own guard
class Item
  def initialize(n) = @n = n
  def cost = @n * 2
  def kind = :item
end
class Box < Item
  def cost = @n + 1000
end
class Bag < Item
  def cost = price + 1
  def price = @n
end
class Crate < Box; end
def total(xs)
  sum = 0
  xs.each { |x| sum += x.cost }
  sum
end
def kinds(xs) = xs.map { |x| x.kind }
items = [Item.new(1), Box.new(2), Bag.new(3), Crate.new(4)]
30.times { total(items) }
p total(items), kinds(items)
class Bag
  def price = 50
end
p total(items)
class Crate
  define_method(:cost, proc { -1 }) # (not a `def` the compiler sees)
end
p total(items)
special = Item.new(5)
def special.cost = 0.5
p total(items + [special])
module Doubled
  define_method(:cost, proc { super() * 2 })
end
class Box
  prepend Doubled
end
p total(items)
class Whatever
  def method_missing(m, *) = m == :cost ? 7 : super
  def respond_to_missing?(m, p = false) = m == :cost || super
end
p total(items + [Whatever.new])
class Item
  private :cost
end
begin
  total(items)
rescue NoMethodError => e
  p e.message[/private method/] ? :private : e.message
end
class Item
  public :cost
end
p total(items)
broken = Bag.new(nil)
class Bag
  remove_method :price
  def price = @n
end
begin
  total(items + [broken])
rescue NoMethodError => e
  p e.backtrace.map { |l| l[/\d+:in .*/] }.reject { |l| l =~ /Integer#|Array#/ }.first(3)
end
