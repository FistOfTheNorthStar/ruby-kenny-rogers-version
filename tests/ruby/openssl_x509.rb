# OpenSSL::X509: certificates (building, signing, DER/PEM, to_text), names (formats, escaping,
# comparison), extensions (OpenSSL's printed values, ExtensionFactory's config syntax), CSRs, CRLs
# and Store verification (chains, OpenSSL's errors, the verify callback), RSA, ECDSA and Ed25519
require "openssl"
require "tmpdir"

X = OpenSSL::X509
A = OpenSSL::ASN1

def t
  r = yield
  p r
rescue => e
  puts "#{e.class}: #{e.message}"
end

def hex(s) = OpenSSL::Digest::SHA256.hexdigest(s)[0, 16]

K1 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEpAIBAAKCAQEAt8h+DqCgQoljINwpq7sYQSQxEukaWvzz/BAGeG8JsZw4dg/q
  N73hbFohzEdwgwaSMzN99MQPjKmAtcgPUOd73Z9CKVATEnh6pD++u3xjGpInMCvQ
  F5sFqjA7pGCyjfjcFkNRT96OM6o0hGLG1vTLDUVO6MwUsLF6tuL9QE9SLuh4kZ/S
  jlHOaBhfw03NQdmnL3/Vq7G71ijeeI2NetcDcZ3BW+F26FsB/opmCwZAGr+lNQBJ
  pfUpHJMIS5EyeWsOHRh4NY+pNtvrr8kwApBqz64xH+V15lSIQ9+XdpMKLe7Ca4ul
  jr6+nT5bOjif9FZrZrmS000azCQZQGFaVtjIGQIDAQABAoIBAFjvloT4mSUpvAVC
  VY4Lx6R6YZ8fm8ftQhq4HJF6VmwTR8VmVTctVaGsClADQEC/1M8fO3RPzqCIxEbz
  rem+Y5QvhgJdBCW7lMnYKTIscv8APCOVCCnsNHUTy0D0JGQMymChqf8nGZSJ6Mkp
  zcRnCL3+zL1eSSLqeBJ5zQkqeYBF3lBkhLt1+NvSvW+N0b8W4Lr/gvoQqjPa61Z2
  5powzakP9TiCbkK3WhjNNMPw9C1Eks3xo10210wy+pZGugLML6NjRw3LhmL84t42
  Xy5mCI4XxT8vOy9YoZWK2aXyl4bhz/3i4h6FE3lKtN9pDFj1MjbgfWKHwcHnuXon
  fWCQ1fsCgYEA+cPafOaQ7uiLbfNv8c4A0at+xaU/R9/6P+/tUSo5wyyws2YoTzup
  MrZqfw7loZOVqxOIOna/I2PZLXQnJk/YAGdsv/tbtGK7Q6zUmTMHA/uhy4WE9o4B
  wiBKOSFXO+oQLR39l5ivvVQHMacUng98Bjnt4zPVizKbwbLPazIr/eMCgYEAvF75
  xgB1jfPMhpjZiEFGWkPXfe3awu8fRM5br/+PUvDfAcaBOMcrfSGu59XU/dcwHHkW
  UBGIWD/jwzA6Zm1zT5SsTSGbXImtWUOhangK2l255euT3FmlBCvKCW/n2U/XcMRz
  45iRIlJhOnfzYSXTDWUuWzOZeivXkudLei/2QtMCgYEAsSoa1hPaZIzxMgnGszeT
  3yYNo3bGv14Brc6rgLI6DNardGWXXDSOnRlFy2CuxADi3lnZfNb0V88goxxMFOH3
  7rluK3PJpdeOy79RgmXuqQWZy7qnCRjL6H5tB789mJM77DTkx96Xce2juLWyazfC
  mswGkO4YoEdCso/6jygkik8CgYAVrMQG0YS1SclN74aQVBp5RnckSVsztPofG7OX
  I8vlGkw+0w2Jb/mEE30htgwlDT6+9/vmSK4soWVez0zY/l0XWh8dXPGYi0sEC3Em
  ZBuzlrdOSnYgE7yWJ0e/ID5IVhSPKDvBg4unrFz9slGx6v1G/E6CNgTB9wt+Htkj
  fSNT+wKBgQDWqzH3/tOxDef0K3Leb20MNk8L2LaWMXxSUNn4wwUjrNKO5M54zcwA
  pQ75sLjrv+oci7zkgerT0lyitJZqrs+wR6QoCic6uYJVdVtmOC+s3jAw/wbRPC4u
  BJy8ZPqne1tW5oJupcIVIeYcBTBZFcBIgm+7382Nva4PLO7i9cIS4w==
  -----END RSA PRIVATE KEY-----
