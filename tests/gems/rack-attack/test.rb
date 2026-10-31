require "bundler/setup"
require "rack/attack"
require "rack/test"
require "active_support"
require "active_support/cache"
require "active_support/notifications"

Rack::Attack.cache.store = ActiveSupport::Cache::MemoryStore.new
EVENTS = []
ActiveSupport::Notifications.subscribe(/rack_attack/) do |name, _s, _f, _id, payload|
  EVENTS << [name, payload[:request].env["rack.attack.matched"]]
end

Rack::Attack.safelist("allow localhost") { |req| req.ip == "10.0.0.1" }
Rack::Attack.blocklist("block bad ua") { |req| req.user_agent == "BadBot" }
Rack::Attack.blocklist_ip("192.168.5.0/24")
Rack::Attack.safelist_ip("172.16.0.9")
Rack::Attack.throttle("req/ip", limit: 3, period: 60) { |req| req.ip unless req.path.start_with?("/free") }
Rack::Attack.throttle("logins/email", limit: 2, period: 60) do |req|
  req.params["email"].to_s.downcase if req.path == "/login" && req.post?
end
Rack::Attack.track("special") { |req| req.path == "/special" }
Rack::Attack.blocklist("fail2ban") do |req|
  Rack::Attack::Fail2Ban.filter("f2b-#{req.ip}", maxretry: 2, findtime: 60, bantime: 60) { req.path.include?("wp-admin") }
end
Rack::Attack.blocklisted_responder = ->(req) { [403, { "content-type" => "text/plain" }, ["Blocked #{req.env["rack.attack.matched"]}\n"]] }
Rack::Attack.throttled_responder = lambda do |req|
  d = req.env["rack.attack.match_data"]
  [429, { "content-type" => "text/plain" }, ["Throttled #{req.env["rack.attack.matched"]} #{d[:count]}/#{d[:limit]}\n"]]
end

include Rack::Test::Methods
def app
  @app ||= Rack::Builder.new do
    use Rack::Attack
    run ->(env) { [200, { "content-type" => "text/plain" }, ["ok #{env["PATH_INFO"]}"]] }
  end.to_app
end

def hit(method, path, ip: "1.2.3.4", **env)
  send(method, path, env.delete(:params) || {}, { "REMOTE_ADDR" => ip }.merge(env))
  puts "#{method.upcase} #{path} #{ip}: #{last_response.status} #{last_response.body.strip}"
end

4.times { hit :get, "/x" }
hit :get, "/free/y"
hit :get, "/x", ip: "10.0.0.1"
5.times { hit :get, "/x", ip: "10.0.0.1" } and nil
hit :get, "/x", ip: "5.5.5.5", "HTTP_USER_AGENT" => "BadBot"
hit :get, "/x", ip: "192.168.5.77"
hit :get, "/x", ip: "172.16.0.9"
3.times { |i| hit :post, "/login", ip: "9.9.9.#{i}", params: { "email" => "A@x.io" } }
hit :get, "/special", ip: "7.7.7.7"
3.times { hit :get, "/wp-admin/x", ip: "8.8.8.8" }
hit :get, "/ok", ip: "8.8.8.8"
p EVENTS.map(&:first).tally.sort
p EVENTS.select { |n, _| n.start_with?("track") }
Rack::Attack.reset!
hit :get, "/x"
p Rack::Attack.throttles.keys.sort, Rack::Attack.safelists.keys.sort, Rack::Attack.blocklists.keys.sort
Rack::Attack.enabled = false
hit :get, "/x", ip: "192.168.5.1"
