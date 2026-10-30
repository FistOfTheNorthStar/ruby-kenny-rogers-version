# The socket library for Kenny binaries: BasicSocket, IPSocket, TCPSocket, TCPServer,
# UNIXSocket, UNIXServer, Socket (raw sockets, socket addresses, name resolution),
# Addrinfo and Socket::Option on native primitives (kenny-runtime/src/socket.rs).
# Sockets are IO objects on their file descriptor; a socket address is the bytes of a
# `struct sockaddr_*` in a binary String, as in CRuby. UDP's sendto/recvfrom and passing
# file descriptors (send_io/recv_io) are not provided.
require "kenny/socket_native"

class SocketError < StandardError; end

class BasicSocket < IO
  # (IO#initialize, for sockets made from a file descriptor)
  alias_method :__kenny_io_init, :initialize
  private :__kenny_io_init

  def self.for_fd(fd)
    s = allocate
    s.__send__(:__kenny_io_init, fd, "r+")
    s.binmode
    s.sync = true
    s
  end

  def self.do_not_reverse_lookup = true
  def self.do_not_reverse_lookup=(_v); end

  def setsockopt(level, optname = nil, optval = nil)
    if level.is_a?(Socket::Option)
      optval = level.data
      optname = level.optname
      level = level.level
    end
    level = Socket.__kenny_level(level)
    optname = Socket.__kenny_optname(level, optname)
    optval = optval.to_str if !optval.is_a?(Integer) && optval != true && optval != false && optval.respond_to?(:to_str)
    Kenny::SocketNative.setsockopt(fileno, level, optname, optval)
  end

  def getsockopt(level, optname)
    level = Socket.__kenny_level(level)
    optname = Socket.__kenny_optname(level, optname)
    data = Kenny::SocketNative.getsockopt(fileno, level, optname)
    Socket::Option.new(__kenny_family, level, optname, data)
  end

  def getsockname = Kenny::SocketNative.getsockname(fileno)
  def getpeername = Kenny::SocketNative.getpeername(fileno)

  def local_address
    Addrinfo.__kenny(getsockname, nil, __kenny_socktype, 0)
  end

  def remote_address
    Addrinfo.__kenny(getpeername, nil, __kenny_socktype, 0)
  end

  def connect_address = local_address

  def shutdown(how = Socket::SHUT_RDWR)
    how = Socket.__kenny_const(how, "SHUT_", "shutdown how") unless how.is_a?(Integer)
    Kenny::SocketNative.shutdown(fileno, how)
  end

  def close_read = shutdown(Socket::SHUT_RD)
  def close_write = shutdown(Socket::SHUT_WR)
  def do_not_reverse_lookup = true
  def do_not_reverse_lookup=(_v); end

  def send(mesg, _flags = 0, *_dest)
    write(mesg)
  end

  def recv(maxlen, _flags = 0, outbuf = nil)
    outbuf ? readpartial(maxlen, outbuf) : readpartial(maxlen)
  rescue EOFError
    ""
  end

  def recv_nonblock(maxlen, _flags = 0, buf = nil, exception: true)
    r = buf ? read_nonblock(maxlen, buf, exception: exception) : read_nonblock(maxlen, exception: exception)
    r.nil? ? "" : r
  rescue EOFError
    ""
  end

  private

  def __kenny_family
    Kenny::SocketNative.sockaddr_family(getsockname)
  rescue SystemCallError
    Socket::AF_UNSPEC
  end

  def __kenny_socktype
    Kenny::SocketNative.getsockopt(fileno, Socket::SOL_SOCKET, Socket::SO_TYPE).unpack1("i")
  rescue SystemCallError
    0
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
end

class TCPSocket < IPSocket
  def self.gethostbyname(host)
    info = Socket.getaddrinfo(host, nil)
    [host, [], info.first[4], *info.map { |i| i[3] }.uniq]
  end

  def self.__for_fd(fd) = new(fd, :__kenny_fd)

  def initialize(host, port, _local_host = nil, _local_port = nil, connect_timeout: nil, resolv_timeout: nil, fast_fallback: nil)
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

  def listen(backlog) = Kenny::SocketNative.listen(fileno, backlog)
  def accept = TCPSocket.__for_fd(Kenny::SocketNative.accept(fileno))
  def sysaccept = Kenny::SocketNative.accept(fileno)

  def accept_nonblock(exception: true)
    r = Kenny::SocketNative.accept2(fileno, true, exception)
    r.is_a?(Symbol) ? r : TCPSocket.__for_fd(r[0])
  end
