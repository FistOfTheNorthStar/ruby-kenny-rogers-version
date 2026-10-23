# Speculative loops: locals guessed to be Floats / Fixnums run unboxed in a fast copy of
# the loop; a value of another type hands over to the generic copy mid-loop.

class Pt
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
end

# the guess holds
def dist_sum(pts)
  s = 0.0
  i = 0
  n = pts.size
  while i < n - 1
    dx = pts[i].x - pts[i + 1].x
    dy = pts[i].y - pts[i + 1].y
    s += Math.sqrt(dx * dx + dy * dy)
    i += 1
  end
  s
end
p dist_sum([Pt.new(0.0, 0.0), Pt.new(3.0, 4.0), Pt.new(6.0, 8.0)])

# Integers where Floats were guessed: hands over at the first one, same results
p dist_sum([Pt.new(0, 0), Pt.new(3, 4), Pt.new(6, 8)])
p dist_sum([Pt.new(0.0, 0.0), Pt.new(3, 4), Pt.new(6.0, 8.0), Pt.new(6, 8)])
# a Rational / a value with coerce midway
p dist_sum([Pt.new(0.0, 0.0), Pt.new(Rational(3), 4.0), Pt.new(6.0, 8.0)])

# a non-number midway raises; the rescue sees the locals as they were
def sum_until(vals)
  s = 0.0
  k = 0
  begin
    while k < vals.size
      v = vals[k] * 1.5
      s = s + v
      k += 1
    end
  rescue NoMethodError, TypeError => e
    return [:stopped, s, k, e.class]
  end
  [s, k]
end
p sum_until([1.0, 2.0, 3.0])
p sum_until([1.0, 2.0, nil, 4.0])
p sum_until([1.0, "x", 3.0])
p sum_until([1, 2, 3])

# a live Float local that turns into something else after the loop
def acc(vals)
  t = 0.0
  j = 0
  while j < vals.size
    t = t + vals[j]
    j += 1
  end
  t
end
p acc([0.5, 0.25]), acc([1, 2]), acc([0.5, 1, 2.5]), acc([]), acc([1e300, 1e300]), acc([0.5, 2**70])
p acc([0.5, Rational(1, 2)]), acc([0.5, Complex(1, 1)])

# Fixnum locals overflowing into Bignums
def fact_loop(n, x)
  f = 1
  i = 1
  r = 0.0
  while i <= n
    f = f * i
    r = r + x
    i += 1
  end
  [f, r]
end
p fact_loop(5, 0.5), fact_loop(25, 0.5), fact_loop(30, 1)

# break with a value, next, nested loops, until
def nested(n, scale)
  total = 0.0
  a = 0
  found = while a < n
    b = 0
    while b < n
      w = a * scale + b
      b += 1
      next if w < 1.0
      total += w
    end
    break [:big, total] if total > 100.0
    a += 1
  end
  [found, total, a]
end
p nested(3, 0.5), nested(10, 2.0), nested(4, 1)

def countdown(x)
  steps = 0
  until x < 1.0
    x = x / 2
    steps += 1
  end
  [x, steps]
end
p countdown(100.0), countdown(100), countdown(0.5)

# heap Floats (no flonum encoding) stay the same object while held
def heap_floats(seed)
  h = {}
  v = seed
  q = 0
  while q < 3
    v = v * 1.0
    h[v] = q
    q += 1
  end
  [h.size, v.equal?(v), v]
end
p heap_floats(1e300), heap_floats(0.0 / 0.0).first(2), heap_floats(-0.0)

# a speculated local read by a block after the loop and returned
def mixed(list)
  m = 0.0
  i = 0
  while i < list.size
    m = list[i] > m ? list[i] : m
    i += 1
  end
  [m, [1, 2].map { |e| e * 2 }]
end
p mixed([1.5, 3.25, 2.0]), mixed([1, 5, 2]), mixed([0.5, 7, 2.5])

# the loop's value and a local that only lives in it
def inner_only(n)
  i = 0
  r = while i < n
    z = i * 0.5
    i += 1
  end
  [r, i]
end
p inner_only(4)

# exceptions from inside the fast copy through ensure
def with_ensure(vals)
  s = 0.0
  i = 0
  log = []
  begin
    while i < vals.size
      s = s + vals[i]
      i += 1
    end
  ensure
    log << [s, i]
  end
  [s, log]
rescue => e
  [e.class, log]
end
p with_ensure([1.0, 2.0]), with_ensure([1.0, :x])

# an inlined method's own locals named like the loop's speculated ones
class Clash
  attr_reader :start, :f
  def initialize
    @start = 0
    @f = 0.5
  end
  def get(i)
    i + i
  end
  def fl(x)
    x * 2
  end
end
def clash_run(a)
  x = a.f
  i = a.start
  s = nil
  t = nil
  while i < 3
    x = x + 1.0
    s = a.get("ab")
    t = a.fl("cd")
    i = i + 1
  end
  [x, s, t, i]
end
p clash_run(Clash.new)

# Integer locals in speculated loops: overflow into Bignum mid-loop, a Float showing up,
# a Bignum on entry
def grow(start, steps)
  x = start
  i = 0
  while i < steps
    x = x * 3
    i += 1
  end
  x
end
p grow(1, 50), grow(-7, 45)

def add_up(start, step, n)
  s = start
  i = 0
  while i < n
    s += step
    i = i + 1
  end
  s
end
p add_up(4611686018427387900, 1, 10), add_up(-4611686018427387900, -1, 10), add_up(0, 3, 1000)

def count_down(from)
  k = from
  n = 0
  while k > 0
    k = k - 1
    n -= 2
  end
  [k, n]
end
p count_down(100), count_down(2**64 + 3 - 2**64)

def mixed(vals)
  t = 0
  i = 0
  while i < vals.size
    t = t + vals[i]
    i += 1
  end
  t
end
p mixed([1, 2, 3]), mixed([1, 2.5, 3]), mixed([2**70, 1]), mixed([1, Rational(1, 2)])

def big_entry(n)
  i = n
  c = 0
  while i < n + 5
    i = i + 1
    c = c + i
  end
  c
end
p big_entry(2**80), big_entry(10)
