# define_method with a literal block that could be a def is compiled as one:
# it must keep define_method's meaning.

# top level: a public method of Object (a def would be private)
define_method(:zero) { 0 }
define_method(:one) { |a| a }
define_method(:add) { |a, b| a + b }
p zero, one(7), add(2, 3)
p 5.zero, "x".one(:y)
p Object.public_method_defined?(:zero), Object.private_method_defined?(:zero)
p method(:add).arity, method(:add).parameters, method(:one).owner

total = 0
i = 0
while i < 1000
  zero
  total = total + one(3) + add(i, 1)
  i += 1
end
p total

# the call's value is the name
r = define_method(:two) { 2 }
p r, two

# __method__, return, a nested block with return
define_method(:meth) { __method__ }
p meth
define_method(:early) do |x|
  return :neg if x < 0
  [1, 2, 3].each { |y| return y * 10 if y == x }
  :none
end
p early(-1), early(2), early(9)

# argument count errors name the block, like CRuby
begin
  one
rescue ArgumentError => e
  puts e.message
end

class Shape
  attr_reader :w, :h
  def initialize(w, h)
    @w = w
    @h = h
  end
  define_method(:area) { @w * @h }
  define_method(:scaled) { |k| Shape.new(@w * k, @h * k) }
  private
  define_method(:secret) { :hidden }
  public
  define_method(:reveal) { secret }
end
s = Shape.new(2, 3)
p s.area, s.scaled(2).area, s.reveal
begin
  s.secret
rescue NoMethodError => e
  puts e.message
end
sum = 0
k = 0
while k < 100
  sum += s.area
  k += 1
end
p sum

# method_added sees it
class Watched
  def self.method_added(m)
    puts "added #{m}"
  end
  define_method(:w1) { 1 }
end
p Watched.new.w1

# a block that uses an outer local stays a block
class Closure
  factor = 4
  define_method(:times_factor) { |x| x * factor }
end
p Closure.new.times_factor(5)

# a class with its own define_method gets the call
class Custom
  def self.define_method(name, &blk)
    puts "custom define_method #{name}"
    super(name) { |*a| [:wrapped, blk.call(*a)] }
  end
  define_method(:hello) { |x| x + 1 }
end
p Custom.new.hello(1)

# redefined later by a def, and by another define_method
define_method(:later) { :first }
p later
def later = :second
p later
define_method(:later) { :third }
p later

# a module, then included
module Greeter
  define_method(:greet) { |name| "hi #{name} from #{self.class}" }
end
class Person
  include Greeter
end
p Person.new.greet("bob")

# yield and super make it a block
class Base
  def go = :base
end
class Derived < Base
  define_method(:go) { [super(), :derived] }
end
p Derived.new.go
