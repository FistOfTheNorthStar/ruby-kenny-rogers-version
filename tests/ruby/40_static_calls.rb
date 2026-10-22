# Receiver-less calls to methods defined in this file go straight to the method (guarded by
# the receiver's class and the method serial); methods with only required parameters take
# them in registers. Everything below must behave as a normal call would.

def add3(a, b, c) = a + b + c
def twice(x) = add3(x, x, 0)
p twice(4), add3(1, 2, 3)

# redefinition after the sites are warm
def greet(n) = "hi #{n}"
def call_greet(n) = greet(n)
3.times { |i| call_greet(i) }
p call_greet(1)
def greet(n) = "hello #{n}"
p call_greet(2)

# a subclass overriding a method its parent's method calls
class Base
  def run(x) = step(x) * 10
  def step(x) = x + 1
  def outer(x) = run(x)
end
class Sub < Base
  def step(x) = x + 100
end
b = Base.new
s = Sub.new
3.times { [b.outer(1), s.outer(1)] }
p [b.outer(1), s.outer(1), b.run(2), s.run(2)]

# singleton method on one object
o = Base.new
3.times { o.run(1) }
def o.step(x) = -x
p o.run(1), Base.new.run(1)

# super from a method with a fast entry (implicit and explicit arguments)
class Sup
  def f(a, b) = [:sup, a, b]
end
class Kid < Sup
  def f(a, b)
    a = a * 2
    [super, super(a, 0)]
  end
end
p Kid.new.f(1, 2)

# blocks: &blk, block_given?, yield, return from a block
def with_blk(x, &blk) = blk ? blk.call(x) : :none
def yielder(x) = block_given? ? yield(x) : :noblock
def finder(list)
  list.each { |v| return v if v > 2 }
  nil
end
def use_blocks(x) = [with_blk(x), yielder(x), finder([1, 5, 3])]
p use_blocks(1), with_blk(2) { |v| v * 3 }, yielder(3) { |v| v + 1 }

# destructuring parameters
def pair((a, b), c) = [a, b, c]
def call_pair = pair([1, 2], 3)
p call_pair, pair([4], 5)

# arity errors (through the method's normal entry), send, method objects
def two(a, b) = a + b
begin
  two(1)
rescue ArgumentError => e
  p e.message, e.backtrace.first[/\d+/].to_i
end
begin
  send(:two, 1, 2, 3)
rescue ArgumentError => e
  p e.message
end
p method(:two).arity, method(:two).parameters, method(:two).call(2, 3), send(:two, 4, 5)
alias two_again two
p two_again(1, 1)
class Base
  define_method(:dm_step, instance_method(:step))
  alias_method :step2, :step
end
p Base.new.dm_step(1), Sub.new.step2(1)

# exceptions and deep recursion
def boom(x) = x > 2 ? raise(ArgumentError, "too big #{x}") : boom(x + 1)
begin
  boom(0)
rescue ArgumentError => e
  p e.message
end
def depth(n) = n == 0 ? 0 : 1 + depth(n - 1)
p depth(10_000)
begin
  depth(100_000_000)
rescue SystemStackError => e
  p e.class
end
p depth(100)

# eval redefining a method; method_missing; private methods called without a receiver
def ev(x) = x
def call_ev(x) = ev(x)
3.times { call_ev(1) }
eval("def ev(x) = x * 1000")
p call_ev(2)
class MM
  def method_missing(n, *a) = [:mm, n, a]
  def respond_to_missing?(*) = true
  def go = nothing_here(1)
end
p MM.new.go
class Priv
  def pub(x) = secret(x)
  private def secret(x) = x * 7
end
p Priv.new.pub(3)

# methods defined in modules, included and extended
module Helper
  def help(x) = x * 2
  def use_help(x) = help(x) + 1
end
class UsesHelper
  include Helper
  def help(x) = x * 3
end
class PlainHelper
  include Helper
end
p UsesHelper.new.use_help(1), PlainHelper.new.use_help(1)
obj = Object.new.extend(Helper)
p obj.use_help(5)

# threads: calls still correct while several OS threads run
def tsum(n) = n == 0 ? 0 : n + tsum(n - 1)
p 4.times.map { Thread.new { tsum(1000) } }.map(&:value)

# top-level methods called from class bodies and blocks, many arguments
def six(a, b, c, d, e, f) = [a, b, c, d, e, f].sum
def seven(a, b, c, d, e, f, g) = [a, b, c, d, e, f, g].sum
class UsesTop
  def go = six(1, 2, 3, 4, 5, 6) + seven(1, 2, 3, 4, 5, 6, 7)
end
p UsesTop.new.go, [1, 2].map { |x| six(x, x, x, x, x, x) }
