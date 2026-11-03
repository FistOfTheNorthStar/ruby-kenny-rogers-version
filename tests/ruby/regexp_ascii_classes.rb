# Ruby's \w \W \d \D \s \S are ASCII-only (Discourse's username allowlist: "π" doesn't match
# /[\w.-]/), while \b, \p{Word}, POSIX brackets and \p{L} stay Unicode
p "π".match?(/[\w.-]/), "π" =~ /\w/, "é".match?(/\w/), "π".match?(/\A\w\z/), "ä".match?(/[äöü]/)
r = Regexp.new("[äöüßÄÖÜẞ]")
p "π".match?(r), "ä".match?(r)
p "été café".scan(/\w+/), "xéy" =~ /\bé/, "٣" =~ /\d/, "a　b".split(/\s/), "é" =~ /[[:alpha:]]/, "é" =~ /\p{L}/, "é" =~ /\p{Word}/
p "AbCÉ".gsub(/\W/, "_"), "é" =~ /[\w]/, "é" =~ /[^\w]/, "É" =~ /é/i
p "x-y z".scan(/[\w-]+/), "a1 b2".scan(/[^\W\d]/), "tab\there".split(/[\s]/), "\\w".match?(/\\w/), "é1".scan(/[[:alpha:]\d]/), "aé".scan(/[\p{L}&&[^\w]]/)
p "ab ab" =~ /(?<w>\w+) \k<w>/, "q" =~ /\x71/, "ab12g".scan(/\h/), "Ab" =~ /\Ab\z/i, "é" =~ /\W/, "é".match?(/[\W]/), "1é".scan(/[\D]/)
p Regexp.new("\\w+").source, /\w/.source, /[\s\S]/.match?("é"), "a\vb".split(/\s/), "x y".split(/\s/)
p "word wörd" =~ /\bwörd\b/, "über".match?(/\Aü\w+\z/), "日本 語".scan(/\S+/), "12٣".scan(/\d+/)
p "a_b".tr("^\\w", "*") rescue p $!.class
p "Hello Wörld".scan(/\w+/i), "UPPER" =~ /\A[[:upper:]]+\z/
