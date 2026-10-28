require "bundler/setup"
require "jwt"

secret = "my$ecretK3y-that-is-long-enough-for-hs256!"
iat = 1_700_000_000
payload = { "sub" => "user1", "name" => "Kenny", "admin" => true, "iat" => iat, "exp" => 4_102_444_800, "nbf" => iat }
token = JWT.encode(payload, secret, "HS256")
puts token
decoded = JWT.decode(token, secret, true, algorithm: "HS256")
p decoded
p JWT.decode(token, nil, false)[0]["name"]
t384 = JWT.encode({ "a" => 1 }, secret, "HS384", { "kid" => "k1" })
puts t384
p JWT.decode(t384, secret, true, algorithms: ["HS384"])
p JWT.encode({ "x" => [1, 2] }, nil, "none")
p JWT.decode(JWT.encode({ "x" => 1 }, nil, "none"), nil, false)

expired = JWT.encode({ "exp" => 1_000 }, secret, "HS256")
begin
  JWT.decode(expired, secret, true, algorithm: "HS256")
rescue JWT::ExpiredSignature => e
  puts "ExpiredSignature: #{e.message}"
end
begin
  JWT.decode(token, "wrong-secret-wrong-secret-wrong-secret!!", true, algorithm: "HS256")
rescue JWT::VerificationError => e
  puts "VerificationError: #{e.message}"
end
begin
  JWT.decode("not.a.token", secret, true, algorithm: "HS256")
rescue JWT::DecodeError => e
  puts "DecodeError: #{e.class}"
end
begin
  JWT.decode(token, secret, true, algorithm: "HS256", iss: "me", verify_iss: true)
rescue JWT::InvalidIssuerError => e
  puts "InvalidIssuerError: #{e.message}"
end
aud = JWT.encode({ "aud" => ["api"], "iss" => "me" }, secret, "HS512")
p JWT.decode(aud, secret, true, algorithm: "HS512", aud: "api", verify_aud: true, iss: "me", verify_iss: true)[0]
p JWT::Base64.url_encode("\xff\xfe binary".b), JWT::Base64.url_decode("__4gYmluYXJ5")
rsa = OpenSSL::PKey::RSA.new(2048) rescue (puts "RSA: #{$!.class}"; nil)
if rsa
  rt = JWT.encode({ "r" => 1 }, rsa, "RS256")
  p JWT.decode(rt, rsa.public_key, true, algorithm: "RS256")[0]
end
