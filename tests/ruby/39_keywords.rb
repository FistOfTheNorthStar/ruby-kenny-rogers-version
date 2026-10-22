# keyword parameters read from the caller's hash without copying it

def add(left:, right: 10) = left + right
p add(left: 1), add(left: 1, right: 2), add(right: 3, left: 4)
def err
  yield
rescue ArgumentError => e
  e.message
end
p err { add(right: 1) }
p err { add(left: 1, bogus: 2) }
p err { add(left: 1, x: 1, y: 2) }
p err { add(left: 1, "str" => 2) }
p err { add }
h = { left: 5, right: 6 }
p add(**h), h
h2 = { left: 1 }
p add(**h2, right: 9), h2

# defaults that use earlier keywords and positionals; the hash isn't changed
def f(a, b = 2, c:, d: c * 2, e: a + b) = [a, b, c, d, e]
opts = { c: 3 }
p f(1, c: 3), f(1, 5, c: 3, e: 0), f(1, **opts), opts

# with **rest (consumes a copy)
def g(a:, **rest) = [a, rest]
o = { a: 1, b: 2, c: 3 }
p g(**o), o

# blocks and procs with keywords
blk = proc { |x, k: 1, j: 2| [x, k, j] }
p blk.call(0), blk.call(0, k: 5), blk.call(0, j: 7, k: 8)
l = ->(k:, m: :dm) { [k, m] }
p l.call(k: 1), l.(k: 2, m: 3)
p err { l.call(k: 1, z: 2) }
p [1, 2].map { |v| add(left: v, right: v) }

# keywords through send, method objects, super, define_method
p send(:add, left: 2), method(:add).call(left: 3, right: 3)
class KA
  def m(x:, y: 0) = x + y
end
class KB < KA
  def m(x:, y: 5) = super
end
class KC < KA
  def m(x:, **kw) = super(x: x * 10, **kw)
end
p KB.new.m(x: 1), KC.new.m(x: 1, y: 2)
KA.define_method(:dm) { |p:, q: 4| [p, q] }
p KA.new.dm(p: 1), err { KA.new.dm(q: 1) }

# many calls (the benchmark's pattern)
t = 0
100_000.times { |i| t += add(left: i, right: 1) }
p t

# hash literals with Symbol keys: distinct, repeated, many (indexed table), quoted
p({ a: 1, b: 2 }, { "a": 1, b: 2, c: 3 }.keys)
h = eval("{ a: 1, a: 2 }") rescue nil
p h
big = { k1: 1, k2: 2, k3: 3, k4: 4, k5: 5, k6: 6, k7: 7, k8: 8, k9: 9, k10: 10, k11: 11 }
p big[:k11], big.size, big.key?(:k1), big[:nope]
def many(k1:, k2:, k3:, k4:, k5:, k6:, k7:, k8:, k9:, k10: 0, k11: 0) = k1 + k9 + k10 + k11
p many(**big), many(**big, k11: 100)
s = { a: 1 }
s[:b] = 2
s.delete(:a)
p s, s[:a], s[:b]
