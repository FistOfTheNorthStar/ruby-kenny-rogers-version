# OpenSSL::PKey::EC (P-256, P-384, P-521, secp256k1) and Ed25519 keys: their formats, ECDSA,
# ECDH and point arithmetic, against keys and signatures OpenSSL made
require "openssl"

def t(label)
  r = yield
  puts "#{label}: #{r.inspect}"
rescue StandardError, NotImplementedError => e
  puts "#{label}: #{e.class}: #{e.message}"
end

h = ->(s) { OpenSSL::Digest.hexdigest("SHA256", s)[0, 16] }

KEYS = {
  "prime256v1" => [<<~PEM, "3045022100c2fa3410f0ee996dccf4ae92ace4b4d3cc551b7f708f95ad9ed2f73764f66442022035a7562b5fd3eb6ae7301ca7cf41acf3a83de69e8f7bddf300b1b558c57734e9"],
  -----BEGIN EC PRIVATE KEY-----
  MHcCAQEEIEsk3nBOyaTQaIYP9dajPuXuu0HK7HT2b/wxlKK7iRshoAoGCCqGSM49
  AwEHoUQDQgAEv1KnYMLPfjl/26IkaWAvemiq5VpnRDuVgG+osToZJNuzDsZ8ESvV
  G6Mst1yciwqJOGjHO+xsZ5V9++OroRGWZA==
  -----END EC PRIVATE KEY-----
  PEM
  "secp384r1" => [<<~PEM, "306502310095c6ea6b3953e2d27ebc6805064295910b3bd076f74e40fe9057bb59f258fdcd34d8d9a12eec927d06268c99754fa4ea02304af0bf2793d91ded9a4d1f84ccd31714e701e595261c7a527f975978ab640958713cbb21b01e942123a8c6917d039a11"],
  -----BEGIN EC PRIVATE KEY-----
  MIGkAgEBBDBqDfX41PGa9IRvizqQsa+2VYk/dqNarI0TEbX9dj4ZBIYk+yfeCIA0
  8c3D1sYZWVigBwYFK4EEACKhZANiAAQEVIDlfylC7IWa4h5UWrW6a7CUJqTAKcQ3
  HADPV+VdanUo7UzNq88tVu/zgwWcF9CRc9tF+HJ+ynin26nPTc0C6szsOUX0cGFF
  lG19yzf7fgbJuSwH0xlR6uDWl5A9/GM=
  -----END EC PRIVATE KEY-----
  PEM
  "secp521r1" => [<<~PEM, "308188024200e95245a31c7ee487086252bf1365843248e691418102fc155f5da056fa3eaa2996db04d526046b8fe5359405e36baa0f23ac64bc4e34a478e747b515828a49207b024201a3d362ebc46507a4fcab529ffcddecfc2053da520df62c74cb7e0b1141a0e8fcc26f07872299e399c0c5e773c9570170c2a4c6bb50874cca7a1b665029abe2dfeb"],
  -----BEGIN EC PRIVATE KEY-----
  MIHcAgEBBEIAXRDbrybdga5bEgk6Ab99jhsjnb9G/ipBi9ttz/etyViQg/f6Vvh7
  6I7lXYTienplzTKOOgZUFrwnZviGvrqtoTqgBwYFK4EEACOhgYkDgYYABAG4VvrD
  M+iNGA1kgppHvN0v+rZ7i6oaoKSoAcMI7NNTJzBbOHuBmEq9WpGhWj20BXPo7xn4
  en7zIzj7vzXF6OuvIgAV9N8qofbtxRNWjK7bwJRTILIvt8OG7VrxoYg5xHhyr7AK
  248lvXSpb8BME3YbiioPTc/stOuk3AXBg+RFBROB9Q==
  -----END EC PRIVATE KEY-----
  PEM
  "secp256k1" => [<<~PEM, "304502210095c3857402ac8ac1fa0ae98351266b29835dc861f42f06dccb43b3f91a8114cd0220020b9dac79614e4361413ea54d6457fbd4875449544aecb488e2e94fa5495099"],
  -----BEGIN EC PRIVATE KEY-----
  MHQCAQEEIE3V1+9KOFmEGOaRfrpE+NQ+Zd//HPMr9hjgC84SxpYVoAcGBSuBBAAK
  oUQDQgAELyLzzpP6gSeDY2C5f2ECFaC+KuRxQOsbFxljozTExtywuDfkVNl0dmXM
  jbaMB+BJmNtR/yI3mNEHzWLDlaXHXg==
  -----END EC PRIVATE KEY-----
  PEM
}.freeze
PEER = <<~PEM
  -----BEGIN PRIVATE KEY-----
  MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgl7OI4c1ncHNMDfS5
  /+vApMA57W374EKDR3oOQevEqaOhRANCAASZ1ihg0D8OtcnuqRD5GoRJqwz2tYdh
  ejps3ndMrpl7L2tEgXT8QraeR2+QB6LrnMqQxkHxDfzZ4A3/1CftB3L6
  -----END PRIVATE KEY-----
