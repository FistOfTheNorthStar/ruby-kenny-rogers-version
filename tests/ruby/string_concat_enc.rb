# Encodings of << / concat / + (CRuby rb_enc_check rules)
def t
  r = yield
  r.is_a?(String) ? [r.bytes, r.encoding.to_s, r.valid_encoding?] : r
rescue => e
  [e.class, e.message]
end
p t { +"q" << "\xff".b }
p t { "q".b << "é" }
p t { +"é" << "\xff".b }
p t { "\xff".b << "é" }
p t { "".b << "é" }
p t { +"" << "\xff".b }
p t { +"é" << "".b }
p t { "\xff".b << "a" }
p t { +"é" << "a".b }
p t { "".b << 255 }
p t { "".b << 256 }
p t { +"" << 255 }
p t { (+"a").force_encoding("US-ASCII") << 200 }
p t { (+"a").force_encoding("US-ASCII") << "é" }
p t { "é".force_encoding("US-ASCII").valid_encoding? }
p t { +"é" + "\xff".b }
p t { "q" + "\xff".b }
p t { +"x" << "é".encode("UTF-16LE") }
p t { "x".encode("UTF-16LE") << "y".encode("UTF-16LE") }
p t { "x".encode("ISO-8859-1") << "é" }
p t { "\xe9".b.force_encoding("ISO-8859-1") << "é" }
p t { "é".encode("ISO-8859-1") + "a" }
p t { s = +"é"; (s.concat("a".b, "\xff".b) rescue p $!.class); s }

# ljust/rjust/center: a binary String pads by bytes and stays binary; the pad's encoding mixes
# with the String's as for +
bin = "\xfc\x1e\x37".b
[bin.rjust(5, "\x00"), bin.ljust(5, "\x00"), bin.center(8, "ab"), bin.rjust(2), "ab".rjust(4, "\xff".b),
 "héllo".rjust(8, "é"), "héllo".center(9, "*-")].each { |x| p [x, x.encoding, x.bytesize] }
p "ab".ljust(5).encoding, "x".encode("US-ASCII").rjust(3).encoding
[-> { "\xfc".b.rjust(4, "é") }, -> { "é".rjust(4, "\xff".b) }, -> { "abc".rjust(6, "") }].each do |f|
  f.call
rescue => e
  p [e.class, e.message]
end
