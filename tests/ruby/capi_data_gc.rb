# kenny-run-flags: --ext tests/ruby/capi_data_gc
# A data object with a mark function that dies young stayed on the list minor collections
# trace, so they traced its cell after it was freed. Once the cell held something else (a
# dead Hash, its table's memory reused) they followed garbage and crashed: Mastodon's specs.
#
# It takes objects of another size to set off the collections, while the page of data objects
# is partly used: the rest of that page is freed cells.
require "capi_data_gc"

kept = []
total = 0
600.times do |i|
  1037.times do |j|
    b = Box.new.set(j)
    kept << b if j == 0 && i % 60 == 0
  end
  40_000.times { total += [i, total].size }
end
puts total
puts kept.size
puts kept.map { |b| b.get }.uniq
puts Box.freed_count > 0
puts Box.marked_after_free
