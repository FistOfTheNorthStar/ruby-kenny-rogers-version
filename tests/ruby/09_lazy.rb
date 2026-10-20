p (1..Float::INFINITY).lazy.map { |x| x * 2 }.select { |x| x % 3 == 0 }.first(4)
p (1..).lazy.select(&:even?).map { _1 ** 2 }.take(5).to_a
p (1..20).lazy.reject(&:odd?).take_while { |x| x < 12 }.to_a
p (1..10).lazy.drop_while { |x| x < 7 }.force
p [[1, 2], [3], [4, 5, 6]].lazy.flat_map { |a| a }.map { |x| x * 10 }.first(4)
p %w[a b c d].lazy.with_index.map { |s, i| "#{i}:#{s}" }.to_a
p [3, 1, 3, 2, 1, nil, 2].lazy.compact.uniq.to_a
p (1..).lazy.filter_map { |x| x * 3 if x.odd? }.first(3)
primes = (2..).lazy.select { |n| (2..Math.sqrt(n)).none? { |d| (n % d).zero? } }
p primes.first(10)
p (1..3).lazy.map { |x| x + 1 }.include?(4), (1..).lazy.map { |x| x * x }.find { |x| x > 50 }
