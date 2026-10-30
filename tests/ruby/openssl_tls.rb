# OpenSSL::SSL over local sockets: handshakes, certificate verification (OpenSSL's errors on
# both ends, the verify callback, hostname checks), client certificates, Net::HTTP over https,
# ALPN, protocol versions, non-blocking IO, session resumption and the Buffering IO methods
require "openssl"
require "socket"
require "net/http"

X = OpenSSL::X509
S = OpenSSL::SSL

K1 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEpAIBAAKCAQEAt8h+DqCgQoljINwpq7sYQSQxEukaWvzz/BAGeG8JsZw4dg/q
  N73hbFohzEdwgwaSMzN99MQPjKmAtcgPUOd73Z9CKVATEnh6pD++u3xjGpInMCvQ
  F5sFqjA7pGCyjfjcFkNRT96OM6o0hGLG1vTLDUVO6MwUsLF6tuL9QE9SLuh4kZ/S
  jlHOaBhfw03NQdmnL3/Vq7G71ijeeI2NetcDcZ3BW+F26FsB/opmCwZAGr+lNQBJ
  pfUpHJMIS5EyeWsOHRh4NY+pNtvrr8kwApBqz64xH+V15lSIQ9+XdpMKLe7Ca4ul
  jr6+nT5bOjif9FZrZrmS000azCQZQGFaVtjIGQIDAQABAoIBAFjvloT4mSUpvAVC
  VY4Lx6R6YZ8fm8ftQhq4HJF6VmwTR8VmVTctVaGsClADQEC/1M8fO3RPzqCIxEbz
  rem+Y5QvhgJdBCW7lMnYKTIscv8APCOVCCnsNHUTy0D0JGQMymChqf8nGZSJ6Mkp
  zcRnCL3+zL1eSSLqeBJ5zQkqeYBF3lBkhLt1+NvSvW+N0b8W4Lr/gvoQqjPa61Z2
  5powzakP9TiCbkK3WhjNNMPw9C1Eks3xo10210wy+pZGugLML6NjRw3LhmL84t42
  Xy5mCI4XxT8vOy9YoZWK2aXyl4bhz/3i4h6FE3lKtN9pDFj1MjbgfWKHwcHnuXon
  fWCQ1fsCgYEA+cPafOaQ7uiLbfNv8c4A0at+xaU/R9/6P+/tUSo5wyyws2YoTzup
  MrZqfw7loZOVqxOIOna/I2PZLXQnJk/YAGdsv/tbtGK7Q6zUmTMHA/uhy4WE9o4B
  wiBKOSFXO+oQLR39l5ivvVQHMacUng98Bjnt4zPVizKbwbLPazIr/eMCgYEAvF75
  xgB1jfPMhpjZiEFGWkPXfe3awu8fRM5br/+PUvDfAcaBOMcrfSGu59XU/dcwHHkW
  UBGIWD/jwzA6Zm1zT5SsTSGbXImtWUOhangK2l255euT3FmlBCvKCW/n2U/XcMRz
  45iRIlJhOnfzYSXTDWUuWzOZeivXkudLei/2QtMCgYEAsSoa1hPaZIzxMgnGszeT
  3yYNo3bGv14Brc6rgLI6DNardGWXXDSOnRlFy2CuxADi3lnZfNb0V88goxxMFOH3
  7rluK3PJpdeOy79RgmXuqQWZy7qnCRjL6H5tB789mJM77DTkx96Xce2juLWyazfC
  mswGkO4YoEdCso/6jygkik8CgYAVrMQG0YS1SclN74aQVBp5RnckSVsztPofG7OX
  I8vlGkw+0w2Jb/mEE30htgwlDT6+9/vmSK4soWVez0zY/l0XWh8dXPGYi0sEC3Em
  ZBuzlrdOSnYgE7yWJ0e/ID5IVhSPKDvBg4unrFz9slGx6v1G/E6CNgTB9wt+Htkj
  fSNT+wKBgQDWqzH3/tOxDef0K3Leb20MNk8L2LaWMXxSUNn4wwUjrNKO5M54zcwA
  pQ75sLjrv+oci7zkgerT0lyitJZqrs+wR6QoCic6uYJVdVtmOC+s3jAw/wbRPC4u
  BJy8ZPqne1tW5oJupcIVIeYcBTBZFcBIgm+7382Nva4PLO7i9cIS4w==
  -----END RSA PRIVATE KEY-----
