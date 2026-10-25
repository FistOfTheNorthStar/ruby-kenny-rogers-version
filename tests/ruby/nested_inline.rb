# Blocks inlined with blocks inside them (inline.rs `Check`, scopes.rs `inline_envs`), the
# inlined `sum`, and calls with 3+ arguments.
# nested blocks in inlined loops
OFF = [-1, 0, 1]
p OFF.sum { |a| OFF.count { |b| a + b == 0 } }
p [1, 2, 3].map { |x| [10, 20].map { |y| x * y } }
p (1..3).each { |i| [4, 5].each { |j| print i * j, " " } }
puts
# inner block closing over outer block params and method locals
def m(base)
  r = []
  3.times { |i| r << [1, 2].map { |j| base + i * j } }
  r
end
p m(100)
# fallback: inner call's receiver isn't a builtin
class Weird
  def each
    yield 7
    yield 8
  end
  def map
    [yield(1), yield(2)]
  end
end
w = Weird.new
p [1, 2].map { |x| w.map { |y| x * 10 + y } }
saved = []
class Keeper
  def initialize(s) = @s = s
  def map(&b) = (@s << b; [b.call(1)])
end
k = Keeper.new(saved)
p [5, 6].map { |x| k.map { |y| x + y } }
p saved.map { |b| b.call(100) }
# break / next inside nested
p [1, 2, 3].map { |x| [4, 5, 6].each { |y| break y * x if y == 5 } }
p [1, 2, 3].map { |x| [4, 5, 6].map { |y| next 0 if y == 5; y + x } }
# outer var assigned: not inlined with nested, must still be right
p [1, 2].map { |x| t = x * 2; [1].map { |y| t + y } }
p [1, 2].map { |x| t = 0; [1, 2].each { |y| t += y * x }; t }
# sum semantics
p [1, 2, 3].sum { |x| x }
p [1, 2.5, 3].sum { |x| x }
p [0.1, 0.2, 0.3].sum { |x| x }
p [4611686018427387903, 1, 2].sum { |x| x }
p [1, 2].sum { |x| x.to_r }
p [].sum { |x| x }
p (begin; [1, "a"].sum { |x| x }; rescue TypeError => e; e.class; end)
p [3, 4].sum { |x| break 99 if x == 4; x }
p (1..4).sum { |x| x * 1.5 }
p [1, 2].sum { |x| x + 4611686018427387903 }
# 3-arg inlined calls
def add3(a, b, c) = a + b * c
p add3(1, 2, 3)
def kw3(a, b, c) = [a, b, c]
p kw3(*[1, 2], 3)
class P; def f(a, b, c) = a - b - c; end
p P.new.f(10, 2, 3)
