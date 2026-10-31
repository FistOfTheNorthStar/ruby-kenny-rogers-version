require "bundler/setup"
require "bcrypt"

salt = "$2a$04$abcdefghijklmnopqrstuu"
p BCrypt::Engine.hash_secret("secret", salt)
p BCrypt::Engine.hash_secret("", "$2a$05$CCCCCCCCCCCCCCCCCCCCC.")
pw = BCrypt::Password.create("correct horse", cost: 4)
p pw.cost, pw.version, pw.salt.size, pw.to_s.size
p pw == "correct horse", pw == "wrong", pw.is_password?("correct horse")
again = BCrypt::Password.new(pw.to_s)
p again == "correct horse", again.checksum == pw.checksum
p BCrypt::Engine.valid_salt?(salt), BCrypt::Engine.valid_salt?("nope")
p BCrypt::Password.valid_hash?(pw.to_s), BCrypt::Password.valid_hash?("x")
begin
  BCrypt::Password.new("not a hash")
rescue BCrypt::Errors::InvalidHash => e
  p e.class
end
p BCrypt::Engine.generate_salt(5).start_with?("$2a$05$")
p BCrypt::Engine.calibrate(1).is_a?(Integer)
