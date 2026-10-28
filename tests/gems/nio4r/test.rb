require "bundler/setup"
require "nio"
require "socket"

selector = NIO::Selector.new
p selector.empty?, NIO::Selector.backends.is_a?(Array)
r, w = IO.pipe
mon = selector.register(r, :r)
p mon.interests, mon.io.equal?(r), selector.registered?(r), selector.empty?
p selector.select(0.05)
w.write "hello"
ready = selector.select(1)
p ready.size, ready.first.readable?, ready.first.writable?, ready.first.readiness
p r.read_nonblock(100)
mon.value = :payload
selector.select(1) { |m| p [:block, m.value] } if (w << "x")
r.read_nonblock(10)

a, b = UNIXSocket.pair
wm = selector.register(a, :w)
selector.select(1).each { |m| p [m.io == a, m.writable?] }
wm.interests = :rw
p wm.interests
wm.add_interest(:r) rescue p $!.class
b.write("ping")
ev = selector.select(1).select { |m| m.io == a }.first
p ev.readiness
p a.read_nonblock(4)
selector.deregister(a)
p selector.registered?(a), wm.closed?
mon.close
p mon.closed?, selector.registered?(r)

# wakeup from another thread
t = Thread.new { sleep 0.05; selector.wakeup }
p selector.select(5)
t.join

buf = NIO::ByteBuffer.new(16)
buf << "hello world"
p buf.position, buf.remaining, buf.size
buf.flip
p buf.get(5), buf.position
buf.rewind
p buf.get
buf.clear
p buf.position, buf.limit
selector.close
p selector.closed?
