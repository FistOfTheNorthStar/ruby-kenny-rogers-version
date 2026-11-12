# Threads asleep in Kernel#sleep, after natives that called blocks: a compaction goes ahead.
# (Linux aarch64: the sleep closure's frame record is 8- not 16-aligned, and the stack walk
# took that for a broken chain and scanned the stack word by word, finding stale native
# return addresses.)
def natives = 5.times { [1, 2, 3].each_slice(1) { |x| x }; [3, 1].each_cons(1) { |x| x }; { a: 1 }.each_with_index { |x, i| x } }
ts = 8.times.map { |i| Thread.new { natives; sleep(i.even? ? 30 : 30.0) } }
sleep 0.01 until ts.all? { |t| t.status == "sleep" }
r = []
i = 0
while i < 5
  Array.new(300) { |k| "j#{k}" * 2 }
  r << !GC.compact.nil?
  i += 1
end
p r
ts.each(&:kill)
