require "bundler/setup"
require "googleauth"
require "stringio"
require "json"
require "faraday"

key = OpenSSL::PKey::RSA.new(2048)
json = JSON.generate(
  type: "service_account", project_id: "kenny-proj", private_key_id: "kid123",
  private_key: key.to_pem, client_email: "svc@kenny-proj.iam.gserviceaccount.com", client_id: "1234",
  token_uri: "https://oauth2.googleapis.com/token"
)
creds = Google::Auth::ServiceAccountCredentials.make_creds(json_key_io: StringIO.new(json), scope: "https://www.googleapis.com/auth/cloud-platform")
p creds.class, creds.issuer, creds.project_id, creds.client_id, creds.scope, creds.token_credential_uri.to_s

seen = nil
stubs = Faraday::Adapter::Test::Stubs.new do |s|
  s.post("/oauth2/v4/token") do |env|
    seen = URI.decode_www_form(env.body).to_h
    [200, { "Content-Type" => "application/json" }, '{"access_token":"ya29.svc","token_type":"Bearer","expires_in":3599}']
  end
end
conn = Faraday.new { |f| f.adapter :test, stubs }
creds.fetch_access_token!(connection: conn)
p seen["grant_type"]
payload, header = JWT.decode(seen["assertion"], key.public_key, true, algorithm: "RS256")
p header, payload.slice("iss", "aud", "scope"), payload["exp"] - payload["iat"]
p creds.access_token, creds.expires_in
p creds.apply({})

user = Google::Auth::UserRefreshCredentials.make_creds(json_key_io: StringIO.new(JSON.generate(
  type: "authorized_user", client_id: "cid", client_secret: "csecret", refresh_token: "rtok", quota_project_id: "qp")), scope: ["email"])
p user.class, user.client_id, user.quota_project_id
stubs.post("/token") { |env| seen = URI.decode_www_form(env.body).to_h; [200, { "Content-Type" => "application/json" }, '{"access_token":"ya29.user","expires_in":100}'] }
user.fetch_access_token!(connection: conn)
p seen.sort.to_h, user.access_token
p user.apply({})

jwt_creds = Google::Auth::ServiceAccountJwtHeaderCredentials.make_creds(json_key_io: StringIO.new(json))
h = jwt_creds.apply({ jwt_aud_uri: "https://pubsub.googleapis.com/" })
tok = h[:authorization].sub("Bearer ", "")
pl, hd = JWT.decode(tok, key.public_key, true, algorithm: "RS256")
p hd, pl.slice("iss", "sub", "aud")
p Google::Auth::ServiceAccountCredentials.ancestors.include?(Signet::OAuth2::Client)
begin
  Google::Auth::ServiceAccountCredentials.make_creds(json_key_io: StringIO.new('{"type":"service_account"}'))
rescue => e
  p e.class
end
