# Mandelbrot set escape-time counts (tight Float loops, Integer/Float mixing).
size = 600
iter = 50
limit = 4.0
count = 0
checksum = 0
y = 0
while y < size
  ci = (2.0 * y / size) - 1.0
  x = 0
  while x < size
    zr = 0.0
    zi = 0.0
    cr = (2.0 * x / size) - 1.5
    i = 0
    while i < iter
      tr = zr * zr - zi * zi + cr
      ti = 2.0 * zr * zi + ci
      zr = tr
      zi = ti
      break if zr * zr + zi * zi > limit
      i += 1
    end
    count += 1 if i == iter
    checksum += i
    x += 1
  end
  y += 1
end
puts count, checksum
