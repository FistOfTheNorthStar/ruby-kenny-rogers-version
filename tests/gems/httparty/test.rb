require "bundler/setup"
require "httparty"
require "webmock"
include WebMock::API
WebMock.enable!
WebMock.disable_net_connect!

stub_request(:get, "http://api.example.com/users/1").to_return(status: 200, body: '{"id":1,"name":"Ann","tags":["a","b"]}', headers: { "Content-Type" => "application/json" })
stub_request(:get, "http://api.example.com/items.xml").to_return(body: "<items><item>1</item><item>2</item></items>", headers: { "Content-Type" => "application/xml" })
stub_request(:post, "http://api.example.com/users").with(body: { name: "Bob" }.to_json, headers: { "Content-Type" => "application/json" })
  .to_return(status: 201, body: '{"id":2}', headers: { "Content-Type" => "application/json", "Location" => "/users/2" })
stub_request(:get, "http://api.example.com/search").with(query: { "q" => "kenny", "page" => "2" }).to_return(body: "found", headers: { "Content-Type" => "text/plain" })
stub_request(:get, "http://api.example.com/missing").to_return(status: 404, body: '{"error":"nope"}', headers: { "Content-Type" => "application/json" })
stub_request(:get, "http://api.example.com/redirect").to_return(status: 302, headers: { "Location" => "http://api.example.com/users/1" })
stub_request(:put, "http://api.example.com/form").with(body: "a=1&b=two").to_return(body: "formed")
stub_request(:get, "http://api.example.com/auth").with(basic_auth: ["u", "p"]).to_return(body: "authed")
stub_request(:delete, "http://api.example.com/users/1").to_return(status: 204)
stub_request(:get, "http://api.example.com/slow").to_timeout
stub_request(:get, "http://api.example.com/csv").to_return(body: "a,b\n1,2\n", headers: { "Content-Type" => "text/csv" })

r = HTTParty.get("http://api.example.com/users/1")
p r.code, r.success?, r.parsed_response, r["name"], r.headers["content-type"], r.body.size
p r.class, r.ok?, r.response.class, r.request.http_method
x = HTTParty.get("http://api.example.com/items.xml")
p x.parsed_response
r = HTTParty.post("http://api.example.com/users", body: { name: "Bob" }.to_json, headers: { "Content-Type" => "application/json" })
p r.code, r.parsed_response, r.headers["location"], r.created?
p HTTParty.get("http://api.example.com/search", query: { q: "kenny", page: 2 }).body
r = HTTParty.get("http://api.example.com/missing")
p r.code, r.not_found?, r.parsed_response, r.success?
p HTTParty.get("http://api.example.com/redirect").parsed_response["name"]
p HTTParty.get("http://api.example.com/redirect", follow_redirects: false).code
p HTTParty.put("http://api.example.com/form", body: { a: 1, b: "two" }).body
p HTTParty.get("http://api.example.com/auth", basic_auth: { username: "u", password: "p" }).body
p HTTParty.delete("http://api.example.com/users/1").code
p HTTParty.get("http://api.example.com/csv").parsed_response

class Api
  include HTTParty
  base_uri "api.example.com"
  format :json
  headers "X-Client" => "kenny"
  default_params token: "t0k"
  def user(id) = self.class.get("/users/#{id}")
end
stub_request(:get, "http://api.example.com/users/1?token=t0k").with(headers: { "X-Client" => "kenny" }).to_return(body: '{"via":"class"}')
p Api.new.user(1).parsed_response, Api.base_uri, Api.default_options[:format]

class Raw
  include HTTParty
  base_uri "http://api.example.com"
  parser(proc { |body, format| "#{format}:#{body.upcase}" })
end
p Raw.get("/search", query: { q: "kenny", page: 2 }).parsed_response

begin
  HTTParty.get("http://api.example.com/slow", timeout: 1)
rescue Net::OpenTimeout => e
  p e.class
end
begin
  HTTParty.get("http://unstubbed.example.com/")
rescue WebMock::NetConnectNotAllowedError => e
  p e.class
end
begin
  HTTParty.get("not a url")
rescue HTTParty::UnsupportedURIScheme, URI::InvalidURIError => e
  p e.class
end
