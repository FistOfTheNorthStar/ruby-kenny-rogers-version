# Method#super_method (prepend, include, inheritance)
module Pre; def to_s = "pre(#{super})"; end
class E1 < StandardError; end
E1.prepend Pre
e = E1.new("m")
m = e.method(:to_s)
p m.owner, m.super_method&.owner, m.super_method&.call
class A; def f = 1; end
class B < A; def f = 2; end
p B.new.method(:f).super_method.owner, B.new.method(:f).super_method.call
module Inc; def f = 3; end
class C < A; include Inc; def f = 4; end
p C.new.method(:f).super_method.owner, C.new.method(:f).super_method.super_method.owner
p A.new.method(:f).super_method
p e.method(:message).super_method
