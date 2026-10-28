require "bundler/setup"
require "faraday"

stubs = Faraday::Adapter::Test::Stubs.new do |stub|
  stub.get("/users/1") { [200, { "Content-Type" => "application/json" }, '{"id":1,"name":"kenny"}'] }
  stub.post("/users") { |env| [201, { "Content-Type" => "application/json" }, %({"echo":#{env.body}})] }
  stub.get("/search") { |env| [200, {}, env.params.sort.inspect] }
  stub.get("/fail") { [500, {}, "boom"] }
  stub.get("/missing") { [404, {}, "nope"] }
end

conn = Faraday.new(url: "https://api.example.com", headers: { "User-Agent" => "kenny-test" }) do |f|
  f.request :json
  f.response :json, content_type: /\bjson$/
  f.response :raise_error
  f.adapter :test, stubs
end

r = conn.get("/users/1")
p r.status, r.body, r.headers["content-type"], r.success?
r = conn.post("/users", { name: "new", tags: [1, 2] })
p r.status, r.body
r = conn.get("/search", { q: "a b", page: 2 })
p r.body, r.env.url.to_s
begin
  conn.get("/fail")
rescue Faraday::ServerError => e
  p e.class, e.response[:status], e.response[:body]
end
begin
  conn.get("/missing")
rescue Faraday::ResourceNotFound => e
  p e.class, e.response_status
end
stubs.verify_stubbed_calls

p Faraday::Utils.build_query(a: 1, b: [1, 2], c: "x y")
p Faraday::Utils.build_nested_query(a: { b: [1, 2] })
p Faraday::Utils.parse_nested_query("a[b][]=1&a[b][]=2&c=3")
p Faraday::Utils.escape("a b&c"), Faraday::Utils.unescape("a%20b")
h = Faraday::Utils::Headers.new("content-type" => "x")
p h["Content-Type"], h.keys

class Shout < Faraday::Middleware
  def on_complete(env) = env.body = env.body.upcase
end
c2 = Faraday.new do |f|
  f.use Shout
  f.request :url_encoded
  f.adapter(:test) { |s| s.post("/form") { |env| [200, {}, "got #{env.body} #{env.request_headers["Content-Type"]}"] } }
end
p c2.post("/form", { x: 1, y: "z" }).body
p c2.builder.handlers.map(&:name)
