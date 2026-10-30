# Fixes found running Rails' own test suite (activesupport, activemodel, actionview,
# activerecord): checked against CRuby.
require "date"
require "bigdecimal/util"

# Proc#binding sees the locals where the block was written (assert_difference, atom_feed)
def helper(&block) = block.binding.local_variable_get(:xml)
def tmpl
  xml = "builder"
  helper { |feed| feed }
end
p tmpl
class Object
  module_eval <<~RUBY, "inline template", 1
    def tmpl2
      xml = "builder from eval"
      helper { |feed| feed }
    end
  RUBY
end
p tmpl2
xml = 1
p proc {}.binding.local_variables.include?(:xml)
p proc {}.binding.local_variable_get(:xml)

# Binding#source_location
b = binding
p [File.basename(b.source_location[0]), b.source_location[1]]

# Time#getlocal(timezone object), Time.at(obj with to_r), Rational#to_d
class Zone
  def utc_to_local(t) = t + 3600
  def local_to_utc(t) = t - 3600
  def inspect = "#<Zone>"
end
t = Time.utc(2000, 1, 1, 12)
l = t.getlocal(Zone.new)
p l.hour, l.utc_offset, l.zone, l, l.utc?, l.to_i == t.to_i
p t.getlocal(3600.0).utc_offset, t.getlocal(Rational(7200)).hour
begin
  t.getlocal(Object.new)
rescue TypeError => e
  p e.message
end
class Secs
  def to_r = 30r
  def to_int = 30
end
p Time.at(Secs.new).utc.sec
p 1.5r.to_d(5), Rational(1, 3).to_d(3)

# Date's private hour/min/sec (public on DateTime)
d = Date.new(2024, 5, 6)
%i[hour min minute sec second].each { |m| p [m, d.send(m), d.private_methods.include?(m)] }
p DateTime.new(2024, 5, 6, 7, 8, 9).then { [_1.hour, _1.min, _1.sec] }

# Range#cover?(range); === and include? with a Range compare it as one value
p (1..5).cover?(5..1), (1...5).cover?(1...5), (1..5).cover?(1...6), (1..5).cover?(..3)
p (1..).cover?(2..), (1...1).cover?(1...1), (1.0...5.0).cover?(1.0...5.0), (1..5).cover?(1...6.0)
p ("a".."z").cover?("b"..."d"), (1..5).cover?(3..), (..5).cover?(..3), (1..5).cover?(5..5)
p (1..5) === (2..3), (1..5).include?(2..3)

# Time#<=> and == with a non-Time: the inverse comparison
class Wrapped
  include Comparable
  def initialize(t) = @t = t
  def <=>(o) = @t <=> (o.is_a?(Wrapped) ? o.t : o)
  protected attr_reader :t
end
w = Wrapped.new(t)
p t <=> w, t == w, t < Wrapped.new(t + 1), t <=> Object.new

# Time.at(Rational), Time#+ / - Rational
p Time.at(Rational(3, 2)).utc.usec, (t + Rational(1, 2)).usec, (t - Rational(1, 4)).usec

# Numeric's generic methods for Numeric subclasses
class Scalar < Numeric
  def initialize(v) = @v = v
  def ==(o) = @v == o
  def <=>(o) = @v <=> o
  def coerce(o) = [Scalar.new(o), self]
  def -(o) = Scalar.new(@v - (o.is_a?(Scalar) ? o.v : o))
  def /(o) = @v.to_f / o
  def to_f = @v.to_f
  def to_s = "S(#{@v})"
  def inspect = to_s
  protected def v = @v
end
z = Scalar.new(0)
s5 = Scalar.new(5)
p z.zero?, z.nonzero?, s5.nonzero?, z.positive?, s5.positive?, s5.negative?
p s5.integer?, s5.finite?, s5.infinite?, +s5, s5.abs, s5.div(2), s5.fdiv(2), s5.floor, s5.round
p Scalar.new(-3).abs, -s5

# a bare module_function ends with its module body
module Kernel
  module_function
  def helper_a = :a
end
module Kernel
  def helper_b = :b
end
p Kernel.public_method_defined?(:helper_b), Object.new.helper_b

# Thread#status while a killed thread runs its ensure
q = Queue.new
th = Thread.new do
  q << :ready
  sleep
ensure
  $status = Thread.current.status
end
q.pop
sleep 0.05
th.kill
th.join
p $status

# coerce errors name the class; srand needs an Integer
class Pay; def inspect = "#<Pay>"; end
[-> { 1 + Pay.new }, -> { 1.5 * Pay.new }, -> { 1 + nil }, -> { 1 + :a }, -> { srand(nil) }, -> { srand("1") }].each do |f|
  f.call
rescue TypeError => e
  p e.message
end
p srand(1.5).class
