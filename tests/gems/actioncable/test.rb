require "bundler/setup"
require "active_support/core_ext/string/inflections"
require "action_cable"
require "action_cable/server/base"
require "action_cable/subscription_adapter/inline"
require "action_cable/subscription_adapter/async"
require "json"
require "active_support/core_ext/hash/indifferent_access"

server = ActionCable.server
server.config.cable = { "adapter" => "inline" }
server.config.logger = Logger.new(nil)
p server.config.pubsub_adapter, ActionCable::INTERNAL[:message_types][:welcome], ActionCable::VERSION::STRING.class

# pubsub roundtrip with the inline adapter
received = []
server.pubsub.subscribe("chat_room", ->(msg) { received << JSON.parse(msg) })
server.broadcast("chat_room", { text: "hello", n: 1 })
server.broadcast("chat_room", { text: "again" })
p received

# a channel driven by the test helper stubs
require "action_cable/channel/test_case"
class ChatChannel < ActionCable::Channel::Base
  def subscribed
    reject if params[:room] == "forbidden"
    stream_from "chat_#{params[:room]}"
  end
  def speak(data) = transmit({ "echo" => data["message"].upcase })
  def ping = transmit({ "pong" => true })
end
conn = ActionCable::Channel::ConnectionStub.new(user: "kenny")
ch = ChatChannel.new(conn, "{\"channel\":\"ChatChannel\",\"room\":\"lobby\"}", { "room" => "lobby" }.with_indifferent_access)
ch.subscribe_to_channel
p ch.send(:streams).keys
ch.perform_action("action" => "speak", "message" => "hi")
ch.perform_action("action" => "ping")
p conn.transmissions.map { _1.except("identifier") }
bad = ChatChannel.new(conn, "{}", { "room" => "forbidden" }.with_indifferent_access)
bad.subscribe_to_channel
p bad.send(:subscription_rejected?), conn.transmissions.last.except("identifier")
p ChatChannel.action_methods.sort, ChatChannel.broadcasting_for("x") rescue p $!.class
class AppConnection < ActionCable::Connection::Base
  identified_by :current_user
end
p AppConnection.identifiers.to_a