PEM
K2 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEogIBAAKCAQEApr+VKDLsWOwkwLhReCgDW2DHmrA49lk5QHxxidJwMyDP6O7N
  wUpTo0Ek5SrDbGI7+83glisScEm8NMtgU07F9OL4LAwIwe3pAl+hyZoWI1yY7EaF
  OMmf1whdAldmVJAHoa0QXGjokkJ2ahNEjiUd392xzG++ZTwN+Rfvfd5hqqfvylxw
  M/3tKWbbGoQ7G5njGoTNSoHI3qmevkytoP9ppCBiF2Eet/ZShXqRimquMUwC1h+/
  8epUyC7dXj7KFyNgJQYZAJAgyxOcj/3M96t6dWk0Pq9Q+T461nBCFAPpcwuFET8m
  X4YzG1vPpmBRnBDSUkS2bq7l1fxF87oBmT21cwIDAQABAoIBAAn8/IdIPZtkpx2a
  14PLXyAbH6Rmx+gMbNSDCnNcqqHgEPQn/HqGjun+3fc9j4UvgTOnymXUYKoJ5f8s
  64aHTRj1JjPP6LpicOGr2XoaWRch1dCEDOOGQvp+IsF1e3Nh8+29pcw98H3KWLjT
  502zGs4CbphgsoFH4/e2ylQPfRQxcEyqIunLuNHzej95Gvhm4HGzOIF7DOI8sbJE
  IuhfqlLrwam8UdvDimfT5RvLL3w3MqRJkNzQguDdJZ4WDh5CEsaCUbcWt5DWRWZg
  NwfU3er5eLPmQRje2yOdJbfMDlpmP3JCBvzaJE6aMP8L76qdUIs2HHnJ1CNIZOvM
  qfbWFikCgYEA0dVyXz/09b6RIbyMeqQJZXx40UCdOfp4rmK9ev2vife9xJ4ocLL5
  2gOKqhMPjrMP/wJpJFUi1lVshxfj77xXeq6gLvrLL1C7M8EUE/giwYeHtIpTF+UV
  FB6TiwleF4eSA3265eZxnftIJmZaYYBg85T3+HmZoyZh1sQhb/TeqKcCgYEAy29r
  Qbbj3T7GlPto3Y/a3S4qRdsvxGPLeT1rsIHMmamvPpR1l6V34kz3KyeauthVm+my
  qeQIDU2QosvuHBbrkLxt8TwE5SkD24+HypozmAHw0qxdHUpVRv06mTMuRuFT3gN/
  ebGAf20rDg1WFIwinGJ7brAiX3V2/T+I49dzWlUCgYA1AvTTJYhQcWWfHVBsR3bs
  QccQLn/KPwIwJFf0c5TeiU1TGBXlawH+Vzi4APrENW7MISLJAA0+EvTJKHr4fR+o
  qedkFw4wFPhGefgFl4HIXOIm9W9KsSBjJdpmnOY9kY2M/j2yiUaY//vZX7Bjw5g6
  0Z7YEniDJFZBeOltrHJ4LwKBgGHFTa4KgN7PEc9+OLrrLUvJqSVjn0RqldrR6xSs
  fTGIvKF95joYm1JWnR/Pa+zKlfJV0bkWz8vc8FtdWTWEyngIJ2qd5thf6uu1wpRb
  odo3bI9DqYivNeblQyln0KDeIFl+IuDQryoOxHd/b1R2uRehuSLMYeiw4pv50gd2
  f4j1AoGAAnxc3GvbBwXqv3sdXQtGO8nbtqAzCQTb2n7SakTwwrBMWUodjIV51Cjq
  rUq738+Yl9DDG5+wgnmpqa6w8AUwX596TmiVd5N0JlCirDk6n+vlXStvphv2K6tR
  dLZmNaIVOmp2+PjXa/fr1c2t5qB9uL0yKr9He48Bj6ECeKutbiU=
  -----END RSA PRIVATE KEY-----
