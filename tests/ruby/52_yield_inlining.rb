# Small methods that yield, called with a literal block, are inlined with the block's body
# at each yield: it must behave like the call.

def foo
  yield
end

def bar
  foo { return 1 }
  :not_reached
end

def twice(x)
  yield x
  yield x + 1
end

def wrap(v)
  [:before, yield(v), :after]
end

def pair
  yield 1, 2
end

def one_array
  yield [3, 4]
end

def nothing_yielded
  yield
end

total = 0
i = 0
while i < 1000
  total = total + bar
  i += 1
end
p total

acc = []
twice(10) { |v| acc << v * 2 }
p acc

p wrap(5) { |v| v * 3 }

# next: the yield's value; break: the call's
p wrap(1) { |v| next 42 if v == 1; 0 }
r = wrap(1) { |v| break :broke }
p r

# the block sees and changes the caller's locals; its own locals are fresh each time
count = 0
twice(1) { |v| t ||= 0; t += v; count += t }
p count

# block parameters: several values, autosplat of one Array, missing ones nil
p(pair { |a, b| [a, b] })
p(one_array { |a, b| [a, b] })
p(one_array { |a| a })
p(nothing_yielded { |a, b| [a, b] })

# return from a block inside a block's method frame (a closure): the method returns
def via_each
  [1, 2, 3].each do |x|
    foo { return x * 100 if x == 2 }
  end
  :none
end
p via_each

# an exception in the block, rescued around the call
def safe
  foo { raise "boom" }
rescue => e
  "rescued #{e.message}"
end
p safe

# backtraces show the block's line, then the method's
def bt
  foo { raise "deep" }
rescue => e
  e.backtrace.first(1).map { |l| l[/:(\d+):/, 1] }
end
p bt

# self inside the block is the caller's, inside the method the receiver's
class Box
  def initialize(v)
    @v = v
  end
  def open
    yield @v
  end
  def who = :box
end
class User
  def who = :user
  def run
    Box.new(7).open { |v| [v, who, self.class] }
  end
end
p User.new.run

# a subclass overriding the method: the call reaches it
class Box2 < Box
  def open
    yield :overridden
  end
end
p [Box.new(1), Box2.new(2)].map { |b| b.open { |v| v } }

# a yielding method called without a block raises as usual
begin
  foo
rescue LocalJumpError => e
  puts e.message
end

# a block with return at the top level stays a closure
p(foo { 5 })

# a method's frame record is made when its first closure is: returns from closures made
# on some paths only, in loops, and after the method has returned
def find_first(list, want)
  return :empty if list.empty?
  list.each_with_index.each { |x, i| return i if x == want }
  :missing
end
p find_first([], 1), find_first([5, 6, 7], 7), find_first([1], 9)

def maybe_closure(flag)
  if flag
    pr = proc { return :from_proc }
    pr.call
  end
  :no_closure
end
p maybe_closure(true), maybe_closure(false)

def many_closures
  procs = []
  3.times.each { |i| procs << proc { return i * 10 } }
  procs[1].call
  :unreached
end
p many_closures

def lambda_inside
  l = lambda { return :from_lambda }
  [l.call, :after]
end
p lambda_inside

def ensure_then_closure
  begin
    [1, 2].each { |x| return x * 7 if x == 2 }
  ensure
    puts "ensure ran"
  end
end
p ensure_then_closure
