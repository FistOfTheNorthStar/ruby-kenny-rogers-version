require "bundler/setup"
require "aws-sigv4"

signer = Aws::Sigv4::Signer.new(
  service: "s3", region: "us-east-1",
  access_key_id: "AKIDEXAMPLE", secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"
)
sig = signer.sign_request(
  http_method: "PUT",
  url: "https://examplebucket.s3.amazonaws.com/photos/photo1.jpg?acl=private",
  headers: { "X-Amz-Date" => "20130524T000000Z", "Content-Type" => "image/jpeg" },
  body: "hello world"
)
sig.headers.sort.each { |k, v| puts "#{k}: #{v}" }
puts sig.canonical_request
puts sig.string_to_sign
puts sig.content_sha256

url = signer.presign_url(
  http_method: "GET", url: "https://examplebucket.s3.amazonaws.com/test.txt",
  expires_in: 86400, time: Time.utc(2013, 5, 24)
)
puts url

sts = Aws::Sigv4::Signer.new(service: "sts", region: "eu-west-1",
  credentials_provider: Aws::Sigv4::StaticCredentialsProvider.new(access_key_id: "AK", secret_access_key: "SK", session_token: "TOK"),
  unsigned_headers: ["x-custom"], uri_escape_path: true, apply_checksum_header: false)
s2 = sts.sign_request(http_method: "POST", url: "https://sts.amazonaws.com/a%20b/c",
  headers: { "x-amz-date" => "20200101T120000Z", "x-custom" => "skip" }, body: "Action=GetCallerIdentity")
s2.headers.sort.each { |k, v| puts "#{k}: #{v}" }
p Aws::Sigv4::Signer.uri_escape("a b/c~d*e"), Aws::Sigv4::Signer.uri_escape_path("/a%20b/c")
begin
  Aws::Sigv4::Signer.new(service: "s3", region: "us-east-1")
rescue => e
  p e.class
end
