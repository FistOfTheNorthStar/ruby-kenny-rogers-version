# Branches and loops of every kind (measured by coverage_lines.rb).
module CovControl
  def self.grade(n)
    if n > 90
      :a
    elsif n > 75
      :b
    elsif n > 50 then :c
    else
      :f
    end
  end

  def self.sign(n)
    return :zero if n == 0
    n > 0 ? :pos : :neg
  end

  def self.check(x)
    unless x
      "no"
    else
      "yes"
    end
  end

  def self.kind(v)
    case v
    when Integer, Float
      "num"
    when /\A\d+\z/
      "digits"
    when String then "str"
    when *[:a, :b]
      "ab"
    else
      "other"
    end
  end

  def self.bare(n)
    case
    when n < 0 then -1
    when n.zero?
      0
    end
  end

  def self.shape(v)
    case v
    in [Integer => a, Integer => b]
      a + b
    in {name: String => nm} if nm.size > 3
      nm.upcase
    in {name:}
      name
    in nil
      nil
    else
      :unknown
    end
  end

  def self.loops(n)
    i = 0
    total = 0
    while i < n
      total += i
      i += 1
    end
    until i == 0
      i -= 1
    end
    begin
      i += 2
    end while i < 5
    for x in [1, 2, 3]
      total += x
    end
    total
  end

  def self.safe(o)
    o&.length
  end

  def self.logic(a, b)
    a && b || !a
  end

  def self.mods(x)
    y = 0
    y += 1 if x
    y += 2 unless x
    y -= 1 while y > 5
    y
  end

  def self.never
    1.times do
      :never
    end
  end
end

p CovControl.grade(95), CovControl.grade(80), CovControl.grade(60), CovControl.grade(10)
p CovControl.sign(0), CovControl.sign(3), CovControl.sign(-3)
p CovControl.check(nil), CovControl.check(1)
p CovControl.kind(1), CovControl.kind("12"), CovControl.kind("x"), CovControl.kind(:a), CovControl.kind([])
p CovControl.bare(-5), CovControl.bare(0), CovControl.bare(5)
p CovControl.shape([1, 2]), CovControl.shape({name: "kenny"}), CovControl.shape({name: "ab"}), CovControl.shape(nil), CovControl.shape(3)
p CovControl.loops(4)
p CovControl.safe("abc"), CovControl.safe(nil)
p CovControl.logic(true, false), CovControl.logic(false, true)
p CovControl.mods(true), CovControl.mods(false)