end

class Socket < BasicSocket
  class ResolutionError < SocketError
    def error_code = @error_code
  end

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

  # (the names Socket's methods take for constants: :INET, "AF_INET", :STREAM, :SOCKET ...)
  def self.__kenny_const(v, prefix, what)
    return v if v.is_a?(Integer)
    name = v.is_a?(Symbol) ? v.to_s : v.to_str
    [prefix + name, name].each do |n|
      return Constants.const_get(n) if n.match?(/\A[A-Z][A-Z0-9_]*\z/) && Constants.const_defined?(n, false)
    end
    raise SocketError, "unknown #{what}: #{name}"
  end

  def self.__kenny_family(v)
    return v if v.is_a?(Integer)
    v = v.to_s
    return const_get(v.sub(/\APF_/, "AF_")) if v.match?(/\A[AP]F_[A-Z0-9_]+\z/) && const_defined?(v.sub(/\APF_/, "AF_"), false)
    __kenny_const(v, "AF_", "socket domain")
  end

  def self.__kenny_socktype(v) = __kenny_const(v, "SOCK_", "socket type")

  LEVELS = { "SOCKET" => SOL_SOCKET, "IP" => IPPROTO_IP, "IPV6" => IPPROTO_IPV6, "TCP" => IPPROTO_TCP, "UDP" => IPPROTO_UDP, "ICMP" => IPPROTO_ICMP }.freeze
  private_constant :LEVELS

  def self.__kenny_level(v)
    return v if v.is_a?(Integer)
    name = v.to_s
    return LEVELS[name] if LEVELS.key?(name)
    return SOL_SOCKET if name == "SOL_SOCKET"
    return Constants.const_get(name) if name.match?(/\A(IPPROTO|SOL)_[A-Z0-9_]+\z/) && Constants.const_defined?(name, false)
    raise SocketError, "unknown protocol level: #{name}"
  end

  def self.__kenny_optprefix(level)
    case level
    when SOL_SOCKET then ["SO_", "socket level option name"]
    when IPPROTO_IP then ["IP_", "IP level option name"]
    when IPPROTO_IPV6 then ["IPV6_", "IPv6 level option name"]
    when IPPROTO_TCP then ["TCP_", "TCP level option name"]
    when IPPROTO_UDP then ["UDP_", "UDP level option name"]
    else [nil, "option name"]
    end
  end

  def self.__kenny_optname(level, v)
    return v if v.is_a?(Integer)
    prefix, what = __kenny_optprefix(level)
    name = v.to_s
    if prefix && Constants.const_defined?(prefix + name, false) && name.match?(/\A[A-Z0-9_]+\z/)
      return Constants.const_get(prefix + name)
    end
    return Constants.const_get(name) if name.match?(/\A[A-Z][A-Z0-9_]*\z/) && Constants.const_defined?(name, false)
    raise SocketError, "unknown #{what}: #{name}"
  end

  # The constant name for `value` among those starting with `prefix` (nil when none).
  def self.__kenny_name(prefix, value)
    n = Kenny::SocketNative::CONSTANTS.find { |k, v| v == value && k.start_with?(prefix) }
    n && n[0].delete_prefix(prefix)
  end

  def self.__kenny_sockaddr(addr)
    addr.is_a?(Addrinfo) ? addr.to_sockaddr : addr.to_str
  end

  def self.gethostname = Kenny::SocketNative.gethostname

  def self.getaddrinfo(host, port, family = nil, socktype = nil, protocol = nil, flags = nil, _reverse_lookup = nil, timeout: nil)
    family = family.nil? ? 0 : __kenny_family(family)
    socktype = socktype.nil? ? 0 : __kenny_socktype(socktype)
    Kenny::SocketNative.getaddrinfo(host, port, family, socktype, protocol || 0, flags || 0).map { |r| r[0, 7] }
  end

  def self.getservbyname(service, proto = "tcp")
    return service.to_i if service.to_s.match?(/\A\d+\z/)
    type = proto.to_s == "udp" ? SOCK_DGRAM : SOCK_STREAM
    Kenny::SocketNative.getaddrinfo(nil, service.to_s, AF_INET, type, 0, 0).first[1]
  rescue SocketError
    raise SocketError, "no such service #{service}/#{proto}"
  end

  def self.sockaddr_in(port, host) = Kenny::SocketNative.pack_sockaddr_in(port, host)
  def self.pack_sockaddr_in(port, host) = Kenny::SocketNative.pack_sockaddr_in(port, host)
  def self.unpack_sockaddr_in(addr) = Kenny::SocketNative.unpack_sockaddr_in(__kenny_sockaddr(addr))
  def self.sockaddr_un(path) = Kenny::SocketNative.pack_sockaddr_un(path)
  def self.pack_sockaddr_un(path) = Kenny::SocketNative.pack_sockaddr_un(path)
  def self.unpack_sockaddr_un(addr) = Kenny::SocketNative.unpack_sockaddr_un(__kenny_sockaddr(addr))

  def self.pair(domain, type, protocol = 0)
    fds = Kenny::SocketNative.socketpair(__kenny_family(domain), __kenny_socktype(type), protocol || 0)
    fds.map { |fd| for_fd(fd) }
  end

  def self.socketpair(domain, type, protocol = 0) = pair(domain, type, protocol)

  def self.tcp(host, port, local_host = nil, local_port = nil, connect_timeout: nil, resolv_timeout: nil, fast_fallback: nil)
    last = nil
    sock = nil
    Addrinfo.getaddrinfo(host, port, nil, :STREAM).each do |ai|
      s = Socket.new(ai.pfamily, ai.socktype, ai.protocol)
      begin
        s.bind(Addrinfo.getaddrinfo(local_host, local_port, ai.pfamily, :STREAM).first) if local_host || local_port
        if connect_timeout
          begin
            s.connect_nonblock(ai)
          rescue IO::WaitWritable
            raise Errno::ETIMEDOUT, "user specified timeout" unless s.wait_writable(connect_timeout)
            begin
              s.connect_nonblock(ai)
            rescue Errno::EISCONN
            end
          end
        else
          s.connect(ai)
        end
        sock = s
        break
      rescue SystemCallError => e
        s.close
        last = e
      end
    end
    raise last if sock.nil? && last
    raise SocketError, "no appropriate local address" unless sock
    return sock unless block_given?
    begin
      yield sock
    ensure
      sock.close unless sock.closed?
    end
  end

  def self.unix(path)
    s = Socket.new(AF_UNIX, SOCK_STREAM, 0)
    begin
      Kenny::SocketNative.connect(s.fileno, sockaddr_un(path), false, true, "connect(2) for #{path}")
    rescue Exception
      s.close
      raise
    end
    return s unless block_given?
    begin
      yield s
    ensure
      s.close unless s.closed?
    end
  end

  def self.ip_address_list = []

  def initialize(domain, socktype, protocol = nil)
    fd = Kenny::SocketNative.socket(Socket.__kenny_family(domain), Socket.__kenny_socktype(socktype), protocol || 0)
    super(fd, "r+")
    binmode
    self.sync = true
  end

  def connect(addr) = Kenny::SocketNative.connect(fileno, Socket.__kenny_sockaddr(addr), false, true)

  def connect_nonblock(addr, exception: true)
    Kenny::SocketNative.connect(fileno, Socket.__kenny_sockaddr(addr), true, exception)
  end

  def bind(addr) = Kenny::SocketNative.bind(fileno, Socket.__kenny_sockaddr(addr))
  def listen(backlog) = Kenny::SocketNative.listen(fileno, backlog)

  def accept
    fd, sa = Kenny::SocketNative.accept2(fileno, false, true)
    [Socket.for_fd(fd), Addrinfo.__kenny(sa, nil, __kenny_socktype, 0)]
  end

  def accept_nonblock(exception: true)
    r = Kenny::SocketNative.accept2(fileno, true, exception)
    return r if r.is_a?(Symbol)
    [Socket.for_fd(r[0]), Addrinfo.__kenny(r[1], nil, __kenny_socktype, 0)]
  end

  def sysaccept
    fd, sa = Kenny::SocketNative.accept2(fileno, false, true)
    [fd, Addrinfo.__kenny(sa, nil, __kenny_socktype, 0)]
  end

  def ipv6only!
    setsockopt(IPPROTO_IPV6, IPV6_V6ONLY, true) if local_address.ipv6?
  end

  class Option
    attr_reader :family, :level, :optname, :data

    def self.int(family, level, optname, integer) = new(family, level, optname, [integer].pack("i"))
    def self.bool(family, level, optname, bool) = new(family, level, optname, [bool ? 1 : 0].pack("i"))
    def self.byte(family, level, optname, byte) = new(family, level, optname, [byte].pack("C"))

    def self.linger(onoff, secs)
      onoff = onoff == true ? 1 : (onoff == false || onoff.nil? ? 0 : onoff)
      new(:UNSPEC, :SOCKET, :LINGER, [onoff, secs].pack("ii"))
    end

    def initialize(family, level, optname, data)
      @family = Socket.__kenny_family(family)
      @level = Socket.__kenny_level(level)
      @optname = Socket.__kenny_optname(@level, optname)
      @data = data.to_str.b
    end

    def to_s = @data
    def unpack(template) = @data.unpack(template)

    def int
      raise TypeError, "size differ.  expected as sizeof(int)=4 but #{@data.bytesize}" unless @data.bytesize == 4
      @data.unpack1("i")
    end

    def bool
      case @data.bytesize
      when 1 then @data.unpack1("C") != 0
      when 4 then @data.unpack1("i") != 0
      else raise TypeError, "size differ.  expected as sizeof(int)=4 but #{@data.bytesize}"
      end
    end

    def byte
      raise TypeError, "size differ.  expected as sizeof(char)=1 but #{@data.bytesize}" unless @data.bytesize == 1
      @data.unpack1("C")
    end

    def linger
      unless @level == SOL_SOCKET && @optname == SO_LINGER
        raise TypeError, "linger socket option expected"
      end
      onoff, secs = @data.unpack("ii")
      [onoff == 0 ? false : (onoff == 1 ? true : onoff), secs]
    end

    def inspect
      fam = Socket.__kenny_name("AF_", @family) || "family:#{@family}"
      lvl = @level == SOL_SOCKET ? "SOCKET" : (LEVELS.key(@level) || "level:#{@level}")
      prefix, = Socket.__kenny_optprefix(@level)
      opt = (prefix && Socket.__kenny_name(prefix, @optname)) || "optname:#{@optname}"
      value =
        if @level == SOL_SOCKET && @optname == SO_LINGER && @data.bytesize == 8
          onoff, secs = @data.unpack("ii")
          "#{onoff == 0 ? "off" : "on"} #{secs}sec"
        elsif @level == SOL_SOCKET && @optname == SO_TYPE && @data.bytesize == 4
          (n = Socket.__kenny_name("SOCK_", int)) ? "SOCK_#{n}" : int.to_s
        elsif @data.bytesize == 4
          int.to_s
        else
          @data.inspect
        end
      "#<Socket::Option: #{fam} #{lvl} #{opt} #{value}>"
    end
  end
