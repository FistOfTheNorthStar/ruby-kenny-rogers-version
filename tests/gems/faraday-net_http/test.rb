require "bundler/setup"
require "faraday"
require "faraday/net_http"
require "socket"

server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
th = Thread.new do
  3.times do
    c = server.accept
    line = c.gets
    headers = {}
    while (h = c.gets) && h != "\r\n"
      k, v = h.split(": ", 2)
      headers[k.downcase] = v.strip
    end
    body = headers["content-length"] ? c.read(headers["content-length"].to_i) : ""
    resp = "#{line.strip} ua=#{headers["user-agent"]} body=#{body}"
    c.write "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Test: yes\r\nContent-Length: #{resp.bytesize}\r\nConnection: close\r\n\r\n#{resp}"
    c.close
  end
end

conn = Faraday.new(url: "http://127.0.0.1:#{port}", headers: { "User-Agent" => "kenny" }) do |f|
  f.request :url_encoded
  f.adapter :net_http
end
p Faraday.default_adapter
r = conn.get("/hello", q: "1")
p r.status, r.body, r.headers["x-test"], r.headers["content-type"]
r = conn.post("/submit", { a: "b c" })
p r.status, r.body
r = conn.put("/thing") { |req| req.body = "raw"; req.headers["Content-Type"] = "text/plain" }
p r.body
th.join
server.close
begin
  conn.get("/down")
rescue Faraday::ConnectionFailed => e
  p e.class
end
