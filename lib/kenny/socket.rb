# The socket library for Kenny binaries: BasicSocket, IPSocket, TCPSocket, TCPServer and
# Socket (constants, name resolution) on native primitives (kenny-runtime/src/socket.rs).
# Sockets are IO objects on their file descriptor. UNIX-domain and UDP sockets, Addrinfo
# and the low-level Socket#connect/bind API are not provided.
require "kenny/socket_native"

class SocketError < StandardError; end

class BasicSocket < IO
  def self.do_not_reverse_lookup = true
  def self.do_not_reverse_lookup=(_v); end

  def setsockopt(level, optname, optval = nil)
    level = Socket.const_get(level.to_s.sub(/\A(SOL_)?/, "SOL_")) if level.is_a?(Symbol) || level.is_a?(String)
    optname = Socket.const_get(optname) if optname.is_a?(Symbol) || optname.is_a?(String)
    Kenny::SocketNative.setsockopt(fileno, level, optname, optval)
  end

  def getsockopt(*) = nil
  def shutdown(how = Socket::SHUT_RDWR) = Kenny::SocketNative.shutdown(fileno, how)
  def close_read = shutdown(Socket::SHUT_RD)
  def close_write = shutdown(Socket::SHUT_WR)
  def do_not_reverse_lookup = true
  def do_not_reverse_lookup=(_v); end

  def send(mesg, _flags = 0, *_dest)
    write(mesg)
  end

  def recv(maxlen, _flags = 0)
    readpartial(maxlen)
  rescue EOFError
    ""
  end

  def recv_nonblock(maxlen, _flags = 0, exception: true)
    read_nonblock(maxlen, exception: exception)
  end
end

class IPSocket < BasicSocket
  def self.getaddress(host)
    info = Socket.getaddrinfo(host, nil)
    raise SocketError, "getaddrinfo: nodename nor servname provided, or not known" if info.empty?
    info.first[3]
  end

  def addr(_reverse = nil) = Kenny::SocketNative.sockname(fileno)
  def peeraddr(_reverse = nil) = Kenny::SocketNative.peername(fileno)
  def local_address = addr
  def remote_address = peeraddr
end

class TCPSocket < IPSocket
  def self.gethostbyname(host)
    info = Socket.getaddrinfo(host, nil)
    [host, [], info.first[4], *info.map { |i| i[3] }.uniq]
  end

  def self.__for_fd(fd) = new(fd, :__kenny_fd)

  def initialize(host, port, _local_host = nil, _local_port = nil, connect_timeout: nil, resolv_timeout: nil)
    fd = port.equal?(:__kenny_fd) ? host : Kenny::SocketNative.tcp_connect(host, port)
    super(fd, "r+")
    binmode
    self.sync = true
  end
end

class TCPServer < TCPSocket
  def initialize(host = nil, port)
    super(Kenny::SocketNative.tcp_listen(host, port, Socket::SOMAXCONN), :__kenny_fd)
  end

  def listen(_backlog) = 0
  def accept = TCPSocket.__for_fd(Kenny::SocketNative.accept(fileno))

  def accept_nonblock(exception: true)
    accept
  end
end

class Socket < BasicSocket
  module Constants; end
  Kenny::SocketNative::CONSTANTS.each do |name, value|
    const_set(name, value)
    Constants.const_set(name, value)
    pf = name.sub(/\AAF_/, "PF_")
    if pf != name
      const_set(pf, value)
      Constants.const_set(pf, value)
    end
  end
  INADDR_ANY = 0

  def self.gethostname = Kenny::SocketNative.gethostname

  def self.getaddrinfo(host, port, family = nil, socktype = nil, _protocol = nil, flags = nil)
    family = const_get(family) if family.is_a?(Symbol) || family.is_a?(String)
    Kenny::SocketNative.getaddrinfo(host, port, family || 0, socktype || 0, flags || 0)
  end

  def self.tcp(host, port, *_rest, connect_timeout: nil, resolv_timeout: nil)
    s = TCPSocket.new(host, port)
    return s unless block_given?
    begin
      yield s
    ensure
      s.close unless s.closed?
    end
  end

  def self.ip_address_list = []

  def initialize(*)
    raise NotImplementedError, "Socket.new: use TCPSocket / TCPServer (Kenny's socket library has no raw sockets)"
  end
end

class UNIXSocket < BasicSocket
  def initialize(*)
    raise NotImplementedError, "UNIX-domain sockets are not supported by Kenny's socket library"
  end
end
class UNIXServer < UNIXSocket; end
