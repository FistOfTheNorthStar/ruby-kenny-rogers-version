class Calc
  def add(a, b = 1, *rest, k:, opt: 2, **kw, &blk) = a + b
  def double(x) = x * 2
end
c = Calc.new
m = c.method(:double)
p m.call(21), m.arity, m.owner, m.name, m.parameters, [1, 2].map(&m)
p Calc.instance_method(:add).parameters, Calc.instance_method(:add).arity
um = Calc.instance_method(:double)
p um.bind(c).call(5), um.bind_call(c, 6), um.inspect
p Kernel.instance_method(:to_s).bind_call(c)[0, 7], 1.method(:+).arity, [].method(:push).arity
def top(x, y) = x - y
p method(:top).to_proc.call(10, 3), method(:puts).owner
