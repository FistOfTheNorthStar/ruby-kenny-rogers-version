require "bundler/setup"
require "thread_safe"

cache = ThreadSafe::Cache.new
threads = 8.times.map do |i|
  Thread.new do
    100.times { |j| cache[[i, j]] = i * j }
    cache.compute_if_absent(:shared) { "first" }
  end
end
threads.each(&:join)
p cache.size, cache[:shared], cache[[7, 99]]
p cache.put_if_absent(:shared, "x"), cache.fetch(:nope, :default), cache.fetch_or_store(:k) { 5 }
p cache.key?(:k), cache.delete(:k), cache.key?(:k)
cache.compute(:counter) { |v| (v || 0) + 1 }
cache.merge_pair(:counter, 1) { |v| v + 10 }
p cache[:counter]
p cache.replace_pair(:counter, 11, 12), cache[:counter], cache.get_and_set(:counter, 0)
counter = ThreadSafe::Cache.new
m = Mutex.new
10.times.map { Thread.new { 100.times { m.synchronize { counter[:n] = (counter[:n] || 0) + 1 } } } }.each(&:join)
p counter[:n]
cache2 = ThreadSafe::Cache.new { |h, k| h[k] = k.to_s * 2 }
p cache2[:ab], cache2.keys
arr = ThreadSafe::Array.new
5.times.map { |i| Thread.new { 20.times { arr << i } } }.each(&:join)
p arr.size, arr.sort.uniq
h = ThreadSafe::Hash.new
h[:a] = 1
p h, h.class.ancestors.include?(Hash)
c3 = ThreadSafe::Cache.new
c3[:b] = 2; c3[:a] = 1
r = []
c3.each_pair { |k, v| r << [k, v] }
p r.sort, c3.values.sort, c3.empty?, c3.clear.empty? rescue p $!
