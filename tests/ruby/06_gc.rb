# Allocation-heavy code must keep working while the GC runs underneath.
class Node
  attr_accessor :value, :next_node
  def initialize(value, next_node = nil)
    @value = value
    @next_node = next_node
  end
end

list = nil
200_000.times { |i| list = Node.new(i, list) }
sum = 0
n = list
while n
  sum += n.value
  n = n.next_node
end
puts sum

keep = []
300_000.times do |i|
  s = "item-#{i}"
  keep << s if i % 1000 == 0
  _tmp = [s, s.upcase, { k: s }]
end
puts keep.size, keep.last

h = Hash.new { |hash, k| hash[k] = "v#{k}" * 3 }
100_000.times { |i| h[i % 5000] }
puts h.size, h[4999]

words = (1..50_000).map { |i| "w#{i % 977}" }
counts = words.tally
puts counts.size, counts["w5"]

sorted = (1..100_000).map { |i| (i * 7919) % 100_003 }.sort_by { |x| -x }.first(3)
p sorted

closures = (1..10_000).map { |i| -> { i * 2 } }
puts closures.sum(&:call)

GC.start
puts GC.count > 0
