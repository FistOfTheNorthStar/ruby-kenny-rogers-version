require "bundler/setup"
require "websocket/extensions"

class Deflate
  attr_reader :name, :type, :rsv1, :rsv2, :rsv3
  def initialize = (@name, @type, @rsv1, @rsv2, @rsv3 = "permessage-deflate", "permessage", true, false, false)
  def create_client_session = ClientSession.new
  def create_server_session(offers) = ServerSession.new(offers)
end
class Session
  def process_incoming_message(m) = (m.data = m.data.reverse; m)
  def process_outgoing_message(m) = (m.data = m.data.upcase; m)
  def close = nil
end
class ClientSession < Session
  def generate_offer = [{ "client_max_window_bits" => true }, { "x" => "1" }]
  def activate(params) = (@params = params; true)
end
class ServerSession < Session
  def initialize(offers) = @offers = offers
  def generate_response = { "server_no_context_takeover" => true, "offers" => @offers.size }
end
Message = Struct.new(:data, :rsv1, :rsv2, :rsv3, :opcode)

p WebSocket::Extensions::Parser.parse_header("permessage-deflate; client_max_window_bits=10, foo; a=\"b c\"; flag").to_a
p WebSocket::Extensions::Parser.serialize_params("ext", { "a" => 1, "b" => true, "c" => "x y" })

client = WebSocket::Extensions.new
client.add(Deflate.new)
offer = client.generate_offer
p offer
server = WebSocket::Extensions.new
server.add(Deflate.new)
resp = server.generate_response(offer)
p resp
client.activate(resp)
p client.valid_frame_rsv?(Message.new("", true, false, false, 1)), client.valid_frame_rsv?(Message.new("", false, true, false, 1))
m = client.process_outgoing_message(Message.new("hello", false, false, false, 1))
p m.data
p server.process_incoming_message(Message.new("abc", true, false, false, 1)).data
begin
  client.activate("unknown-ext")
rescue WebSocket::Extensions::ExtensionError => e
  p e.class, e.message
end
begin
  WebSocket::Extensions::Parser.parse_header("bad header ;;")
rescue => e
  p e.class
end
