require "bundler/setup"
require "signet/oauth_2/client"
require "signet/oauth_1"
require "faraday"

client = Signet::OAuth2::Client.new(
  authorization_uri: "https://accounts.example.com/o/oauth2/auth",
  token_credential_uri: "https://oauth2.example.com/token",
  client_id: "client-id", client_secret: "client-secret",
  scope: ["email", "profile"], redirect_uri: "https://app.example.com/callback",
  state: "xyz"
)
uri = client.authorization_uri(access_type: "offline")
p uri.host, uri.path, uri.query.split("&").sort
p client.scope

seen = []
stubs = Faraday::Adapter::Test::Stubs.new do |s|
  s.post("/token") do |env|
    seen << URI.decode_www_form(env.body).sort
    [200, { "Content-Type" => "application/json" }, '{"access_token":"ya29.tok","token_type":"Bearer","expires_in":3600,"refresh_token":"r1","id_token":null}']
  end
end
conn = Faraday.new { |f| f.adapter :test, stubs }
client.code = "authcode"
client.fetch_access_token!(connection: conn)
p seen.last
p client.access_token, client.refresh_token, client.expires_in, client.expired?, client.grant_type
req = client.generate_authenticated_request(method: "GET", uri: "https://api.example.com/me", headers: { "Accept" => "*/*" }, connection: conn)
p req.http_method, req.path, req.headers.sort
client.refresh_token = "r1"
client.code = nil
client.fetch_access_token!(connection: conn)
p seen.last
p client.grant_type

jwt = Signet::OAuth2::Client.new(token_credential_uri: "https://oauth2.example.com/token", issuer: "svc@example.com",
  audience: "https://oauth2.example.com/token", scope: "a b", signing_key: "hmac-secret", signing_algorithm: "HS256")
assertion = jwt.to_jwt
payload, header = JWT.decode(assertion, "hmac-secret", true, algorithm: "HS256")
p header, payload.slice("iss", "aud", "scope"), payload["exp"] - payload["iat"]
p Signet::OAuth2.parse_credentials('{"access_token":"t","expires_in":10}', "application/json")
p Signet::OAuth2.generate_basic_authorization_header("user", "pass")
p Signet::OAuth2.parse_authorization_header("Basic dXNlcjpwYXNz"), Signet::OAuth2.parse_authorization_header("OAuth abc")
p Signet::OAuth2.parse_www_authenticate_header('OAuth realm="example", error="invalid_token"')

params = [["oauth_consumer_key", "dpf43f3p2l4k3l03"], ["oauth_token", "nnch734d00sl2jdk"],
          ["oauth_signature_method", "HMAC-SHA1"], ["oauth_timestamp", "1191242096"],
          ["oauth_nonce", "kllo9940pd9333jh"], ["oauth_version", "1.0"], ["file", "vacation.jpg"], ["size", "original"]]
base = Signet::OAuth1.generate_base_string("GET", "http://photos.example.net/photos", params)
puts base
p Signet::OAuth1.sign_parameters("GET", "http://photos.example.net/photos", params, "kd94hf93k423kf44", "pfkkdhi9sl3r4s00")
p Signet::OAuth1.encode("a b+c~"), Signet::OAuth1.unencode("a%20b")
p Signet::OAuth1.parse_authorization_header('OAuth realm="x", oauth_token="t%20k"')
begin
  stubs.post("/bad") { [400, { "Content-Type" => "application/json" }, '{"error":"invalid_grant","error_description":"Bad Request"}'] }
  Signet::OAuth2::Client.new(token_credential_uri: "https://x/bad", client_id: "c", refresh_token: "old").fetch_access_token!(connection: conn)
rescue Signet::AuthorizationError => e
  p e.class, e.message.lines.first.strip, e.response.status
end
