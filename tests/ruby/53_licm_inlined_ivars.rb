# Instance variables of an inlined method's receiver, remembered across a loop's
# iterations (licm.rs): every way the variable can change inside the loop must be seen.

class Pt
  attr_accessor :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
  def len2
    @x * @x + @y * @y
  end
  def sum_with(o)
    @x + o.x
  end
  def set_x(v)
    @x = v
  end
  def bump
    @y += 1
  end
  def poke
    instance_variable_set(:@x, @x + 100)
  end
  def each_part
    yield @x
    yield @y
  end
end

# changed by an inlined writer on the same object
p1 = Pt.new(1, 2)
out = []
i = 0
while i < 4
  out << p1.len2
  p1.set_x(i + 3)
  i += 1
end
p out

# changed through an alias, by attr_writer
p2 = Pt.new(2, 0)
alias2 = p2
out = []
i = 0
while i < 4
  out << p2.len2
  alias2.x = alias2.x + 1
  i += 1
end
p out

# changed by a method that writes another ivar, then one that uses instance_variable_set
p3 = Pt.new(1, 1)
out = []
i = 0
while i < 3
  out << p3.len2
  p3.bump
  p3.poke
  i += 1
end
p out

# the receiver's method redefined mid-loop
class Q
  def initialize
    @a = 2
  end
  def get
    @a * 10
  end
end
q = Q.new
out = []
i = 0
while i < 4
  out << q.get
  if i == 1
    Q.class_eval { def get = @a * 100 }
  end
  i += 1
end
p out

# a block that writes the ivar between two reads of the inlined body
p4 = Pt.new(5, 6)
out = []
i = 0
while i < 3
  p4.each_part { |v| out << v; p4.y = v + 1 }
  i += 1
end
p out

# two receivers, one written
a = Pt.new(1, 1)
b = Pt.new(10, 10)
out = []
i = 0
while i < 3
  out << a.sum_with(b) << b.sum_with(a)
  b.x += 5
  i += 1
end
p out

# the receiver local is reassigned in the loop: not remembered
c = Pt.new(1, 0)
out = []
i = 0
while i < 3
  out << c.len2
  c = Pt.new(c.x + 1, 0)
  i += 1
end
p out

# an ivar that isn't set yet, then set
class Lazy
  def val
    @v
  end
  def fill
    @v = 7
  end
end
l = Lazy.new
out = []
i = 0
while i < 3
  out << l.val
  l.fill if i == 1
  i += 1
end
p out

# in a method, with a speculated loop counter
def run(pt, n)
  t = 0
  i = 0
  while i < n
    t += pt.len2
    pt.x = 0 if i == 2
    i += 1
  end
  t
end
p run(Pt.new(3, 4), 5)

# a different object of another class in the same local each call
class Other
  def initialize
    @x = 9
    @y = 9
  end
  def len2
    @x - @y
  end
end
def total(o)
  t = 0
  i = 0
  while i < 3
    t += o.len2
    i += 1
  end
  t
end
p total(Pt.new(1, 2)), total(Other.new), total(Pt.new(0, 3))