PEM
K3 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEpAIBAAKCAQEApxPP+Nje1Je677fv8Ow458qG5ZkzcJfKXxF5i8LBbgN7Brkn
  EQbi+8/i+ZDIg0OSly9ASfjDl2ZhMW14FZE0dLlahsUwAX5BGr8eWvM9Cmfjfxf2
  SESVT/67T8MiWsHscZ5mHHedBGqWSGEXlY6ms6s3E4/wOzJuCUJCOdmHDZtjyRDP
  jGg8ioEiJ8W8vDXqRcNj4zLXG0HNXXr9fsokO+8Lmwav2bNQH9fFDxcd+8iDWxMY
  9dVxHeKwVqVTmAelCKO3mYMig78Ep/+VW0J2xvteKfageZFvskRhVTK9Gte2dPM8
  e5T7APwNemdKAiSyo9+sYockY6cGpvZqQf5apwIDAQABAoIBAE077+ZZnnyJHX3G
  OjFViaLA8SX+8ZQJXyAebGEKqS8phbwgaCpLVC9StNpoLwDbAk9IJQBs8kGxLpnS
  QHI72xNgQbROQDhYfFEYazhg++UtDDHKymsy5REDMOSY0G9C+QG21v4u3xNVEF6q
  H2Rf+rjQm5e78LH0yIAYNJQYdgF4BeiSvA5VEsmWok2BvEKDtVqi3abV8J1Lr4sh
  7pxUaOrGj4wi5it+9EGHlwjRzLjRjG5Vwt0eLMd1e7LRkNhZ+PXPKX5+3EcSgQxl
  qNP9kvdq8Wi6BmPUtGYGK+JK+3JjekPzgIizZvfi/YQwvkIxF16PPzPONmQsjj56
  q9RlFH0CgYEA0eOPm09Zq8ErL11OxaZ7F4tO3m07UWeLFYT2xkd2aB6ZyNJyWkkH
  GqwIxGzQt/XmFshTrVzRB9IZ73GjXeNXRX50AfjLos9vQbuHiqCEHCntkAeg09a4
  5tAfaVMl8ZVIHWWTVHINtoWxQNco9VOoLfMtDdm5oCZVGutS40FGzGsCgYEAy8h5
  IhQj8pHU+KIH2Sxzzi752DX/tOTRp3mo+uJpb9NU3GnB0HbaNG0DBU/Q67CcutSh
  rJ0i5m/Im8d31UQ+uaUh+KX/1FL/ckkFZte9rJEdXQHJMnlFQpxEBg+SPtMNnxKp
  52+AIUWWgkRN7hs7ZJQdQd/W7JC/j8MENOFEObUCgYEAl7HGatOQ1afJFHR1qDhI
  op2WpP+BbbZOOYFd8bRYiawELbab21EIh2hq9bxHpY1dfH3QrG12zhY1lG8nSYYz
  3xAjvAEa/FK78GAe+DxG/yv2fLBmtnRsMrWc3jywIcho1IZDUzcTXiWGFHztJ1IN
  LRiiKW6A0JSj0c7em0zkZH0CgYEAjz7Yh3AI5NZGEpAzXCG3NdQnpCgxGwNNrKAE
  fHbzDrxLBrUAnH0vXXLTkMROY56+TUZyb1f11G33YimQvsU+IllFy/dEIghFsBxM
  Q8KbfdZ0AZDj0SsV2IDoACqp7qeFB7Ci5iCmvLyh0+gXr6xMz6Kd/lROS/Rj87Zu
  Go3O25ECgYBaoz7FRSIuOGgOud4tXaZp3ul5TGE1/1ei4RYe/JV3/vlzuyyw2cTT
  ux1dZoD3cREvv7QBV+4/eieOg8F9yTEyr8N/cXIrJuhXAs+cqiZ1+Id0Ri04McKJ
  6bLftQ6gT5+B+yU7veh+gWIiNRVw3O3SmJYcROQJdPaD7IGsKoxdKQ==
  -----END RSA PRIVATE KEY-----
