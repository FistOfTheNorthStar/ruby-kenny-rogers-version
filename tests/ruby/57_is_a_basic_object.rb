# a BasicObject subclass has no is_a?: method_missing (or NoMethodError) answers
class A; end; class B; end; class C; end; class D; end; class E; end
def check(x) = x.is_a?(A)
xs = [A.new, B.new, C.new, D.new, E.new]
p xs.map { |x| check(x) }
class Bare < BasicObject
  def method_missing(n, *a) = [:mm, n]
end
class Plain < BasicObject; end
p check(Bare.new)
begin
  check(Plain.new)
rescue NoMethodError => e
  puts "NoMethodError"
end
p xs.map { |x| check(x) }
