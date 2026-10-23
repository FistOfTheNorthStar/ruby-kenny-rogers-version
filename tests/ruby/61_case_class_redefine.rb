# case/when on classes, then Class#=== is redefined
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
class Class
  def ===(o) = o.is_a?(Symbol) || o.is_a?(self)
end
p xs.map { |x| kind(x) }
class Class
  remove_method :===
end
p xs.map { |x| kind(x) }
