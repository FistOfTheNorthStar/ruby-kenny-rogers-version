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

# ECDSA: a token CRuby signed, then each curve jwt has, and JWKs (export, import, thumbprint)
es_pem = <<~PEM
  -----BEGIN EC PRIVATE KEY-----
  MHcCAQEEICqCYONapr4q5OUTiiMCCGasKbPE3APg1Zodp2cvmZ4AoAoGCCqGSM49
  AwEHoUQDQgAEIu3EYoSl7QLVRQ2iEWrBBesgCJver7BUnv8s5Sp6cNf1tpvf2d89
  GK05MLOIJO5AfdwH8lxbUfcRdqoa2wFPfw==
  -----END EC PRIVATE KEY-----
PEM
es_key = OpenSSL::PKey::EC.new(es_pem)
fixed = "eyJraWQiOiJwMjU2IiwiYWxnIjoiRVMyNTYifQ.eyJmaXhlZCI6dHJ1ZSwibiI6N30." \
        "hUgoduuSUDH1q101yK1C8CzSkw0MzscuOKOddZ1HCilDsdstJz-YT4ndshPju6XvJGt-sDj-FCPxTN6VWJ3sHw"
p JWT.decode(fixed, OpenSSL::PKey::EC.new(es_key.public_to_pem), true, algorithm: "ES256")
begin
  JWT.decode(fixed.sub(/.\z/, "A"), es_key, true, algorithm: "ES256")
rescue JWT::VerificationError => e
  puts "ES256 tampered: #{e.message}"
end
{ "ES256" => "prime256v1", "ES384" => "secp384r1", "ES512" => "secp521r1", "ES256K" => "secp256k1" }.each do |alg, curve|
  key = OpenSSL::PKey::EC.generate(curve)
  t = JWT.encode({ "alg" => alg }, key, alg)
  pub = OpenSSL::PKey::EC.new(key.public_to_der)
  p [alg, t.split(".")[2].size, JWT.decode(t, pub, true, algorithm: alg)[0]]
  begin
    JWT.decode(t, OpenSSL::PKey::EC.generate(curve), true, algorithm: alg)
  rescue JWT::VerificationError => e
    puts "#{alg} other key: #{e.message}"
  end
end
jwk = JWT::JWK.new(es_key, kid: "p256")
exported = jwk.export
p exported
p jwk.export(include_private: true).keys
p JWT::JWK::Thumbprint.new(jwk).to_s
imported = JWT::JWK.import(jwk.export(include_private: true))
p imported.signing_key.private?, imported.verify_key.to_pem == es_key.public_to_pem
p JWT.decode(fixed, JWT::JWK.import(exported).verify_key, true, algorithm: "ES256")[0]
jwks = JWT::JWK::Set.new([jwk])
p JWT.decode(JWT.encode({ "s" => 1 }, imported.signing_key, "ES256", { kid: "p256" }), nil, true, algorithms: ["ES256"], jwks: jwks)[0]
