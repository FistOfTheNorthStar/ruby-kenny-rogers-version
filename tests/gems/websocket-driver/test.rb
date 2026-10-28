require "bundler/setup"
require "websocket/driver"

# A fake socket: the driver writes to it, we feed bytes to the other side.
class FakeSocket
  attr_reader :url, :env, :out
  def initialize(url, env = {}) = (@url, @env, @out = url, env, [])
  def write(data) = @out << data.b
  def take = (s = @out.join; @out.clear; s)
end

csock = FakeSocket.new("ws://example.com/chat")
client = WebSocket::Driver.client(csock, protocols: ["chat"])
client.set_header("X-Extra", "1")
cevents = []
client.on(:open) { cevents << :open }
client.on(:message) { |e| cevents << [:message, e.data] }
client.on(:close) { |e| cevents << [:close, e.code, e.reason] }
p client.start
req = csock.take
key = req[/Sec-WebSocket-Key: (\S+)/, 1]
p req.lines.first, req.include?("Upgrade: websocket"), req.include?("Sec-WebSocket-Protocol: chat"), req.include?("X-Extra: 1")

# server side: built from a Rack-style env
env = {
  "REQUEST_METHOD" => "GET", "HTTP_HOST" => "example.com", "PATH_INFO" => "/chat", "QUERY_STRING" => "",
  "HTTP_CONNECTION" => "Upgrade", "HTTP_UPGRADE" => "websocket", "HTTP_SEC_WEBSOCKET_KEY" => key,
  "HTTP_SEC_WEBSOCKET_VERSION" => "13", "HTTP_SEC_WEBSOCKET_PROTOCOL" => "chat", "rack.url_scheme" => "http"
}
p WebSocket::Driver.websocket?(env)
ssock = FakeSocket.new("ws://example.com/chat", env)
server = WebSocket::Driver.rack(ssock, protocols: ["chat"])
sevents = []
server.on(:open) { sevents << :open }
server.on(:message) { |e| sevents << [:message, e.data]; server.text("echo: #{e.data}") }
server.on(:close) { |e| sevents << [:close, e.code] }
p server.class, server.state
p server.start
p sevents
resp = ssock.take
p resp.lines.first, resp[/Sec-WebSocket-Accept: (\S+)/, 1] == WebSocket::Driver::Hybi.generate_accept(key), server.protocol

client.parse(resp)
p client.state, client.protocol, client.status, client.headers["Upgrade"]
client.text("hello")
client.binary([1, 2, 3])
frames = csock.take
p frames.bytesize, frames.getbyte(0), frames.getbyte(1) & 0x80   # FIN+text, masked
server.parse(frames)
p sevents
client.parse(ssock.take)
p cevents
client.ping("p1") { cevents << :pong }
server.parse(csock.take)
client.parse(ssock.take)
big = "x" * 70000
server.text(big)
client.parse(ssock.take)
p cevents.last[1].size, cevents.include?(:pong)
client.close("bye", 1000)
server.parse(csock.take)
client.parse(ssock.take)
p cevents.last, sevents.last, client.state, server.state
p WebSocket::Driver::Hybi.generate_accept("dGhlIHNhbXBsZSBub25jZQ==")
