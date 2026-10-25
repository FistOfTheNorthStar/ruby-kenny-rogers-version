# Float elements read unboxed in speculated loops (kenny-compiler felem.rs) and Integer
# operations computed once (hoist.rs), with the cases that hand over to the generic loop.
def dot_rows(c, b, n, m)
  i = 0
  s = 0.0
  while i < n
    j = 0
    while j < m
      c[i * m + j] = c[i * m + j] + s * b[i * m + j]
      s = s + 0.5
      j += 1
    end
    i += 1
  end
  c
end
p dot_rows(Array.new(6, 0.0), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3)
p dot_rows(Array.new(6, 0.0), [1, 2, 3, 4, 5, 6], 2, 3)
# an element that isn't a Float partway through
c = Array.new(6, 0.0)
c[4] = 1r
p dot_rows(c, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3)
c = Array.new(6, 0.0)
c[2] = 7
p dot_rows(c, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3)
# out of range: nil + Float
begin
  p dot_rows(Array.new(4, 0.0), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3)
rescue NoMethodError => e
  puts "NoMethodError #{e.message[/undefined method '\+'/]}"
end
# a frozen Array, an Array subclass
begin
  dot_rows(Array.new(6, 0.0).freeze, [1.0] * 6, 2, 3)
rescue FrozenError => e
  puts "FrozenError"
end
class MyAry < Array
  def [](i) = super(i) * 2
end
p dot_rows(MyAry.new(6, 0.25), [1.0] * 6, 2, 3)
# negative indexes count from the end
def neg(a, n)
  s = 0.0
  k = 0
  while k < 2
    j = 1
    while j <= n
      s = s + a[k - j] * 1.5
      j += 1
    end
    k += 1
  end
  s
end
p neg([1.0, 2.0, 3.0, 4.0], 3)
# a hoisted product that overflows before the inner loop
def big(m)
  t = 0
  f = 0.0
  arr = Array.new(2, 1.25)
  i = 0
  while i < 4
    j = 0
    while j < 2
      t = i * m + j
      f = f + arr[j] * 2.0
      j += 1
    end
    i += 1
  end
  [t, f]
end
p big(3)
p big(2**61)
p big(2**62)