PEM
K2 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEogIBAAKCAQEApr+VKDLsWOwkwLhReCgDW2DHmrA49lk5QHxxidJwMyDP6O7N
  wUpTo0Ek5SrDbGI7+83glisScEm8NMtgU07F9OL4LAwIwe3pAl+hyZoWI1yY7EaF
  OMmf1whdAldmVJAHoa0QXGjokkJ2ahNEjiUd392xzG++ZTwN+Rfvfd5hqqfvylxw
  M/3tKWbbGoQ7G5njGoTNSoHI3qmevkytoP9ppCBiF2Eet/ZShXqRimquMUwC1h+/
  8epUyC7dXj7KFyNgJQYZAJAgyxOcj/3M96t6dWk0Pq9Q+T461nBCFAPpcwuFET8m
  X4YzG1vPpmBRnBDSUkS2bq7l1fxF87oBmT21cwIDAQABAoIBAAn8/IdIPZtkpx2a
  14PLXyAbH6Rmx+gMbNSDCnNcqqHgEPQn/HqGjun+3fc9j4UvgTOnymXUYKoJ5f8s
  64aHTRj1JjPP6LpicOGr2XoaWRch1dCEDOOGQvp+IsF1e3Nh8+29pcw98H3KWLjT
  502zGs4CbphgsoFH4/e2ylQPfRQxcEyqIunLuNHzej95Gvhm4HGzOIF7DOI8sbJE
  IuhfqlLrwam8UdvDimfT5RvLL3w3MqRJkNzQguDdJZ4WDh5CEsaCUbcWt5DWRWZg
  NwfU3er5eLPmQRje2yOdJbfMDlpmP3JCBvzaJE6aMP8L76qdUIs2HHnJ1CNIZOvM
  qfbWFikCgYEA0dVyXz/09b6RIbyMeqQJZXx40UCdOfp4rmK9ev2vife9xJ4ocLL5
  2gOKqhMPjrMP/wJpJFUi1lVshxfj77xXeq6gLvrLL1C7M8EUE/giwYeHtIpTF+UV
  FB6TiwleF4eSA3265eZxnftIJmZaYYBg85T3+HmZoyZh1sQhb/TeqKcCgYEAy29r
  Qbbj3T7GlPto3Y/a3S4qRdsvxGPLeT1rsIHMmamvPpR1l6V34kz3KyeauthVm+my
  qeQIDU2QosvuHBbrkLxt8TwE5SkD24+HypozmAHw0qxdHUpVRv06mTMuRuFT3gN/
  ebGAf20rDg1WFIwinGJ7brAiX3V2/T+I49dzWlUCgYA1AvTTJYhQcWWfHVBsR3bs
  QccQLn/KPwIwJFf0c5TeiU1TGBXlawH+Vzi4APrENW7MISLJAA0+EvTJKHr4fR+o
  qedkFw4wFPhGefgFl4HIXOIm9W9KsSBjJdpmnOY9kY2M/j2yiUaY//vZX7Bjw5g6
  0Z7YEniDJFZBeOltrHJ4LwKBgGHFTa4KgN7PEc9+OLrrLUvJqSVjn0RqldrR6xSs
  fTGIvKF95joYm1JWnR/Pa+zKlfJV0bkWz8vc8FtdWTWEyngIJ2qd5thf6uu1wpRb
  odo3bI9DqYivNeblQyln0KDeIFl+IuDQryoOxHd/b1R2uRehuSLMYeiw4pv50gd2
  f4j1AoGAAnxc3GvbBwXqv3sdXQtGO8nbtqAzCQTb2n7SakTwwrBMWUodjIV51Cjq
  rUq738+Yl9DDG5+wgnmpqa6w8AUwX596TmiVd5N0JlCirDk6n+vlXStvphv2K6tR
  dLZmNaIVOmp2+PjXa/fr1c2t5qB9uL0yKr9He48Bj6ECeKutbiU=
  -----END RSA PRIVATE KEY-----
PEM
K3 = OpenSSL::PKey::RSA.new(<<~PEM)
  -----BEGIN RSA PRIVATE KEY-----
  MIIEpAIBAAKCAQEApxPP+Nje1Je677fv8Ow458qG5ZkzcJfKXxF5i8LBbgN7Brkn
  EQbi+8/i+ZDIg0OSly9ASfjDl2ZhMW14FZE0dLlahsUwAX5BGr8eWvM9Cmfjfxf2
  SESVT/67T8MiWsHscZ5mHHedBGqWSGEXlY6ms6s3E4/wOzJuCUJCOdmHDZtjyRDP
  jGg8ioEiJ8W8vDXqRcNj4zLXG0HNXXr9fsokO+8Lmwav2bNQH9fFDxcd+8iDWxMY
  9dVxHeKwVqVTmAelCKO3mYMig78Ep/+VW0J2xvteKfageZFvskRhVTK9Gte2dPM8
  e5T7APwNemdKAiSyo9+sYockY6cGpvZqQf5apwIDAQABAoIBAE077+ZZnnyJHX3G
  OjFViaLA8SX+8ZQJXyAebGEKqS8phbwgaCpLVC9StNpoLwDbAk9IJQBs8kGxLpnS
  QHI72xNgQbROQDhYfFEYazhg++UtDDHKymsy5REDMOSY0G9C+QG21v4u3xNVEF6q
  H2Rf+rjQm5e78LH0yIAYNJQYdgF4BeiSvA5VEsmWok2BvEKDtVqi3abV8J1Lr4sh
  7pxUaOrGj4wi5it+9EGHlwjRzLjRjG5Vwt0eLMd1e7LRkNhZ+PXPKX5+3EcSgQxl
  qNP9kvdq8Wi6BmPUtGYGK+JK+3JjekPzgIizZvfi/YQwvkIxF16PPzPONmQsjj56
  q9RlFH0CgYEA0eOPm09Zq8ErL11OxaZ7F4tO3m07UWeLFYT2xkd2aB6ZyNJyWkkH
  GqwIxGzQt/XmFshTrVzRB9IZ73GjXeNXRX50AfjLos9vQbuHiqCEHCntkAeg09a4
  5tAfaVMl8ZVIHWWTVHINtoWxQNco9VOoLfMtDdm5oCZVGutS40FGzGsCgYEAy8h5
  IhQj8pHU+KIH2Sxzzi752DX/tOTRp3mo+uJpb9NU3GnB0HbaNG0DBU/Q67CcutSh
  rJ0i5m/Im8d31UQ+uaUh+KX/1FL/ckkFZte9rJEdXQHJMnlFQpxEBg+SPtMNnxKp
  52+AIUWWgkRN7hs7ZJQdQd/W7JC/j8MENOFEObUCgYEAl7HGatOQ1afJFHR1qDhI
  op2WpP+BbbZOOYFd8bRYiawELbab21EIh2hq9bxHpY1dfH3QrG12zhY1lG8nSYYz
  3xAjvAEa/FK78GAe+DxG/yv2fLBmtnRsMrWc3jywIcho1IZDUzcTXiWGFHztJ1IN
  LRiiKW6A0JSj0c7em0zkZH0CgYEAjz7Yh3AI5NZGEpAzXCG3NdQnpCgxGwNNrKAE
  fHbzDrxLBrUAnH0vXXLTkMROY56+TUZyb1f11G33YimQvsU+IllFy/dEIghFsBxM
  Q8KbfdZ0AZDj0SsV2IDoACqp7qeFB7Ci5iCmvLyh0+gXr6xMz6Kd/lROS/Rj87Zu
  Go3O25ECgYBaoz7FRSIuOGgOud4tXaZp3ul5TGE1/1ei4RYe/JV3/vlzuyyw2cTT
  ux1dZoD3cREvv7QBV+4/eieOg8F9yTEyr8N/cXIrJuhXAs+cqiZ1+Id0Ri04McKJ
  6bLftQ6gT5+B+yU7veh+gWIiNRVw3O3SmJYcROQJdPaD7IGsKoxdKQ==
  -----END RSA PRIVATE KEY-----
