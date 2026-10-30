require "bundler/setup"
require "google/protobuf"
require "google/protobuf/descriptor_pb"
require "google/protobuf/timestamp_pb"
require "google/protobuf/struct_pb"
require "google/protobuf/wrappers_pb"
require "google/protobuf/any_pb"
require "google/protobuf/well_known_types"

# What protoc would generate for:
#   syntax = "proto3"; package demo;
#   enum Kind { UNKNOWN = 0; BOOK = 1; MUSIC = 2; }
#   message Item { string name = 1; int64 qty = 2; double price = 3; Kind kind = 4;
#     repeated string tags = 5; map<string, int32> attrs = 6; Item child = 7;
#     oneof pick { string code = 8; int32 num = 9; } google.protobuf.Timestamp at = 10; bytes blob = 11; }
F = Google::Protobuf::FieldDescriptorProto
def field(name, num, type, label: :LABEL_OPTIONAL, type_name: nil, oneof: nil, json: nil)
  F.new(name: name, number: num, type: type, label: label, type_name: type_name, oneof_index: oneof, json_name: json)
end
entry = Google::Protobuf::DescriptorProto.new(name: "AttrsEntry", options: Google::Protobuf::MessageOptions.new(map_entry: true),
  field: [field("key", 1, :TYPE_STRING), field("value", 2, :TYPE_INT32)])
item = Google::Protobuf::DescriptorProto.new(name: "Item", nested_type: [entry],
  oneof_decl: [Google::Protobuf::OneofDescriptorProto.new(name: "pick")],
  field: [field("name", 1, :TYPE_STRING), field("qty", 2, :TYPE_INT64), field("price", 3, :TYPE_DOUBLE),
          field("kind", 4, :TYPE_ENUM, type_name: ".demo.Kind"), field("tags", 5, :TYPE_STRING, label: :LABEL_REPEATED),
          field("attrs", 6, :TYPE_MESSAGE, label: :LABEL_REPEATED, type_name: ".demo.Item.AttrsEntry"),
          field("child", 7, :TYPE_MESSAGE, type_name: ".demo.Item"), field("code", 8, :TYPE_STRING, oneof: 0),
          field("num", 9, :TYPE_INT32, oneof: 0), field("at", 10, :TYPE_MESSAGE, type_name: ".google.protobuf.Timestamp"),
          field("blob", 11, :TYPE_BYTES)])
kind = Google::Protobuf::EnumDescriptorProto.new(name: "Kind", value: %w[UNKNOWN BOOK MUSIC].each_with_index.map { |n, i| Google::Protobuf::EnumValueDescriptorProto.new(name: n, number: i) })
file = Google::Protobuf::FileDescriptorProto.new(name: "demo.proto", package: "demo", syntax: "proto3",
  dependency: ["google/protobuf/timestamp.proto"], message_type: [item], enum_type: [kind])
Google::Protobuf::DescriptorPool.generated_pool.add_serialized_file(Google::Protobuf::FileDescriptorProto.encode(file))
Item = Google::Protobuf::DescriptorPool.generated_pool.lookup("demo.Item").msgclass
Kind = Google::Protobuf::DescriptorPool.generated_pool.lookup("demo.Kind").enummodule

m = Item.new(name: "Gambler", qty: 3, price: 9.5, kind: :MUSIC, tags: %w[a b], attrs: { "x" => 1 },
             child: Item.new(name: "kid"), code: "C1", at: Google::Protobuf::Timestamp.new(seconds: 1_700_000_000, nanos: 5), blob: "\x00\xff".b)
p m.name, m.qty, m.kind, m.tags.to_a, m.attrs.to_h, m.child.name, m.pick, m.at.to_time.utc, m.blob
bin = Item.encode(m)
p bin.bytesize, bin.encoding, bin.unpack1("H*")
m2 = Item.decode(bin)
p m2 == m, m2.to_h
puts Item.encode_json(m)
puts Item.encode_json(m, preserve_proto_fieldnames: true, emit_defaults: true).size
p Item.decode_json('{"name":"j","qty":"7","kind":"BOOK","num":5,"attrs":{"q":2}}').to_h
m.num = 4
p m.pick, m.code, m.has_child?, Item.new.has_child?, Item.new.kind, Kind::BOOK, Kind.lookup(2), Kind.resolve(:MUSIC)
m.tags << "c"; m.attrs["y"] = 2; m.clear_child
p m.tags.size, m.attrs.keys.sort, m.child
p m.inspect.size > 0, Item.descriptor.map(&:name), Item.descriptor.lookup("qty").type
def try
  yield
rescue => e
  puts "#{e.class}: #{e.message}"
end
try { Item.new(qty: "x") }
try { Item.new(nope: 1) }
try { m.kind = :NOPE }
try { m.tags << 5 }
try { Item.decode("\xff\xff\xff".b) }
try { Item.decode_json("{bad") }
s = Google::Protobuf::Struct.from_hash({ "a" => 1, "b" => [true, nil, "s"], "c" => { "d" => 2.5 } })
p s.to_h.sort, Google::Protobuf::Struct.decode(Google::Protobuf::Struct.encode(s)).to_h.sort, s["c"]["d"], s.fields.keys.sort
p Google::Protobuf::Int32Value.new(value: 7).value
any = Google::Protobuf::Any.pack(Item.new(name: "packed"))
p any.type_url, any.unpack(Item).name, any.is(Item)
ts = Google::Protobuf::Timestamp.new; ts.from_time(Time.at(1_600_000_000, 123_456_789, :nsec))
p ts.seconds, ts.nanos, ts.to_f
p Item.new(name: "f").frozen?, Item.new(name: "f").freeze.frozen?
