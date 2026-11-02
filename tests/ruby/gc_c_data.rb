# kenny-run-flags: --ext tests/ruby/gc_c_data/gc_c_data.c
# C-extension data objects with a mark function that die young: a minor collection kept
# them on its list of data to re-mark, so a later one ran the mark function on freed data
# (pg's type maps crashed Mastodon's RSpec suite in multi-file runs).
require "gc_c_data"

# allocates objects of another size than the boxes until a collection has run
def until_gc
  n = GC.count
  i = 0
  last = nil
  while GC.count == n
    last = (i..i + 1)
    i += 1
  end
  last
end

kept = []
10.times do |round|
  2_000.times { |i| GcCData::Box.new("dead #{i}") }
  until_gc
  # a page of dead boxes is swept to make room for these few
  3.times { |i| kept << GcCData::Box.new("kept #{round} #{i}") }
  until_gc
end
GC.start
puts kept.size
puts kept.map(&:ref).each_slice(3).first.inspect
puts kept.each_with_index.all? { |b, j| b.ref == "kept #{j / 3} #{j % 3}" }
puts GcCData.freed?