PEM
ED = <<~PEM
  -----BEGIN PRIVATE KEY-----
  MC4CAQAwBQYDK2VwBCIEIP6uPjtNof6ombfV2260bWUHDoRrraTn7R54BpRppsOF
  -----END PRIVATE KEY-----
PEM

KEYS.each do |curve, (pem, sig)|
  k = OpenSSL::PKey::EC.new(pem)
  pub = OpenSSL::PKey.read(k.public_to_pem)
  g = k.group
  puts "-- #{curve}"
  t("key") { [k.class, k.private?, k.public?, pub.private?, g.curve_name, g.degree, g.cofactor.to_i, g.order.num_bits] }
  t("formats") { [k.to_pem == pem, h.(k.to_der), h.(k.private_to_der), h.(k.public_to_der), pub.to_der == k.public_to_der] }
  t("reread") { [OpenSSL::PKey.read(k.private_to_pem).to_der == k.to_der, OpenSSL::PKey::EC.new(k.to_der).to_pem == pem, Marshal.load(Marshal.dump(k)).to_der == k.to_der] }
  t("OpenSSL's signature") { [pub.verify("SHA256", [sig].pack("H*"), "fixed message"), pub.verify("SHA256", [sig].pack("H*"), "other message")] }
  t("sign") do
    %w[SHA1 SHA256 SHA384 SHA512].map { |d| s = k.sign(d, "data"); [pub.verify(d, s, "data"), pub.verify(d, s, "date")] }
  end
  t("sign Digest") { pub.verify(OpenSSL::Digest.new("SHA384"), k.sign(OpenSSL::Digest::SHA384.new, "x"), "x") }
  t("dsa_sign_asn1") { [10, 20, 32, 48, 64].map { |n| d = "\x5a".b * n; pub.dsa_verify_asn1(d, k.dsa_sign_asn1(d)) } }
  t("private key") { [k.private_key.class, h.(k.private_key.to_s(2))] }
  t("public key") { [h.(k.public_key.to_octet_string(:uncompressed)), h.(k.public_key.to_octet_string(:compressed)), k.public_key.to_octet_string(:hybrid).getbyte(0)] }
  t("generator") { h.(g.generator.to_octet_string(:compressed)) }
  t("k * G") { g.generator.mul(k.private_key) == k.public_key }
  t("mul") { [h.(k.public_key.mul(123456789).to_octet_string(:uncompressed)), h.(k.public_key.mul(7, 11).to_octet_string(:compressed))] }
  t("add") { k.public_key.add(k.public_key) == k.public_key.mul(2) }
  t("invert") { p = k.public_key.dup.invert!; [p.add(k.public_key).infinity?, h.(p.to_octet_string(:compressed))] }
  t("order") { [g.generator.mul(g.order).infinity?, g.generator.mul(g.order + 5) == g.generator.mul(5), g.generator.mul(-1) == g.generator.dup.invert!] }
  t("Point.new") do
    pt = k.public_key
    %i[uncompressed compressed hybrid].map { |f| OpenSSL::PKey::EC::Point.new(g, pt.to_octet_string(f)) == pt } +
      [OpenSSL::PKey::EC::Point.new(g, pt.to_bn) == pt, OpenSSL::PKey::EC::Point.new(g).infinity?]
  end
  t("check_key") { [k.check_key, pub.check_key] }
  t("compare?") { [k.compare?(pub), pub.compare?(k), k.compare?(OpenSSL::PKey::EC.generate(curve))] }
  t("generate") { e = OpenSSL::PKey::EC.generate(curve); [e.private?, e.group == g, e.verify("SHA256", e.sign("SHA256", "y"), "y")] }
  t("ECDH") { e = OpenSSL::PKey::EC.generate(curve); [k.derive(e) == e.derive(k), k.dh_compute_key(e.public_key) == e.derive(k)] }
  t("group der") { [g.to_der.unpack1("H*"), OpenSSL::PKey::EC::Group.new(g.to_pem) == g, OpenSSL::PKey::EC.new(g.to_pem).public?] }
