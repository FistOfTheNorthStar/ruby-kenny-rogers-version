require "openssl"
require "securerandom"

def hx(s)
  s.bytes.map { |b| "%02x" % b }.join
end

# --- digests ---
%w[MD5 SHA1 SHA224 SHA256 SHA384 SHA512].each do |n|
  d = OpenSSL::Digest.new(n)
  puts "#{d.name} #{d.digest_length} #{d.block_length} #{d.hexdigest}"
  d << "The quick brown fox " << "jumps over the lazy dog"
  puts d.hexdigest
end
puts OpenSSL::Digest.new("sha256").name
puts OpenSSL::Digest::SHA256.hexdigest("abc")
puts OpenSSL::Digest::SHA1.new.hexdigest("abc")
puts OpenSSL::Digest::SHA256.base64digest("x")
puts OpenSSL::Digest.hexdigest("SHA1", "abc")
puts hx(OpenSSL::Digest.digest("MD5", "abc"))
puts OpenSSL::Digest::SHA512.new("abc").hexdigest[0, 32]
d = OpenSSL::Digest.new("SHA256")
d.update("a")
e = d.dup
e << "b"
puts d.hexdigest[0, 16], e.hexdigest[0, 16]
puts d.inspect
puts d.digest.encoding, d.hexdigest.encoding, d.digest.bytesize
puts d == OpenSSL::Digest::SHA256.hexdigest("a")
d.reset
puts d.hexdigest[0, 16]
puts OpenSSL::Digest::SHA256.digest("x").size
begin
  OpenSSL::Digest.new("nope")
rescue RuntimeError => ex
  puts "RuntimeError #{ex.message.start_with?("Unsupported digest algorithm (nope)")}"
end
# multi-block input crossing the 64-byte boundary
puts OpenSSL::Digest::SHA256.hexdigest("x" * 1000)

# --- HMAC ---
puts OpenSSL::HMAC.hexdigest("SHA256", "key", "The quick brown fox jumps over the lazy dog")
puts OpenSSL::HMAC.hexdigest("SHA1", "key", "The quick brown fox jumps over the lazy dog")
puts OpenSSL::HMAC.hexdigest("MD5", "key", "The quick brown fox jumps over the lazy dog")
puts OpenSSL::HMAC.hexdigest(OpenSSL::Digest.new("SHA512"), "k" * 200, "long key")
puts hx(OpenSSL::HMAC.digest("SHA384", "secret", "data"))
h = OpenSSL::HMAC.new("key", "SHA256")
h << "The quick brown fox "
h.update("jumps over the lazy dog")
puts h.hexdigest, h.to_s == h.hexdigest
h2 = h.dup
h2 << "!"
puts h.hexdigest == h2.hexdigest
h.reset
puts h.hexdigest
puts OpenSSL::HMAC.new("key", OpenSSL::Digest::SHA1.new).hexdigest

# --- PBKDF2 / KDF ---
puts hx(OpenSSL::PKCS5.pbkdf2_hmac_sha1("password", "salt", 2, 20))
puts hx(OpenSSL::PKCS5.pbkdf2_hmac("password", "salt", 4096, 32, "sha256"))
puts hx(OpenSSL::PKCS5.pbkdf2_hmac("secret", "signed cookie", 1000, 64, OpenSSL::Digest::SHA1.new))
puts hx(OpenSSL::PKCS5.pbkdf2_hmac("secret", "authenticated encrypted cookie", 1000, 32, OpenSSL::Digest::SHA256.new))
puts hx(OpenSSL::KDF.pbkdf2_hmac("pw", salt: "NaCl", iterations: 10, length: 48, hash: "SHA512"))
puts hx(OpenSSL::KDF.hkdf("ikm" * 7, salt: "salt", info: "info", length: 42, hash: "SHA256"))

# --- ciphers ---
key32 = (0...32).map(&:chr).join
key16 = key32[0, 16]
iv16 = (100...116).map(&:chr).join
iv12 = iv16[0, 12]
msg = "Attack at dawn! " * 3 + "tail"

%w[aes-256-gcm aes-128-gcm aes-256-cbc aes-128-cbc aes-192-cbc aes-256-ctr aes-128-ecb].each do |name|
  c = OpenSSL::Cipher.new(name)
  puts [name, c.name, c.key_len, c.iv_len, c.block_size, c.authenticated?].inspect
