class PS
  def initialize(a); @a = a; end
  def to_ary; @a; end
  def ==(o); o == @a; end
end
p Array(PS.new([1, 2])), [0] + PS.new([3]), [1, 2] - PS.new([2]), [1] | PS.new([2]), [1, 2] & PS.new([2]), [1].concat(PS.new([5])), [1, 2] == PS.new([1, 2])
begin
  [1] + 5
rescue TypeError => e
  p e.message
end
p Array(1..3), Array(nil), Array("a"), Array({a: 1}), Array(:s)
S = Struct.new(:a)
p Array(S.new(1))
e = LoadError.new("x")
p e.path
begin
  require "definitely/not/here"
rescue LoadError => e
  p e.path
end
class Kls
  @@a = nil unless defined?(@@a)
  p defined?(@@a), defined?(@@b)
end
p "a//b///c".squeeze("/"), /[\u{41}-\u{43}]+/.match("xABCy")[0]
