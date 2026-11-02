# Many threads, each parked on its own socket; woken in some order, one by one and all at once
require "socket"

n = 1000
pairs = n.times.map { UNIXSocket.pair }
results = Array.new(n)
threads = pairs.each_with_index.map do |(a, _), i|
  Thread.new { results[i] = a.readpartial(16) }
end
sleep 0.2
p threads.count(&:alive?)

# wake them in a shuffled order
order = (0...n).to_a.shuffle(random: Random.new(42))
order.each { |i| pairs[i][1].write("msg#{i}") }
threads.each(&:join)
p results.each_with_index.all? { |m, i| m == "msg#{i}" }

# while many wait, a round trip between two others is unaffected
waiting = pairs.map { |a, _| Thread.new { a.readpartial(16) } }
x, y = UNIXSocket.pair
echo = Thread.new { 100.times { y.write(y.readpartial(8)) } }
p 100.times.all? { |i| x.write(i.to_s); x.readpartial(8) == i.to_s }
echo.join
pairs.each { |_, b| b.write("bye") }
p waiting.map(&:value).uniq

# threads waiting with timeouts, most of which pass
pipes = 300.times.map { IO.pipe }
timed = pipes.each_with_index.map do |(r, _), i|
  Thread.new { r.wait_readable(i % 3 == 0 ? 5 : 0.1) ? :read : :timeout }
end
sleep 0.05
pipes.each_with_index { |(_, w), i| w.write("x") if i % 3 == 0 }
p timed.map(&:value).tally.sort

# Process.wait parks too while threads wait on sockets
pid = spawn("sleep", "0.1")
Process.wait(pid)
p $?.success?
pids = 3.times.map { |i| spawn("sh", "-c", "exit #{i}") }
p pids.map { |pid| Process.wait2(pid)[1].exitstatus }
