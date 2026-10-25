class A
  def initialize(*a) = @a = a
  ruby2_keywords(:initialize)
  attr_reader :a
  def m(*a) = a
  def k(*a, **kw) = [a, kw]
end
class B < A
  def initialize(*arguments, **kwargs)
    super
  end
  def m(*args, **kw) = super
  def k(*args, **kw) = super
end
p B.new(1, :s).a, B.new(1, x: 2).a, B.new.a, B.new(1, {}).a, B.new(1, {"a" => 2}).a
p B.new.m(1), B.new.m(1, y: 3), B.new.k(1), B.new.k(1, z: 1)
class C < A
  def m(*args, **) = super
  def k(*, **) = super
end
p C.new.m(1), C.new.m(1, q: 1), C.new.k(2), C.new.k(2, r: 1)
class D < A
  def m(a, b = 2, **kw) = super
end
p D.new.m(1), D.new.m(1, 3, w: 1)
p Hash.ruby2_keywords_hash?(B.new(1, x: 2).a.last)