PEM

def cert(subject, issuer, key, issuer_key, serial, exts, digest: "SHA256", nb: Time.utc(2020, 1, 1), na: Time.utc(2090, 1, 1))
  c = X::Certificate.new
  c.version = 2
  c.serial = serial
  c.subject = subject.is_a?(X::Name) ? subject : X::Name.parse(subject)
  c.issuer = issuer ? issuer.subject : c.subject
  c.public_key = key
  c.not_before = nb
  c.not_after = na
  ef = X::ExtensionFactory.new(issuer || c, c)
  exts.each { |e| c.add_extension(ef.create_extension(*e)) }
  c.sign(issuer_key, digest)
  c
end

CA_EXTS = [["basicConstraints", "CA:TRUE", true], ["keyUsage", "keyCertSign,cRLSign", true], ["subjectKeyIdentifier", "hash"]]
root = cert("/C=FI/O=Kenny Test/CN=Kenny Root CA", nil, K1, K1, 1, CA_EXTS + [["authorityKeyIdentifier", "keyid:always"]])
int = cert("/C=FI/O=Kenny Test/CN=Kenny Intermediate CA", root, K2, K1, 2,
           [["basicConstraints", "CA:TRUE, pathlen:0", true], ["keyUsage", "keyCertSign, cRLSign", true],
            ["subjectKeyIdentifier", "hash"], ["authorityKeyIdentifier", "keyid,issuer"],
            ["crlDistributionPoints", "URI:http://crl.example.com/root.crl"]])
leaf = cert("/CN=www.example.com/O=Kenny Test", int, K3, K2, 0x1234_5678_9abc_def0_1234,
            [["basicConstraints", "CA:FALSE"], ["keyUsage", "digitalSignature,keyEncipherment", true],
             ["extendedKeyUsage", "serverAuth,clientAuth"], ["subjectKeyIdentifier", "hash"],
             ["authorityKeyIdentifier", "keyid"],
             ["subjectAltName", "DNS:www.example.com,DNS:*.example.com,IP:192.0.2.1,IP:2001:db8::1,email:admin@example.com"],
             ["authorityInfoAccess", "OCSP;URI:http://ocsp.example.com,caIssuers;URI:http://ca.example.com/int.der"],
             ["nsComment", "Kenny test leaf"]],
            nb: Time.utc(2021, 6, 1, 12), na: Time.utc(2089, 6, 1, 12))

puts "== certificates"
[root, int, leaf].each { |c| p [c.serial.to_i, c.subject.to_s, c.issuer.to_s, hex(c.to_der), c.signature_algorithm] }
puts leaf.to_text
puts root.to_pem
p leaf.version, leaf.not_before, leaf.not_after, leaf.serial.class, leaf.subject.class
p leaf.extensions.map(&:to_a)
p leaf.extensions.map(&:to_s)
p leaf.extensions.map(&:to_h).last
p leaf.verify(K2), leaf.verify(K1), root.verify(K1), leaf.check_private_key(K3), leaf.check_private_key(K2)
p leaf.subject_key_identifier.unpack1("H*"), leaf.authority_key_identifier == int.subject_key_identifier
p leaf.ocsp_uris, leaf.ca_issuer_uris, int.crl_uris, leaf.crl_uris
p leaf.public_key.class, leaf.public_key.public_to_der == K3.public_to_der
p leaf.inspect.gsub(/0x\h+/, "0x?")
p X::Certificate.new(leaf.to_pem) == leaf, X::Certificate.new(leaf.to_der) == leaf, leaf == root, leaf == "x"
p X::Certificate.load(root.to_pem + "\n" + int.to_pem + leaf.to_pem).map { |c| c.serial.to_i }
p X::Certificate.load(root.to_der).size, Marshal.load(Marshal.dump(leaf)) == leaf
t { X::Certificate.new("garbage") }
t { X::Certificate.load("garbage") }
t { X::Certificate.new.to_der }
t { leaf.dup.tap { |c| c.serial = 5 }.verify(K2) }
t { leaf.tbs_bytes.bytesize }
t { root.sign(K1.public_key, "SHA256") }
t { root.dup.sign(K1, "NOPE") }
t { root.dup.subject = "x" }
t { root.dup.serial = "x" }
t { root.dup.version = -1 }
t { X::Certificate.new.verify(K1) }
c2 = root.dup
c2.not_after = Time.utc(2049, 12, 31, 23, 59, 59)
c2.not_before = Time.utc(1950, 1, 1)
c2.sign(K1, "SHA384")
p c2.to_der.unpack1("H*")[0, 80], c2.not_before, c2.not_after, c2.signature_algorithm

