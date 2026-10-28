# OpenSSL::PKey::RSA, OpenSSL::BN and the SSL/X509 names gems refer to when loading
require "openssl"
require "stringio"
require "tmpdir"

PEM = <<~KEY
  -----BEGIN RSA PRIVATE KEY-----
  MIICXAIBAAKBgQDLArLIrSxas0/sipFpEMcv0tcm38zhk2/j03hvojz+j2eqN71K
  z3vo0CXM/kmPaH/Jsv+dSlwcxmz3fzpzQR6dk1fbqo41H+VpDs9ZUQLnZLS/wjU5
  uehW12ABeO8Nm9ciqLcpaMKbY6n/VhBtNXXCNumeB3n84eBHtO7wqT+A3QIDAQAB
  AoGAF5qktviLo7Adpt8lsRWmU0UshvSWmdPOuqqn4jL27JI3926rr7sYKSWqf39B
  hk8cLJOHbP5sX1wi51P/s87qhCqWdV17X4C+AlhqJ8IImiQhqsJ+BptZF+NzDvpQ
  zcT/cXzkxapZlQ3XtFa5PQ//cmQqg0gV9Lh9MsIAvCMcJvECQQD5l4VsJ4NGVnZz
  M7fmZg3XJOFbaadPVDafb3asDVnl++AjIgFK748mgidbquQ5XcOpDXwzXoh1tI8w
  /wqHLJNPAkEA0DkDvAZPEwYKtyPBQkkfir8TEwxOxoG9sohQTCZec24xCmllbLuM
  /1BapsHdESy+VPxErk3SRjXTJmxmeWnOEwJALT5ySTDAczepMSjm5TUKvT1KPK/i
  TFcPr8cc1AN3kGEgOD4pzdOvN2PQjqVjyaJdnX98TDOOuQeKDW+uEOLbsQJASqnn
  GOj6OcwU047Sob4uvUW+xTnIlY24Q+h1QpkPjTyaAS3B3Ldz26komFszthDhPwAm
  cIofOGXcjQ6ZwDecWQJBAJETAqc3kcb2pbLvHuWqIN43INI+bH5bTsmL7fOJzT7R
  CZZf7ItH0JllN+GJuQRnT/igchqc/0M1zgwPu6sbx6E=
  -----END RSA PRIVATE KEY-----
KEY

k = OpenSSL::PKey::RSA.new(PEM)
p k.private?, k.public?, k.n.num_bits, k.e.to_i, k.oid
p k.to_pem == PEM, k.export == PEM, k.to_s == PEM
p k.to_der.bytesize, k.to_der.encoding
puts k.private_to_pem.lines.first, k.public_to_pem
p k.public_to_der.unpack1("H*")[0, 48]

pub = k.public_key
p pub.class, pub.private?, pub.public?, pub.to_pem == k.public_to_pem, pub.d
p k.compare?(pub), pub.compare?(k)

# PKCS#1 v1.5 signatures are deterministic
%w[SHA256 SHA1 SHA384 SHA512 MD5].each do |d|
  sig = k.sign(d, "hello")
  puts "#{d} #{sig.unpack1("H*")[0, 32]} #{sig.bytesize} #{pub.verify(d, sig, "hello")} #{pub.verify(d, sig, "hellx")}"
end
sig = k.sign(OpenSSL::Digest.new("SHA256"), "hello")
p sig == k.sign("SHA256", "hello"), sig.encoding, pub.verify(OpenSSL::Digest::SHA256.new, sig, "hello")
p pub.verify("SHA256", "short", "hello"), pub.verify("SHA256", sig.reverse, "hello")

# PSS (salted, so just the round trip)
s = k.sign_pss("SHA256", "m", salt_length: :digest, mgf1_hash: "SHA256")
p s.bytesize, pub.verify_pss("SHA256", s, "m", salt_length: :auto, mgf1_hash: "SHA256"),
  pub.verify_pss("SHA256", s, "m", salt_length: 32, mgf1_hash: "SHA256"),
  pub.verify_pss("SHA256", s, "n", salt_length: :auto, mgf1_hash: "SHA256")
s = k.sign_pss("SHA512", "m", salt_length: :max, mgf1_hash: "SHA512")
p pub.verify_pss("SHA512", s, "m", salt_length: :auto, mgf1_hash: "SHA512")

# loading: PKCS#8, SPKI, DER, PKey.read, surrounding text
p OpenSSL::PKey::RSA.new(k.private_to_pem).to_der == k.to_der
p OpenSSL::PKey::RSA.new(k.public_to_pem).private?
p OpenSSL::PKey::RSA.new(k.to_der).private?, OpenSSL::PKey::RSA.new(k.public_to_der).private?
p OpenSSL::PKey::RSA.new(k.private_to_der).to_pem == PEM
p OpenSSL::PKey.read(PEM).class, OpenSSL::PKey.read(k.public_to_pem).private?
p OpenSSL::PKey::RSA.new("leading text\n#{PEM}trailing\n").private?
begin
  OpenSSL::PKey::RSA.new(StringIO.new(PEM))
