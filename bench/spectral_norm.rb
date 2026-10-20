# Spectral norm (Benchmarks Game): Float arrays, blocks, Integer#times, Array#[] / []=.
def eval_a(i, j)
  1.0 / ((i + j) * (i + j + 1) / 2 + i + 1)
end

def eval_a_times_u(u)
  n = u.size
  v = Array.new(n, 0.0)
  n.times do |i|
    sum = 0.0
    n.times { |j| sum += eval_a(i, j) * u[j] }
    v[i] = sum
  end
  v
end

def eval_at_times_u(u)
  n = u.size
  v = Array.new(n, 0.0)
  n.times do |i|
    sum = 0.0
    n.times { |j| sum += eval_a(j, i) * u[j] }
    v[i] = sum
  end
  v
end

def eval_ata_times_u(u)
  eval_at_times_u(eval_a_times_u(u))
end

n = 600
u = [1.0] * n
v = nil
10.times do
  v = eval_ata_times_u(u)
  u = eval_ata_times_u(v)
end
vbv = 0.0
vv = 0.0
n.times do |i|
  vbv += u[i] * v[i]
  vv += v[i] * v[i]
end
puts "%0.9f" % Math.sqrt(vbv / vv)
