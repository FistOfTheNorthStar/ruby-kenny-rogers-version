# Thread#kill / Thread#raise / Timeout of threads waiting on pipes and sockets; closing an
# IO another thread waits on
require "socket"
require "timeout"

def parked(&blk)
  t = Thread.new do
    blk.call
    :finished
  rescue Exception => e
    [e.class, e.message]
  end
  sleep 0.05
  t
end

# Thread#raise into a read, a gets, an IO.select, an accept, a write to a full pipe
r, w = IO.pipe
t = parked { r.read(1) }
t.raise(RuntimeError, "stop read")
p t.value
t = parked { r.gets }
t.raise(ArgumentError, "stop gets")
p t.value
t = parked { IO.select([r]) }
t.raise(IOError, "stop select")
p t.value
server = TCPServer.new("127.0.0.1", 0)
t = parked { server.accept }
t.raise(RuntimeError, "stop accept")
p t.value
r2, w2 = IO.pipe
t = parked { w2.write("x" * 1_000_000) }
t.raise(RuntimeError, "stop write")
p t.value
# (the IOs still work afterwards)
w.write "after\n"
p r.gets

# Thread#kill: ensure blocks run, the thread is dead
log = []
t = Thread.new do
  r.read(1)
ensure
  log << :ensure
end
sleep 0.05
t.kill
t.join
p log, t.status, t.alive?

# Timeout.timeout around blocking reads
p (Timeout.timeout(0.1) { r.read(1) } rescue $!.class)
a, b = UNIXSocket.pair
p (Timeout.timeout(0.1) { a.readpartial(10) } rescue $!.class)
p (Timeout.timeout(0.1) { server.accept } rescue $!.class)
b.write "ok"
p Timeout.timeout(1) { a.readpartial(10) }

# closing an IO another thread waits on
r, w = IO.pipe
t = parked { r.read(1) }
r.close
p t.value
r, w = IO.pipe
t = parked { r.gets }
r.close
p t.value
r, w = IO.pipe
t = parked { r.wait_readable }
r.close
p t.value
r, w = IO.pipe
t = parked { IO.select([r]) }
r.close
p t.value
t = parked { server.accept }
server.close
p t.value
x, y = UNIXSocket.pair
t = parked { x.recv(10) }
x.close
p t.value
x, y = UNIXSocket.pair
t = parked { x.write("x" * 10_000_000) }
x.close
p t.value
r, w = IO.pipe
t = parked { w.write("x" * 10_000_000) }
w.close
p t.value

# the main thread parked on a pipe while others finish isn't a deadlock
r, w = IO.pipe
Thread.new { sleep 0.1; w.write "late" }
p r.readpartial(10)
