# Randomized churn with compaction at top level: a graph of objects, arrays, hashes and
# strings that keeps changing (new links, replaced values, dropped nodes), compacted every few
# hundred steps; after each compaction every node's invariants are checked.

class GNode
  attr_accessor :id, :links, :props, :label
  def initialize(id)
    @id = id
    @links = []
    @props = { "id" => id, sym: id.to_s, nested: [id, { deep: id }] }
    @label = "node-#{id}"
  end
  def ok?
    @props["id"] == @id && @props[:sym] == @id.to_s && @props[:nested] == [@id, { deep: @id }] &&
      @label == "node-#{@id}" && @links.all? { |l| l.is_a?(GNode) && l.label == "node-#{l.id}" }
  end
end

seed = 12345
rand_i = ->(n) { seed = (seed * 1103515245 + 12345) % 2147483648; seed % n }

nodes = []
by_label = {}
next_id = 0
compactions = 0
moved = 0
step = 0
bad = 0
while step < 30_000
  case rand_i.(6)
  when 0, 1
    n = GNode.new(next_id)
    next_id += 1
    nodes << n
    by_label[n.label] = n
  when 2
    if nodes.size > 1
      a = nodes[rand_i.(nodes.size)]
      b = nodes[rand_i.(nodes.size)]
      a.links << b
      a.links.shift if a.links.size > 5
    end
  when 3
    if nodes.size > 10
      gone = nodes.delete_at(rand_i.(nodes.size))
      by_label.delete(gone.label)
    end
  when 4
    if nodes.size > 0
      n = nodes[rand_i.(nodes.size)]
      n.props[:extra] = [n.id] * rand_i.(4)
      n.props.delete(:extra) if rand_i.(2) == 0
    end
  else
    junk = Array.new(rand_i.(20)) { |i| "junk #{i}" }
  end
  if step % 500 == 499
    r = GC.compact
    if r
      compactions += 1
      moved += r[:moved]
    end
    nodes.each { |n| bad += 1 unless n.ok? && by_label[n.label].equal?(n) }
  end
  step += 1
end
p bad, nodes.all?(&:ok?), by_label.size == nodes.size
p compactions > 0, moved > 0
p :done
