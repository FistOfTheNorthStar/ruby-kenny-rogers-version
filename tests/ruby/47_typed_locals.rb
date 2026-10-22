# Locals whose type is fixed (always Integer / always Float): Floats live unboxed in
# registers, and must still behave exactly like Ruby's Floats.

# heap Floats keep their identity while a local holds them
a = 1e300
b = a
p a.equal?(a), a.equal?(b), [a, a].uniq.size, a.object_id == a.object_id
n = 0.0 / 0.0
h = { n => :nan }
p h[n], n.equal?(n), [n].include?(n), n == n, n != n, n < 1.0, n > 1.0
z = -0.0
p z, z.equal?(z), 1.0 / z, z == 0.0
t = 1e-300 * 1e-300
p t, t.equal?(t)
big = 1e200
big2 = big * big
p big2, big2.infinite?, (big2 - big2).nan?

# into the heap range and back inside a loop
x = 1.0
i = 0
while i < 20
  x = x * 1e10
  i += 1
end
p x
while i > 0
  x = x / 1e10
  i -= 1
end
p x

# Integer locals mixed with Floats, including Bignums
k = 2**70
f = 0.5
p k * f, f * k, k + f, f - k, k / f, f / k, k > f, f < k, k == f * 2 * k / k
m = 9007199254740993
g = 9007199254740992.0
p m > g, g < m, m == g, g == m, m != g
q = 3
r = 1.5
p q + r, q * r, q / r, q - r, r / q, q > r, q < r, r <= q, q == 3.0, r % 1
s = 7
p s / 2, s % 3, -s / 2, s * 0.5

# comparisons in conditions
lo = 0.0
hi = 10.0
c = 0
while lo < hi
  lo = lo + 0.75
  c += 1 if lo >= 5.0
end
p lo, c

# a local assigned before it is defined, or in a block, is not typed
def maybe(flag)
  v = 1.5 if flag
  v
end
p maybe(true), maybe(false)
w = 0.0
[1, 2, 3].each { |e| w += e * 0.5 }
p w
u = 1.0
u = u + 1 if u > 0.5
u = "str" if u > 100
p u

# Float methods on typed locals and values passed to methods
y = 2.25
p y.round, y.floor, y.to_s, y.class, y.frozen?, Math.sqrt(y), y.to_i, -y, y ** 2
def twice(v) = v * 2
p twice(y), [y, y * 2].max, "#{y}"

# multiple-assignment and op-assign
d1 = 1.0
d1 += 2
d1 *= 1.5
d1 -= 0.25
d1 /= 2
p d1

# exceptions inside typed code
e1 = 10
e2 = 0
begin
  p e1 / e2
rescue ZeroDivisionError => ex
  p ex.class
end
p e1 / 0.0, -e1 / 0.0, (0 / 0.0).nan?

# inlined methods with typed locals
class P
  def initialize(x) = @x = x
  def scaled
    k = 2.5
    k * @x
  end
end
pt = P.new(4)
acc = 0.0
j = 0
while j < 3
  acc += pt.scaled
  j += 1
end
p acc
