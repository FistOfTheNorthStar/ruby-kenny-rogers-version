# Attribute reads that a loop doesn't change are read once (kenny-compiler licm.rs); every
# way of changing them must still be seen.
class Mat
  attr_reader :data, :cols, :rows
  attr_accessor :scale
  def initialize(rows, cols)
    @rows = rows
    @cols = cols
    @data = Array.new(rows * cols) { |k| k % 5 }
    @scale = 1
  end
  def grow!
    @cols += 1
  end
  def swap_data(d)
    @data = d
  end
end

def total(m)
  s = 0
  i = 0
  while i < m.rows
    j = 0
    while j < m.cols
      s += m.data[i * m.cols + j] * m.scale
      j += 1
    end
    i += 1
  end
  s
end
p total(Mat.new(3, 4))

# an attr_writer in the loop
def writer(m)
  out = []
  i = 0
  while i < 4
    out << m.scale
    m.scale = m.scale + 10
    i += 1
  end
  out
end
p writer(Mat.new(1, 1))

# a method that writes the instance variable
def by_method(m)
  out = []
  i = 0
  while i < 3
    out << m.cols
    m.grow!
    i += 1
  end
  out
end
p by_method(Mat.new(1, 1))

# instance_variable_set, and an inlined block that writes
def by_ivar_set(m)
  out = []
  i = 0
  while i < 3
    out << m.rows
    m.instance_variable_set(:@rows, m.rows * 2)
    2.times { m.scale = m.scale + 1 }
    out << m.scale
    i += 1
  end
  out
end
p by_ivar_set(Mat.new(1, 1))

# the getter redefined, or a singleton method, while the loop runs
def redefined(m)
  out = []
  i = 0
  while i < 4
    out << m.cols
    if i == 1
      Mat.class_eval { def cols; @cols * 100; end }
    end
    if i == 2
      def m.cols; -1; end
    end
    i += 1
  end
  out
end
p redefined(Mat.new(1, 7))
class Mat
  attr_reader :cols
end

# a getter that isn't an attr_reader runs every time
class Counter
  def initialize
    @n = 0
  end
  def tick
    @n += 1
  end
end
def ticks(c)
  out = []
  i = 0
  while i < 3
    out << c.tick
    i += 1
  end
  out
end
p ticks(Counter.new)

# the receiver reassigned in the loop
def reassigned
  m = Mat.new(1, 1)
  out = []
  i = 0
  while i < 3
    out << m.cols
    m = Mat.new(1, i + 5)
    i += 1
  end
  out
end
p reassigned

# swapped data seen by the next read
def swapped(m)
  out = []
  i = 0
  while i < 3
    out << m.data[0]
    m.swap_data([i + 100])
    i += 1
  end
  out
end
p swapped(Mat.new(1, 1))

# another thread changes it: the loop must notice
class Flag
  attr_accessor :v
  def initialize
    @v = 0
  end
end
def setter_thread(obj)
  Thread.new(obj) { |o| sleep 0.01; o.v = 1 }
end
def spin(f)
  t = setter_thread(f)
  spins = 0
  spins += 1 while f.v == 0
  t.join
  f.v
end
p spin(Flag.new)

# self's instance variables
class Acc
  attr_accessor :step
  def initialize
    @step = 1
    @total = 0
  end
  def bump!
    @step += 1
  end
  def run
    out = []
    i = 0
    while i < 3
      out << @step
      bump!
      i += 1
    end
    j = 0
    while j < 2
      out << @step
      self.step = @step * 10
      j += 1
    end
    k = 0
    while k < 2
      out << @step
      instance_variable_set(:@step, -k)
      k += 1
    end
    n = 0
    while n < 2
      out << @step
      2.times { @step += 100 }
      n += 1
    end
    out
  end
end
p Acc.new.run

module Conf
  @level = 1
  def self.raise_level
    @level += 1
  end
  def self.sum
    s = 0
    i = 0
    while i < 4
      s += @level
      raise_level if i.odd?
      i += 1
    end
    s
  end
end
p Conf.sum
p Conf.instance_variable_get(:@level)

# Array checks remembered across iterations
def frozen_mid_loop(a)
  out = []
  i = 0
  while i < 3
    begin
      a[0] = i
      out << a[0]
      a.freeze if i == 1
    rescue FrozenError => e
      out << e.class
    end
    i += 1
  end
  out
end
p frozen_mid_loop([0, 0])

class Array
  alias_method :orig_aref, :[]
end
def redefined_aref(a)
  out = []
  i = 0
  while i < 3
    out << a[0]
    if i == 1
      Array.class_eval { def [](i); orig_aref(i) * 10; end }
    end
    i += 1
  end
  Array.class_eval { def [](*args); orig_aref(*args); end }
  out
end
p redefined_aref([7])

def not_an_array(s)
  out = []
  i = 0
  while i < 3
    out << s[i]
    i += 1
  end
  out
end
p not_an_array("abc")
p not_an_array({0 => :x, 2 => :z})

class Holder
  attr_accessor :items
  def initialize
    @items = [1, 2, 3]
  end
  def sum_swapping
    s = 0
    i = 0
    while i < 3
      s += @items[i]
      @items = [10, 20, 30] if i == 0
      i += 1
    end
    s
  end
end
def via_reader(h)
  s = 0
  i = 0
  while i < 3
    s += h.items[i]
    h.items = [100, 200, 300] if i == 0
    i += 1
  end
  s
end
p via_reader(Holder.new)
p Holder.new.sum_swapping
