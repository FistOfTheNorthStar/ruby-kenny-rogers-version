# arithmetic, bignum promotion, floats
puts 1 + 2 * 3
puts 7 / 2, -7 / 2, 7 % 3, -7 % 3
puts 2 ** 100
puts 4611686018427387903 + 1
puts 10.0 / 4
puts 0.1 + 0.2
puts 1e20
puts 3.14159.round(2)
puts 10.fdiv(3).round(3)
puts (1..10).sum
puts 100.digits.inspect
x = 5
x += 10
x *= 2
puts x
a, b = 1, 2
a, b = b, a
puts "#{a} #{b}"
first, *rest = [1, 2, 3, 4]
p first, rest
*init, last = [1, 2, 3]
p init, last
puts [1, 2, 3].sum { |v| v * 2 }
puts "%05.2f|%-5s|%3d|%x" % [3.14159, "ab", 42, 255]
puts format("%s has %d items", "cart", 3)
p nil.to_a, nil.to_s, :sym, "str", 1.5, [1, [2, nil]], {a: 1}
puts 10.times.map { |i| i * i }.select(&:even?).inspect
