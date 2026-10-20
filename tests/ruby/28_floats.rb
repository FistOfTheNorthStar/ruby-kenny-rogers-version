# Floats: flonum/heap representations, inline arithmetic and comparisons, unboxed locals.

# literals of every representation
vals = [1.5, 0.1, -2.25, 0.0, -0.0, 1e300, -1e300, 1e-300, 2.5e-310, 1e77, 1e-77, 3.0e76, 4.9e-324,
        Float::MAX, Float::MIN, Float::EPSILON, Float::INFINITY, -Float::INFINITY, 123456789.123456789]
vals.each { |v| puts "#{v.inspect} #{v.to_s} #{v.class} #{v.frozen?} #{v.finite?} #{v.hash == (v + 0.0).hash rescue 'x'}" }
nan = 0.0 / 0.0
p nan, nan.nan?, nan == nan, nan.equal?(nan), [nan].include?(nan), nan.to_s

# arithmetic: Float op Float, Float op Integer, Integer op Float, into heap range and back
a = 1.5
b = 2
p a + b, b + a, a - b, b - a, a * b, b * a, a / b, b / a, 7 / 2, 7 / 2.0, 7.0 / 2, 7.fdiv(2)
p 1e200 * 1e200, -1e200 * 1e200, 1e-200 * 1e-200, 1e300 / 1e-10, 1e300 * 10 / 10
p 0.0 * -1, -0.0 + 0.0, -0.0 - 0.0, 0.0 / 1, -1.0 / 0, 1.0 / 0, 0 / 1.0, 1 / -0.0
p 1.0 / 3 * 3 == 1.0, 0.1 + 0.2, 0.1 + 0.2 == 0.3, (0.1 + 0.2).round(15) == 0.3
p 5.5 % 2, -5.5 % 2, 5.5 % -2, 5 % 2.5, 2**0.5, 4 ** 0.5, 2.0 ** 10, 10.0.divmod(3), -7.5.divmod(2)
x = 3.7
p -x, x.abs, (-x).abs, x.floor, x.ceil, x.round, x.truncate, x.to_i, -x.to_i, x.round(1), 2.5.round, 3.5.round, -2.5.round

# big integers mixed with floats
big = 2**70
p big + 1.5, 1.5 + big, big * 0.5, big / 2.0, big.to_f, big == big.to_f, big.to_f == big, big < big.to_f + 1e10
p (2**53 + 1) == 2.0**53, (2**53 + 1) > 2.0**53, 2.0**53 < 2**53 + 1, 2**53 == 2.0**53, 9007199254740993 < 9007199254740992.0
p 2**62 - 1 + 0.0, (2**62) * 1.0, 4611686018427387903 < 4.611686018427388e18, 4611686018427387903.to_f

# comparisons, including NaN and Infinity, mixed with Integer
p 1.5 < 2, 2 > 1.5, 1.5 <= 1.5, 1.5 >= 1.6, 1 == 1.0, 1.0 == 1, 1 != 1.0, 1.5 != 1.5
p nan < 1, nan > 1, nan == 1, 1 < nan, nan != nan, Float::INFINITY > 10**100, -Float::INFINITY < -(10**100)
p 1.5 <=> 2, 2 <=> 1.5, 1.0 <=> 1, nan <=> 1, 1.5 <=> "x", [3.5, 1, 2.25, -0.0, 0].sort, [3.5, 1, 2.25].max, [0.5, 1].min

# identity and hashing: 1 == 1.0 but !1.eql?(1.0)
p 1.eql?(1.0), 1.0.eql?(1.0), 0.0.eql?(-0.0), 0.0 == -0.0, 1.5.equal?(1.5), (0.5 + 1.0).equal?(1.5), -0.0.equal?(-0.0)
h = { 1 => :int, 1.0 => :float, 0.0 => :zero, 1e300 => :big, nan => :nan }
p h[1], h[1.0], h[2 / 2.0], h[-0.0], h[1e300], h[1e150 * 1e150], h.size, h[nan]
p [1, 1.0, 1.0, 2.5, 2.5].uniq, [1.5, 1.5].uniq.size, { 1.5 => 1 }.key?(3.0 / 2), [0.1 * 3, 0.3].uniq.size

