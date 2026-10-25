# String#encode: UTF-8 / UTF-16 / UTF-32 / US-ASCII / ASCII-8BIT / ISO-8859-1 / Windows-1252, options and errors
def t
  r = yield
  r.is_a?(String) ? [r, r.encoding.to_s, r.bytes.size] : r
rescue => e
  [e.class, e.message]
end
p t { "\xff".encode("UTF-8", invalid: :replace) }
p t { "a\xffb".encode("UTF-16LE", invalid: :replace).encode("UTF-8") }
p t { "a\xffb".encode("UTF-16LE") }
p t { "a\xe2\x82".encode("UTF-16LE") }
p t { "\xff".b.encode("UTF-8") }
p t { "ab".b.encode("UTF-8") }
p t { "é".encode("US-ASCII") }
p t { "é".encode("BINARY") }
p t { "é€".encode("ISO-8859-1") }
p t { "é€".encode("ISO-8859-1", undef: :replace) }
p t { "é€".encode("US-ASCII", undef: :replace, replace: "*") }
p t { "é".encode("US-ASCII", fallback: { "é" => "e" }) }
p t { "é".encode("US-ASCII", fallback: ->(c) { "[#{c.ord}]" }) }
p t { "<a&\"b\">é".encode("US-ASCII", xml: :text) }
p t { "<a&\"b\">é".encode("US-ASCII", xml: :attr) }
p t { "<a>".encode(xml: :text) }
p t { "a\r\nb\rc".encode(universal_newline: true) }
p t { "a\nb".encode(crlf_newline: true) }
p t { "a\nb".encode("UTF-8", newline: :cr) }
p t { "x".encode("Shift_JIS") }
p t { "x".encode("KLINGON") }
p t { "x".encode }
p t { "x".encode(Encoding::UTF_16BE) }
p t { "€".encode("Windows-1252").bytes }
p t { "\x80".b.force_encoding("Windows-1252").encode("UTF-8") }
p t { "\x81".b.force_encoding("Windows-1252").encode("UTF-8") }
p t { "hé".force_encoding("UTF-16LE").valid_encoding? }
p t { "h\x00".force_encoding("UTF-16LE").encode("UTF-8", "UTF-16BE") }
p t { "😀".encode("UTF-16BE").bytes }
p t { "\x3d\xd8".b.encode("UTF-8", "UTF-16LE") }
p t { "é".encode("UTF-32").bytes }
p t { s = +"é"; s.encode!("UTF-16LE"); [s.bytes, s.encoding.to_s] }
p t { "é".encode("UTF-8", "ISO-8859-1") }
p t { s = "hé".encode("UTF-16LE"); [s.valid_encoding?, s.dup.encoding.to_s, s.inspect] }
p "é".encode("utf-16le").encoding.name
