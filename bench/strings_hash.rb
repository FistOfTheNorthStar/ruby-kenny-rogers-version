words = %w[alpha beta gamma delta epsilon zeta eta theta]
counts = Hash.new(0)
buf = []
1_000_000.times do |i|
  w = words[i % words.size]
  counts[w] += 1
  buf << "#{w}-#{i}" if i % 1000 == 0
end
puts counts.size, buf.size, buf.last, counts["gamma"]