PEM

def cert(subject, issuer, key, issuer_key, serial, exts, nb: Time.utc(2020, 1, 1), na: Time.utc(2090, 1, 1))
  c = X::Certificate.new
  c.version = 2
  c.serial = serial
  c.subject = X::Name.parse(subject)
  c.issuer = issuer ? issuer.subject : c.subject
  c.public_key = key
  c.not_before = nb
  c.not_after = na
  ef = X::ExtensionFactory.new(issuer || c, c)
  exts.each { |e| c.add_extension(ef.create_extension(*e)) }
  c.sign(issuer_key, "SHA256")
  c
end

CA = cert("/CN=TLS Test CA", nil, K1, K1, 1, [["basicConstraints", "CA:TRUE", true], ["keyUsage", "keyCertSign,cRLSign", true]])
LEAF = cert("/CN=localhost", CA, K2, K1, 2, [["basicConstraints", "CA:FALSE"], ["subjectAltName", "DNS:localhost,IP:127.0.0.1"],
                                             ["extendedKeyUsage", "serverAuth"]])
CLIENT = cert("/CN=client", CA, K3, K1, 3, [["basicConstraints", "CA:FALSE"], ["extendedKeyUsage", "clientAuth"]])
SELF = cert("/CN=localhost", nil, K3, K3, 4, [["subjectAltName", "DNS:localhost"]])
OLD = cert("/CN=localhost", CA, K2, K1, 5, [["subjectAltName", "DNS:localhost"]], na: Time.utc(2021, 1, 1))

def clean(msg) = msg.sub(/peeraddr=\S+/, "peeraddr=-")

def store(*certs)
  s = X::Store.new
  certs.each { |c| s.add_cert(c) }
  s
end

# A server on 127.0.0.1 that answers each connection with `handler`; yields its port, and
# returns what the server side raised (nil when the handshake went through).
def serve(ctx, conns: 1, &handler)
  tcp = TCPServer.new("127.0.0.1", 0)
  ssrv = S::SSLServer.new(tcp, ctx)
  errors = []
  th = Thread.new do
    conns.times do
      s = ssrv.accept
      handler&.call(s)
      s.close
    rescue S::SSLError, EOFError, SystemCallError => e
      errors << "#{e.class}: #{clean(e.message)}"
    end
  end
  [tcp.addr[1], th, errors]
end

def server_ctx(cert: LEAF, key: K2, chain: nil)
  ctx = S::SSLContext.new
  ctx.cert = cert
  ctx.key = key
  ctx.extra_chain_cert = chain if chain
  ctx
end

def connect(port, ctx, hostname: "localhost")
  s = S::SSLSocket.new(TCPSocket.new("127.0.0.1", port), ctx)
  s.hostname = hostname if hostname
  s.sync_close = true
  s.connect
  s
end

def client(port, label, hostname: "localhost", **opts)
  ctx = S::SSLContext.new
  opts.each { |k, v| ctx.__send__("#{k}=", v) }
  s = connect(port, ctx, hostname: hostname)
  r = yield(s) if block_given?
  puts "#{label}: ok verify_result=#{s.verify_result} #{r.inspect}"
  s.close
rescue S::SSLError, SystemCallError => e
  puts "#{label}: #{e.class}: #{clean(e.message)} verify_result=#{s&.verify_result.inspect}"
end

def echo(s) = s.puts("echo #{s.gets.chomp}")

