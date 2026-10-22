# Math functions: inline Math.sqrt (while it is the builtin), argument conversion, domain errors.
def s(x) = Math.sqrt(x)
def s2(x) = Math.sqrt(x * 1.0) + 1.0
vals = [4.0, 2, 0, -0.0, 0.0, 1e300, 1e-300, Float::INFINITY, Float::NAN, 2**70, 10**400, 3r, 1e308*10]
3.times { vals.each { |v| s(v) rescue nil } }
vals.each { |v| p [v, s(v), (s2(v) rescue $!.class)] }
[-1.0, -1, -Float::INFINITY, "x", nil].each { |v| begin; p s(v); rescue => e; p [e.class, e.message]; end }
p ::Math.sqrt(9), Math.sqrt(2.0).class
module Math
  def self.sqrt(x) = :mine
end
p s(4.0), s2(4.0) rescue p $!.class
p Math.log(-0.0), Math.sin(3r), (Math.sin(nil) rescue $!.message), Math.log(8, 2), (Math.log(-1, 2) rescue $!.message), (Math.log(2, -1) rescue $!.message), Math.log(0), (Math.atan2(1, nil) rescue $!.message), Math.hypot(3r, 4), Math.cbrt(-27)
%i[acosh atanh gamma log2 log10 sqrt].each { |m| begin; Math.send(m, -2); rescue => e; p [m, e.class, e.message]; end }

# the other Math functions, to_f, unary minus and Float#** (inline while builtin)
def m1(x) = [Math.sin(x), Math.cos(x), Math.tan(x), Math.atan(x), Math.exp(x), Math.cbrt(x), Math.sinh(x), Math.cosh(x), Math.tanh(x)]
def logs(x) = [Math.log(x), Math.log2(x), Math.log10(x)]
def trig_inv(x) = [Math.asin(x), Math.acos(x)]
[0.5, 2, -3, 0.0, -0.0, 1e-5, 100].each { |v| p m1(v) }
[1, 8.0, 0.5, 0, 0.0, -0.0, Float::INFINITY, 2**80].each { |v| p logs(v) }
[-1, 1.0, 0.25, -0.75, 0].each { |v| p trig_inv(v) }
[-2.0, 1.5, Float::NAN].each { |v| begin; p trig_inv(v); rescue => e; p [e.class, e.message]; end }
[-1, -0.5].each { |v| begin; p logs(v); rescue => e; p [e.class, e.message]; end }
p Math.log(8, 2), Math.log(2**100), Math.sin(3r), (Math.cos("1") rescue $!.class)

def tof(x) = x.to_f
def neg(x) = -x
def pw(x, y) = x ** y
p tof(3), tof(-7), tof(2.5), tof(2**70), tof(3r), tof("1.5e3"), tof(nil), tof(10**400)
p neg(3), neg(-4611686018427387904), neg(4611686018427387903), neg(2.5), neg(-0.0), neg(0), neg(2**70), neg(3r)
p pw(2.0, 10), pw(2.0, 0.5), pw(-8.0, 3), pw(-8.0, 1.0 / 3), pw(0.0, -1), pw(-0.0, -1), pw(2.0, -2), pw(Float::NAN, 0), pw(1.0, Float::NAN)
p pw(2, 10), pw(2, 0.5), pw(-8, 1.0 / 3), pw(2, -1), pw(10, 30), pw(1.5, 2**70)
x = 0.0
i = 0
while i < 100
  d = i.to_f
  x = x + (0.5 ** d) - Math.sin(d) * Math.cos(d) + -d / 1000.0
  i += 1
end
p x

# redefinitions after warm-up
class Integer
  alias_method :orig_to_f, :to_f
  def to_f = :int_to_f
end
p tof(3), tof(2.5)
class Integer
  alias_method :to_f, :orig_to_f
end
p tof(4)
class Float
  def **(o) = :pow
  def -@ = :neg
end
p pw(2.0, 3), neg(2.5), neg(3)
module Math
  def self.sin(x) = :my_sin
end
p m1(1).first, Math.cos(0)
