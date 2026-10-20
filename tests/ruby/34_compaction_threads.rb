# Compaction while other threads are in the middle of changing their own object graphs:
# they are stopped at safepoints in compiled loops (no native frames), the main thread
# compacts, and they continue on the moved objects.

T = 6
results = Queue.new
workers = T.times.map do |t|
  Thread.new do
    items = []
    index = {}
    i = 0
    errors = 0
    while i < 20_000
      key = "t#{t}-#{i}"
      obj = { key: key, val: [i, key, { n: i }], tag: "x" * (i % 7) }
      items << obj
      index[key] = obj
      if i % 3 == 0 && items.size > 50
        old = items.shift
        index.delete(old[:key])
      end
      if i % 1000 == 999
        errors += items.count { |o| !(o[:val][0].is_a?(Integer) && o[:val][1] == o[:key] && o[:val][2][:n] == o[:val][0] && index[o[:key]].equal?(o)) }
        Thread.pass
      end
      i += 1
    end
    results << [t, errors, items.size, index.size]
  end
end

compactions = 0
moved = 0
rounds = 0
while results.size < T && rounds < 2000
  junk = Array.new(2000) { |k| "junk#{k}" }
  r = GC.compact
  if r
    compactions += 1
    moved += r[:moved]
  end
  rounds += 1
  Thread.pass
end
workers.each(&:join)
res = []
res << results.pop until results.empty?
p res.sort.map { |t, e, a, b| [t, e, a == b] }
p compactions > 0
p :done
