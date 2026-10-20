# Array and Hash heavy: indexing, <<, size, each/map with blocks, Hash#[] / []=, String keys.
keys = (0...1000).map { |i| "key#{i}" }
syms = %i[a b c d e f g h]
h = {}
keys.each_with_index { |k, i| h[k] = i }
sh = {}
syms.each_with_index { |s, i| sh[s] = i }

total = 0
2000.times do
  keys.each do |k|
    total += h[k]
  end
  arr = []
  1000.times { |i| arr << i * 2 }
  i = 0
  n = arr.size
  while i < n
    arr[i] = arr[i] + sh[syms[i & 7]]
    i += 1
  end
  total += arr.map { |x| x + 1 }.size
  total += arr[500]
  counts = Hash.new(0)
  arr.each { |x| counts[x & 15] += 1 }
  total += counts[3]
end
grid = Array.new(100) { Array.new(100, 0) }
2000.times do |t|
  y = 0
  while y < 100
    row = grid[y]
    x = 0
    while x < 100
      row[x] = (row[x] + x * y + t) & 1023
      x += 1
    end
    y += 1
  end
end
puts total, grid[50][50], grid.map(&:sum).sum
