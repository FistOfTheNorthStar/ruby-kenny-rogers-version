# Integer copies of small recursive methods (kenny-compiler ispec.rs): results that leave
# the Fixnums, division by zero, Float arguments, redefinition and overriding, threads.
def fact(n)
  n <= 1 ? 1 : n * fact(n - 1)
end
p fact(10)
p fact(30)
p fact(10)

def dv(a, b)
  return a if b == 0 && a > 100
  a / b + dv(a - 1, b) * 0 if a > 0
  a % b
end
p dv(7, 3)
p dv(-7, 3)
p dv(7, -3)
begin
  p dv(5, 0)
rescue ZeroDivisionError => e
  p e.class
end
p dv(7, 3)

def depth(n)
  n == 0 ? 0 : 1 + depth(n - 1)
end
p depth(10000)
p depth(100)

def fib(n)
  if n < 2
    n
  else
    fib(n - 1) + fib(n - 2)
  end
end
p fib(20)
p fib(20.0)

class A
  def g(n)
    n < 1 ? 0 : 1 + g(n - 1)
  end
end
class B < A
end
p B.new.g(5)
class B
  def g(n)
    n < 1 ? 100 : 2 + super(n - 1)
  end
end
p B.new.g(5)
p A.new.g(5)

def h(n)
  n < 1 ? 0 : 1 + h(n - 1)
end
p h(5)
def h(n)
  n < 1 ? 7 : 10 + h(n - 1)
end
p h(5)

def u(n)
  unless n > 3 || !(n != 2)
    return n * 3
  end
  if n > 100
    return -1
  elsif n > 50
    return -2
  end
  (n - 1) + u(n - 2) - (n % 4)
end
p u(9), u(60), u(200), u(2)

def big(n)
  n + 4611686018427387903
end
def bigr(n) = n > 0 ? bigr(n - 1) + 4611686018427387903 : 0
p bigr(3)
def neg(n) = n < -5 ? n : neg(n - 1) - 1
p neg(0)
p (-4611686018427387904 / -1 rescue $!)
def mn(a, b) = b == 0 ? a / -1 : mn(a, b - 1)
p mn(-4611686018427387904, 2)
p mn(-7, 2)
ts = 4.times.map { |i| Thread.new { fib(22 + i) } }
p ts.map(&:value)
p fib(24)
def cnt(n, acc) = n == 0 ? acc : cnt(n - 1, acc + 1)
p cnt(1000, 0)
def ack(m, n)
  if m == 0
    n + 1
  elsif n == 0
    ack(m - 1, 1)
  else
    ack(m - 1, ack(m, n - 1))
  end
end
p ack(2, 3), ack(3, 4)
