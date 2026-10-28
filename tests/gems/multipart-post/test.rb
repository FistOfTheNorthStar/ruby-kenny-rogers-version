require "bundler/setup"
require "net/http/post/multipart"
require "stringio"

io = Multipart::Post::UploadIO.new(StringIO.new("file content\nline2"), "text/plain", "notes.txt")
p io.original_filename, io.content_type, io.local_path
req = Net::HTTP::Post::Multipart.new("/upload", { "title" => "hello", "file" => io, "list" => ["a", "b"] }, {}, "BOUNDARY123")
p req.path, req["Content-Type"], req.content_length
puts req.body_stream.read
put = Net::HTTP::Put::Multipart.new(URI("http://example.com/x"), { "n" => 1 }, { "X-Foo" => "bar" }, "B2")
p put.method, put["X-Foo"], put["Content-Type"]
puts put.body_stream.read
p UploadIO.equal?(Multipart::Post::UploadIO)
part = Multipart::Post::Parts::ParamPart.new("B", "k", "v")
p part.length, part.to_io.read
fp = Multipart::Post::Parts::FilePart.new("B", "f", UploadIO.new(StringIO.new("xy"), "application/octet-stream", "a.bin"))
p fp.length, fp.to_io.read
c = Multipart::Post::CompositeReadIO.new(StringIO.new("abc"), StringIO.new("def"))
p c.read(2), c.read(3), c.read, c.read(1)
