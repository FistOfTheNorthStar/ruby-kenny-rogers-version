# frozen_string_literal: true
# Kenny: conversions to BigDecimal (bigdecimal/util from the bigdecimal gem 3.1).
require "bigdecimal"

class Integer < Numeric
  def to_d
    BigDecimal(self)
  end
end

class Float < Numeric
  def to_d(precision = 0)
    BigDecimal(self, precision)
  end
end

class String
  def to_d
    BigDecimal.interpret_loosely(self)
  end
end

class BigDecimal < Numeric
  def to_digits
    if nan? || infinite? || zero?
      to_s
    else
      i = to_i.to_s
      _, f, _, z = frac.split
      i + "." + ("0" * -z) + f
    end
  end

  def to_d
    self
  end
end

class Rational < Numeric
  def to_d(precision)
    BigDecimal(self, precision)
  end
end

class Complex < Numeric
  def to_d(*args)
    BigDecimal(self) unless imag.zero? # (raises)
    if args.empty? && real.is_a?(Rational)
      BigDecimal(real) # (raises: a precision is needed)
    end
    real.to_d(*args)
  end
end

class NilClass
  def to_d
    BigDecimal(0)
  end
end