puts "== names"
n = X::Name.new
n.add_entry("CN", "a")
n.add_entry("O", "b")
n.add_entry("OU", "c", set: -1)
n.add_entry("DC", "d", loc: 0)
n.add_entry("L", "e", loc: 1, set: 1)
p n.to_a, n.to_s, n.to_utf8, n.to_der.unpack1("H*"), n.hash, n.hash_old
[X::Name::COMPAT, X::Name::RFC2253, X::Name::ONELINE, X::Name::MULTILINE].each { |f| p n.to_s(f) }
s = X::Name.new([["CN", " #lead,ing+\"q\"\\ <x>;=/trail "], ["O", "é ü\x01\x7f"], ["1.2.3.4", "unk"], ["OU", "a+b/c"], ["emailAddress", "a@b"]])
[nil, X::Name::COMPAT, X::Name::RFC2253, X::Name::ONELINE, X::Name::MULTILINE].each { |f| p s.to_s(*[f].compact) }
p s.to_utf8, s.inspect, s.to_a
u = X::Name.new([["CN", "\x00a\x00b".b, A::BMPSTRING], ["CN", "\xe9t\xe9".b, A::T61STRING], ["CN", "x\x01y", A::PRINTABLESTRING]])
[nil, X::Name::RFC2253, X::Name::ONELINE, X::Name::MULTILINE].each { |f| p u.to_s(*[f].compact) }
p X::Name.new([["CN", "a\x80".b, A::UTF8STRING]]).to_s(X::Name::RFC2253)
t { X::Name.new([["CN", "a\x80".b, A::UTF8STRING]]).to_s(X::Name::MULTILINE) }
a = X::Name.parse("/CN=Foo  Bar/O=x")
b = X::Name.parse("/CN= foo bar /O=X")
p a.cmp(b), a == b, a.eql?(b), a.hash == b.hash, a <=> X::Name.parse("/CN=a"), a.cmp("x")
p X::Name.parse("/CN=a").cmp(X::Name.parse("/CN=b")), X::Name.parse("/CN=a/O=b") <=> X::Name.parse("/CN=a")
p X::Name.parse_rfc2253("CN=a\\,b,O=c").to_a, X::Name.parse("CN=x, O=y").to_a
p X::Name.new(n.to_der).to_a, X::Name.new(n.to_der) == n
p X::Name.new([["C", "FI"], ["CN", "x", 0x1000], ["DC", "dc", 0x1000], ["CN", "\xe9".b, 0x1001]]).to_a
t { X::Name.new([["XX", "a"]]) }
t { X::Name.new([["CN", "a", 1]]) }
t { X::Name.new([["CN", "a", 28]]) }
t { X::Name.new([["C", "FIN", 0x1000]]) }
t { X::Name.new("garbage") }
t { X::Name.new(["CN", "x"]) }
p X::Name::OBJECT_TYPE_TEMPLATE["C"], X::Name::OBJECT_TYPE_TEMPLATE["CN"], X::Name::DEFAULT_OBJECT_TYPE
p X::Name.new.to_s, X::Name.new.to_der, X::Name.new.inspect

puts "== extensions"
ef = X::ExtensionFactory.new(root, leaf)
[
  ["basicConstraints", "CA:TRUE, pathlen:3", true],
  ["basicConstraints", "critical,CA:FALSE"],
  ["keyUsage", "digitalSignature,nonRepudiation,keyEncipherment,dataEncipherment,keyAgreement,keyCertSign,cRLSign,encipherOnly,decipherOnly"],
  ["extendedKeyUsage", "serverAuth,clientAuth,codeSigning,emailProtection,timeStamping,OCSPSigning,1.2.3.4"],
  ["subjectKeyIdentifier", "hash"],
  ["subjectKeyIdentifier", "01:02:ab"],
  ["authorityKeyIdentifier", "keyid:always,issuer:always"],
  ["subjectAltName", "DNS:a.example,IP:10.0.0.1,IP:::1,email:x@y.example,URI:https://u.example/p,RID:1.2.3.4"],
  ["issuerAltName", "issuer:copy"],
  ["crlDistributionPoints", "URI:http://a/1.crl,URI:http://b/2.crl"],
  ["authorityInfoAccess", "OCSP;URI:http://ocsp,caIssuers;URI:http://ca"],
  ["nsCertType", "client,server,email,objsign,sslCA,emailCA,objCA"],
  ["nsComment", "a comment, with a comma"],
  ["1.2.3.4", "DER:04:02:68:69"],
  ["keyUsage", "bogus"],
  ["basicConstraints", "CA:maybe"],
  ["noSuchExt", "x"],
  ["subjectAltName", "IP:1.2.3"],
  ["2.5.29.17", "DNS:oid.example"],
  ["certificatePolicies", "2.23.140.1.2.1"],
].each do |oid, val, crit|
  t do
    e = ef.create_ext(oid, val, crit)
    [e.oid, e.value, e.critical?, e.to_der.unpack1("H*")]
  end
