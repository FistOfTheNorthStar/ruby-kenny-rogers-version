s = "Once upon a time in a world"
p s.rindex(" ", 11), s.rindex(" ", 3), s.rindex(" "), s.rindex(" ", -5), s.rindex(" ", -100), s.rindex(" ", 1000)
p "abcabc".rindex("c", 4), "abcabc".rindex("c", -2), "hello".rindex(/l/, 2), "hello".rindex(/l+/), $~[0], "ab".rindex("", 5), "aXbXc".rindex("X", 0), "abc".rindex("")
p "héllo wörld".index("o"), "héllo wörld".index("o", 5), "héllo wörld".rindex("ö"), "héllo wörld".rindex("l", 3), "héllo".index("l", -2), "héllo".index(/l/, 3), "héllo".index("x", 10), "héllo".index("", 5), "héllo".index("", 6)
p "日本語日本".rindex("日本", 2), "日本語日本".index("本", 2), "aaa".rindex("aa"), "aaa".rindex("aa", 0), "abc".index("c", -1), "abc".rindex(/b/, -1)
p "x".rindex(/y/), $~
# a binary String's positions are bytes (Net::BufferedIO#readuntil on a gzip body)
b = "\x1F\x8B\b\x00\xC0\xDD\xE3j\x00\x03\xED\xC6\xB1\t\x00\x00\r\n0\r\n".b
p b.index("\n", 17), b.index("\n", 18), b.rindex("\r", 18), b.rindex("\xC6".b), b.index("\xC6".b, 2)
