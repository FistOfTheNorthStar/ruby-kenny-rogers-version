require "bundler/setup"
require "base64"

p Base64.encode64("Hello, World!")
p Base64.encode64("x" * 100)
p Base64.decode64("SGVsbG8sIFdvcmxkIQ==\n")
p Base64.strict_encode64("\x00\xff\xfe binary".b)
p Base64.strict_decode64("AP/+IGJpbmFyeQ==").bytes
p Base64.urlsafe_encode64("???>>>"), Base64.urlsafe_encode64("???>", padding: false)
p Base64.urlsafe_decode64("Pz8_Pg"), Base64.urlsafe_decode64("Pz8-Pj4-")
p Base64.encode64("é").encoding, Base64.decode64("w6k=").encoding, Base64.decode64("w6k=").force_encoding("UTF-8")
p Base64.decode64("bad!!data")
begin
  Base64.strict_decode64("not base64!")
rescue ArgumentError => e
  puts "ArgumentError: #{e.message}"
end
p Base64.encode64(""), Base64.strict_encode64("a"), Base64.strict_encode64("ab")
