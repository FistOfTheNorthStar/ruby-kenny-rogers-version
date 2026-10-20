# GC.compact moves objects out of sparsely used pages and rewrites the references to them.
# Builds a fragmented heap of every kind of object, compacts at top level (where no native
# frame is active), and checks every value, identity, hash lookup and mutation afterwards.

Point = Struct.new(:x, :y)
class Node
  attr_accessor :val, :next_node, :meta
  def initialize(v, n = nil); @val = v; @next_node = n; @meta = { "v" => v.to_s, list: [v, v.to_s] }; end
end

def build(n)
  keep = []
  junk = []
  n.times do |i|
    o = case i % 7
        when 0 then Node.new(i)
        when 1 then "string #{i}" * 2
        when 2 then [i, i.to_s, [i, [i.to_f]], { i => i.to_s }]
        when 3 then { "k#{i}" => i, i.to_s.to_sym => [i], [i, i] => "pair" }
        when 4 then (i..i + 10)
        when 5 then Point.new(i, i.to_s)
        else i.to_f + 0.5 # flonum or heap float
        end
    (i % 40 == 0 ? keep : junk) << [i, o]
  end
  junk.clear
  keep
end

def check(keep)
  keep.all? do |i, o|
    case i % 7
    when 0 then o.val == i && o.meta == { "v" => i.to_s, list: [i, i.to_s] } && o.next_node.nil?
    when 1 then o == "string #{i}" * 2
    when 2 then o == [i, i.to_s, [i, [i.to_f]], { i => i.to_s }]
    when 3 then o["k#{i}"] == i && o[i.to_s.to_sym] == [i] && o[[i, i]] == "pair"
    when 4 then o == (i..i + 10) && o.sum == (i..i + 10).sum
    when 5 then o.x == i && o.y == i.to_s
    else o == i.to_f + 0.5
    end
  end
end

keep = build(60_000)
# a linked list and cross references
list = nil
500.times { |i| list = Node.new(i, list) }
shared = "shared"
holders = 100.times.map { |i| [shared, { s: shared }, Node.new(shared)] }
ids = keep.first(20).map { |_, o| o.object_id }
id_keys = { keep[0][1] => :first, keep[7][1] => :eighth }
sing = Object.new
def sing.hi = :hi
lam = ->(x) { x + keep.size }
GC.start
r = GC.compact
p r.is_a?(Hash)
p check(keep)
n = 0
node = list
while node
  n += node.val
  node = node.next_node
end
p n
p holders.all? { |a, h, nd| a.equal?(shared) && h[:s].equal?(shared) && nd.val.equal?(shared) }
p keep.first(20).map { |_, o| o.object_id } == ids
p id_keys[keep[0][1]], id_keys[keep[7][1]]
p sing.hi, lam.call(1)

# mutate after compacting, compact again, check again
keep.each { |i, o| o.val = o.val + 1 if o.is_a?(Node) }
keep.each { |i, o| o << "x" if o.is_a?(String) && !o.frozen? }
GC.compact
p keep.select { |i, o| o.is_a?(Node) }.all? { |i, o| o.val == i + 1 }
p keep.select { |i, o| o.is_a?(String) }.all? { |i, o| o == "string #{i}" * 2 + "x" }

# several rounds of churn and compaction
3.times do |round|
  more = build(30_000)
  GC.compact
  p [round, check(more), check(keep.reject { |i, o| o.is_a?(Node) || o.is_a?(String) })]
end
h = {}
1000.times { |i| h["key#{i}"] = [i] }
GC.compact
p (0...1000).all? { |i| h["key#{i}"] == [i] }, h.size
p :done