end

class Addrinfo
  attr_reader :afamily, :pfamily, :socktype, :protocol, :canonname

  def self.__kenny(sockaddr, pfamily, socktype, protocol, canonname = nil, inspectname = nil)
    ai = allocate
    ai.__send__(:__kenny_set, sockaddr, pfamily, socktype, protocol, canonname, inspectname)
    ai
  end

  def initialize(sockaddr, family = nil, socktype = nil, protocol = nil)
    iname = nil
    if sockaddr.is_a?(Array)
      if sockaddr[0].to_s == "AF_UNIX"
        sockaddr = Socket.pack_sockaddr_un(sockaddr[1])
      else
        host = sockaddr[2]
        sockaddr = Socket.pack_sockaddr_in(sockaddr[1], sockaddr[3] || host)
        # (a host name other than the address shows in #inspect, as in CRuby)
        iname = host if host.is_a?(String) && host != Socket.unpack_sockaddr_in(sockaddr)[1]
      end
    end
    __kenny_set(sockaddr.to_str, family && Socket.__kenny_family(family), socktype ? Socket.__kenny_socktype(socktype) : 0, protocol || 0, nil, iname)
  end

  def self.getaddrinfo(nodename, service, family = nil, socktype = nil, protocol = nil, flags = nil, timeout: nil)
    fam = family.nil? ? 0 : Socket.__kenny_family(family)
    st = socktype.nil? ? 0 : Socket.__kenny_socktype(socktype)
    iname = nil
    iname = nodename.to_s if nodename.is_a?(String) && !nodename.empty? && !__kenny_numeric_host?(nodename)
    iname = "#{iname}:#{service}" if service.is_a?(String) && !service.match?(/\A\d+\z/)
    Kenny::SocketNative.getaddrinfo(nodename, service, fam, st, protocol || 0, flags || 0).map do |r|
      __kenny(r[7], r[4], r[5], r[6], r[8], iname)
    end
  end

  def self.__kenny_numeric_host?(h)
    h.match?(/\A\d+\.\d+\.\d+\.\d+\z/) || h.include?(":")
  end

  def self.foreach(*args, **kw, &block) = getaddrinfo(*args, **kw).each(&block)
  def self.tcp(host, port) = getaddrinfo(host, port, nil, :STREAM, Socket::IPPROTO_TCP).first
  def self.udp(host, port) = getaddrinfo(host, port, nil, :DGRAM, Socket::IPPROTO_UDP).first

  def self.ip(host)
    ai = getaddrinfo(host, nil, nil, 0, 0).first
    __kenny(ai.to_sockaddr, ai.pfamily, 0, 0, nil, ai.instance_variable_get(:@inspectname))
  end

  def self.unix(path, socktype = :STREAM)
    __kenny(Socket.pack_sockaddr_un(path), Socket::AF_UNIX, Socket.__kenny_socktype(socktype), 0)
  end

  def ip? = afamily == Socket::AF_INET || afamily == Socket::AF_INET6
  def ipv4? = afamily == Socket::AF_INET
  def ipv6? = afamily == Socket::AF_INET6
  def unix? = afamily == Socket::AF_UNIX

  def ip_address
    raise SocketError, "need IPv4 or IPv6 address" unless ip?
    Socket.unpack_sockaddr_in(@sockaddr)[1]
  end

  def ip_port
    raise SocketError, "need IPv4 or IPv6 address" unless ip?
    Socket.unpack_sockaddr_in(@sockaddr)[0]
  end

  def ip_unpack
    raise SocketError, "need IPv4 or IPv6 address" unless ip?
    Socket.unpack_sockaddr_in(@sockaddr).reverse
  end

  def unix_path
    raise SocketError, "need AF_UNIX address" unless unix?
    Socket.unpack_sockaddr_un(@sockaddr)
  end

  def ipv4_loopback? = ipv4? && ip_address.start_with?("127.")
  def ipv6_loopback? = ipv6? && ip_address == "::1"
  def ipv4_private? = ipv4? && ip_address.match?(/\A(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/)
  def ipv4_multicast? = ipv4? && (224..239).cover?(ip_address.to_i)
  def ipv6_linklocal? = ipv6? && ip_address.match?(/\Afe[89ab]/i)
  def ipv6_multicast? = ipv6? && ip_address.match?(/\Aff/i)

  def to_sockaddr = @sockaddr
  def to_s = @sockaddr
  def inspect_sockaddr = Kenny::SocketNative.inspect_sockaddr(@sockaddr)

  def inspect
    s = "#<Addrinfo: #{inspect_sockaddr}"
    s << " #{Socket.__kenny_name("PF_", @pfamily) ? "PF_#{Socket.__kenny_name("PF_", @pfamily)}" : "pfamily:#{@pfamily}"}" if @pfamily != 0 && @pfamily != @afamily
    internet = @pfamily == Socket::AF_INET || @pfamily == Socket::AF_INET6
    if internet && @socktype == Socket::SOCK_STREAM && (@protocol == 0 || @protocol == Socket::IPPROTO_TCP)
      s << " TCP"
    elsif internet && @socktype == Socket::SOCK_DGRAM && (@protocol == 0 || @protocol == Socket::IPPROTO_UDP)
      s << " UDP"
    else
      if @socktype != 0
        n = Socket.__kenny_name("SOCK_", @socktype)
        s << (n ? " SOCK_#{n}" : " socktype:#{@socktype}")
      end
      if @protocol != 0
        n = internet && Socket.__kenny_name("IPPROTO_", @protocol)
        s << (n ? " #{n}" : " protocol:#{@protocol}")
      end
    end
    s << " #{@canonname}" if @canonname
    s << " (#{@inspectname})" if @inspectname
    s << ">"
  end

  def connect(timeout: nil, &block)
    sock = Socket.new(pfamily, socktype, protocol)
    begin
      if timeout
        begin
          sock.connect_nonblock(self)
        rescue IO::WaitWritable
          raise Errno::ETIMEDOUT, "user specified timeout" unless sock.wait_writable(timeout)
          begin
            sock.connect_nonblock(self)
          rescue Errno::EISCONN
          end
        end
      else
        sock.connect(self)
      end
    rescue Exception
      sock.close
      raise
    end
    return sock unless block
    begin
      yield sock
    ensure
      sock.close unless sock.closed?
    end
  end

  def bind
    sock = Socket.new(pfamily, socktype, protocol)
    sock.setsockopt(:SOCKET, :REUSEADDR, 1) if ip?
    sock.bind(self)
    return sock unless block_given?
    begin
      yield sock
    ensure
      sock.close unless sock.closed?
    end
  end

  def listen(backlog = Socket::SOMAXCONN)
    sock = Socket.new(pfamily, socktype, protocol)
    sock.setsockopt(:SOCKET, :REUSEADDR, 1) if ip?
    sock.bind(self)
    sock.listen(backlog)
    return sock unless block_given?
    begin
      yield sock
    ensure
      sock.close unless sock.closed?
    end
  end

  private

  def __kenny_set(sockaddr, pfamily, socktype, protocol, canonname, inspectname)
    @sockaddr = sockaddr.b
    @afamily = Kenny::SocketNative.sockaddr_family(@sockaddr)
    @pfamily = pfamily || @afamily
    @socktype = socktype || 0
    @protocol = protocol || 0
    @canonname = canonname
    @inspectname = inspectname
  end
end

class UNIXSocket < BasicSocket
  def self.pair(type = nil, protocol = nil)
    st = type.nil? ? Socket::SOCK_STREAM : Socket.__kenny_socktype(type)
    Kenny::SocketNative.socketpair(Socket::AF_UNIX, st, protocol || 0).map { |fd| for_fd(fd) }
  end

  def self.socketpair(type = nil, protocol = nil) = pair(type, protocol)

  def initialize(path)
    path = path.to_path if path.respond_to?(:to_path)
    fd = Kenny::SocketNative.socket(Socket::AF_UNIX, Socket::SOCK_STREAM, 0)
    begin
      Kenny::SocketNative.connect(fd, Socket.pack_sockaddr_un(path), false, true, "connect(2) for #{path}")
    rescue Exception
      IO.for_fd(fd).close
      raise
    end
    __kenny_io_init(fd, "r+")
    binmode
    self.sync = true
  end

  def path = @path || Socket.unpack_sockaddr_un(getsockname)
  def addr = ["AF_UNIX", path]
  def peeraddr = ["AF_UNIX", Socket.unpack_sockaddr_un(getpeername)]

  def recvfrom(maxlen, flags = 0)
    [recv(maxlen, flags), ["AF_UNIX", ""]]
  end

  def send_io(_io) = raise(NotImplementedError, "UNIXSocket#send_io is not supported by Kenny's socket library")
  def recv_io(*) = raise(NotImplementedError, "UNIXSocket#recv_io is not supported by Kenny's socket library")
end

class UNIXServer < UNIXSocket
  def initialize(path)
    path = path.to_path if path.respond_to?(:to_path)
    fd = Kenny::SocketNative.socket(Socket::AF_UNIX, Socket::SOCK_STREAM, 0)
    begin
      # (CRuby names connect(2) here too)
      Kenny::SocketNative.bind(fd, Socket.pack_sockaddr_un(path), "connect(2) for #{path}")
      Kenny::SocketNative.listen(fd, Socket::SOMAXCONN)
    rescue Exception
      IO.for_fd(fd).close
      raise
    end
    __kenny_io_init(fd, "r+")
    binmode
    self.sync = true
    @path = path
  end

  def listen(backlog) = Kenny::SocketNative.listen(fileno, backlog)
  def accept = UNIXSocket.for_fd(Kenny::SocketNative.accept2(fileno, false, true)[0])
  def sysaccept = Kenny::SocketNative.accept2(fileno, false, true)[0]

  def accept_nonblock(exception: true)
    r = Kenny::SocketNative.accept2(fileno, true, exception)
    r.is_a?(Symbol) ? r : UNIXSocket.for_fd(r[0])
  end
end
