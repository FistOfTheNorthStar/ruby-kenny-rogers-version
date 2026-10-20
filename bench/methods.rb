# Method-call heavy OO code: small methods, attr readers/writers, polymorphism, ==, nil?, !.
class Shape
  attr_reader :name
  def initialize(name)
    @name = name
  end
  def big?
    area > 100
  end
end

class Rect < Shape
  attr_accessor :w, :h
  def initialize(w, h)
    super(:rect)
    @w = w
    @h = h
  end
  def area
    w * h
  end
end

class Square < Rect
  def initialize(s)
    super(s, s)
  end
end

class Circle < Shape
  attr_reader :r
  def initialize(r)
    super(:circle)
    @r = r
  end
  def area
    3 * r * r
  end
end

class Counter
  attr_reader :count
  def initialize
    @count = 0
  end
  def incr(by)
    @count += by
    self
  end
  def reset?
    @count.nil?
  end
end

shapes = []
100.times do |i|
  shapes << Rect.new(i % 13 + 1, i % 7 + 1)
  shapes << Square.new(i % 11 + 1)
  shapes << Circle.new(i % 9 + 1)
end

c = Counter.new
big = 0
rects = 0
20_000.times do
  shapes.each do |s|
    big += 1 if s.big?
    rects += 1 if s.name == :rect
    c.incr(1) unless c.reset?
    if s.is_a?(Rect) && !s.nil?
      s.w = s.w
    end
  end
end
puts big, rects, c.count
