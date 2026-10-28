require "bundler/setup"
require "excon"
require "socket"

server = TCPServer.new("127.0.0.1", 0)
port = server.addr[1]
th = Thread.new do
  loop do
    Thread.new(server.accept) do |c|
    begin
      while (line = c.gets)
        headers = {}
        while (h = c.gets) && h != "\r\n"
          k, v = h.split(": ", 2)
          headers[k.downcase] = v.strip
        end
        body = headers["content-length"] ? c.read(headers["content-length"].to_i) : ""
        resp = "#{line.strip} body=#{body} x=#{headers["x-req"]}"
        status = line.include?("/missing") ? "404 Not Found" : "200 OK"
        c.write "HTTP/1.1 #{status}\r\nContent-Type: text/plain\r\nContent-Length: #{resp.bytesize}\r\n\r\n#{resp}"
      end
    ensure
      c.close
    end
    end
  end
end

base = "http://127.0.0.1:#{port}"
r = Excon.get("#{base}/hello?a=1")
p r.status, r.body, r.headers["Content-Type"], r.reason_phrase
r = Excon.post("#{base}/form", body: URI.encode_www_form(x: "1 2"), headers: { "Content-Type" => "application/x-www-form-urlencoded", "X-Req" => "abc" })
p r.status, r.body
conn = Excon.new(base, persistent: true)
3.times { |i| p conn.get(path: "/p#{i}", query: { n: i }).body }
chunks = []
conn.get(path: "/stream", response_block: ->(chunk, _rem, _tot) { chunks << chunk })
p chunks.join
begin
  conn.get(path: "/missing", expects: [200])
rescue Excon::Error::NotFound => e
  p e.class, e.response.status
end
p conn.request(method: :delete, path: "/d").body
conn.reset

Excon.defaults[:mock] = true
Excon.stub({ method: :get, path: "/stubbed" }, { body: "stubbed!", status: 201, headers: { "X" => "y" } })
r = Excon.get("http://example.invalid/stubbed")
p r.status, r.body, r.headers["X"]
begin
  Excon.get("http://example.invalid/other")
rescue Excon::Error::StubNotFound => e
  p e.class
end
Excon.stubs.clear
Excon.defaults[:mock] = false
p Excon::Utils.escape_uri("a b/c"), Excon::Utils.query_string(query: { a: 1, b: nil })