end

puts "-- P-256"
k = OpenSSL::PKey::EC.new(KEYS["prime256v1"][0])
pub = OpenSSL::PKey::EC.new(k.public_to_der)
peer = OpenSSL::PKey.read(PEER)
t("to_text") { k.to_text }
t("public to_text") { pub.to_text }
t("group to_text") { k.group.to_text }
t("ECDH secret") { [k.derive(peer).unpack1("H*"), peer.derive(k) == k.derive(peer)] }
t("inspect") { [k.inspect.sub(/0x\h+/, "X"), OpenSSL::PKey::EC.generate("prime256v1").inspect.sub(/0x\h+/, "X")] }
t("PKCS#8 read") { [peer.class, peer.private?, peer.private_to_pem == PEER] }
t("EC.new(name)") { e = OpenSSL::PKey::EC.new("prime256v1"); [e.private?, e.public?, e.public_key, e.group.curve_name] }
t("EC.new(group)") { OpenSSL::PKey::EC.new(OpenSSL::PKey::EC::Group.new("secp384r1")).group.curve_name }
t("PKey.generate_key EC") { OpenSSL::PKey.generate_key("EC", ec_paramgen_curve: "P-384").group.curve_name }
t("PKey.generate_key params") { OpenSSL::PKey.generate_key(OpenSSL::PKey::EC.new("secp256k1")).group.curve_name }
t("generate_parameters") { OpenSSL::PKey.generate_parameters("EC", ec_paramgen_curve: "prime256v1").public? }
t("builtin_curves") { OpenSSL::PKey::EC.builtin_curves.select { |n, _| %w[prime256v1 secp384r1 secp521r1 secp256k1].include?(n) } }

puts "-- errors"
t("public sign") { pub.sign("SHA256", "x") }
t("public dsa_sign_asn1") { pub.dsa_sign_asn1("x" * 32) }
t("public private_to_der") { pub.private_to_der }
t("public derive") { pub.derive(peer) }
t("verify garbage") { pub.verify("SHA256", "junk", "x") }
t("verify r=1 s=1") { pub.verify("SHA256", "\x30\x06\x02\x01\x01\x02\x01\x01".b, "x") }
t("dsa_verify garbage") { pub.dsa_verify_asn1("x" * 32, "junk") }
t("generate_key!") { OpenSSL::PKey::EC.new("prime256v1").generate_key! }
t("private_key=") { k.dup.private_key = 5 }
t("bad curve") { OpenSSL::PKey::EC.generate("nope") }
t("NIST name") { OpenSSL::PKey::EC.new("P-256") }
t("Group NIST name") { OpenSSL::PKey::EC::Group.new("P-521") }
t("bad point") { OpenSSL::PKey::EC::Point.new(k.group, "\x04".b + "\x01".b * 64) }
t("bad encoding") { OpenSSL::PKey::EC::Point.new(k.group, "\x09\x01".b) }
t("garbage key") { OpenSSL::PKey::EC.new("garbage") }
t("read garbage") { OpenSSL::PKey.read("garbage") }
t("RSA as EC") { OpenSSL::PKey::EC.new(OpenSSL::PKey::RSA.new(1024).public_to_pem) }
t("EC as RSA") { OpenSSL::PKey::RSA.new(KEYS["prime256v1"][0]) }
t("other curve, other digest") { k.derive(OpenSSL::PKey::EC.generate("secp384r1")) }