# ---- verification outcomes, client and server side
[
  ["trusted CA", { cert_store: store(CA), verify_mode: S::VERIFY_PEER }],
  ["verify_hostname", { cert_store: store(CA), verify_mode: S::VERIFY_PEER, verify_hostname: true }],
  ["no store", { verify_mode: S::VERIFY_PEER }],
  ["VERIFY_NONE", { verify_mode: S::VERIFY_NONE }],
  ["callback accepts", { verify_mode: S::VERIFY_PEER, verify_callback: ->(ok, c) { true } }],
  ["callback rejects", { cert_store: store(CA), verify_mode: S::VERIFY_PEER, verify_callback: ->(ok, c) { false } }],
].each do |label, opts|
  port, th, errs = serve(server_ctx) { |s| echo(s) }
  client(port, label, **opts) do |s|
    s.puts "hi"
    [s.gets, s.peer_cert.subject.to_s, s.peer_cert_chain.map { |c| c.subject.to_s }, s.cert]
  end
  th.join
  puts "  server: #{errs.inspect}"
end

port, th, errs = serve(server_ctx)
client(port, "hostname mismatch", hostname: "other.example", cert_store: store(CA), verify_mode: S::VERIFY_PEER, verify_hostname: true)
th.join
puts "  server: #{errs.inspect}"

seen = []
port, th, errs = serve(server_ctx)
client(port, "callback sees", cert_store: store(CA), verify_mode: S::VERIFY_PEER,
       verify_callback: ->(ok, c) { seen << [ok, c.error, c.error_depth, c.current_cert.subject.to_s]; ok })
th.join
p seen

port, th, errs = serve(server_ctx(cert: SELF, key: K3))
client(port, "self-signed, not trusted", cert_store: store(CA), verify_mode: S::VERIFY_PEER)
th.join
puts "  server: #{errs.inspect}"
port, th, errs = serve(server_ctx(cert: SELF, key: K3))
client(port, "self-signed in the store", cert_store: store(SELF), verify_mode: S::VERIFY_PEER, verify_hostname: true)
th.join
port, th, errs = serve(server_ctx(cert: OLD))
client(port, "expired", cert_store: store(CA), verify_mode: S::VERIFY_PEER)
th.join
puts "  server: #{errs.inspect}"

# ---- client certificates
[["no client cert", {}], ["client cert", { cert: CLIENT, key: K3 }]].each do |label, opts|
  sctx = server_ctx
  sctx.verify_mode = S::VERIFY_PEER | S::VERIFY_FAIL_IF_NO_PEER_CERT
  sctx.cert_store = store(CA)
  peer = nil
  port, th, errs = serve(sctx) do |s|
    peer = s.peer_cert&.subject&.to_s
    echo(s)
  end
  client(port, label, cert_store: store(CA), verify_mode: S::VERIFY_PEER, **opts) do |s|
    s.puts "x"
    s.gets
  end
  th.join
  puts "  server: peer=#{peer.inspect} #{errs.inspect}"
end

# ---- Net::HTTP over https (hostname verification, keep-alive, POST)
port, th, errs = serve(server_ctx, conns: 2) do |s|
  2.times do
    req = s.gets or break
    headers = {}
    while (l = s.gets) && l != "\r\n"
      k, v = l.chomp.split(": ", 2)
      headers[k.downcase] = v
    end
    body = s.read(headers["content-length"].to_i) if headers["content-length"]
    out = "#{req.chomp} #{body.inspect} #{s.ssl_version}"
    s.write("HTTP/1.1 200 OK\r\nContent-Length: #{out.bytesize}\r\n\r\n#{out}")
  end
end
http = Net::HTTP.new("localhost", port)
http.use_ssl = true
http.cert_store = store(CA)
http.ipaddr = "127.0.0.1"
http.start do
  puts http.get("/one").body
  puts http.post("/two", "data=1").body
end
http2 = Net::HTTP.new("localhost", port)
http2.use_ssl = true
http2.ipaddr = "127.0.0.1"
begin
  http2.get("/three")