rescue TypeError => e
  puts "TypeError: #{e.message}"
end
path = File.join(Dir.tmpdir, "kenny_openssl_pkey_#{$$}.pem")
File.write(path, PEM)
p File.open(path) { |f| OpenSSL::PKey::RSA.new(f).private? }
File.delete(path)
p OpenSSL::PKey::RSA.new(PEM, "ignored-pass").private?

[-> { OpenSSL::PKey::RSA.new("junk") }, -> { OpenSSL::PKey.read("junk") }, -> { pub.sign("SHA256", "x") },
 -> { pub.private_to_pem }, -> { k.sign("ROT13", "x") }, -> { k.set_key(1, 2, 3) }].each do |f|
  f.call
rescue => e
  puts "#{e.class}: #{e.message}"
end

# parameters as OpenSSL::BN
pr = k.params
p pr.keys, pr.values.map(&:class).uniq, pr["e"].to_i
p k.p.to_i * k.q.to_i == k.n.to_i, k.d.num_bits > 1000, (k.q.to_i * k.iqmp.to_i) % k.p.to_i
p k.dmp1.to_i == k.d.to_i % (k.p.to_i - 1), k.dmq1.to_i == k.d.to_i % (k.q.to_i - 1)
p pub.params["d"].to_i, pub.p

# copies, Marshal
p k.dup.to_der == k.to_der, k.clone.private?
p Marshal.load(Marshal.dump(k)).to_der == k.to_der, Marshal.load(Marshal.dump(pub)).private?

# new keys
g = OpenSSL::PKey::RSA.new(1024)
p g.n.num_bits, g.e.to_i, g.verify("SHA256", g.sign("SHA256", "x"), "x"), g.compare?(k)
p OpenSSL::PKey::RSA.new(1024, 3).e.to_i, OpenSSL::PKey::RSA.generate(1024).n.num_bytes
p OpenSSL::PKey.generate_key("RSA", "rsa_keygen_bits" => 1024).n.num_bits
p OpenSSL::PKey::RSA.new(OpenSSL::PKey::RSA.new(1024).to_pem).private?

# OpenSSL::BN
b = OpenSSL::BN.new("65537")
p b.to_s(16), b.to_s(2).bytes, b.to_s, b.to_i, b.num_bits, b.num_bytes
p OpenSSL::BN.new("ff", 16).to_i, OpenSSL::BN.new("-1F", 16).to_i, OpenSSL::BN.new("\x01\x00".b, 2).to_i
p OpenSSL::BN.new(255).to_s(16), OpenSSL::BN.new(0).to_s(16), OpenSSL::BN.new(0).to_s(2), OpenSSL::BN.new(-255).to_s(16)
p 5.to_bn.class, 5.to_bn == 5, 5.to_bn == OpenSSL::BN.new(5), 5.to_bn.eql?(5), OpenSSL::BN.new(7) > 3
p (OpenSSL::BN.new(7) + 3).to_i, (OpenSSL::BN.new(7) * 3).to_i, (OpenSSL::BN.new(-7) / 2).map(&:to_i), (OpenSSL::BN.new(-7) % 2).to_i
p OpenSSL::BN.new(4).mod_exp(13, 497).to_i, OpenSSL::BN.new(2**200).num_bytes, OpenSSL::BN.new(3).zero?
begin
  OpenSSL::BN.new("xyz")
rescue => e
  p e.class
end

# names gems refer to when loading
p OpenSSL::PKey::RSA.ancestors.include?(OpenSSL::PKey::PKey), OpenSSL::PKey::RSAError.superclass, OpenSSL::PKey::PKeyError.superclass
p OpenSSL::SSL::SSLErrorWaitReadable.ancestors.take(3), OpenSSL::SSL::SSLErrorWaitWritable.superclass
p OpenSSL::SSL::OP_ALL, OpenSSL::SSL::OP_NO_SSLv3, OpenSSL::SSL::OP_NO_COMPRESSION, OpenSSL::SSL::TLS1_2_VERSION
p OpenSSL::X509::V_OK, OpenSSL::X509::V_FLAG_CRL_CHECK_ALL, OpenSSL::X509::PURPOSE_ANY, OpenSSL::X509::DEFAULT_CERT_FILE_ENV
st = OpenSSL::X509::Store.new
st.set_default_paths
st.flags = OpenSSL::X509::V_FLAG_CRL_CHECK
p st.class, OpenSSL::X509::Certificate.new.class
