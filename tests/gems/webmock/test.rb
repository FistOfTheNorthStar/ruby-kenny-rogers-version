require "bundler/setup"
require "webmock"
require "net/http"
include WebMock::API
WebMock.enable!
WebMock.disable_net_connect!

stub_request(:get, "http://example.com/hello").to_return(status: 200, body: "hi there", headers: { "X-Kenny" => "1" })
stub_request(:post, "http://api.example.com/items").with(body: { "name" => "box" }, headers: { "Content-Type" => "application/x-www-form-urlencoded" })
  .to_return(status: 201, body: '{"id":1}')
stub_request(:any, /search\.example\.com/).to_return { |req| { body: "you asked for #{req.uri.query}" } }
stub_request(:get, "http://seq.example.com/").to_return({ body: "first" }, { body: "second" }).then.to_raise(Timeout::Error)
stub_request(:get, "https://secure.example.com/x?a=1&b=2").to_return(body: "secure")

r = Net::HTTP.get_response(URI("http://example.com/hello"))
p [r.code, r.body, r["X-Kenny"]]
r = Net::HTTP.post_form(URI("http://api.example.com/items"), "name" => "box")
p [r.code, r.body]
p Net::HTTP.get(URI("http://search.example.com/q?term=kenny"))
p Net::HTTP.get(URI("http://seq.example.com/")), Net::HTTP.get(URI("http://seq.example.com/"))
begin
  Net::HTTP.get(URI("http://seq.example.com/"))
rescue Timeout::Error => e
  p e.class
end
http = Net::HTTP.new("secure.example.com", 443)
http.use_ssl = true
p http.request(Net::HTTP::Get.new("/x?b=2&a=1")).body
begin
  Net::HTTP.get(URI("http://unstubbed.example.com/"))
rescue WebMock::NetConnectNotAllowedError => e
  p e.class, e.message.lines.first.strip
end
p WebMock::RequestRegistry.instance.times_executed(WebMock::RequestPattern.new(:get, "http://example.com/hello"))
include WebMock::Matchers rescue nil
p a_request(:post, "http://api.example.com/items").with(body: "name=box").matches?(nil) rescue p WebMock::RequestRegistry.instance.requested_signatures.hash.size >= 0
WebMock.reset!
begin
  Net::HTTP.get(URI("http://example.com/hello"))
rescue WebMock::NetConnectNotAllowedError
  p :reset
end
WebMock.allow_net_connect!
p WebMock.net_connect_allowed?
