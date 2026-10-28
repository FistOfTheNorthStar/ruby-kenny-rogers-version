require "bundler/setup"
require "puma"
require "puma/configuration"
require "puma/server"
require "puma/log_writer"
require "net/http"

p Puma::Const::PUMA_VERSION.is_a?(String)

conf = Puma::Configuration.new({}, {}, {}) do |user|
  user.threads 1, 3
  user.workers 0
  user.bind "tcp://127.0.0.1:9292"
  user.environment "production"
  user.persistent_timeout 7
  user.app { |env| [200, {}, ["dsl app"]] }
end
conf.clamp
o = conf.options
p o[:min_threads], o[:max_threads], o[:workers], o[:binds], o[:environment], o[:persistent_timeout]
p conf.app.call({})[2]

app = lambda do |env|
  body = env["rack.input"].read
  [200, { "content-type" => "text/plain", "x-method" => env["REQUEST_METHOD"] },
   ["#{env["REQUEST_METHOD"]} #{env["PATH_INFO"]} q=#{env["QUERY_STRING"]} body=#{body} ver=#{env["SERVER_PROTOCOL"]}"]]
end
server = Puma::Server.new(app, nil, { min_threads: 0, max_threads: 2, log_writer: Puma::LogWriter.null })
server.add_tcp_listener "127.0.0.1", 0
port = server.connected_ports.first
p port.is_a?(Integer) && port > 0
thread = server.run
Net::HTTP.start("127.0.0.1", port) do |http|
  r = http.get("/hello?x=1")
  p r.code, r["content-type"], r["x-method"], r.body
  r = http.post("/form", "a=1&b=2", "Content-Type" => "application/x-www-form-urlencoded")
  p r.code, r.body
  r = http.request(Net::HTTP::Put.new("/put"), "data")
  p r.code, r.body
end
3.times.map { |i| Thread.new { Net::HTTP.get(URI("http://127.0.0.1:#{port}/t#{i}")) } }.map(&:value).each { |b| p b }
server.stop(true)
p thread.alive?
p Puma::Util.parse_query("a=1&b=2")
