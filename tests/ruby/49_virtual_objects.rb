# Objects that never leave a method are not allocated (kenny-compiler vobj.rs); everything
# must behave as if they were.
class Pt
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
end

class Box
  def initialize(w)
    @w = w
    @h = w * 2
  end
  def w; @w; end
  def h; @h; end
end

def sum_pts(n)
  s = 0
  i = 0
  while i < n
    pt = Pt.new(i, i + 1)
    s += pt.x * pt.y
    i += 1
  end
  s
end
puts sum_pts(1000)

def boxes(n)
  t = 0
  i = 0
  while i < n
    b = Box.new(i)
    t += b.w + b.h
    i += 1
  end
  t
end
puts boxes(100)

# the previous iteration's object
def prev_iter
  out = []
  q = Pt.new(-1, 0) if false
  i = 0
  while i < 4
    out << q.x if i > 0
    q = Pt.new(i * 10, 0)
    i += 1
  end
  out
end
p prev_iter

# a getter redefined while the loop runs: later reads call the new method
def redefined(n)
  out = []
  i = 0
  while i < n
    pt = Pt.new(i, 0)
    Pt.class_eval { def x; @x * 100; end } if i == 2
    out << pt.x
    i += 1
  end
  out
end
p redefined(4)

# ... to one that returns the object itself: it is built for real, with its ivars in order
class Pt
  def x; self; end
end
def escapes
  pt = Pt.new(7, 8)
  o = pt.x
  [o.class, o.instance_variables, o.instance_variable_get(:@y), o.equal?(pt.x)]
end
p escapes

# initialize that does more than assign ivars: an ordinary object
class Logged
  attr_reader :v
  def initialize(v)
    @v = v
    note(v)
  end
  def note(v)
    $notes << v
  end
end
$notes = []
def logged
  s = 0
  3.times do |i|
    l = Logged.new(i)
    s += l.v
  end
  t = 0
  i = 0
  while i < 3
    l2 = Logged.new(i + 10)
    t += l2.v
    i += 1
  end
  [s, t]
end
p logged
p $notes

# a getter that doesn't exist: NoMethodError as usual
class Bare
  attr_reader :a
  def initialize(a)
    @a = a
  end
end
def missing
  b = Bare.new(1)
  b.a + b.zzz
rescue NoMethodError => e
  e.message[/undefined method [`']zzz'/]
end
p missing

# arguments with side effects run once, in order
def sidefx
  log = []
  pt = Pt.new(log.push(1).size, log.push(2).size)
  [pt.y, log]
end
p sidefx

# a private getter: not virtual, the call raises
class Priv
  def initialize(a)
    @a = a
  end
  private
  attr_reader :a
end
def priv
  pr = Priv.new(3)
  pr.a
rescue NoMethodError => e
  e.message[/private method [`']a'/]
end
p priv