end
t { ef.create_extension("subjectAltName = DNS:foo.example").to_a }
t { ef.create_extension(["keyUsage", "digitalSignature", true]).to_a }
t { ef.create_extension("oid" => "nsComment", "value" => "x").to_a }
t { X::ExtensionFactory.new.create_ext("subjectKeyIdentifier", "hash") }
ctx = ->(v, n) { A::ASN1Data.new(v, n, :CONTEXT_SPECIFIC) }
[
  ["certificatePolicies", A::Sequence([A::Sequence([A::ObjectId("2.23.140.1.2.1")]),
                                       A::Sequence([A::ObjectId("2.5.29.32.0"), A::Sequence([A::Sequence([A::ObjectId("1.3.6.1.5.5.7.2.1"), A::IA5String("http://cps.example")])])])])],
  ["nameConstraints", A::Sequence([A::ASN1Data.new([A::Sequence([ctx.(".example.com", 2)]), A::Sequence([ctx.("\x0a\x00\x00\x00\xff\x00\x00\x00".b, 7)])], 0, :CONTEXT_SPECIFIC)])],
  ["subjectAltName", A::Sequence([A::ASN1Data.new([A::ObjectId("1.3.6.1.4.1.311.20.2.3"), A::ASN1Data.new([A::UTF8String("upn@x")], 0, :CONTEXT_SPECIFIC)], 0, :CONTEXT_SPECIFIC),
                                  ctx.("\x01\x02\x03".b, 7), A::ASN1Data.new([A.decode(X::Name.parse("/CN=dir").to_der)], 4, :CONTEXT_SPECIFIC)])],
  ["policyConstraints", A::Sequence([ctx.("\x00".b, 0), ctx.("\x02".b, 1)])],
  ["inhibitAnyPolicy", A::Integer(3)],
  ["keyUsage", A::BitString("".b)],
  ["subjectAltName", A::Sequence([])],
  ["1.2.3.4", A::OctetString("abc")],
  ["1.3.6.1.4.1.11129.2.4.3", A::Null(nil)],
  ["tlsfeature", A::Sequence([A::Integer(5)])],
].each do |oid, v|
  t do
    e = X::Extension.new(oid, v)
    [e.oid, e.value, e.value_der.unpack1("H*")]
  end
end
e = X::Extension.new("basicConstraints", "\x30\x00".b, true)
p e.to_der.unpack1("H*"), X::Extension.new(e.to_der).to_a, e == X::Extension.new(e.to_der)
e.oid = "keyUsage"
e.critical = false
e.value = "\x03\x02\x05\xa0".b
p e.to_a, e.oid.frozen?, e.value.encoding
t { X::Extension.new("nonsense", "x") }
t { X::Extension.new("\x01".b) }

puts "== verification"
def show(store, cert, chain = nil, &blk)
  r = store.verify(cert, chain, &blk)
  p [r, store.error, store.error_string, store.chain&.map { |c| c.subject.to_s }]
rescue => e
  puts "#{e.class}: #{e.message}"
end
st = X::Store.new
p [st.error, st.error_string, st.chain]
st.add_cert(root)
show(st, leaf, [int])
show(st, leaf)
show(st, int)
show(st, root)
show(X::Store.new, root)
show(X::Store.new, leaf, [int, root])
show(X::Store.new.add_cert(int), leaf)
show(X::Store.new.add_cert(int).tap { |s| s.flags = X::V_FLAG_PARTIAL_CHAIN }, leaf)
show(X::Store.new.add_cert(leaf).tap { |s| s.flags = X::V_FLAG_PARTIAL_CHAIN }, leaf)
expired = X::Store.new.add_cert(root)
expired.time = Time.utc(2095)
show(expired, leaf, [int])
early = X::Store.new.add_cert(root)
early.time = Time.utc(2021, 1, 1)
show(early, leaf, [int])
early.time = Time.utc(2089, 6, 1, 12)
show(early, leaf, [int])
# an intermediate that isn't a CA, one without keyCertSign, a path too long, a bad signature
not_ca = cert("/CN=Not A CA", root, K2, K1, 10, [["basicConstraints", "CA:FALSE"]])
under = cert("/CN=under", not_ca, K3, K2, 11, [])
show(st, under, [not_ca])
no_sign = cert("/CN=No Cert Sign", root, K2, K1, 12, [["basicConstraints", "CA:TRUE"], ["keyUsage", "digitalSignature"]])
show(st, cert("/CN=under2", no_sign, K3, K2, 13, []), [no_sign])
deeper = cert("/CN=Deeper CA", int, K3, K2, 14, [["basicConstraints", "CA:TRUE"]])
show(st, cert("/CN=too deep", deeper, K1, K3, 15, []), [deeper, int])
forged = cert("/CN=forged", int, K3, K1, 16, [])
show(st, forged, [int])
v1 = X::Certificate.new
v1.subject = v1.issuer = X::Name.parse("/CN=v1 root")
v1.serial = 7
v1.public_key = K1
v1.not_before = Time.utc(2020)
v1.not_after = Time.utc(2090)
v1.sign(K1, "SHA256")
show(X::Store.new.add_cert(v1), cert("/CN=under v1", v1, K2, K1, 8, []))
crit = cert("/CN=critical", root, K3, K1, 17, [["1.2.3.4", "critical,DER:05:00"]])
show(st, crit)
st_ic = X::Store.new.add_cert(root)
st_ic.flags = X::V_FLAG_IGNORE_CRITICAL
show(st_ic, crit)

