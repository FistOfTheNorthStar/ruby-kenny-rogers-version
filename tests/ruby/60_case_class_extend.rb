# case/when on classes, then a class is extended with a module that has ===
class A; end; class B < A; end; class C; end; module M; end; class D; include M; end
def kind(x)
  case x
  when B then :b
  when A then :a
  when M then :m
  when C then :c
  when Integer then :int
  when String then :str
  else :other
  end
end
xs = [A.new, B.new, C.new, D.new, 1, "s", nil, :x, 1.5]
p xs.map { |x| kind(x) }
module Loose
  def ===(o) = o.nil?
end
A.extend(Loose)
p xs.map { |x| kind(x) }
