total = 0
5_000_000.times do |i|
  total += i if i.even?
end
arr = (1..1_000_000).map { |x| x * 2 }
puts total, arr.select { |x| x % 3 == 0 }.sum, arr.sort_by { |x| -x }.first
