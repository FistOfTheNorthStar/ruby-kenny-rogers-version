require "bundler/setup"
require "rest-client"
require "webmock"
require "json"
include WebMock::API
WebMock.enable!
WebMock.disable_net_connect!

stub_request(:get, "http://api.example.com/users/1").to_return(status: 200, body: '{"id":1,"name":"Ann"}', headers: { "Content-Type" => "application/json", "Set-Cookie" => "sid=abc; path=/" })
stub_request(:post, "http://api.example.com/users").with(body: "name=Bob&age=3", headers: { "Content-Type" => "application/x-www-form-urlencoded" })
  .to_return(status: 201, body: '{"id":2}')
stub_request(:post, "http://api.example.com/json").with(body: '{"a":1}', headers: { "Content-Type" => "application/json" }).to_return(body: "json ok")
stub_request(:get, "http://api.example.com/search?q=kenny&page=2").to_return(body: "found")
stub_request(:get, "http://api.example.com/missing").to_return(status: 404, body: "not here")
stub_request(:get, "http://api.example.com/boom").to_return(status: 500, body: "err")
stub_request(:get, "http://api.example.com/redirect").to_return(status: 301, headers: { "Location" => "http://api.example.com/users/1" })
stub_request(:put, "http://api.example.com/users/1").to_return(status: 200, body: "updated")
stub_request(:delete, "http://api.example.com/users/1").to_return(status: 204)
stub_request(:patch, "http://api.example.com/users/1").to_return(body: "patched")
stub_request(:head, "http://api.example.com/users/1").to_return(headers: { "X-Head" => "yes" })
stub_request(:get, "http://api.example.com/auth").with(basic_auth: ["u", "p"]).to_return(body: "authed")
stub_request(:get, "http://api.example.com/hdr").with(headers: { "X-Custom" => "1", "Accept" => "application/json" }).to_return(body: "hdr ok")
stub_request(:get, "http://api.example.com/slow").to_timeout

r = RestClient.get("http://api.example.com/users/1")
p r.code, r.body, r.headers[:content_type], r.cookies, r.class
p JSON.parse(r.body)["name"], r.request.method, r.request.url
r = RestClient.post("http://api.example.com/users", { name: "Bob", age: 3 })
p r.code, r.body
p RestClient.post("http://api.example.com/json", { a: 1 }.to_json, content_type: :json).body
p RestClient.get("http://api.example.com/search", params: { q: "kenny", page: 2 }).body
p RestClient.put("http://api.example.com/users/1", "x").body
p RestClient.delete("http://api.example.com/users/1").code
p RestClient.patch("http://api.example.com/users/1", "y").body
p RestClient.head("http://api.example.com/users/1").headers[:x_head]
p RestClient.get("http://api.example.com/redirect").body
p RestClient::Request.execute(method: :get, url: "http://api.example.com/auth", user: "u", password: "p").body
p RestClient.get("http://api.example.com/hdr", x_custom: "1", accept: :json).body
res = RestClient::Resource.new("http://api.example.com", headers: { accept: :json })
p res["users/1"].get.code, res["users"].url
RestClient.get("http://api.example.com/missing") { |resp, _req, _res| p [:block, resp.code, resp.body] }

begin
  RestClient.get("http://api.example.com/missing")
rescue RestClient::NotFound => e
  p e.class, e.http_code, e.response.body, e.message
end
begin
  RestClient.get("http://api.example.com/boom")
rescue RestClient::ExceptionWithResponse => e
  p e.class, e.http_code, e.class.ancestors.include?(RestClient::ServerBrokeConnection)
end
begin
  RestClient::Request.execute(method: :get, url: "http://api.example.com/slow", timeout: 1)
rescue RestClient::Exceptions::Timeout => e
  p e.class.ancestors.include?(RestClient::Exceptions::Timeout)
end
begin
  RestClient.get("http://unstubbed.example.com/")
rescue WebMock::NetConnectNotAllowedError => e
  p e.class
end
p RestClient::STATUSES[418], RestClient::Exceptions::EXCEPTIONS_MAP[404]
