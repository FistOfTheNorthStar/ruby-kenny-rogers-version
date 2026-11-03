# A forked child waits on sockets and pipes as its parent does (a fresh event loop: the
# parent's isn't inherited)
require "socket"

# the parent waits on a pipe first, so its event loop exists before fork
r, w = IO.pipe
Thread.new { sleep 0.02; w.write "parent" }
p r.readpartial(10)

server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
res_r, res_w = IO.pipe
pids = 2.times.map do |k|
  fork do
    res_r.close
    # in the child: threads parked on a pipe and on accept
    a, b = IO.pipe
    t = Thread.new { a.readpartial(10) }
    sleep 0.02
    b.write "child#{k}"
    got = t.value
    c = server.accept
    line = c.gets
    c.write "pong #{line}"
    c.close
    res_w.write "#{got} #{line.chomp}\n"
    exit!(0)
  end
end
res_w.close
server.close
replies = 2.times.map do |i|
  s = TCPSocket.new("127.0.0.1", port)
  s.write "ping#{i}\n"
  x = s.read
  s.close
  x
end
pids.each { |pid| Process.wait(pid) }
p replies.sort
p res_r.read.lines.map { |l| l.split }.transpose.map(&:sort)
