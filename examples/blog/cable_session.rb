# Action Cable over WebSocket against a running copy of the blog: two clients connect, get
# the welcome message, subscribe to a channel, one of them speaks (a channel action that
# broadcasts), an HTTP request broadcasts too, both clients receive both messages, then they
# unsubscribe and disconnect. Uses the websocket-driver gem (installed with Rails) as client.
#
#   ruby examples/blog/cable_session.rb --setup APP_DIR   # add the test channel to a copy
#   ruby examples/blog/cable_session.rb PORT               # run the session
#
# --setup writes into APP_DIR (a copy of examples/blog, not the example itself): an
# EchoChannel, a CableTestController#broadcast action and its route, the async adapter in
# config/cable.yml for every environment (production's redis isn't installed here) and
# allowed origins for 127.0.0.1. Serve the copy with the Kenny binary or with Puma.
require "socket"
require "json"
require "uri"
require "net/http"
require "timeout"
require "websocket/driver"

if ARGV[0] == "--setup"
  app = ARGV[1] or abort "usage: cable_session.rb --setup APP_DIR"
  File.write(File.join(app, "app/channels/echo_channel.rb"), <<~RUBY)
    class EchoChannel < ApplicationCable::Channel
      def subscribed
        stream_from "echo"
      end

      def speak(data)
        ActionCable.server.broadcast("echo", "text" => data["text"], "via" => "speak")
      end
    end
  RUBY
  File.write(File.join(app, "app/controllers/cable_test_controller.rb"), <<~RUBY)
    class CableTestController < ActionController::Base
      skip_forgery_protection

      def broadcast
        ActionCable.server.broadcast("echo", "text" => params[:text], "via" => "http")
        head :ok
      end
    end
  RUBY
  routes = File.join(app, "config/routes.rb")
  src = File.read(routes)
  unless src.include?("cable_test")
    File.write(routes, src.sub(/^end\s*\z/, "  post \"cable_test/broadcast\" => \"cable_test#broadcast\"\nend\n"))
  end
  File.write(File.join(app, "config/cable.yml"), "development:\n  adapter: async\n\ntest:\n  adapter: async\n\nproduction:\n  adapter: async\n")
  File.write(File.join(app, "config/initializers/cable_test.rb"),
             "Rails.application.config.action_cable.allowed_request_origins = [%r{\\Ahttp://(127\\.0\\.0\\.1|localhost)(:\\d+)?\\z}]\n")
  puts "cable test files written to #{app}"
  exit
end

PORT = Integer(ARGV[0] || 3000)
ID = JSON.generate(channel: "EchoChannel")

class Client
  attr_reader :url

  def initialize(name)
    @name = name
    @url = "ws://127.0.0.1:#{PORT}/cable"
    @sock = TCPSocket.new("127.0.0.1", PORT)
    @events = Queue.new
    @driver = WebSocket::Driver.client(self)
    @driver.set_header("Origin", "http://127.0.0.1:#{PORT}")
    @driver.on(:open) { @events << [:open] }
    @driver.on(:message) { |e| @events << [:message, JSON.parse(e.data)] }
    @driver.on(:close) { |e| @events << [:close, e.code] }
    @driver.start
    @reader = Thread.new do
      loop { @driver.parse(@sock.readpartial(4096)) }
    rescue IOError, SystemCallError
      @events << [:eof]
    end
  end

  def write(data)
    @sock.write(data)
  end

  def command(h)
    @driver.text(JSON.generate(h))
  end

  # next event that isn't a ping
  def next_event(timeout = 5)
    Timeout.timeout(timeout) do
      loop do
        ev = @events.pop
        return ev unless ev[0] == :message && ev[1]["type"] == "ping"
      end
    end
  rescue Timeout::Error
    [:timeout]
  end

  def close
    @driver.close
    ev = next_event
    @sock.close rescue nil
    ev
  end
end

$failed = 0
def check(name, got, want)
  ok = got == want
  $failed += 1 unless ok
  puts "#{ok ? 'ok  ' : 'FAIL'} #{name}#{ok ? '' : ": got #{got.inspect}, want #{want.inspect}"}"
end

a = Client.new("a")
b = Client.new("b")
check "a connects", a.next_event, [:open]
check "a welcome", a.next_event, [:message, { "type" => "welcome" }]
check "b connects", b.next_event, [:open]
check "b welcome", b.next_event, [:message, { "type" => "welcome" }]

[a, b].each { |c| c.command(command: "subscribe", identifier: ID) }
check "a subscribed", a.next_event, [:message, { "identifier" => ID, "type" => "confirm_subscription" }]
check "b subscribed", b.next_event, [:message, { "identifier" => ID, "type" => "confirm_subscription" }]

a.command(command: "message", identifier: ID, data: JSON.generate(action: "speak", text: "hello from a"))
want = [:message, { "identifier" => ID, "message" => { "text" => "hello from a", "via" => "speak" } }]
check "a receives its broadcast", a.next_event, want
check "b receives a's broadcast", b.next_event, want

res = Net::HTTP.post_form(URI("http://127.0.0.1:#{PORT}/cable_test/broadcast"), "text" => "from a request")
check "HTTP broadcast", res.code, "200"
want = [:message, { "identifier" => ID, "message" => { "text" => "from a request", "via" => "http" } }]
check "a receives the HTTP broadcast", a.next_event, want
check "b receives the HTTP broadcast", b.next_event, want

a.command(command: "unsubscribe", identifier: ID)
sleep 0.3
b.command(command: "message", identifier: ID, data: JSON.generate(action: "speak", text: "only b"))
check "b still subscribed", b.next_event, [:message, { "identifier" => ID, "message" => { "text" => "only b", "via" => "speak" } }]
check "a unsubscribed (no message)", a.next_event(1.5), [:timeout]

check "a disconnects", a.close, [:close, 1000]
check "b disconnects", b.close, [:close, 1000]

puts $failed.zero? ? "cable session: all checks passed" : "cable session: #{$failed} checks failed"
exit($failed.zero? ? 0 : 1)
