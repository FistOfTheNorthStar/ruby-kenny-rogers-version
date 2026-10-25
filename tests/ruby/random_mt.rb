r = Random.new(42)
p r.rand(100), r.rand, r.rand(1..6), r.rand(2.5), r.rand(1.0...2.0), r.rand(1.0..2.0), r.rand(10**20), r.rand(2**40), r.bytes(7).bytes, r.seed
[0, 1, 7, 2**32, 2**32 + 5, 2**40 + 5, 2**64 - 1, 2**100 + 3, -42, -(2**70)].each { |s| x = Random.new(s); p [s, x.rand(1000), x.rand] }
p (1..10).to_a.shuffle(random: Random.new(1)), [1, 2, 3].shuffle(random: Random.new(1)), %w[a b c d e].shuffle(random: Random.new(99))
(1..12).each { |n| p [n, (1..20).to_a.sample(n, random: Random.new(n * 7))] }
p [1, 2, 3].sample(random: Random.new(3)), [].sample(random: Random.new(3)), [5].sample(3, random: Random.new(1))
srand(1234); a = [rand(100), rand, rand(1..3), [1, 2, 3, 4].shuffle, [1, 2, 3, 4].sample]
p a, srand(5), Random.rand(10), Random.random_number(10)
srand(1234); p [rand(100), rand, rand(1..3), [1, 2, 3, 4].shuffle, [1, 2, 3, 4].sample] == a
p rand(-5).class, rand(0).class, rand(1.5).class, (Random.new(1).rand(0) rescue $!.class), (Random.new(1).rand(-1) rescue $!.message)
x = Random.new(3); y = x.dup; p x.rand(100) == y.rand(100), x == Random.new(3), Random.new(3) == Random.new(3)
o = Object.new; def o.rand(n) = n - 1; p [1, 2, 3].shuffle(random: o)
p Random.new(2024).random_number(6), Random.new(2024).random_number, Random.new(7).random_number(1.5)