# floats are frozen and can't get singleton methods or ivars
f = 2.5
p f.frozen?, 1e300.frozen?, (1e300 * 10).frozen?, f.dup.equal?(f), f.clone.equal?(f)
begin; def f.foo; end; rescue TypeError => e; p e.class, e.message; end
begin; f.singleton_class; rescue TypeError => e; p e.message; end
begin; f.instance_variable_set(:@a, 1); rescue => e; p e.class; end
begin; f.extend(Comparable); rescue TypeError => e; p e.message; end
begin; 1e300.define_singleton_method(:x) {}; rescue TypeError => e; p e.message; end
p f.class, f.is_a?(Numeric), f.is_a?(Comparable), Float === f, f.respond_to?(:+), f.method(:+).arity

# Marshal and JSON round trips
data = [1.5, 1.5, -0.0, 0.0, 1e300, 2.5e-310, 0.1, 100.0, -3.25, Float::INFINITY]
back = Marshal.load(Marshal.dump(data))
p back, back == data, back.map { |v| v.to_s } == data.map(&:to_s), Marshal.dump([1.5, 1.5]).bytes.size
p Marshal.load(Marshal.dump(nan)).nan?, Marshal.dump(2.5) == Marshal.dump(5 / 2.0)
require "json"
p JSON.generate([1.5, 0.1 + 0.2, -0.0, 100.0]), JSON.parse("[1.5, -0.0, 2e3, 1E-5]"), 2.5.to_json

# formatting
p "%.3f|%5.1f|%e|%g|%d" % [3.14159, 2.25, 12345.678, 0.0001, 3.99], 1e16.to_s, 1e15.to_s, 1.0e-5.to_s, 0.0001.to_s
p 12.0.to_s, 1.23456789012345e+15, 100.0.inspect, -0.0.to_s, 1.5.to_r, 0.75.rationalize, Rational(3, 4).to_f, Complex(1.5, 2).abs

# unboxed locals in loops (values that leave the flonum range on the way)
x = 1.0
acc = 0.0
i = 0
while i < 400
  x = x * 3.0
  acc += x / 1e10
  i += 1
end
p x, acc, x.class
y = 1e-70
40.times { y = y / 1e10 }
p y, y * 1e300 * 1e100
z = 0.0
z -= 0.0
p z, -z, 1 / z
s = 0.0
1.upto(10) { |k| s += 1.0 / k }
p s
m = 0.5
m = m * 2 + 1 - 0.5 / 2
p m
q = 10
q /= 4
r = 10
r /= 4.0
p q, r
t = 3
t = t * 1.5 if t > 2
p t
u = nil
u = (u || 0) + 2.5
p u
# mandelbrot-style escape loop
count = 0
-2.0.step(0.5, 0.25) do |cr|
  -1.0.step(1.0, 0.25) do |ci|
    zr = 0.0
    zi = 0.0
    n = 0
    while n < 50
      tr = zr * zr - zi * zi + cr
      zi = 2.0 * zr * zi + ci
      zr = tr
      break if zr * zr + zi * zi > 4.0
      n += 1
    end
    count += n
  end
end
p count
def poly(x)
  v = x * x - 2 * x + 1.0
  w = v / 3
  w + v
end
p poly(3), poly(2.5), poly(1e200), (poly(1e200) rescue :err)
def mixed(a, b)
  c = a + b
  d = c * c
  [c, d, c < d]
end
p mixed(1, 2), mixed(1.5, 2), mixed(2**62, 1)
begin; mixed("a", "b"); rescue TypeError => e; p e.message; end
p 1.5.object_id == (-1.5).object_id, [1.5, -1.5, 2.5].map(&:object_id).uniq.size, 1.5.object_id == (3.0 / 2).object_id
[[1.0, 0], [1, 0.0], [1.0, 0.0]].each { |x, y| begin; x.divmod(y); rescue => e; p e.class; end; begin; x.div(y); rescue => e; p e.class; end }
p 7.5.divmod(2), 7.5.div(2), "ab" * 2.5, ("ab" * nil rescue $!.class), 1.upto(Float::INFINITY) { |i| break i if i > 3 }
