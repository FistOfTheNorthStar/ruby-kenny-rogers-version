module Shapes
  def self.area(kind, size)
    case kind
    when :square
      size * size
    when :circle
      (3.14159 * size * size).round(2)
    else
      raise ArgumentError, "unknown #{kind}"
    end
  end

  def self.unused
    :never_called
  end

  def self.describe(n)
    n > 1 ? "big" : "small"
  end
end
