# String#downcase/upcase/capitalize/swapcase options (:turkic: Discourse's TextCleaner with a Turkish locale; :ascii, :fold), titlecase digraphs, no final sigma
p "GIDA".downcase(:turkic), "istanbul".capitalize(:turkic), "I".downcase(:turkic), "i".upcase(:turkic), "İ".downcase, "GIDA".downcase
s = "GIDA".dup; s.downcase!(:turkic); p s
p "ıi".upcase(:turkic), "İSTANBUL".capitalize(:turkic), "ß".upcase, "ǆ".capitalize, "ŉ".upcase
p "Iİ".swapcase(:turkic)
p "ÀB".downcase(:ascii), "àb".upcase(:ascii), "Straße".downcase(:fold), "ΣΑΣ".downcase, "ab".upcase(:lithuanian)
p(("a".upcase(:bogus) rescue [$!.class, $!.message]))
p(("a".upcase("x") rescue [$!.class, $!.message]))
b = "ab\xff".b
p b.upcase, b.upcase.encoding, b.capitalize
