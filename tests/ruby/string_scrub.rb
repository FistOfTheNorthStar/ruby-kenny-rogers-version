# String#scrub! with "" (Discourse's TextCleaner.clean)
s = "abcあ\x81"
t = s.dup
p t.scrub!("")
p t, t.valid_encoding?
p s.scrub(""), s.scrub("?"), s.scrub { |b| "<#{b.unpack1('H*')}>" }
u = "ok".dup
p u.scrub!(""), u
p "a\xffb\xfe".dup.scrub!("")
p "x\xE3\x81".scrub, "\xF0\x80y".scrub.bytes.size
a = "z\xff".force_encoding("US-ASCII")
p a.scrub, a.scrub.encoding
p "\xff".b.scrub == "\xff".b, "\xff".b.scrub.encoding
p((("a\xff".scrub(1)) rescue $!.class))
