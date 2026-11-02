# A Method or UnboundMethod object of a `define_method` method keeps its block alive after
# the method is removed or redefined: the collector freed the block, and calling the Method
# ran whatever reused its slot (rspec-mocks keeps the original Method of what it stubs, and
# Mastodon's specs failed with wrong arities or crashed in multi-file runs).
class Calc
  define_method(:add) { |a, b| a + b }
  define_method(:greet) { |name, punct: "!"| "hi #{name}#{punct}" }
  define_singleton_method(:twice) { |x| x * 2 }
end

def grab
  c = Calc.new
  [c.method(:add), Calc.instance_method(:greet), Calc.method(:twice), c.method(:add).unbind]
end

def churn
  # other blocks, to take the freed blocks' slots
  ps = Array.new(20_000) { |i| proc { |x, y, z| [x, y, z, i] } }
  ps.size
end

add, greet, twice, unbound_add = grab
class Calc
  remove_method :add
  define_method(:greet) { "replaced" }
end
class << Calc
  remove_method :twice
end
3.times do
  GC.start
  churn
end
GC.start

p add.call(1, 2)
p add.arity, add.parameters
p greet.bind(Calc.new).call("bob")
p greet.bind_call(Calc.new, "amy", punct: "?")
p greet.parameters
p twice.call(21)
p twice.to_proc.call(5)
p unbound_add.bind_call(Calc.new, 3, 4)
p Calc.new.greet