end

c = OpenSSL::Cipher.new("aes-256-cbc")
c.encrypt
c.key = key32
c.iv = iv16
ct = c.update(msg) + c.final
puts hx(ct), ct.encoding
d = OpenSSL::Cipher.new("aes-256-cbc")
d.decrypt
d.key = key32
d.iv = iv16
puts d.update(ct) + d.final

c = OpenSSL::Cipher.new("aes-128-cbc")
c.encrypt
c.key = key16
c.iv = iv16
puts hx(c.update("exactly16bytes!!") + c.final)
# streaming in odd pieces gives the same result
c = OpenSSL::Cipher.new("aes-128-cbc")
c.encrypt
c.key = key16
c.iv = iv16
parts = ["exa", "ctly16by", "tes!!"].map { |p| c.update(p) }
puts hx(parts.join + c.final)

c = OpenSSL::Cipher.new("aes-256-cbc")
c.encrypt
c.padding = 0
c.key = key32
c.iv = iv16
puts hx(c.update("0123456789abcdef") + c.final)

c = OpenSSL::Cipher.new("aes-256-ctr")
c.encrypt
c.key = key32
c.iv = iv16
ct = c.update(msg) + c.final
puts hx(ct)
c = OpenSSL::Cipher.new("aes-128-ecb")
c.encrypt
c.key = key16
puts hx(c.update("ecb mode") + c.final)

# GCM, the way ActiveSupport::MessageEncryptor uses it
c = OpenSSL::Cipher.new("aes-256-gcm")
c.encrypt
c.key = key32
c.iv = iv12
c.auth_data = ""
ct = c.update(msg)
ct << c.final
tag = c.auth_tag
puts hx(ct), hx(tag), tag.bytesize, hx(c.auth_tag(12))

c = OpenSSL::Cipher.new("aes-128-gcm")
c.encrypt
c.key = key16
c.iv = iv12
c.auth_data = "header"
ct2 = c.update("short")
ct2 << c.final
puts hx(ct2), hx(c.auth_tag)

d = OpenSSL::Cipher.new("aes-256-gcm")
d.decrypt
d.key = key32
d.iv = iv12
d.auth_tag = tag
d.auth_data = ""
pt = d.update(ct)
pt << d.final
puts pt

# tampered tag
d = OpenSSL::Cipher.new("aes-256-gcm")
d.decrypt
d.key = key32
d.iv = iv12
d.auth_tag = "x" * 16
d.auth_data = ""
d.update(ct)
begin
  d.final
  puts "no error"
rescue OpenSSL::Cipher::CipherError => ex
  puts "CipherError #{ex.message.inspect}"
end

# random key/iv round trip
c = OpenSSL::Cipher.new("aes-256-gcm").encrypt
k = c.random_key
iv = c.random_iv
puts k.bytesize, iv.bytesize
c.auth_data = "aad"
secret = c.update("round trip ✓") + c.final
t = c.auth_tag
d = OpenSSL::Cipher.new("aes-256-gcm").decrypt
d.key = k
d.iv = iv
d.auth_tag = t
d.auth_data = "aad"
puts (d.update(secret) + d.final).force_encoding("UTF-8")

# errors
c = OpenSSL::Cipher.new("aes-256-gcm")
c.encrypt
[-> { c.key = "short" }, -> { c.iv = "x" }].each do |l|
  l.call
rescue ArgumentError => ex
  puts "ArgumentError #{ex.message}"
end
c.key = key32
c.iv = iv12
begin
  c.update("")
rescue ArgumentError => ex
  puts "ArgumentError #{ex.message}"
end
begin
  c.auth_tag
rescue OpenSSL::Cipher::CipherError => ex
  puts "CipherError #{ex.message}"
end
begin
  OpenSSL::Cipher.new("rot13")
rescue RuntimeError => ex
  puts "RuntimeError #{ex.message}"
end
d = OpenSSL::Cipher.new("aes-256-cbc")
d.decrypt
d.key = key32
d.iv = iv16
d.update("q" * 16)
begin
  d.final
rescue OpenSSL::Cipher::CipherError => ex
  puts "CipherError #{ex.message}"
