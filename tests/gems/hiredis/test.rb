require "bundler/setup"
require "hiredis"
require "socket"

p Hiredis::Connection
reader = Hiredis::Ext::Reader.new
reader.feed("+OK\r\n:5\r\n$3\r\nabc\r\n*2\r\n$1\r\na\r\n$-1\r\n")
p reader.gets, reader.gets, reader.gets, reader.gets, reader.gets

# (a stand-in server: rb_thread_fd_select waits on its socket)
server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
t = Thread.new do
  s = server.accept
  s.readpartial(1024)
  sleep 0.05
  s.write("+PONG\r\n")
  s.close
end
c = Hiredis::Connection.new
c.connect("127.0.0.1", port, 2_000_000)
p c.connected?
c.write(["PING"])
p c.read
t.join
begin
  Hiredis::Connection.new.connect("127.0.0.1", 1, 200_000)
rescue => e
  p e.class
end
