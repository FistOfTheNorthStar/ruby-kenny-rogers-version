# 12 CPU-bound threads (Ruby's GVL runs them one at a time; Kenny runs them on all cores)
results = 12.times.map do |t|
  Thread.new do
    s = 0
    i = 0
    while i < 3_000_000
      s += (i * t) % 7
      i += 1
    end
    s
  end
end.map(&:value)
puts results.sum
