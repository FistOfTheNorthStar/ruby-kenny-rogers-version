# is_a? made private, or undefined in a subclass, after a site saw many classes
class A; end; class B; end; class C; end; class D; end; class E; end
def check(x) = x.is_a?(A)
xs = [A.new, B.new, C.new, D.new, E.new]
p xs.map { |x| check(x) }
class D
  private :is_a?
end
begin
  check(D.new)
rescue NoMethodError
  puts "NoMethodError private"
end
class E
  undef_method :kind_of?
end
p (xs - [xs[3]]).map { |x| check(x) }
begin
  E.new.kind_of?(A)
rescue NoMethodError
  puts "NoMethodError undef"
end
module Kernel
  private :is_a?
end
begin
  A.new.is_a?(A)
rescue NoMethodError
  puts "NoMethodError kernel"
end
