# Blocking reads and writes on pipes and sockets park the thread; a write (or read) from
# another thread wakes it.
require "socket"

# read / gets / readpartial / read(n) woken by writes from another thread
r, w = IO.pipe
got = []
reader = Thread.new do
  got << r.gets
  got << r.read(3)
  got << r.readpartial(100)
  got << r.read
end
[["line\n", 0.02], ["abc", 0.02], ["partial", 0.02]].each { |s, d| sleep d; w.write s }
sleep 0.02
w.close
reader.join
p got

# a socket pair, both directions
a, b = UNIXSocket.pair
echo = Thread.new { 3.times.map { m = b.readpartial(10); b.write(m.upcase); m } }
replies = %w[one two three].map { |m| a.write(m); a.readpartial(10) }
p replies, echo.value

# ping-pong between two threads, many rounds
r1, w1 = IO.pipe
r2, w2 = IO.pipe
n = 2000
pong = Thread.new { n.times { w2.write(r1.readpartial(1)) } }
ok = n.times.all? { |i| c = (97 + i % 26).chr; w1.write(c); r2.readpartial(1) == c }
pong.join
p ok

# a big write fills the pipe: the writer waits parked while the reader drains it, and a
# busy thread keeps running meanwhile
r, w = IO.pipe
data = "x" * 1_000_000
writer = Thread.new { w.write(data); w.close; :written }
spins = 0
busy = Thread.new { t = Time.now; spins += 1 while Time.now - t < 0.2 }
sleep 0.05
read = r.read
p [read.size, writer.value, busy.join && spins > 0]

# the same over TCP, with the server and the clients in threads
server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
srv = Thread.new do
  c = server.accept
  total = 0
  while (chunk = c.readpartial(65536) rescue nil)
    total += chunk.size
  end
  c.write("got #{total}")
  c.close
end
s = TCPSocket.new("127.0.0.1", port)
s.write("y" * 3_000_000)
s.close_write
p s.read
s.close
srv.join
server.close

# a reader inside a Fiber parks too (the thread's other Fibers wait with it)
r, w = IO.pipe
f = Fiber.new { r.readpartial(10) }
Thread.new { sleep 0.05; w.write "in fiber" }
p f.resume

# STDIN-like inherited pipes still work: a child's output read while another thread runs
out = IO.popen(["echo", "from child"], &:read)
p out
