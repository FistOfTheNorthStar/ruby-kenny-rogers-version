# The low-level socket API: Socket.new + sockaddr strings, connect_nonblock, Addrinfo,
# Socket::Option, UNIXSocket.pair, UNIX-domain servers, the IO::E*Wait* errors
require "socket"

p IO::EAGAINWaitReadable.ancestors.take(3), IO::EWOULDBLOCKWaitWritable, IO::EINPROGRESSWaitWritable.superclass

sa = Socket.sockaddr_in(80, "127.0.0.1")
p sa.encoding, Socket.unpack_sockaddr_in(sa), Socket.unpack_sockaddr_in(Socket.pack_sockaddr_in("8080", "::1"))
p Socket.unpack_sockaddr_in(Socket.pack_sockaddr_in(1, ""))
p((Socket.unpack_sockaddr_in(Socket.sockaddr_un("/tmp/x")) rescue $!))
p Socket.unpack_sockaddr_un(Socket.sockaddr_un("/tmp/x"))
p((Socket.unpack_sockaddr_un(sa) rescue $!))
p((Socket.new(:FOO, :STREAM) rescue $!), (Socket.new(:INET, :FOO) rescue $!))
p((Socket.pair(:INET, :STREAM) rescue $!.class))
p((Socket.getaddrinfo("nope.invalid", 80) rescue [$!.class, $!.is_a?(SocketError)]))

# a closed port
srv = TCPServer.new("127.0.0.1", 0); port = srv.addr[1]; srv.close
s = Socket.new(:INET, :STREAM)
begin
  s.connect(Socket.sockaddr_in(port, "127.0.0.1"))
rescue SystemCallError => e
  p e.class, e.message.sub(port.to_s, "PORT")
end
s.close

# listen / connect / accept with Socket
l = Socket.new(Socket::AF_INET, Socket::SOCK_STREAM, 0)
l.setsockopt(:SOCKET, :REUSEADDR, true)
l.bind(Addrinfo.tcp("127.0.0.1", 0))
l.listen(5)
port = l.local_address.ip_port
p l.accept_nonblock(exception: false)
begin
  l.accept_nonblock
rescue IO::WaitReadable => e
  p e.class, e.message
end
c = Socket.new(:INET, :STREAM)
begin
  c.connect_nonblock(Socket.sockaddr_in(port, "127.0.0.1"))
  p :connected
rescue IO::WaitWritable => e
  p e.class, e.message
  p c.wait_writable(5).equal?(c)
  begin
    c.connect_nonblock(Socket.sockaddr_in(port, "127.0.0.1"))
  rescue Errno::EISCONN
    p :isconn
  end
end
p c.connect_nonblock(Socket.sockaddr_in(port, "127.0.0.1"), exception: false)
sv, ai = l.accept
p sv.class, ai.class, ai.ip_address, c.remote_address.ip_port == port
p c.getpeername == Socket.sockaddr_in(port, "127.0.0.1"), Socket.unpack_sockaddr_in(c.getsockname)[1]
c.setsockopt(Socket::IPPROTO_TCP, Socket::TCP_NODELAY, 1)
o = c.getsockopt(:TCP, :NODELAY)
p o.class, o.bool, o.int != 0, o.level == Socket::IPPROTO_TCP
p sv.getsockopt(:SOCKET, :TYPE), sv.getsockopt(:SOCKET, :KEEPALIVE)
sv.write("hello")
p c.wait_readable(5).equal?(c), c.read_nonblock(10), c.read_nonblock(10, exception: false), c.wait_readable(0.01)
begin
  c.read_nonblock(10)
rescue IO::WaitReadable => e
  p e.class, e.message, e.errno == Errno::EAGAIN::Errno
end
c.close; sv.close; l.close

t = Socket.tcp("127.0.0.1", TCPServer.new("127.0.0.1", 0).tap { |x| port = x.addr[1] }.then { port })
p t.class, t.remote_address.inspect.sub(port.to_s, "PORT")

# Addrinfo and Socket::Option
p Addrinfo.tcp("127.0.0.1", 80), Addrinfo.udp("127.0.0.1", 53), Addrinfo.ip("127.0.0.1"), Addrinfo.ip("::1")
p Addrinfo.unix("/tmp/s"), Addrinfo.unix("/tmp/s", :DGRAM), Addrinfo.new(Socket.sockaddr_in(1, "::1"))
p Addrinfo.new(Socket.sockaddr_in(80, "127.0.0.1"), :INET, :STREAM), Addrinfo.new(["AF_INET", 80, "x", "127.0.0.1"])
a = Addrinfo.getaddrinfo("127.0.0.1", 80, nil, :STREAM).first
p a.pfamily == Socket::PF_INET, a.socktype == Socket::SOCK_STREAM, a.protocol == Socket::IPPROTO_TCP
p a.ip?, a.ipv4?, a.ipv6?, a.unix?, a.ip_unpack, a.to_sockaddr == sa, a.inspect_sockaddr, a.ipv4_loopback?
p Socket.getaddrinfo("127.0.0.1", nil, Socket::AF_UNSPEC, Socket::SOCK_STREAM).map { |r| r[0, 4] }
p Socket::Option.int(:INET, :SOCKET, :KEEPALIVE, 1), Socket::Option.bool(:INET, :TCP, :NODELAY, true)
p Socket::Option.linger(true, 5), Socket::Option.linger(true, 5).linger

# UNIX-domain sockets
x, y = UNIXSocket.pair
p x.class, x.addr, x.peeraddr, x.local_address, x.getsockopt(:SOCKET, :TYPE)
x.write "ping"
p y.read_nonblock(10), y.read_nonblock(10, exception: false)
y.close
p x.read
x, y = Socket.pair(:UNIX, :STREAM, 0)
p x.class, y.class
x, y = UNIXSocket.socketpair
p x.class

path = "/tmp/kenny-socket-raw-#{$$}.sock"
begin
  us = UNIXServer.new(path)
  p us.path == path, us.accept_nonblock(exception: false)
  uc = UNIXSocket.new(path)
  ua = us.accept
  p ua.class, ua.path == path, uc.path, uc.peeraddr[1] == path
  uc.write "x"
  p ua.read(1)
  begin
    UNIXServer.new(path)
  rescue SystemCallError => e
    p e.class, e.message.sub(path, "PATH")
  end
ensure
  File.delete(path) if File.exist?(path)
end
begin
  UNIXSocket.new(path)
rescue SystemCallError => e
  p e.class, e.message.sub(path, "PATH")
end

r, w = IO.pipe
p w.write_nonblock("x"), r.wait_readable(1).equal?(r), r.read(1), r.wait_readable(0.01), w.wait_writable(0).equal?(w)
