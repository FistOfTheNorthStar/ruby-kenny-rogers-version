require "bundler/setup"
require "aws-sdk-s3"

creds = Aws::Credentials.new("AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")
s3 = Aws::S3::Client.new(region: "us-east-1", credentials: creds, stub_responses: true)
s3.stub_responses(:list_buckets, buckets: [{ name: "alpha" }, { name: "beta" }])
p s3.list_buckets.buckets.map(&:name)
s3.stub_responses(:list_objects_v2, contents: [{ key: "a.txt", size: 3 }, { key: "dir/b.txt", size: 10 }], is_truncated: false, key_count: 2)
resp = s3.list_objects_v2(bucket: "alpha", prefix: "")
p resp.contents.map { |o| [o.key, o.size] }, resp.key_count
s3.stub_responses(:get_object, ->(ctx) { { body: "contents of #{ctx.params[:key]}", content_type: "text/plain", etag: '"abc"' } })
g = s3.get_object(bucket: "alpha", key: "a.txt")
p g.body.read, g.content_type, g.etag
put = s3.put_object(bucket: "alpha", key: "new key.txt", body: "hello")
p put.class.name
req = put.context.http_request
p req.http_method, req.endpoint.to_s, req.headers["content-md5"], req.headers["x-amz-checksum-crc32"]
p req.body.read
s3.stub_responses(:head_object, "NotFound")
begin
  s3.head_object(bucket: "alpha", key: "missing")
rescue Aws::S3::Errors::NotFound => e
  p e.class
end
s3.stub_responses(:get_object, "NoSuchKey")
begin
  s3.get_object(bucket: "alpha", key: "zzz")
rescue Aws::S3::Errors::NoSuchKey => e
  p e.class, e.code
end

bucket = Aws::S3::Resource.new(client: s3).bucket("alpha")
p bucket.name, bucket.object("x/y.txt").key
p bucket.objects.map(&:key)

signer = Aws::S3::Presigner.new(client: Aws::S3::Client.new(region: "eu-west-1", credentials: creds))
url = signer.presigned_url(:get_object, bucket: "alpha", key: "photos/cat.jpg", expires_in: 600, time: Time.utc(2024, 1, 2, 3, 4, 5))
puts url
url2 = signer.presigned_url(:put_object, bucket: "alpha", key: "up.bin", time: Time.utc(2024, 1, 2))
puts url2.sub(/X-Amz-Signature=\h+/, "SIG"), url2[/X-Amz-Signature=\h+/].size
pc = Aws::S3::Client.new(region: "us-west-2", credentials: creds, stub_responses: true, force_path_style: true)
p pc.get_object(bucket: "my.bucket", key: "k").context.http_request.endpoint.to_s