puts "-- Ed25519"
e = OpenSSL::PKey.read(ED)
epub = OpenSSL::PKey.read(e.public_to_pem)
t("key") { [e.class, e.oid, e.inspect.sub(/0x\h+/, "X"), e.private_to_pem == ED] }
t("raw") { [e.raw_private_key.bytesize, e.raw_public_key.unpack1("H*"), epub.raw_public_key == e.raw_public_key] }
t("public_to_pem") { e.public_to_pem }
t("sign") { s = e.sign(nil, "ed message"); [s.unpack1("H*"), epub.verify(nil, s, "ed message"), epub.verify(nil, s, "ed massage"), epub.verify(nil, "short", "x")] }
t("to_text") { e.to_text }
t("public to_text") { epub.to_text }
t("new_raw_private_key") { OpenSSL::PKey.new_raw_private_key("ED25519", e.raw_private_key).public_to_der == e.public_to_der }
t("new_raw_public_key") { OpenSSL::PKey.new_raw_public_key("ED25519", e.raw_public_key).compare?(e) }
t("generate_key") { g = OpenSSL::PKey.generate_key("ED25519"); [g.oid, g.verify(nil, g.sign(nil, "z"), "z")] }
t("sign with a digest") { e.sign("SHA256", "x") }
t("public sign") { epub.sign(nil, "x") }
t("public raw_private_key") { epub.raw_private_key.class }
t("bad raw key") { OpenSSL::PKey.new_raw_private_key("ED25519", "x") }
t("PKey.new") { OpenSSL::PKey::PKey.new }
t("Marshal") { Marshal.dump(e) }

puts "-- X509"
def cert(key, digest)
  c = OpenSSL::X509::Certificate.new
  c.version = 2
  c.serial = 1
  c.subject = c.issuer = OpenSSL::X509::Name.parse("/CN=kenny")
  c.public_key = key
  c.not_before = Time.at(1_700_000_000)
  c.not_after = Time.at(1_800_000_000)
  c.sign(key, digest)
end
[[k, "SHA256"], [OpenSSL::PKey::EC.new(KEYS["secp384r1"][0]), "SHA384"], [OpenSSL::PKey::EC.new(KEYS["secp521r1"][0]), "SHA512"],
 [OpenSSL::PKey::EC.new(KEYS["secp256k1"][0]), "SHA256"], [e, nil]].each do |key, d|
  t("certificate #{key.oid} #{d}") do
    c = OpenSSL::X509::Certificate.new(cert(key, d).to_der)
    other = key.is_a?(OpenSSL::PKey::EC) ? peer : OpenSSL::PKey.generate_key("ED25519")
    [c.signature_algorithm, c.verify(c.public_key), c.verify(other), c.public_key.class]
  end
end
t("CSR") do
  r = OpenSSL::X509::Request.new
  r.subject = OpenSSL::X509::Name.parse("/CN=csr")
  r.public_key = k
  r.sign(k, "SHA256")
  r2 = OpenSSL::X509::Request.new(r.to_pem)
  [r2.signature_algorithm, r2.verify(pub), r2.verify(peer)]
end