rescue OpenSSL::SSL::SSLError => e
  puts "Net::HTTP without the CA: #{clean(e.message)}"
end
th.join
puts "  server: #{errs.inspect}"

# ---- ALPN, versions, ciphers
sctx = server_ctx
sctx.alpn_select_cb = ->(protos) { p(protos); protos.include?("http/1.1") ? "http/1.1" : protos.first }
port, th, = serve(sctx, conns: 4) { |s| s.puts([s.alpn_protocol, s.ssl_version].inspect) }
client(port, "alpn", alpn_protocols: ["h2", "http/1.1"]) { |s| [s.alpn_protocol, s.gets.chomp] }
client(port, "TLS 1.2", max_version: S::TLS1_2_VERSION) { |s| [s.ssl_version, s.cipher[0], s.gets.chomp] }
client(port, "TLS 1.2 ChaCha20", max_version: :TLS1_2, ciphers: "ECDHE-RSA-CHACHA20-POLY1305") { |s| [s.cipher, s.gets.chomp] }
client(port, "TLS 1.3", min_version: S::TLS1_3_VERSION) { |s| [s.ssl_version, s.cipher, s.gets.chomp] }
th.join

# ---- non-blocking handshake and reads, big writes, EOF, Buffering methods
port, th, = serve(server_ctx) do |s|
  data = s.read(200_000)
  s.write(data.reverse)
  s.print("line one\nline two\n", "tail")
end
sock = S::SSLSocket.new(TCPSocket.new("127.0.0.1", port))
sock.sync_close = true
p(loop do
  r = sock.connect_nonblock(exception: false)
  break r.class if r.is_a?(S::SSLSocket)
  IO.select(r == :wait_readable ? [sock] : nil, r == :wait_writable ? [sock] : nil)
end)
begin
  sock.read_nonblock(10)
rescue IO::WaitReadable => e
  p e.class
end
sock.write("ab" * 100_000)
got = +""
while got.bytesize < 200_000
  r = sock.read_nonblock([65536, 200_000 - got.bytesize].min, exception: false)
  r == :wait_readable ? IO.select([sock]) : got << r
end
p [got.bytesize, got[0, 4], got.encoding]
p sock.gets, sock.readline
p sock.read
p sock.read_nonblock(5, exception: false)
begin
  sock.sysread(5)
rescue EOFError => e
  p e
end
sock.close
p sock.closed?
th.join

# ---- session resumption (as Net::HTTP reconnects: session_new_cb, then session=)
port, th, = serve(server_ctx, conns: 2) { |s| s.puts "hi" }
sess = nil
ctx = S::SSLContext.new
ctx.session_cache_mode = S::SSLContext::SESSION_CACHE_CLIENT
ctx.session_new_cb = proc { |_sock, se| sess = se }
s1 = connect(port, ctx)
s1.gets
p [s1.session_reused?, sess.class, sess.timeout.class, s1.session == sess]
s1.close
s2 = S::SSLSocket.new(TCPSocket.new("127.0.0.1", port), S::SSLContext.new)
s2.session = sess
s2.connect
s2.gets
p s2.session_reused?
s2.close
th.join

# ---- context settings
ctx = S::SSLContext.new
p [ctx.verify_mode, ctx.verify_hostname, ctx.options, ctx.security_level, ctx.session_cache_mode, ctx.timeout]
ctx.set_params
p [ctx.verify_mode, ctx.verify_hostname, ctx.cert_store.class]
ctx.setup
p ctx.frozen?
begin
  ctx.verify_mode = 0
rescue => e
  p e.class
end
p S::SSLContext.new.ciphers.map(&:first).include?("TLS_AES_256_GCM_SHA384")
p S::SSLSocket.new(TCPSocket.new("127.0.0.1", TCPServer.new("127.0.0.1", 0).addr[1])).then { [_1.state, _1.pending, _1.verify_result, _1.peer_cert] }
