# Drives the built-in HTTP server of a Kenny binary (tests/server/app/config.ru) over raw TCP
# sockets, so the tests control the exact bytes and their timing. Run by tests/server/run.sh:
#
#   ruby tests/server/test.rb BINARY [pattern]    (only the tests whose name matches pattern)
#
# Every server is started on a free port in its own process group and killed afterwards.
require "socket"
require "tmpdir"
require "fileutils"

BIN = File.expand_path(ARGV[0] || abort("usage: test.rb BINARY [pattern]"))
PATTERN = ARGV[1] && Regexp.new(ARGV[1])

class Failure < StandardError; end

def check(cond, msg)
  raise Failure, msg unless cond
end

def check_eq(want, got, what)
  raise Failure, "#{what}: expected #{want.inspect}, got #{got.inspect}" unless want == got
end

def now = Process.clock_gettime(Process::CLOCK_MONOTONIC)

Response = Struct.new(:status, :headers, :body, :head) do
  def [](k) = headers[k.downcase]
end

# A client connection: reads with deadlines (nothing here blocks forever).
class Client
  attr_reader :sock

  def initialize(port)
    @sock = TCPSocket.new("127.0.0.1", port)
    @sock.setsockopt(Socket::IPPROTO_TCP, Socket::TCP_NODELAY, 1)
    @buf = "".b
    @eof = false
    ($clients ||= []) << self
  end

  def write(s)
    @sock.write(s)
    self
  end

  def close
    @sock.close unless @sock.closed?
  end

  # More bytes into the buffer; false at EOF, nil when the deadline passed.
  def fill(deadline)
    return false if @eof
    left = deadline - now
    return nil if left <= 0
    return nil unless IO.select([@sock], nil, nil, left)
    @buf << @sock.read_nonblock(1 << 20)
    true
  rescue IO::WaitReadable
    true
  rescue EOFError, Errno::ECONNRESET
    @eof = true
    false
  end

  def read_until(delim, timeout)
    deadline = now + timeout
    until (i = @buf.index(delim))
      r = fill(deadline)
      raise Failure, "connection closed waiting for #{delim.inspect} (got #{@buf[0, 200].inspect})" if r == false
      raise Failure, "timed out waiting for #{delim.inspect} (got #{@buf[0, 200].inspect})" if r.nil?
    end
    @buf.slice!(0, i + delim.bytesize)
  end

  def read_n(n, timeout)
    deadline = now + timeout
    while @buf.bytesize < n
      r = fill(deadline)
      raise Failure, "connection closed after #{@buf.bytesize} of #{n} bytes" if r == false
      raise Failure, "timed out after #{@buf.bytesize} of #{n} bytes" if r.nil?
    end
    @buf.slice!(0, n)
  end

  # One response (a 1xx is a response of its own).
  def response(timeout = 5, no_body: false)
    head = read_until("\r\n\r\n", timeout)
    lines = head.split("\r\n")
    status = lines.shift[/\AHTTP\/1\.[01] (\d{3})/, 1] or raise Failure, "bad status line in #{head.inspect}"
    headers = {}
    lines.each do |l|
      k, v = l.split(":", 2)
      headers[k.downcase] = v.to_s.strip
    end
    code = status.to_i
    body =
      if no_body || (100..199).cover?(code) || code == 204 || code == 304
        ""
      elsif headers["content-length"]
        read_n(headers["content-length"].to_i, timeout)
      else
        read_to_eof(timeout)
      end
    Response.new(code, headers, body, head)
  end

  # What comes until the server closes the connection.
  def read_to_eof(timeout)
    deadline = now + timeout
    loop do
      r = fill(deadline)
      raise Failure, "connection not closed within #{timeout}s" if r.nil?
      break if r == false
    end
    @buf.slice!(0, @buf.bytesize)
  end

  # Seconds until the server closes the connection (nil if it doesn't within timeout); bytes
  # that came meanwhile are in `pending`.
  def closed_after(timeout)
    t0 = now
    deadline = t0 + timeout
    loop do
      r = fill(deadline)
      return nil if r.nil?
      return now - t0 if r == false
    end
  end

  # Whether any byte arrives within timeout.
  def silent?(timeout)
    deadline = now + timeout
    while @buf.empty?
      r = fill(deadline)
      return true if r.nil?
      return @buf.empty? if r == false
    end
    false
  end

  def pending = @buf
  def eof? = @eof
end

