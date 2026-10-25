# ObjectSpace::WeakMap / WeakKeyMap: the methods Rails' connection pool and others use
k = ObjectSpace::WeakKeyMap.new
a = "lease-a"
b = "lease-b"
k[a] = 1
k[b] = 2
p k[a], k.key?(b), k.getkey("lease-a").equal?(a)
p k.delete(a), k.key?(a)
p k.clear.equal?(k), k.key?(b), k[b]
p ObjectSpace::WeakKeyMap.instance_methods(false).sort - [:inspect]

w = ObjectSpace::WeakMap.new
x = Object.new
y = Object.new
w[x] = :x
w[y] = :y
p w.size, w.length, w.member?(x), w.include?(y), w.key?(Object.new)
seen = []
r = w.each_pair { |kk, v| seen << v }
p seen.sort, r.equal?(w)
seen = []
w.each_key { |kk| seen << (kk.equal?(x) || kk.equal?(y)) }
p seen
seen = []
w.each_value { |v| seen << v }
p seen.sort
p w.values.sort, w.keys.size
p w.delete(x), w.size
