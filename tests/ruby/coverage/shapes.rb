# Statements spread over lines, literals, blocks, methods of every kind (coverage_lines.rb).
class CovShape
  attr_reader :w, :h

  SIDES = {
    square: 4,
    triangle: 3,
  }.freeze

  NAMES = %w[
    a b
  ]

  def initialize(w, h = w * 2, scale: 1)
    @w = w * scale
    @h = h *
      scale
  end

  def area = w * h

  def to_s
    "#{self.class.name}(" \
      "#{w}x#{h})"
  end

  def report
    <<~TXT
      area: #{area}
      sides: #{SIDES.size}
    TXT
  end

  def each_side
    return enum_for(:each_side) unless block_given?
    [w, h, w, h].each do |s|
      yield s
    end
  end

  def risky(n)
    begin
      raise ArgumentError, "bad" if n < 0
      :ok
    rescue ArgumentError => e
      e.message
    ensure
      @checked = true
    end
  end

  def retrying
    tries = 0
    begin
      tries += 1
      raise "again" if tries < 3
      tries
    rescue
      retry
    end
  end

  def self.build(*args)
    new(*args)
  end

  class << self
    def unit
      new(1, 1)
    end
  end

  define_method(:perimeter) do
    2 * (w + h)
  end

  alias surface area

  def multi
    a, b =
      w, h
    x = a
    x = x
    [a, b, x]
  end

  def lazy_chain
    [3, 1, 2]
      .sort
      .map { |v| v * w }
      .select(&:even?)
  end

  def fallback
    Integer("zz") rescue -1
  end

  def unused_method
    :never
  end
end

s = CovShape.new(2, 3)
p s.area, s.surface, s.perimeter, s.to_s
puts s.report
p s.each_side.to_a
s.each_side { |v| v }
p s.risky(1), s.risky(-1)
p s.retrying
p CovShape.build(1).h, CovShape.unit.area
p s.multi, s.lazy_chain, s.fallback
p CovShape.new(1, scale: 3).w
lam = ->(x) {
  x * 2
}
p lam.(4)
pr = proc do |x|
  next 0 if x.nil?
  x + 1
end
p pr.call(nil), pr.call(1)
p [1, 2, 3].map { _1 * 10 }, [4].map { it + 1 }
