# Hash#compare_by_identity: keys hashed and compared by identity, kept by dup/select/merge/...
h = {}.compare_by_identity
a = "k".dup; b = "k".dup
h[a] = 1; h[b] = 2
p h.size, h[a], h[b], h["k".dup], h.compare_by_identity?
p h.keys.map(&:frozen?), h.keys[0].equal?(a)
p h.dup.compare_by_identity?, h.clone.compare_by_identity?, h.select { true }.compare_by_identity?, h.reject { false }.compare_by_identity?
p h.merge({}).compare_by_identity?, h.to_h.compare_by_identity?, h.transform_values { 1 }.compare_by_identity?
p h.filter_map { 1 }.class, h.slice(a).compare_by_identity?, h.except(a).compare_by_identity?
p({}.replace(h).compare_by_identity?)
g = {"x" => 1, "y" => 2}
g.compare_by_identity
p g["x".dup], g.size, g.keys
p h.key?(a), h.delete(b), h.size, h.to_a
p Hash.new.compare_by_identity?
s1 = :sym; p({}.compare_by_identity.tap { |x| x[s1] = 1; x[1] = 2; x[2**70] = 3; x[1.5] = 4 }.then { |x| [x[:sym], x[1], x[2**70], x[1.5]] })
p h.invert.compare_by_identity?, h.group_by { 1 }.compare_by_identity?, h.sort.class
begin; {}.freeze.compare_by_identity; rescue => e; p e.class; end
p h.inspect
p h == {a => 1}, ({a => 1}.compare_by_identity == {a => 1})
p h.transform_keys(&:upcase).compare_by_identity?
# many keys (indexed table), rehash after compare_by_identity on a filled hash
big = {}
strs = (0...50).map { |i| "s#{i % 5}" }
strs.each_with_index { |k, i| big[k] = i }
p big.size
big2 = {}.compare_by_identity
strs.each_with_index { |k, i| big2[k] = i }
p big2.size, big2[strs[7]], big2["s2".dup]
filled = (0...20).to_h { |i| [i.to_s, i] }
k0 = filled.keys[3]
filled.compare_by_identity
p filled.size, filled[k0], filled["3".dup], filled.compare_by_identity?
p filled.except(k0).size, filled.compact.compare_by_identity?
d = {}.compare_by_identity
d[k0] = 1
d.transform_keys! { |k| k }
p d.compare_by_identity?, d[k0]