puts "== purposes"
client_only = cert("/CN=client", int, K3, K2, 20, [["extendedKeyUsage", "clientAuth"]])
[X::PURPOSE_SSL_SERVER, X::PURPOSE_SSL_CLIENT, X::PURPOSE_ANY, X::PURPOSE_SMIME_SIGN].each do |pur|
  s2 = X::Store.new.add_cert(root)
  s2.purpose = pur
  show(s2, leaf, [int])
  show(s2, client_only, [int])
end

puts "== verify callback"
show(st, leaf, [int]) { |ok, c| p [:cb, ok, c.error, c.error_depth, c.current_cert.subject.to_s, c.chain.size]; ok }
show(st, leaf) { |ok, c| p [:cb, ok, c.error, c.error_string, c.error_depth]; true }
show(st, leaf, [int]) { |ok, c| c.error_depth == 0 ? false : ok }
show(st, leaf, [int]) { |ok, c| c.error = X::V_ERR_APPLICATION_VERIFICATION if c.error_depth == 0; ok }
show(st, leaf, [int]) { |ok, c| c.error = X::V_ERR_CERT_REVOKED; true }
show(st, leaf, [int]) { |ok, c| 1 }
show(X::Store.new, leaf, [int, root]) { |ok, c| p [:cb, ok, c.error, c.error_depth]; c.error == X::V_ERR_SELF_SIGNED_CERT_IN_CHAIN || ok }
saved = nil
show(st, root) { |ok, c| saved = c; ok }
t { saved.error }
st.verify_callback = ->(ok, c) { p [:store_cb, ok, c.error_depth]; ok }
show(st, int)
p st.verify_callback.class
st.verify_callback = nil

puts "== store context"
sc = X::StoreContext.new(st, leaf, [int])
p [sc.error, sc.error_string, sc.error_depth, sc.chain]
p sc.verify, sc.error, sc.chain.map { |c| c.serial.to_i }, sc.current_cert.serial.to_i
t { sc.verify }
t { X::StoreContext.new(st).verify }
t { X::StoreContext.new(st, leaf, ["x"]) }
t { X::StoreContext.new("x") }
sc = X::StoreContext.new(st, leaf, [int])
sc.time = Time.utc(2095)
p sc.verify, sc.error, sc.error_string, sc.error_depth
sc = X::StoreContext.new(st, leaf, [int])
sc.flags = X::V_FLAG_NO_CHECK_TIME
sc.purpose = X::PURPOSE_SSL_SERVER
sc.error = 50
p sc.error_string, sc.verify, sc.error
sc.error = 1000
p sc.error_string

puts "== files and directories"
Dir.mktmpdir do |dir|
  bundle = File.join(dir, "bundle.pem")
  File.write(bundle, "junk\n" + int.to_pem + root.to_pem)
  s3 = X::Store.new
  s3.add_file(bundle)
  show(s3, leaf)
  t { X::Store.new.add_file(File.join(dir, "missing.pem")) }
  File.write(File.join(dir, "empty.pem"), "nothing here\n")
  t { X::Store.new.add_file(File.join(dir, "empty.pem")) }
  hashed = File.join(dir, "certs")
  Dir.mkdir(hashed)
  File.write(File.join(hashed, format("%08x.0", root.subject.hash)), root.to_pem)
  s4 = X::Store.new
  s4.add_path(hashed)
  show(s4, leaf, [int])
  show(X::Store.new.add_path(File.join(dir, "none")), leaf, [int])
  old = ENV["SSL_CERT_FILE"], ENV["SSL_CERT_DIR"]
  ENV["SSL_CERT_FILE"] = bundle
  ENV["SSL_CERT_DIR"] = hashed
  s5 = X::Store.new
  s5.set_default_paths
  show(s5, leaf)
  ENV["SSL_CERT_FILE"], ENV["SSL_CERT_DIR"] = old
end

puts "== CRLs"
def crl(issuer, key, serials, last: Time.utc(2022), nxt: Time.utc(2088))
  c = X::CRL.new
  c.version = 1
  c.issuer = issuer.subject
  c.last_update = last
  c.next_update = nxt
  serials.each do |s|
    r = X::Revoked.new
    r.serial = s
    r.time = Time.utc(2022, 2, 2)
    c.add_revoked(r)
  end
  c.sign(key, "SHA256")
  c
end
good = crl(int, K2, [99])
bad = crl(int, K2, [leaf.serial])
p good.to_der.unpack1("H*")[0, 60], hex(bad.to_der), bad.revoked.map { |r| [r.serial.to_i, r.time] }
p good.issuer.to_s, good.last_update, good.next_update, good.version, good.signature_algorithm
p good.verify(K2), good.verify(K1), X::CRL.new(good.to_pem) == good, X::CRL.new(bad.to_der).revoked.size
[[], [good], [bad]].each do |crls|
  s6 = X::Store.new.add_cert(root)
  crls.each { |c| s6.add_crl(c) }
  s6.flags = X::V_FLAG_CRL_CHECK
  show(s6, leaf, [int])
end

puts "== requests"
req = X::Request.new
req.version = 0
req.subject = X::Name.parse("/CN=req.example/O=Kenny")
req.public_key = K3
exts = [X::ExtensionFactory.new.create_ext("subjectAltName", "DNS:req.example")]
req.add_attribute(X::Attribute.new("extReq", A::Set([A::Sequence(exts)])))
req.sign(K3, "SHA256")
p hex(req.to_der), req.verify(K3), req.verify(K1), req.subject.to_s, req.signature_algorithm
r2 = X::Request.new(req.to_pem)
p r2.to_der == req.to_der, r2.attributes.map(&:oid), r2.public_key.public_to_der == K3.public_to_der
p r2.attributes[0].value.value[0].value[0].to_der == exts[0].to_der
attr = X::Attribute.new("challengePassword", A::Set([A::PrintableString("secret")]))
p attr.oid, attr.to_der.unpack1("H*"), X::Attribute.new(attr.to_der) == attr
t { X::Attribute.new("challengePassword", A::Sequence([])) }

