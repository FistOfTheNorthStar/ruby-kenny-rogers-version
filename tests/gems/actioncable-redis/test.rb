require "bundler/setup"
require "active_support"
require "active_support/core_ext"
require "action_cable"
require "action_cable/server/base"
require "action_cable/subscription_adapter/redis"
require "json"

# Action Cable's redis adapter against a running Redis (localhost:6379): a listener thread
# subscribed on one connection, broadcasts published on another.
server = ActionCable.server
prefix = "kenny_test_#{Process.pid}"
server.config.cable = { "adapter" => "redis", "url" => "redis://localhost:6379/1", "channel_prefix" => prefix }.with_indifferent_access
server.config.logger = Logger.new(nil)
p server.config.pubsub_adapter
pubsub = server.pubsub

wait = lambda do |&ready|
  deadline = Process.clock_gettime(Process::CLOCK_MONOTONIC) + 5
  sleep 0.01 until ready.call || Process.clock_gettime(Process::CLOCK_MONOTONIC) > deadline
  ready.call
end

got = Queue.new
confirmed = Queue.new
room = ->(msg) { got << [:room, JSON.parse(msg)] }
news = ->(msg) { got << [:news, JSON.parse(msg)] }
pubsub.subscribe("room", room, -> { confirmed << :room })
pubsub.subscribe("news", news, -> { confirmed << :news })
p [confirmed.pop(timeout: 5), confirmed.pop(timeout: 5)].sort

server.broadcast("room", { text: "hello", n: 1 })
server.broadcast("news", { "title" => "héllo wörld ✓", "tags" => %w[a b] })
server.broadcast("nobody", { lost: true })
server.broadcast("room", { text: "x" * 5000 })
msgs = 3.times.map { got.pop(timeout: 5) }
p msgs[0], msgs[1], [msgs[2][0], msgs[2][1]["text"].size], got.empty?

# a second subscriber to a channel, and several threads broadcasting at once
also = []
pubsub.subscribe("room", ->(msg) { also << JSON.parse(msg)["n"] }, -> { confirmed << :also })
p confirmed.pop(timeout: 5)
4.times.map { |t| Thread.new { 25.times { |i| server.broadcast("room", { n: t * 100 + i }) } } }.each(&:join)
ns = 100.times.map { got.pop(timeout: 5)&.dig(1, "n") }
p ns.compact.size, ns.compact.sort == 4.times.flat_map { |t| 25.times.map { |i| t * 100 + i } }.sort
p wait.call { also.size == 100 }, also.sort == ns.compact.sort

# the channel prefix keeps another application's messages apart: a plain publish to the
# unprefixed name isn't delivered, one to the prefixed name is
raw = Redis.new(url: "redis://localhost:6379/1")
p raw.publish("room", "{\"n\":-1}"), raw.publish("#{prefix}:room", "{\"n\":-2}")
p got.pop(timeout: 5), wait.call { also.last == -2 }

pubsub.unsubscribe("room", room)
pubsub.unsubscribe("news", news)
p wait.call { raw.pubsub("numsub", "#{prefix}:news") == ["#{prefix}:news", 0] }
server.broadcast("room", { n: 7 })
p wait.call { also.last == 7 }, got.empty?
pubsub.shutdown
raw.close
puts "done"