end
puts OpenSSL::Cipher::CipherError.ancestors.take(3).inspect
puts OpenSSL::Cipher::AES.new(256, :GCM).name, OpenSSL::Cipher::AES128.new(:CBC).name

# GCM with a 16-byte IV, AAD and updates that straddle block boundaries
c = OpenSSL::Cipher.new("aes-256-gcm").encrypt
c.iv_len = 16
c.key = key32
c.iv = iv16
c.auth_data = "a" * 20
ct = ["hello wor", "ld, this is a longer gcm message ", "spanning several blocks!"].map { |p| c.update(p) }.join + c.final
puts hx(ct), hx(c.auth_tag)
d = OpenSSL::Cipher.new("aes-256-gcm").decrypt
d.iv_len = 16
d.key = key32
d.iv = iv16
d.auth_tag = c.auth_tag
d.auth_data = "a" * 20
out = (0...ct.bytesize).step(7).map { |i| d.update(ct[i, 7]) }.join
puts out + d.final

# CBC decrypt holds back the last block until final
e = OpenSSL::Cipher.new("aes-128-cbc").encrypt
e.key = key16
e.iv = iv16
ct = e.update("x" * 47) + e.final
d = OpenSSL::Cipher.new("aes-128-cbc").decrypt
d.key = key16
d.iv = iv16
puts d.update(ct[0, 5]).size, d.update(ct[5, 27]).size, d.update(ct[32..]).size, d.final.size

# CTR counter wraps around 2**128
c = OpenSSL::Cipher.new("aes-128-ctr").encrypt
c.key = key16
c.iv = "\xff".b * 16
puts hx(c.update("z" * 40) + c.final)

# RFC 5869 test case 1
okm = OpenSSL::KDF.hkdf("\x0b" * 22, salt: (0..12).map(&:chr).join, info: (0xf0..0xf9).map(&:chr).join.b, length: 42, hash: "SHA256")
puts hx(okm)

# --- misc ---
r = OpenSSL::Random.random_bytes(16)
puts r.bytesize, r.encoding
puts OpenSSL::SSL::VERIFY_PEER, OpenSSL::SSL::VERIFY_NONE
puts OpenSSL::VERSION.is_a?(String), OpenSSL::OPENSSL_VERSION.start_with?("OpenSSL")
if OpenSSL.respond_to?(:fixed_length_secure_compare) # openssl gem >= 2.2 (Ruby 3.x)
  puts OpenSSL.fixed_length_secure_compare("abc", "abc"), OpenSSL.fixed_length_secure_compare("abc", "abd")
  begin
    OpenSSL.fixed_length_secure_compare("a", "ab")
  rescue ArgumentError => ex
    puts "ArgumentError #{ex.message}"
  end
  puts OpenSSL.secure_compare("a", "ab"), OpenSSL.secure_compare("same", "same")
end

# --- SecureRandom ---
puts SecureRandom.hex.size, SecureRandom.hex(4).size
puts SecureRandom.hex(16) =~ /\A\h{32}\z/ ? "hex ok" : "hex bad"
puts SecureRandom.uuid =~ /\A\h{8}-\h{4}-4\h{3}-[89ab]\h{3}-\h{12}\z/ ? "uuid ok" : "uuid bad"
puts SecureRandom.base64(12).size, SecureRandom.base64(10) =~ /\A[A-Za-z0-9+\/]{14}==\z/ ? "b64 ok" : "b64 bad"
puts SecureRandom.urlsafe_base64(16) =~ /\A[A-Za-z0-9_-]{22}\z/ ? "url ok" : "url bad"
puts SecureRandom.urlsafe_base64(16, true).size
puts SecureRandom.random_bytes(10).bytesize, SecureRandom.random_bytes.encoding
puts (1..200).map { SecureRandom.random_number(10) }.all? { |x| x.between?(0, 9) }
puts SecureRandom.random_number.class, SecureRandom.random_number(2.5) < 2.5
puts (1..100).map { SecureRandom.random_number(5..7) }.uniq.sort.inspect
puts SecureRandom.alphanumeric(20) =~ /\A[A-Za-z0-9]{20}\z/ ? "alnum ok" : "alnum bad"
puts SecureRandom.alphanumeric(8, chars: %w[a b]) =~ /\A[ab]{8}\z/ ? "chars ok" : "chars bad"
