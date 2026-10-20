def target(a, b = 2, *rest, k: 0, **opts, &blk)
  [a, b, rest, k, opts, blk&.call]
end
def fwd(...) = target(...)
def anon_rest(*) = target(*)
def anon_kw(**) = target(1, **)
def anon_blk(&) = target(1, &)
def mixed(x, ...) = target(x, ...)
p fwd(1), fwd(1, 2, 3, k: 4, z: 5) { :blk }
p anon_rest(7, 8, 9), anon_kw(k: 3, y: 1), anon_blk { :b }
p mixed(:x, :y, k: 1) { :m }
class Base
  def initialize(*args, **kw) = (@args, @kw = args, kw)
  attr_reader :args, :kw
end
class Child < Base
  def initialize(...) = super(...)
end
c = Child.new(1, 2, a: 3)
p c.args, c.kw
