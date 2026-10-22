# Calls with literal keywords to small methods taking exactly those required keywords are
# inlined; they must still behave as calls.
def add(left:, right:) = left + right
def scale(x, by:) = x * by
t = 0
i = 0
while i < 10
  t += add(left: i, right: 1) + add(right: 2, left: i) + scale(i, by: 3)
  i += 1
end
p t

# evaluation order, missing/unknown keywords, wrong positional count
order = []
p add(right: (order << :r; 1), left: (order << :l; 2)), order
p (add(left: 1) rescue [$!.class, $!.message])
p (add(left: 1, right: 2, extra: 3) rescue [$!.class, $!.message])
h = {left: 5, right: 6}
p add(**h)

# errors inside keep the frame
def bad_kw(a:) = a + nil
def call_bad = bad_kw(a: 1)
p (call_bad rescue [$!.class, $!.backtrace.map { |l| l[/:(\d+):/, 1].to_i }.uniq.first(2)])

# methods on objects, redefinition, overriding, visibility
class Calc
  def initialize(k) = @k = k
  def mul(x:, y:) = x * y * @k
  def run(n) = mul(x: n, y: 2)
  def hidden(v:) = v
  private :hidden
  def peek = hidden(v: :ok)
end
c = Calc.new(10)
def use_calc(c, n) = c.mul(x: n, y: 3)
3.times { use_calc(c, 1); c.run(1) }
p use_calc(c, 2), c.run(4), c.peek
def call_hidden(c) = c.hidden(v: 1)
p (call_hidden(c) rescue $!.class)
Calc.define_method(:mul) { |x:, y:| [:redefined, x, y] }
p use_calc(c, 2), c.run(4)
class Calc2 < Calc
  def mul(x:, y:) = :sub
end
p use_calc(Calc2.new(1), 1)
p (use_calc(nil, 1) rescue $!.class)
Calc.class_eval "def mul(x:, y: 7) = [:opt, x, y]"
p use_calc(c, 1), c.run(5)
Calc.define_method(:mul) { |**kw| kw }
p use_calc(c, 1)
Object.define_method(:add) { |left:, right:| left - right }
p add(left: 10, right: 1)
