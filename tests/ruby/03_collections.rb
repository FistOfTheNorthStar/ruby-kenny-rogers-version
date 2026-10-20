arr = [5, 3, 8, 1, 9, 2]
p arr.sort, arr.sort.reverse, arr.max, arr.min, arr.sum
p arr.map { |x| x * 2 }
p arr.select(&:even?), arr.reject(&:even?)
p arr.inject(:+), arr.reduce(1) { |acc, x| acc * x }
p arr.sort_by { |x| -x }
p arr.each_slice(2).to_a
p arr.each_with_index.map { |x, i| x * i }
p arr.partition(&:odd?)
p arr.group_by(&:odd?)
p arr.min_by { |x| (x - 4).abs }
p arr.take(2), arr.drop(4), arr.first, arr.last(2)
p arr.include?(8), arr.index(8), arr.count(&:even?)
p [1, [2, [3, [4]]]].flatten, [1, nil, 2, nil].compact, [1, 1, 2, 3, 3].uniq
p [1, 2, 3].zip([4, 5, 6], [7, 8, 9])
p [[1, 2], [3, 4]].to_h, %w[a b c].each_with_object({}) { |s, h| h[s] = s.upcase }
p (1..5).to_a, (1...5).to_a, ('a'..'e').to_a, (1..10).step(3).to_a
p [3, 1, 2].sort { |a, b| b <=> a }
p %w[apple banana cherry].map(&:length)
p [1, 2, 3, 4].each_cons(2).to_a
p ["a", "b"].cycle.first(5)
p [1, 2, 3].tally, %w[a b a].tally
h = { "one" => 1, "two" => 2, three: 3 }
h["four"] = 4
p h.keys, h.values, h.size
p h.map { |k, v| [k.to_s, v * 10] }.to_h
p h.select { |k, v| v > 2 }
p h.min_by { |k, v| v }
p h.sort_by { |k, v| -v }.first(2)
p h.fetch("one"), h.fetch("zzz", 0), h.key?("two"), h.dig(:three)
counts = Hash.new(0)
"mississippi".each_char { |c| counts[c] += 1 }
p counts
groups = Hash.new { |hash, k| hash[k] = [] }
%w[ant bee cat ape bat].each { |w| groups[w[0]] << w }
p groups
p h.transform_values { |v| v * 2 }
p h.to_a.first
h.each_with_index { |(k, v), i| puts "#{i}: #{k}=#{v}" }
p h.any? { |k, v| v == 3 }, h.count { |k, v| v.odd? }, h.sum { |k, v| v }
p({ a: 1 }.merge({ b: 2 }) { |k, x, y| x + y })
nested = { user: { profile: { name: "kenny" } } }
p nested.dig(:user, :profile, :name)
