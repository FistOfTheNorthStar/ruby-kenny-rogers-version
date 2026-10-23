# A method calling itself goes straight to its own code while it is the only method of
# that name (kenny-compiler statics.rs `self_call`): every way that can stop being true.

def fib(n)
  n < 2 ? n : fib(n - 1) + fib(n - 2)
end
p fib(20)

# redefined while running: the inner calls reach the new method
def countdown(n)
  if n == 3
    def countdown(n) = [:new, n]
  end
  n == 0 ? [:old, 0] : countdown(n - 1)
end
p countdown(5)
p countdown(2)

# a subclass overrides it: calls from the parent's body reach the override
class Walker
  def walk(n)
    n == 0 ? [] : [self.class.name, *walk(n - 1)]
  end
end
p Walker.new.walk(2)
class LoudWalker < Walker
  def walk(n)
    n == 0 ? [:loud] : super
  end
end
p LoudWalker.new.walk(3)
p Walker.new.walk(2)

# another class defines the name later
class Tree
  def depth(n)
    n == 0 ? 0 : 1 + depth(n - 1)
  end
end
p Tree.new.depth(5)
class Unrelated
  def depth(n) = :other
end
p Tree.new.depth(5)

# the method's singleton override
t = Tree.new
def t.depth(n) = -1
p t.depth(5)
p Tree.new.depth(3)

# removed / undefined in a subclass while running
class Node
  def size(n, tick = nil)
    tick&.call
    n == 0 ? 0 : 1 + size(n - 1)
  end
end
class SubNode < Node; end
s = SubNode.new
begin
  s.size(4, -> { SubNode.send(:undef_method, :size) })
rescue NoMethodError => e
  p [:undef, e.class]
end
class Node2
  def sz(n, tick = nil)
    tick&.call
    n == 0 ? 0 : 1 + sz(n - 1)
  end
end
p Node2.new.sz(3)

# an alias of the old body calls the new method
class Fact
  def fact(n) = n <= 1 ? 1 : n * fact(n - 1)
  alias old_fact fact
  def fact(n) = n == 2 ? 100 : n * fact(n - 1)
end
p Fact.new.old_fact(4)

# in a module (its methods can be bound to any object)
module Rec
  def down(n) = n == 0 ? :bottom : down(n - 1)
end
class UsesRec
  include Rec
end
p UsesRec.new.down(4)
p Rec.instance_method(:down).bind_call(Object.new, 0)
begin
  Rec.instance_method(:down).bind_call(Object.new, 2)
rescue NoMethodError => e
  p [:bound, e.class]
end

# method_missing receives the call once the method is gone
class Ghost
  def haunt(n)
    self.class.send(:remove_method, :haunt) if n == 2
    n == 0 ? :done : haunt(n - 1)
  end
  def method_missing(name, *args) = [:missing, name, *args]
end
p Ghost.new.haunt(3)

# deep recursion: SystemStackError, and the method still works after
def deep(n) = n == 0 ? 0 : 1 + deep(n - 1)
def forever(n) = 1 + forever(n + 1)
begin
  forever(0)
rescue SystemStackError => e
  p e.class
end
p deep(100)

# in threads
th = 3.times.map { |i| Thread.new { fib(15 + i) } }
p th.map(&:value)
p fib(18)
class Tree
  def depth2(n) = n == 0 ? 0 : 1 + depth2(n - 1)
end
p Thread.new { Tree.new.depth2(50) }.value
