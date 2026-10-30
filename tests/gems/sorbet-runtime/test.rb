require "bundler/setup"
require "sorbet-runtime"

class Calc
  extend T::Sig
  sig { params(a: Integer, b: Integer).returns(Integer) }
  def add(a, b) = a + b

  sig { params(x: T.nilable(String), opts: T::Hash[Symbol, Integer]).returns(String) }
  def show(x, opts = {}) = "#{x.inspect} #{opts}"

  sig { params(xs: T::Array[T.any(Integer, Float)]).returns(Float) }
  def avg(xs) = xs.sum.to_f / xs.size

  sig { returns(Integer) }
  def bad = "oops"

  sig { params(blk: T.proc.params(x: Integer).returns(Integer)).returns(T::Array[Integer]) }
  def map3(&blk) = [1, 2, 3].map(&blk)

  sig { params(name: String, age: Integer).void }
  def kw(name:, age: 0) = nil
end

c = Calc.new
p c.add(1, 2), c.show(nil), c.show("s", { a: 1 }), c.avg([1, 2.5]), c.map3 { _1 * 2 }, c.kw(name: "x")
def try
  yield
rescue => e
  puts "#{e.class}: #{e.message.lines.first.strip}"
end
try { c.add(1, "2") }
try { c.bad }
try { c.show(5) }
try { c.kw(name: :sym) }

class Point < T::Struct
  const :x, Integer
  const :y, Integer, default: 0
  prop :label, T.nilable(String)
end
pt = Point.new(x: 1, label: "a")
p pt.x, pt.y, pt.label, pt.serialize
pt.label = "b"
p pt.label, Point.from_hash({ "x" => 5, "y" => 6 }).y
try { Point.new(x: "no") }
try { Point.new }
try { pt.label = 3 }

class Suit < T::Enum
  enums do
    Spades = new
    Hearts = new("h")
  end
end
p Suit::Spades.serialize, Suit::Hearts.serialize, Suit.deserialize("h") == Suit::Hearts, Suit.values.size
try { Suit.deserialize("zz") }

p T.let(3, Integer), T.cast("x", String), T.must(5)
try { T.let("3", Integer) }
try { T.must(nil) }
p T::Array[Integer].valid?([1, 2]), T::Array[Integer].recursively_valid?([1, "a"])
p T.nilable(Integer).name, T::Hash[String, T::Array[Symbol]].name, T.any(Integer, String, NilClass).name
p T::Utils.coerce(Integer).valid?(3), T::Boolean.valid?(false), T.class_of(Integer).valid?(Integer)

module Shape
  extend T::Sig
  extend T::Helpers
  interface!
  sig { abstract.returns(Float) }
  def area; end
end
class Sq
  extend T::Sig
  include Shape
  sig { override.returns(Float) }
  def area = 4.0
end
p Sq.new.area
try { Shape.new }
T::Configuration.call_validation_error_handler = ->(sig, opts) { puts "handled: #{opts[:pretty_message].lines.first.strip}" }
p c.bad, c.add(1, 2.5)
