# a[i] += k / a[i] -= k: one guard for the read and the write on plain Arrays; everything
# else (other classes, frozen, out of range, overflow, Floats) behaves as the two calls.
a = [1, 2, 3]
k = 5
i = 0
while i < 3
  a[i] += 1
  a[i] -= k
  a[-1] += 10
  i += 1
end
p a
b = [4611686018427387903, -4611686018427387904]
b[0] += 1
b[1] -= 1
p b
c = [1.5, "s", nil, [1]]
c[0] += 1
c[1] += "t"
p c
p(begin; c[2] += 1; rescue => e; [e.class, e.message]; end)
p(begin; c[3] += 1; rescue => e; e.class; end)
d = [1, 2].freeze
p(begin; d[0] += 1; rescue => e; e.class; end)
e = [1, 2]
p(begin; e[5] += 1; rescue => e; [e.class, e.message]; end)
p(begin; e[-3] += 1; rescue => e; [e.class, e.message]; end)
h = {x: 1}
h[:x] += 2
p h
class Tracker
  def initialize = @log = []
  def [](i)
    @log << [:get, i]
    $x = 100
    7
  end
  def []=(i, v)
    @log << [:set, i, v]
  end
  attr_reader :log
end
t = Tracker.new
x = 1
t[0] += x
p t.log
$x = 1
x = 3
set = -> { x = 50 }
class Tracker2 < Tracker
  def [](i) = ($set.call; 1)
end
$set = set
t2 = Tracker2.new
t2[0] += x
p t2.log
class SubAry < Array; end
s = SubAry[1, 2]
s[0] += 1
p s
class Array
  alias old_aref []
  def [](i) = i == 99 ? :custom : old_aref(i)
end
f = [1]
f[0] += 1
p f
p(begin; f[99] += 1; rescue => e; e.class; end)
class Array
  alias [] old_aref
end
g = [10]
n = 0
while n < 5
  g[0] += 1
  n += 1
end
p g