puts "== subjectAltName the way ssl.rb reads it"
leaf.extensions.each do |ext|
  next unless ext.oid == "subjectAltName"
  ostr = A.decode(ext.to_der).value.last
  A.decode(ostr.value).value.each do |san|
    case san.tag
    when 1, 2, 6 then p [san.tag, san.value]
    when 7 then p [san.tag, san.value.bytesize == 4 ? san.value.unpack("C4").join(".") : san.value.unpack("n8").map { |x| x.to_s(16) }.join(":")]
    end
  end
end

puts "== ECDSA and Ed25519"
ec_root, ec_int, ec_leaf, ed = X::Certificate.load(<<~PEM)
  -----BEGIN CERTIFICATE-----
  MIIBfTCCASOgAwIBAgIBATAKBggqhkjOPQQDAjAlMREwDwYDVQQKDAhLZW5ueSBF
  QzEQMA4GA1UEAwwHRUMgUm9vdDAgFw0yMDAxMDEwMDAwMDBaGA8yMDkwMDEwMTAw
  MDAwMFowJTERMA8GA1UECgwIS2VubnkgRUMxEDAOBgNVBAMMB0VDIFJvb3QwWTAT
  BgcqhkjOPQIBBggqhkjOPQMBBwNCAASimzRkOg6KfZqjDncUTBUzT4pMomHJg8Nl
  csmOjcgDkyi/1locGP1GbK3HGFd5ZPgxg44NZQ/WjyK5ubgxNp63o0IwQDAPBgNV
  HRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAdBgNVHQ4EFgQUmPwMNFwHu56e
  cg5ibDs3IgD05sEwCgYIKoZIzj0EAwIDSAAwRQIgXw76VqtbGJGmzp8xl7knumii
  aE9QPtz8F1gh+47Y3d8CIQCyLFuHjZeVek6U/v6QLL5xbPONFeO1eDtQBCMvbCFw
  4A==
  -----END CERTIFICATE-----
  -----BEGIN CERTIFICATE-----
  MIIBxzCCAWygAwIBAgIBAjAKBggqhkjOPQQDAjAlMREwDwYDVQQKDAhLZW5ueSBF
  QzEQMA4GA1UEAwwHRUMgUm9vdDAgFw0yMDAxMDEwMDAwMDBaGA8yMDkwMDEwMTAw
  MDAwMFowLTERMA8GA1UECgwIS2VubnkgRUMxGDAWBgNVBAMMD0VDIEludGVybWVk
  aWF0ZTB2MBAGByqGSM49AgEGBSuBBAAiA2IABAmlJr6/UCQQkhN/vAf1j0biXtzl
  0luA5OM3l4b7hBs9yOHme42pDyHfnQmsOUIiiyU8xrCgRzqGqu7JD05UCe7V0E5Y
  JO/gpXbXIsomfcJ1Dsf07POHU+J9rpHlyo8Mi6NmMGQwEgYDVR0TAQH/BAgwBgEB
  /wIBADAOBgNVHQ8BAf8EBAMCAQYwHQYDVR0OBBYEFCZfzjNcR0AIRCBPGdbF8S8S
  ypuOMB8GA1UdIwQYMBaAFJj8DDRcB7uennIOYmw7NyIA9ObBMAoGCCqGSM49BAMC
  A0kAMEYCIQCX7X8HJ8qQiPIqyCZ0TBvVwJ+gyBFWqDrIdf1LX0wW+wIhAOOvzrL4
  hNm84TEZeJWlYe3penKkDcgop9x/Q443tfvZ
  -----END CERTIFICATE-----
  -----BEGIN CERTIFICATE-----
  MIIBzDCCAVKgAwIBAgIBAzAKBggqhkjOPQQDAzAtMREwDwYDVQQKDAhLZW5ueSBF
  QzEYMBYGA1UEAwwPRUMgSW50ZXJtZWRpYXRlMCAXDTIwMDEwMTAwMDAwMFoYDzIw
  OTAwMTAxMDAwMDAwWjAZMRcwFQYDVQQDDA5lYy5leGFtcGxlLmNvbTBZMBMGByqG
  SM49AgEGCCqGSM49AwEHA0IABC8CIoJS8BZNEyGPICqt5oIWPJRANuCPaIrfCugp
  q68fh5V8kKC+3U9Y4oo0QZeU9A1bW69Ob6ZYqf9ZMwS7ZEKjdTBzMCsGA1UdEQQk
  MCKCDmVjLmV4YW1wbGUuY29tghAqLmVjLmV4YW1wbGUuY29tMBMGA1UdJQQMMAoG
  CCsGAQUFBwMBMA4GA1UdDwEB/wQEAwIHgDAfBgNVHSMEGDAWgBQmX84zXEdACEQg
  TxnWxfEvEsqbjjAKBggqhkjOPQQDAwNoADBlAjBvZ42A1ivMlwwMh4AFYzZ3BKFz
  Nh3MhJPgG4XaQcHp1ys+6BwntFit+9EhzBgCIAoCMQDNfC6OtdGE1V8G9Uta6GO2
  FL120oeYybiTdfrL+v57xfERs/F1hVdodEp9hb7EpHA=
  -----END CERTIFICATE-----
  -----BEGIN CERTIFICATE-----
  MIHdMIGQoAMCAQICAmOvMAUGAytlcDAXMRUwEwYDVQQDDAxFZDI1NTE5IFNlbGYw
  IBcNMjAwMTAxMDAwMDAwWhgPMjA5MDAxMDEwMDAwMDBaMBcxFTATBgNVBAMMDEVk
  MjU1MTkgU2VsZjAqMAUGAytlcAMhAFwpOUcdLnhK/DOanxB+Jj9cn+XSe+kJlRY/
  0N4SZjFnMAUGAytlcANBAKcJ4Ms+gbOBnlJ9LGcdKtc/c3Lw+n7xCdnF4QqgoWmE
  ENRV+GW7FzgNPEu65moAcRb90Sb5lkzutIYpglBMhw4=
  -----END CERTIFICATE-----
