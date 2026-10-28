require "bundler/setup"
require "msgpack"

obj = { "a" => 1, "b" => [1.5, nil, true, false], "c" => "str", "d" => -129, "e" => 2**40, "f" => { "x" => "y" } }
packed = obj.to_msgpack
p packed.bytesize, packed.encoding
p packed.unpack1("H*")
p MessagePack.unpack(packed) == obj
p MessagePack.unpack(packed)
p MessagePack.pack([1, 2, 3]).bytes
p MessagePack.unpack("\x93\x01\x02\x03".b)
p MessagePack.unpack(MessagePack.pack(:sym))
p MessagePack.unpack(MessagePack.pack({ a: 1 }), symbolize_keys: true)
p "x".b.to_msgpack, 1.0.to_msgpack.bytes, (2**64 - 1).to_msgpack.bytes

pk = MessagePack::Packer.new
pk.write_array_header(2).write(1).write("two")
pk.write_map_header(1).write("k").write_nil
p pk.to_s.bytes

u = MessagePack::Unpacker.new
u.feed_each(MessagePack.pack(1) + MessagePack.pack("x") + MessagePack.pack([2])) { |o| p o }
u.feed("\x92\x01".b)
u.feed("\x02".b)
u.each { |o| p o }

Point = Struct.new(:x, :y)
f = MessagePack::Factory.new
f.register_type(1, Point, packer: ->(pt) { [pt.x, pt.y].pack("l>l>") }, unpacker: ->(s) { Point.new(*s.unpack("l>l>")) })
s = f.dump(Point.new(3, -4))
p s.bytes
p f.load(s)
p f.load(f.dump([Point.new(1, 2), { "p" => Point.new(5, 6) }]))
p MessagePack.unpack(MessagePack.pack(MessagePack::ExtensionValue.new(5, "raw")), allow_unknown_ext: true)

begin
  MessagePack.unpack("\xc1".b)
rescue MessagePack::MalformedFormatError => e
  puts "MalformedFormatError"
end
begin
  MessagePack.unpack("\x92\x01".b)
rescue EOFError => e
  puts "EOFError"
end
p MessagePack::Timestamp.from_msgpack_ext(MessagePack::Timestamp.to_msgpack_ext(1, 500)).then { |t| [t.sec, t.nsec] }
