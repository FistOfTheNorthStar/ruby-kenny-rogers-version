class Vec
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
  def add(o)
    Vec.new(@x + o.x, @y + o.y)
  end
  def dot(o)
    @x * o.x + @y * o.y
  end
end
acc = Vec.new(0, 0)
d = 0
3_000_000.times do |i|
  v = Vec.new(i, i + 1)
  acc = acc.add(v)
  d += acc.dot(v) % 1000
end
puts acc.x, acc.y, d
