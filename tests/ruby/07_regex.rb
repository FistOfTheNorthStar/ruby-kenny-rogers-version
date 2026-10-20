s = "The year 2024 and 1999 were good; call 555-1234."
p s =~ /\d{4}/, $~[0], $~.pre_match
p s.scan(/\d{4}/), s.scan(/(\d{3})-(\d{4})/)
p s.gsub(/\d/) { |d| (d.to_i + 1) % 10 }
p s.sub(/(\w+) (\w+)/, '\2 \1'), s.gsub(/o/, "0")
p "a1b22c333".split(/\d+/), "one, two,three".split(/,\s*/)
if (m = s.match(/(?<num>\d{3})-(?<rest>\d{4})/))
  p m[:num], m["rest"], m.captures, m.named_captures, m.begin(0)
end
if /(?<first>\w+)@(?<domain>\w+)\.com/ =~ "mail kenny@example.com now"
  p first, domain, $1, $2, $~.post_match
end
p "Hello World".match?(/world/i), "abc" !~ /z/, "2024-10-19" =~ /\A\d{4}-\d{2}-\d{2}\z/
case "error: disk full"
when /^warn/ then p :warn
when /^error: (.*)/ then p [:error, $1]
end
words = "the quick brown fox".scan(/\w+/).map(&:capitalize)
p words.join(" ")
p "CamelCaseString".gsub(/([A-Z])/) { "_" + $1.downcase }.sub(/^_/, "")
p "x=1, y=2".scan(/(\w)=(\d)/).to_h
re = Regexp.new("h(.)llo", Regexp::IGNORECASE)
p re.source, re.inspect, re =~ "HELLO", Regexp.escape("a.b*c")
name = "kenny"
p "hi KENNY" =~ /#{name}/i
p "tel: 123".index(/\d/), "abcabc".rindex("c"), "price $42"[/\d+/], "key=val"[/(\w+)=(\w+)/, 2]
p "a-b_c d".tr("-_", "  ").split, "hello"[1..] =~ /l+/
p "foo bar baz".gsub(/\s+/, "-"), "AbC".swapcase, "ruby is fun".start_with?(/r\w+/)
p "1,2;3".split(/[,;]/).map(&:to_i).sum, "abc".each_char.select { |c| c =~ /[ac]/ }
p "2024-10-19".match(/(\d+)-(\d+)-(\d+)/).to_a.drop(1).map(&:to_i)
p "aaa".gsub(/a/, "a" => "1"), "hello world".split(//).first(3)
