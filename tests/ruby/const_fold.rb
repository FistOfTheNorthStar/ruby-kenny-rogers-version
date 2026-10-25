# Folded top-level constants (kenny-compiler cfold.rs): read before assignment, changed
# by const_set / remove_const, shadowed in a class.
def early = (A rescue $!.class)
p early
A = 40
B = A * 2
C = 0.5
D = -3
p early, A, B, C, D
def calc(x) = x * B + A
p calc(2)
class K
  A = 7
  def a = A
  def b = B
end
p K.new.a, K.new.b
p [1, 2].map { |i| i * A }
Object.send(:remove_const, :B)
p (calc(2) rescue $!.class)
Object.const_set(:B, 1000)
p calc(2)
$VERBOSE = nil
p D
Object.const_set(:D, 4)
p D
def loop_read
  s = 0
  i = 0
  while i < 3
    s += A + C
    i += 1
  end
  s
end
p loop_read
E = 5
E2 = E + 1
p E2