PEM
p [ec_root, ec_int, ec_leaf, ed].map(&:signature_algorithm)
ec_store = X::Store.new.add_cert(ec_root)
show(ec_store, ec_leaf, [ec_int])
show(ec_store, ec_leaf)
ec_store.purpose = X::PURPOSE_SSL_SERVER
show(ec_store, ec_leaf, [ec_int])
p ec_leaf.verify(ec_int.public_key), ec_leaf.verify(ec_root.public_key), ec_int.verify(ec_root.public_key)
k = ec_leaf.public_key
p k.class, k.oid, k.group.curve_name, k.public?, k.private?, k.public_to_der == ec_leaf.public_key.to_der
p ec_int.public_key.group.curve_name, k.public_to_pem == <<~PEM
  -----BEGIN PUBLIC KEY-----
  MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELwIiglLwFk0TIY8gKq3mghY8lEA2
  4I9oit8K6Cmrrx+HlXyQoL7dT1jiijRBl5T0DVtbr05vplip/1kzBLtkQg==
  -----END PUBLIC KEY-----
PEM
SIG = ["304502204693d70bc913803d86ac66ec62f4470403033bc771f47f1441f1659e031f6150022100b79e9ac5f77d6f8a4a7bfb9c73cf789bb656eee3c3e1cd4b6033d7b892a420c0"].pack("H*")
p k.verify("SHA256", SIG, "signed by the EC leaf"), k.verify("SHA256", SIG, "something else")
puts ec_leaf.to_text.lines.first(24).join
ed_store = X::Store.new.add_cert(ed)
show(ed_store, ed)
ed_store.flags = X::V_FLAG_CHECK_SS_SIGNATURE
show(ed_store, ed)
p ed.subject.to_s, ed.serial.to_i

puts "== real-world roots (GTS Root R4: ECDSA P-384; ISRG Root X1: RSA 4096)"
gts, isrg = X::Certificate.load(<<~PEM)
  -----BEGIN CERTIFICATE-----
  MIICCjCCAZGgAwIBAgIQbkepyIuUtui7OyrYorLBmTAKBggqhkjOPQQDAzBHMQsw
  CQYDVQQGEwJVUzEiMCAGA1UEChMZR29vZ2xlIFRydXN0IFNlcnZpY2VzIExMQzEU
  MBIGA1UEAxMLR1RTIFJvb3QgUjQwHhcNMTYwNjIyMDAwMDAwWhcNMzYwNjIyMDAw
  MDAwWjBHMQswCQYDVQQGEwJVUzEiMCAGA1UEChMZR29vZ2xlIFRydXN0IFNlcnZp
  Y2VzIExMQzEUMBIGA1UEAxMLR1RTIFJvb3QgUjQwdjAQBgcqhkjOPQIBBgUrgQQA
  IgNiAATzdHOnaItgrkO4NcWBMHtLSZ37wWHO5t5GvWvVYRg1rkDdc/eJkTBa6zzu
  hXyiQHY7qca4R9gq55KRanPpsXI5nymfopjTX15YhmUPoYRlBtHci8nHc8iMai/l
  xKvRHYqjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNVHRMBAf8EBTADAQH/MB0GA1Ud
  DgQWBBSATNbrdP9JNqPV2Py1PsVq8JQdjDAKBggqhkjOPQQDAwNnADBkAjBqUFJ0
  CMRw3J5QdCHojXohw0+WbhXRIjVhLfoIN+4Zba3bssx9BzT1YBkstTTZbyACMANx
  sbqjYAuG7ZoIapVon+Kz4ZNkfF6Tpt95LY2F45TPI11xzPKwTdb+mciUqXWi4w==
  -----END CERTIFICATE-----
  -----BEGIN CERTIFICATE-----
  MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw
  TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh
  cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4
  WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu
  ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY
  MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc
  h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+
  0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U
  A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW
  T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH
  B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC
  B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv
  KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn
  OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn
  jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw
  qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI
  rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV
  HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq
  hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL
  ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ
  3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK
  NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5
  ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur
  TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC
  jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc
  oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq
  4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA
  mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d
  emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=
  -----END CERTIFICATE-----
PEM
[gts, isrg].each do |c|
  puts c.to_text
  p c.subject.to_s(X::Name::RFC2253), c.subject.hash, c.extensions.map(&:to_a), c.public_key.class
  rs = X::Store.new.add_cert(c)
  rs.flags = X::V_FLAG_CHECK_SS_SIGNATURE
  rs.time = Time.utc(2030)
  show(rs, c)
  rs.time = Time.utc(2050)
  show(rs, c)
end
p gts.verify(gts.public_key), isrg.verify(isrg.public_key), isrg.verify(gts.public_key) rescue p $!
