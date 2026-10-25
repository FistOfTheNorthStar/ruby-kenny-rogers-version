# Loops speculated on locals used with Arrays of Floats (spec.rs `float_arrays`), including
# values that turn out not to be Floats.
def mm(a, b, n)
  c = Array.new(n * n, 0.0)
  i = 0
  while i < n
    k = 0
    while k < n
      aik = a[i * n + k]
      j = 0
      while j < n
        c[i * n + j] = c[i * n + j] + aik * b[k * n + j]
        j += 1
      end
      k += 1
    end
    i += 1
  end
  c
end
p mm([1.0, 2.0, 3.0, 4.0], [0.5, 0.25, 2.0, 1.0], 2)
p mm([1, 2, 3, 4], [5, 6, 7, 8], 2)
p mm([1.5, 2, 3, 4.25], [5, 6.5, 7, 8], 2)
p mm([1r, 2, 3, 4], [5, 6, 7, 8], 2)
p mm([2**70, 1, 1, 1], [1, 1, 1, 1], 2)
