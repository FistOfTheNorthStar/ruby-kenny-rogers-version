# Character operations on a UTF-8 String with invalid bytes: each such byte is a character
# of its own (no replacement characters), as in CRuby; and reverse! returns self.
s = "a\xE3\x81b\xab\xcdé\xF0\x9F"
p s.size, s.chars, s[1,2], s[4], s[-1], s.reverse.bytes, s[2..4], s.slice(6, 9), s.each_char.to_a.size
t = +"x\xffy"; p t.reverse!, t.bytes, "aba".reverse!
begin; "aba".freeze.reverse!; rescue => e; p e.class; end
p "héllo"[1, 3], "héllo".chars, "héllo".reverse, "héllo".size, "\xE3\x81\x82\xff".size
