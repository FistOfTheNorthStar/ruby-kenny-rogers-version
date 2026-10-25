# Threads waiting on pipes and sockets park until the fd is ready (others keep running)
require "socket"

r, w = IO.pipe
got = nil
t = Thread.new { got = r.gets }
sum = 0
1_000_000.times { |i| sum += i }   # runs while t waits
w.puts "hello"
t.join
p got, sum

# IO.select: timeout, then readiness from another thread
r2, w2 = IO.pipe
t0 = Time.now
p IO.select([r2], nil, nil, 0.2)
p (Time.now - t0) >= 0.19
Thread.new { sleep 0.05; w2.write "x" }
p IO.select([r2], nil, nil, 5).map(&:size)
p r2.read_nonblock(10)
p IO.select(nil, [w2], nil, 1).map(&:size)

# many readers, each woken by its own writer
pairs = 20.times.map { IO.pipe }
readers = pairs.each_with_index.map { |(rr, _), i| Thread.new { rr.read(3) } }
pairs.each_with_index.to_a.reverse.each { |(_, ww), i| ww.write(format("%03d", i)) }
p readers.map(&:value) == 20.times.map { |i| format("%03d", i) }

# Thread#raise into a parked reader
r3, _w3 = IO.pipe
t3 = Thread.new { begin; r3.read(1); rescue => e; e.class; end }
sleep 0.05
t3.raise(IOError, "stop")
p t3.value

# TCP: server thread, clients in threads
server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
st = Thread.new do
  3.times.map do
    c = server.accept
    line = c.gets
    c.write "echo #{line}"
    c.close
    line.chomp
  end
end
clients = 3.times.map do |i|
  Thread.new do
    s = TCPSocket.new("127.0.0.1", port)
    s.write "c#{i}\n"
    reply = s.read
    s.close
    reply
  end
end
p clients.map(&:value).sort
p st.value.sort
server.close
