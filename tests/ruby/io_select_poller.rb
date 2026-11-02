# IO.select over several fds, with and without timeouts, woken by other threads
require "socket"

def elapsed
  t = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  r = yield
  [r, Process.clock_gettime(Process::CLOCK_MONOTONIC) - t]
end

pipes = 4.times.map { IO.pipe }
readers = pipes.map(&:first)

# nothing ready: the timeout passes
r, t = elapsed { IO.select(readers, nil, nil, 0.1) }
p r, t >= 0.09, t < 1

# a zero timeout polls
p IO.select(readers, nil, nil, 0)

# woken by a write to the third pipe, from another thread
Thread.new { sleep 0.05; pipes[2][1].write "c" }
r, t = elapsed { IO.select(readers, nil, nil, 5) }
p r[0].map { |io| readers.index(io) }, t < 2

# several ready at once
pipes[0][1].write "a"
pipes[3][1].write "d"
p IO.select(readers)[0].map { |io| readers.index(io) }.sort
readers.each { |io| io.read_nonblock(10, exception: false) }

# readable and writable together
a, b = UNIXSocket.pair
rs, ws, = IO.select([a], [b], nil, 1)
p [rs, ws.size]
b.write "x"
rs, ws, = IO.select([a], [b], nil, 1)
p [rs.size, ws.size]
a.read_nonblock(10)

# no timeout: waits until a write comes
Thread.new { sleep 0.05; b.write "late" }
p IO.select([a])[0].size, a.read_nonblock(10)

# many threads in IO.select at once, each with its own timeout, half of them woken
pairs = 40.times.map { UNIXSocket.pair }
waiters = pairs.each_with_index.map do |(x, _), i|
  Thread.new { r = IO.select([x], nil, nil, i.even? ? 5 : 0.2); r ? :ready : :timeout }
end
sleep 0.05
pairs.each_with_index { |(_, y), i| y.write("!") if i.even? }
res = waiters.map(&:value)
p res.tally

# select's EOF: the writer closing makes the reader readable
r, w = IO.pipe
Thread.new { sleep 0.05; w.close }
p IO.select([r], nil, nil, 5)[0] == [r], r.read

# Kernel#select and IO#wait_readable / wait_writable with timeouts
r, w = IO.pipe
p select([r], nil, nil, 0.05)
p r.wait_readable(0.05)
p w.wait_writable(1) == w
Thread.new { sleep 0.05; w.write "z" }
p r.wait_readable(5) == r
