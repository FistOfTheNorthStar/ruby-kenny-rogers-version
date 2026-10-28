require "bundler/setup"
require "httpclient"
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
        resp = "#{line.strip} body=#{body} ct=#{headers["content-type"]} cookie=#{headers["cookie"]}"
        extra = line.include?("/login") ? "Set-Cookie: sid=xyz; path=/\r\n" : ""
        status = line.include?("/missing") ? "404 Not Found" : "200 OK"
        c.write "HTTP/1.1 #{status}\r\nContent-Type: text/plain\r\n#{extra}Content-Length: #{resp.bytesize}\r\n\r\n#{resp}"
      end
    ensure
      c.close
    end
    end
  end
end

base = "http://127.0.0.1:#{port}"
client = HTTPClient.new
client.cookie_manager = nil
r = client.get("#{base}/hello", query: { "a" => "1 2" })
p r.status, r.body, r.content_type, r.ok?, r.header["Content-Length"]
r = client.post("#{base}/form", body: { "x" => "y", "z" => "w" })
p r.status, r.body
r = client.post("#{base}/json", body: '{"k":1}', header: { "Content-Type" => "application/json" })
p r.body
p client.get_content("#{base}/content")
r = client.get("#{base}/missing")
p r.status, r.reason
begin
  client.get_content("#{base}/missing")
rescue HTTPClient::BadResponseError => e
  p e.class, e.res.status
end
jar = HTTPClient.new
jar.get("#{base}/login")
p jar.get("#{base}/after").body
p jar.cookies.map { |c| [c.name, c.value] }
p HTTP::Message.escape("a b&c"), HTTP::Message.create_query_part_str("a" => "1", "b" => ["x", "y"])
