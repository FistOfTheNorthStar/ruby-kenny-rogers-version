require "bundler/setup"
require "aws-eventstream"

msg = Aws::EventStream::Message.new(
  headers: {
    ":message-type" => Aws::EventStream::HeaderValue.new(value: "event", type: "string"),
    ":event-type" => Aws::EventStream::HeaderValue.new(value: "Records", type: "string"),
    "count" => Aws::EventStream::HeaderValue.new(value: 42, type: "integer"),
    "flag" => Aws::EventStream::HeaderValue.new(value: true, type: "bool_true"),
    "big" => Aws::EventStream::HeaderValue.new(value: 2**40, type: "long"),
    "raw" => Aws::EventStream::HeaderValue.new(value: "\x00\x01\xff".b, type: "bytes"),
  },
  payload: StringIO.new("hello event stream")
)
bytes = Aws::EventStream::Encoder.new.encode(msg)
p bytes.bytesize, bytes.encoding
p bytes.unpack1("H*")

decoder = Aws::EventStream::Decoder.new
# feed in two chunks, as a streaming response would
m1, eof = decoder.decode_chunk(bytes[0, 20])
p m1, eof
m2, eof = decoder.decode_chunk(bytes[20..])
p eof
m2.headers.sort.each { |k, v| p [k, v.type, v.value] }
p m2.payload.read

two = bytes + Aws::EventStream::Encoder.new.encode(Aws::EventStream::Message.new(headers: {}, payload: StringIO.new("x")))
out = []
Aws::EventStream::Decoder.new.decode(StringIO.new(two)) { |m| out << m.payload.read }
p out

bad = bytes.dup
bad.setbyte(bad.bytesize - 1, bad.getbyte(bad.bytesize - 1) ^ 1)
begin
  Aws::EventStream::Decoder.new.decode_chunk(bad)
rescue Aws::EventStream::Errors::MessageChecksumError => e
  p e.class
end