# A running server binary.
class Server
  attr_reader :port, :pid, :tmpdir

  def self.free_port
    s = TCPServer.new("127.0.0.1", 0)
    s.addr[1]
  ensure
    s&.close
  end

  def initialize(env)
    @tmpdir = Dir.mktmpdir("kenny-server-test")
    @port = Server.free_port
    r, w = IO.pipe
    full = { "HOST" => "127.0.0.1", "PORT" => @port.to_s, "TMPDIR" => @tmpdir + "/" }.merge(env)
    @pid = Process.spawn(full, BIN, out: w, err: w, in: File::NULL, pgroup: true)
    w.close
    @log = +""
    deadline = now + 15
    until @log.include?("Kenny serving")
      left = deadline - now
      raise Failure, "server didn't start: #{@log}" if left <= 0 || !IO.select([r], nil, nil, left)
      begin
        @log << r.read_nonblock(65_536)
      rescue EOFError
        raise Failure, "server exited: #{@log}"
      rescue IO::WaitReadable
      end
    end
    # (the server's output, kept for a failure's report; the pipe must not fill up)
    @reader = Thread.new do
      loop { @log << r.readpartial(65_536) }
    rescue EOFError, IOError
    end
  end

  attr_reader :log

  def client = Client.new(@port)

  # One request on a connection of its own.
  def get(path, timeout: 5)
    c = client
    c.write("GET #{path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
    c.response(timeout)
  ensure
    c&.close
  end

  # The server's open descriptors (asked on a connection of its own); with `want`, waits up
  # to 2 s for the count to settle there (connections the clients closed are closed by the
  # server when it notices).
  def fds(want = nil)
    deadline = now + 2
    loop do
      n = get("/fds").body.to_i
      return n if want.nil? || n == want || now > deadline
      sleep 0.02
    end
  end

  # The processes WEB_CONCURRENCY forked (they come after the "serving" line: waits up to 2 s
  # for at least `n`).
  def children(n = 2)
    deadline = now + 2
    loop do
      pids = `pgrep -P #{@pid}`.split.map(&:to_i)
      return pids if pids.size >= n || now > deadline
      sleep 0.02
    end
  end

  def stop
    begin
      Process.kill("KILL", -@pid)
    rescue Errno::ESRCH, Errno::EPERM
    end
    Process.wait(@pid) rescue nil
    @reader&.join(1)
    FileUtils.rm_rf(@tmpdir)
  end
end

# A socket the app's /wait requests connect to: lets a test hold a request inside the app for
# as long as it wants, and know when it got there.
class Gate
  def initialize
    @server = TCPServer.new("127.0.0.1", 0)
  end

  def port = @server.addr[1]

  # The app's connection: it is now inside the request.
  def entered(timeout = 5)
    IO.select([@server], nil, nil, timeout) or raise Failure, "request didn't reach the app"
    @server.accept
  end

  def close = @server.close
end

$tests = []
$groups = []
def group(name, env = {}, &blk)
  $groups << [name, env, blk]
end

def test(name, &blk)
  $tests << [name, blk]
end

def request(method, path, body = nil, headers = {})
  h = +"#{method} #{path} HTTP/1.1\r\nHost: test\r\n"
  h << "Content-Length: #{body.bytesize}\r\n" if body && !headers.keys.any? { |k| k.downcase == "transfer-encoding" }
  headers.each { |k, v| h << "#{k}: #{v}\r\n" }
  h << "\r\n"
  h << body if body
  h
end

def chunked(*chunks, trailers: "")
  chunks.map { |c| "#{c.bytesize.to_s(16)}\r\n#{c}\r\n" }.join + "0\r\n#{trailers}\r\n"
end

# ---------------------------------------------------------------------------------------------

group "basics" do |s|
  test "GET" do
    r = s.get("/hello")
    check_eq 200, r.status, "status"
    check_eq "hello GET", r.body, "body"
    check_eq "close", r["connection"], "connection header"
  end

  test "POST echoes its body" do
    c = s.client
    c.write(request("POST", "/echo", "a=1&b=2", "Content-Type" => "application/x-www-form-urlencoded"))
    r = c.response
    check_eq 200, r.status, "status"
    check_eq "a=1&b=2", r.body, "body"
    check_eq "7", r["x-content-length"], "CONTENT_LENGTH"
    check_eq "false", r["x-io"], "rack.input is a StringIO"
  end

  test "env" do
    c = s.client
    c.write("GET /env?x=1&y=2 HTTP/1.1\r\nHost: example.com:8080\r\nX-One: 1\r\n\r\n")
    r = c.response
    want = "REQUEST_METHOD=GET\nPATH_INFO=/env\nQUERY_STRING=x=1&y=2\nSERVER_PROTOCOL=HTTP/1.1\nSERVER_NAME=example.com\nCONTENT_TYPE=\nHTTP_X_ONE=1"
    check_eq want, r.body, "env"
  end

  test "HEAD has no body" do
    c = s.client
    c.write("HEAD /hello HTTP/1.1\r\nHost: test\r\n\r\nGET /hello HTTP/1.1\r\nHost: test\r\n\r\n")
    r = c.response(no_body: true)
    check_eq 200, r.status, "HEAD status"
    check_eq "10", r["content-length"], "HEAD content-length"
    r = c.response
    check_eq "hello GET", r.body, "the next response follows the HEAD's headers directly"
  end

  test "keep-alive: several requests on one connection" do
    c = s.client
    5.times do |i|
      c.write(request("POST", "/echo", "req #{i}"))
      r = c.response
      check_eq "keep-alive", r["connection"], "connection header"
      check_eq "req #{i}", r.body, "body #{i}"
    end
    c.write("GET /hello HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
    check_eq "hello GET", c.response.body, "last body"
    check c.closed_after(2), "closed after Connection: close"
  end

  test "pipelined requests" do
    c = s.client
    reqs = request("GET", "/hello") + request("POST", "/echo", "one") + request("POST", "/echo", "two" * 1000) +
           request("HEAD", "/hello") + request("POST", "/echo", "last", "Connection" => "close") + request("GET", "/hello")
    c.write(reqs)
    check_eq "hello GET", c.response.body, "1st"
    check_eq "one", c.response.body, "2nd"
    check_eq "two" * 1000, c.response.body, "3rd"
    check_eq "", c.response(no_body: true).body, "4th (HEAD)"
    r = c.response
    check_eq "last", r.body, "5th"
    check_eq "close", r["connection"], "5th connection"
    check_eq "", c.read_to_eof(2), "nothing answered after Connection: close"
  end

  test "pipelined requests split mid-request" do
    c = s.client
    reqs = request("POST", "/echo", "abc") + request("GET", "/hello") + request("POST", "/echo", "xyz")
    reqs.bytes.each_slice(7) { |b| c.write(b.pack("C*")) }
    check_eq "abc", c.response.body, "1st"
    check_eq "hello GET", c.response.body, "2nd"
    check_eq "xyz", c.response.body, "3rd"
  end

  test "HTTP/1.0 closes unless keep-alive" do
    c = s.client
    c.write("GET /hello HTTP/1.0\r\n\r\n")
    r = c.response
    check_eq "close", r["connection"], "1.0 connection"
    check c.closed_after(2), "1.0 closed"
    c = s.client
    c.write("GET /hello HTTP/1.0\r\nConnection: keep-alive\r\n\r\n")
    check_eq "keep-alive", c.response["connection"], "1.0 keep-alive"
    c.write("GET /hello HTTP/1.0\r\nConnection: keep-alive\r\n\r\n")
    check_eq "hello GET", c.response.body, "2nd 1.0 request"
  end

  test "a request sent while the previous one runs" do
    gate = Gate.new
    c = s.client
    c.write(request("GET", "/wait?#{gate.port}"))
    ctl = gate.entered
    c.write(request("POST", "/echo", "second"))
    check c.silent?(0.1), "nothing answered while the first runs"
    ctl.write("go\n")
    check c.response.body.start_with?("waited go"), "1st"
    check_eq "second", c.response.body, "2nd"
    ctl.close
    gate.close
  end

  test "a client that closes its side still gets its responses" do
    c = s.client
    c.write(request("POST", "/echo", "half") + request("GET", "/sleep?0.1"))
    c.sock.close_write
    check_eq "half", c.response.body, "1st"
    check_eq "slept", c.response.body, "2nd"
    check c.closed_after(2), "then closed"
  end

  test "chunked in HTTP/1.0 -> 400" do
    c = s.client
    c.write("POST /echo HTTP/1.0\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n")
    check_eq 400, c.response.status, "status"
  end

  test "a raising app answers 500 and keeps serving" do
    c = s.client
    c.write(request("GET", "/raise"))
    check_eq 500, c.response.status, "status"
    c.write(request("GET", "/hello"))
    check_eq "hello GET", c.response.body, "next request"
  end

  test "malformed request line -> 400" do
    c = s.client
    c.write("BLAH\0 / HTTP/9\r\n\r\n")
    check_eq 400, c.response.status, "status"
    check c.closed_after(2), "closed"
  end

  test "request line and headers up to 112 KB are fine" do
    c = s.client
    c.write("GET /headers HTTP/1.1\r\nHost: test\r\nX-Big: #{"a" * (110 * 1024)}\r\n\r\n")
    r = c.response
    check_eq 200, r.status, "status"
    check_eq "2", r.body, "HTTP_ headers"
  end

  test "request line and headers over 112 KB -> 431" do
    c = s.client
    c.write("GET /headers HTTP/1.1\r\nHost: test\r\nX-Big: #{"a" * (113 * 1024)}\r\n\r\n")
    check_eq 431, c.response.status, "status"
    check c.closed_after(2), "closed"
  end

  test "an endless header -> 431" do
    c = s.client
    c.write("GET /headers HTTP/1.1\r\nHost: test\r\nX-Big: ")
    begin
      200.times { c.write("a" * 1024) }
    rescue Errno::EPIPE, Errno::ECONNRESET
    end
    check_eq 431, c.response.status, "status"
  end

  test "a long URL over 112 KB -> 431" do
    c = s.client
    c.write("GET /hello?#{"q" * (115 * 1024)} HTTP/1.1\r\nHost: test\r\n\r\n")
    check_eq 431, c.response.status, "status"
  end

  test "more than 128 headers -> 431" do
    c = s.client
    c.write("GET /headers HTTP/1.1\r\n" + (1..130).map { |i| "X-H#{i}: v\r\n" }.join + "\r\n")
    check_eq 431, c.response.status, "status"
    c = s.client
    c.write("GET /headers HTTP/1.1\r\n" + (1..120).map { |i| "X-H#{i}: v\r\n" }.join + "\r\n")
    check_eq "120", c.response.body, "120 headers"
  end

  test "no body limit by default" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 10000000000\r\nExpect: 100-continue\r\n\r\n")
    check_eq 100, c.response.status, "10 GB announced: continue"
  end

  test "Content-Length and Transfer-Encoding checks" do
    [
      ["Content-Length: 3\r\nContent-Length: 4", 400],
      ["Content-Length: -1", 400],
      ["Content-Length: 1,1", 400],
      ["Content-Length: 3\r\nTransfer-Encoding: chunked", 400],
      ["Transfer-Encoding: gzip", 501],
      ["Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked", 501],
    ].each do |hdr, code|
      c = s.client
      c.write("POST /echo HTTP/1.1\r\nHost: test\r\n#{hdr}\r\n\r\nabc")
      check_eq code, c.response.status, hdr.inspect
    end
  end
end

group "chunked bodies" do |s|
  test "chunked body with trailers" do
    c = s.client
    body = chunked("hello", " ", "chunked world", trailers: "X-Checksum: 42\r\nX-More: 1\r\n")
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n#{body}")
    r = c.response
    check_eq 200, r.status, "status"
    check_eq "hello chunked world", r.body, "body"
    check_eq "19", r["x-content-length"], "CONTENT_LENGTH given to the app"
    check_eq "", r["x-transfer-encoding"], "Transfer-Encoding hidden from the app"
    c.write(request("GET", "/hello"))
    check_eq "hello GET", c.response.body, "next request on the connection"
  end

  test "odd chunk sizes, extensions, upper-case hex, byte by byte" do
    c = s.client
    parts = ["a", "b" * 15, "c" * 16, "d" * 255, "e" * 4097]
    body = parts.each_with_index.map { |p, i| "#{p.bytesize.to_s(16).upcase}#{i.odd? ? ";ext=#{i}" : ""}\r\n#{p}\r\n" }.join + "0\r\n\r\n"
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n")
    body[0, 300].each_char { |ch| c.write(ch) }
    c.write(body[300..])
    r = c.response
    check_eq parts.join, r.body, "body"
    check_eq parts.join.bytesize.to_s, r["x-content-length"], "CONTENT_LENGTH"
  end

  test "empty chunked body" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n")
    r = c.response
    check_eq "", r.body, "body"
    check_eq "0", r["x-content-length"], "CONTENT_LENGTH"
  end

  test "pipelined after a chunked body" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\nX-T: 1\r\n\r\n" + request("POST", "/echo", "next"))
    check_eq "abc", c.response.body, "1st"
    check_eq "next", c.response.body, "2nd"
  end

  test "malformed chunks -> 400" do
    ["zz\r\nhello\r\n0\r\n\r\n",             # not hex
     "5\r\nhelloXX0\r\n\r\n",                 # no CRLF after the data
     "\r\nhello\r\n0\r\n\r\n",                # empty size
     "fffffffffffffffff\r\nx\r\n0\r\n\r\n",   # size too long
     "-5\r\nhello\r\n0\r\n\r\n"].each do |bad|
      c = s.client
      c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n#{bad}")
      check_eq 400, c.response.status, bad.inspect
      check c.closed_after(2), "closed after #{bad.inspect}"
    end
  end

  test "a chunk size line without end -> 400" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n" + "0" * 5000)
    check_eq 400, c.response.status, "status"
  end

  test "trailers over 112 KB -> 431" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n0\r\n")
    begin
      120.times { |i| c.write("X-T#{i}: #{"t" * 1024}\r\n") }
    rescue Errno::EPIPE, Errno::ECONNRESET
    end
    check_eq 431, c.response.status, "status"
  end
end

group "spooled bodies" do |s|
  big = Random.new(42).bytes(3 << 20)

  test "a body over 1 MB goes to a temporary file" do
    c = s.client
    c.write(request("POST", "/echo", big))
    r = c.response(20)
    check_eq 200, r.status, "status"
    check_eq "true", r["x-io"], "rack.input is an IO"
    check_eq big.bytesize.to_s, r["x-content-length"], "CONTENT_LENGTH"
    check r.body == big, "body intact (#{r.body.bytesize} bytes)"
    check_eq "0", r["x-spool-files"], "the temporary file has no name"
    c.write(request("GET", "/input-closed"))
    check_eq "true", c.response.body, "rack.input closed after the response"
    check_eq [], Dir.children(s.tmpdir), "temporary directory empty"
  end

  test "exactly 1 MB stays in memory" do
    c = s.client
    body = big[0, 1 << 20]
    c.write(request("POST", "/echo", body))
    r = c.response(20)
    check_eq "false", r["x-io"], "rack.input is a StringIO"
    check r.body == body, "body intact"
  end

  test "spooled body read in pieces" do
    c = s.client
    c.write(request("POST", "/echo?pieces", big))
    r = c.response(20)
    check_eq "true", r["x-io"], "rack.input is an IO"
    check r.body == big, "body intact (#{r.body.bytesize} bytes)"
  end

  test "chunked body over 1 MB" do
    c = s.client
    sizes = [1, 65_535, 1 << 20, 3, 900_001, 777_777]
    parts = []
    pos = 0
    sizes.each { |n| parts << big[pos, n]; pos += n }
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n" + chunked(*parts, trailers: "X-Done: yes\r\n"))
    r = c.response(20)
    check_eq "true", r["x-io"], "rack.input is an IO"
    check_eq pos.to_s, r["x-content-length"], "CONTENT_LENGTH"
    check r.body == big[0, pos], "body intact"
  end

  test "no descriptors left open (temporary files closed)" do
    base = s.fds
    3.times do
      c = s.client
      c.write(request("POST", "/echo", big, "Connection" => "close"))
      check_eq "true", c.response(20)["x-io"], "spooled"
      c.close
    end
    check_eq base, s.fds(base), "open descriptors after 3 spooled requests"
  end

  test "a client that goes away mid-upload leaves no file open" do
    base = s.fds
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: #{big.bytesize}\r\n\r\n" + big[0, 2 << 20])
    check c.silent?(0.2), "no answer mid-upload"
    c.close
    check_eq base, s.fds(base), "open descriptors"
    check_eq [], Dir.children(s.tmpdir), "temporary directory empty"
  end
end

group "Expect: 100-continue" do |s|
  test "answered before the body is sent" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\n")
    r = c.response
    check_eq 100, r.status, "interim status"
    c.write("hello")
    r = c.response
    check_eq 200, r.status, "final status"
    check_eq "hello", r.body, "body"
  end

  test "chunked with Expect" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\nExpect: 100-Continue\r\n\r\n")
    check_eq 100, c.response.status, "interim status"
    c.write(chunked("abc", "de"))
    check_eq "abcde", c.response.body, "body"
  end

  test "no 100 when the body came along, or there is none" do
    c = s.client
    c.write(request("POST", "/echo", "hello", "Expect" => "100-continue"))
    r = c.response
    check_eq 200, r.status, "status with the body sent at once"
    check_eq "hello", r.body, "body"
    c.write(request("GET", "/hello", nil, "Expect" => "100-continue"))
    check_eq 200, c.response.status, "status without a body"
  end

  test "no 100 for HTTP/1.0" do
    c = s.client
    c.write("POST /echo HTTP/1.0\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\n")
    check c.silent?(0.3), "no interim response"
    c.write("hello")
    check_eq "hello", c.response.body, "body"
  end
end

group "KENNY_MAX_BODY=100", "KENNY_MAX_BODY" => "100" do |s|
  test "plain bytes" do
    c = s.client
    c.write(request("POST", "/echo", "x" * 100))
    check_eq "x" * 100, c.response.body, "100 bytes taken"
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 101\r\n\r\n")
    check_eq 413, c.response.status, "101 bytes"
    check c.closed_after(2), "closed"
  end

  test "413 reaches a client that sends its body anyway" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 300000\r\n\r\n")
    begin
      30.times { c.write("b" * 10_000) }
    rescue Errno::EPIPE, Errno::ECONNRESET
    end
    check_eq 413, c.response.status, "status"
  end

  test "413 instead of 100 Continue" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 5000\r\nExpect: 100-continue\r\n\r\n")
    check_eq 413, c.response.status, "status"
  end

  test "chunked body over the limit" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n" + chunked("y" * 60, "y" * 40))
    check_eq "y" * 100, c.response.body, "100 bytes in chunks"
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nTransfer-Encoding: chunked\r\n\r\n" + chunked("y" * 60, "y" * 41))
    check_eq 413, c.response.status, "101 bytes in chunks"
  end
end

group "KENNY_MAX_BODY=2k", "KENNY_MAX_BODY" => "2k" do |s|
  test "K suffix" do
    c = s.client
    c.write(request("POST", "/echo", "k" * 2048))
    check_eq 2048, c.response.body.bytesize, "2048 bytes taken"
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 2049\r\n\r\n")
    check_eq 413, c.response.status, "2049 bytes"
  end
end

group "KENNY_MAX_BODY=3M", "KENNY_MAX_BODY" => " 3M " do |s|
  test "M suffix (and a spooled body under it)" do
    c = s.client
    body = "m" * (3 << 20)
    c.write(request("POST", "/echo", body))
    r = c.response(20)
    check_eq "true", r["x-io"], "spooled"
    check r.body == body, "body intact"
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: #{(3 << 20) + 1}\r\n\r\n")
    check_eq 413, c.response.status, "3 MB + 1"
  end
end

group "KENNY_MAX_BODY=1G", "KENNY_MAX_BODY" => "1G" do |s|
  test "G suffix" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: #{1 << 30}\r\nExpect: 100-continue\r\n\r\n")
    check_eq 100, c.response.status, "1 GB: continue"
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: #{(1 << 30) + 1}\r\nExpect: 100-continue\r\n\r\n")
    check_eq 413, c.response.status, "1 GB + 1"
  end
end

TIMEOUTS = { "KENNY_READ_TIMEOUT" => "0.5", "KENNY_KEEPALIVE_TIMEOUT" => "0.5" }
group "timeouts (0.5 s)", TIMEOUTS do |s|
  # (the server looks for expired connections once a second: a timeout of 0.5 s closes after
  # 0.5 to 1.5 s)
  test "headers that don't arrive in time -> 408" do
    c = s.client
    c.write("GET /hello HTTP/1.1\r\nHost: te")
    t = c.closed_after(4)
    check t, "closed"
    check t >= 0.45, "not before the timeout (#{t.round(2)} s)"
    check c.pending.start_with?("HTTP/1.1 408 "), "408 sent (got #{c.pending[0, 40].inspect})"
  end

  test "headers trickling in -> 408" do
    c = s.client
    t0 = now
    begin
      "GET /hello HTTP/1.1\r\nHost: test\r\nX-Slow: #{"s" * 40}".each_char do |ch|
        c.write(ch)
        break if c.silent?(0.05) == false
      end
    rescue Errno::EPIPE, Errno::ECONNRESET
    end
    t = c.closed_after(4)
    check t, "closed"
    check now - t0 < 3, "closed while still trickling (#{(now - t0).round(2)} s)"
    check c.pending.start_with?("HTTP/1.1 408 "), "408 sent (got #{c.pending[0, 40].inspect})"
  end

  test "a stalled upload is cut off" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 100\r\n\r\n" + "u" * 10)
    t = c.closed_after(4)
    check t, "closed"
    check c.pending.start_with?("HTTP/1.1 408 "), "408 sent (got #{c.pending[0, 40].inspect})"
  end

  test "a slow but moving upload is not cut off" do
    c = s.client
    c.write("POST /echo HTTP/1.1\r\nHost: test\r\nContent-Length: 10\r\n\r\n")
    10.times do
      check c.silent?(0.2), "nothing answered mid-upload"
      c.write("v")
    end
    check_eq "v" * 10, c.response.body, "body after 2 s"
  end

  test "a connection that sends nothing is closed silently" do
    c = s.client
    t = c.closed_after(4)
    check t, "closed"
    check_eq "", c.pending, "no response"
  end

  test "an idle keep-alive connection is closed" do
    c = s.client
    c.write(request("GET", "/hello"))
    check_eq "hello GET", c.response.body, "1st"
    check c.silent?(0.3), "still open after 0.3 s"
    c.write(request("GET", "/hello"))
    check_eq "hello GET", c.response.body, "2nd after 0.3 s idle"
    t = c.closed_after(4)
    check t, "closed"
    check t >= 0.45, "not before the keep-alive timeout (#{t.round(2)} s)"
    check_eq "", c.pending, "no response"
  end

  test "a running request is not timed" do
    c = s.client
    c.write(request("GET", "/sleep?1.6"))
    r = c.response(5)
    check_eq "slept", r.body, "body"
  end

  test "a response the client doesn't read is cut off" do
    c = s.client
    n = 64 << 20
    c.write(request("GET", "/big?#{n}"))
    # (not reading: the server's writes stall)
    sleep 2
    got = c.read_to_eof(10)
    check got.bytesize < n, "cut off (#{got.bytesize} of #{n} bytes)"
  end

  test "timed-out connections leave no descriptors open" do
    base = s.fds
    cs = 3.times.map { s.client }
    cs[0].write("GET /hello HTTP/1.1\r\n")
    cs[1].write("POST /echo HTTP/1.1\r\nContent-Length: 5\r\n\r\nab")
    cs.each { |c| check c.closed_after(4), "closed" }
    check_eq base, s.fds(base), "open descriptors"
  end
end

group "keep-alive timeout only", "KENNY_READ_TIMEOUT" => "0", "KENNY_KEEPALIVE_TIMEOUT" => "0.5" do |s|
  test "KENNY_READ_TIMEOUT=0: no head timeout" do
    c = s.client
    c.write("GET /hello HTTP/1.1\r\n")
    check c.silent?(2.2), "still waiting after 2.2 s"
    c.write("Host: test\r\n\r\n")
    check_eq "hello GET", c.response.body, "body"
    check c.closed_after(4), "then closed by the keep-alive timeout"
  end
end

def concurrency_tests(s, name)
  test "#{name}: a request waiting on a socket doesn't block others" do
    gate = Gate.new
    a = s.client
    a.write(request("GET", "/wait?#{gate.port}"))
    ctl = gate.entered
    5.times { |i| check_eq "hello GET", s.get("/hello", timeout: 3).body, "request #{i} while one waits" }
    ctl.write("go\n")
    check a.response.body.start_with?("waited go "), "the waiting request"
    ctl.close
    gate.close
  end

  test "#{name}: sleeping requests run at once" do
    cs = 4.times.map { s.client.write(request("GET", "/sleep?0.6")) }
    t0 = now
    cs.each { |c| check_eq "slept", c.response(5).body, "slept" }
    check now - t0 < 1.8, "4 requests sleeping 0.6 s took #{(now - t0).round(2)} s"
  end

  test "#{name}: many connections at once" do
    cs = 50.times.map { s.client }
    cs.each_with_index { |c, i| c.write(request("POST", "/echo", "c#{i}")) }
    cs.each_with_index { |c, i| check_eq "c#{i}", c.response.body, "connection #{i}" }
  end

  test "#{name}: pipelined requests on many connections" do
    cs = 20.times.map { s.client }
    cs.each_with_index { |c, i| c.write(5.times.map { |j| request("POST", "/echo", "c#{i}r#{j}") }.join) }
    cs.each_with_index { |c, i| 5.times { |j| check_eq "c#{i}r#{j}", c.response.body, "connection #{i} request #{j}" } }
  end

  test "#{name}: large uploads at once" do
    bodies = 4.times.map { |i| Random.new(i).bytes(1_500_000 + i) }
    cs = bodies.map { s.client }
    # (interleaved, so they arrive together)
    reqs = bodies.map { |b| request("POST", "/echo", b) }
    0.step(reqs[0].bytesize + 4, 100_000) { |pos| cs.each_with_index { |c, i| c.write(reqs[i].byteslice(pos, 100_000)) } }
    cs.each_with_index { |c, i| check c.response(20).body == bodies[i], "body #{i} intact" }
  end
end

group "request threads (KENNY_THREADS=1, 4 request threads)", "KENNY_THREADS" => "1", "KENNY_SERVER_THREADS" => "4" do |s|
  concurrency_tests(s, "1x4")
end

group "one request thread", "KENNY_THREADS" => "1", "KENNY_SERVER_THREADS" => "1" do |s|
  test "requests beyond the pool wait for a thread" do
    gate = Gate.new
    a = s.client
    a.write(request("GET", "/wait?#{gate.port}"))
    ctl = gate.entered
    b = s.client
    b.write(request("GET", "/hello"))
    check b.silent?(0.3), "the second request waits"
    ctl.write("go\n")
    check a.response.body.start_with?("waited go"), "the first"
    check_eq "hello GET", b.response.body, "then the second"
    ctl.close
    gate.close
  end
end

group "request threads (default)" do |s|
  concurrency_tests(s, "default")
end

group "KENNY_SERVER_THREADS=0, KENNY_THREADS=1", "KENNY_THREADS" => "1", "KENNY_SERVER_THREADS" => "0" do |s|
  test "requests are served" do
    c = s.client
    3.times { c.write(request("POST", "/echo", "zero")); check_eq "zero", c.response.body, "body" }
  end

  test "requests run one at a time" do
    gate = Gate.new
    a = s.client
    a.write(request("GET", "/wait?#{gate.port}"))
    ctl = gate.entered
    b = s.client
    b.write(request("GET", "/hello"))
    check b.silent?(0.3), "the second request waits for the first"
    ctl.write("go\n")
    check a.response.body.start_with?("waited go"), "the first"
    check_eq "hello GET", b.response.body, "then the second"
    ctl.close
    gate.close
  end
end

group "KENNY_SERVER_THREADS=0 (every core)", "KENNY_SERVER_THREADS" => "0" do |s|
  test "requests are served on every loop" do
    cs = 20.times.map { s.client }
    cs.each_with_index { |c, i| c.write(request("POST", "/echo", "z#{i}")) }
    cs.each_with_index { |c, i| check_eq "z#{i}", c.response.body, "connection #{i}" }
  end
end

def hijack_tests(s)
  test "full hijack" do
    c = s.client
    c.write(request("GET", "/hijack", nil, "Upgrade" => "echo"))
    r = c.response
    check_eq 101, r.status, "status"
    check_eq "echo", r["upgrade"], "upgrade header"
    c.write("ping\n")
    check_eq "echo: ping\n", c.read_to_eof(3), "the app's answer, then closed"
  end

  test "full hijack served by a thread after the request returned" do
    c = s.client
    c.write(request("GET", "/hijack-thread"))
    check_eq 101, c.response.status, "status"
    c.write("one\n")
    check_eq "thread: one\n", c.read_until("\n", 3), "1st line"
    c.write("two\n")
    check_eq "thread: two\n", c.read_until("\n", 3), "2nd line"
    check c.closed_after(3), "closed by the app"
  end

  test "partial hijack" do
    c = s.client
    c.write(request("GET", "/partial"))
    head = c.read_until("\r\n\r\n", 3)
    check head.start_with?("HTTP/1.1 200 OK\r\n"), "status line: #{head.inspect}"
    check head.downcase.include?("content-type: text/plain\r\n"), "headers: #{head.inspect}"
    check !head.downcase.include?("rack.hijack"), "no rack.hijack header: #{head.inspect}"
    check_eq "partial ", c.read_n(8, 3), "first bytes from the callback"
    c.write("hi\n")
    check_eq "got: hi\n", c.read_to_eof(3), "the rest, then closed"
  end

  test "the server keeps serving after hijacks, no descriptors left open" do
    base = s.fds
    3.times do
      c = s.client
      c.write(request("GET", "/hijack"))
      c.response
      c.write("x\n")
      c.read_to_eof(3)
      c.close
      c = s.client
      c.write(request("GET", "/partial"))
      c.read_until("partial ", 3)
      c.write("x\n")
      c.read_to_eof(3)
      c.close
    end
    check_eq "hello GET", s.get("/hello").body, "body"
    check_eq base, s.fds(base), "open descriptors"
  end
end

group("rack.hijack") { |s| hijack_tests(s) }
group("rack.hijack (KENNY_SERVER_THREADS=0, KENNY_THREADS=1)", "KENNY_THREADS" => "1", "KENNY_SERVER_THREADS" => "0") { |s| hijack_tests(s) }

group "WEB_CONCURRENCY=2", "WEB_CONCURRENCY" => "2", "KENNY_THREADS" => "1", "KENNY_SERVER_THREADS" => "0" do |s|
  test "forks two processes that both serve" do
    check_eq 2, s.children.size, "child processes"
    children = s.children
    # one process is held inside a request (one at a time, KENNY_SERVER_THREADS=0): the
    # other one must take the next connection
    gate = Gate.new
    a = s.client
    a.write(request("GET", "/wait?#{gate.port}"))
    ctl = gate.entered
    pid_b = s.get("/pid", timeout: 3).body.to_i
    ctl.write("go\n")
    pid_a = a.response.body.split.last.to_i
    check children.include?(pid_a), "1st served by a child (#{pid_a})"
    check children.include?(pid_b), "2nd served by a child (#{pid_b})"
    check pid_a != pid_b, "by different processes"
    ctl.close
    gate.close
  end
end

group "WEB_CONCURRENCY=2 (request threads)", "WEB_CONCURRENCY" => "2" do |s|
  test "serves from the children" do
    children = s.children
    check_eq 2, children.size, "child processes"
    20.times do
      pid = s.get("/pid").body.to_i
      check children.include?(pid), "served by a child (#{pid} of #{children})"
    end
  end
end

# ---------------------------------------------------------------------------------------------

pass = 0
fail = 0
t_all = now
$groups.each do |gname, env, blk|
  $tests = []
  srv = nil
  begin
    blk.call(Object.new.tap { |o| o.define_singleton_method(:method_missing) { |m, *a, **k, &b| srv.send(m, *a, **k, &b) } })
    todo = $tests.select { |name, _| !PATTERN || PATTERN.match?("#{gname}: #{name}") }
    next if todo.empty?
    srv = Server.new(env)
    todo.each do |name, tb|
      t0 = now
      log_at = srv.log.bytesize
      begin
        tb.call
        pass += 1
        puts "ok   #{gname}: #{name} (#{(now - t0).round(2)} s)"
      rescue Failure, StandardError => e
        fail += 1
        puts "FAIL #{gname}: #{name}: #{e.message}#{e.is_a?(Failure) ? "" : " (#{e.class})\n  #{e.backtrace.first(3).join("\n  ")}"}"
        log = srv.log.byteslice(log_at..).lines.last(10).join
        puts log.gsub(/^/, "  | ") unless log.empty?
      ensure
        ($clients || []).each(&:close)
        $clients = []
      end
    end
  rescue Failure => e
    fail += 1
    puts "FAIL #{gname}: #{e.message}"
  ensure
    srv&.stop
  end
end
puts "#{pass} passed, #{fail} failed (#{(now - t_all).round(1)} s)"
exit(fail == 0 ? 0 : 1)
