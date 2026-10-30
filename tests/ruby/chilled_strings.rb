# String literals in a file without a `frozen_string_literal` comment are chilled (Ruby 3.4,
# 4.0): not frozen, a new String each time, and the first change warns under
# Warning[:deprecated]
s = "a"; p s.frozen?; p "a".equal?("a"); t = +s; p t.equal?(s); s << "b"; p s, s.frozen?
p eval("%q()").frozen?
u = "x"; p((-u).equal?(u), u.frozen?)
v = "y"; p v.dup.frozen?, v.clone.frozen?
Warning[:deprecated] = true
w = "w"; w << "1"; z = "z"; z.upcase!; z.upcase!
p "lit #{1}".frozen?
def m = "q"; p m.equal?(m)
c = "chill"; p c.clone.frozen?; c2 = c.clone; c2 << "x"
c3 = "chill3"; c4 = c3.dup; c4 << "x"
h = {}; key = "key"; h[key] = 1; p h.keys[0].equal?(key), h.keys[0].frozen?
q = "qq"; q.instance_variable_set(:@a, 1)
a = %w[a b]; p a[0].frozen?; a[0] << "x"
b = ["c"]; b[0] << "x"
d = <<~E
  heredoc
E
p d.frozen?; d << "x"
e = "e"; p e.to_s.equal?(e)
j = "j"; p((+j).equal?(j)); j << "1"; p((+j).equal?(j))
def foo(s) = s << "!"
p foo("yo")
o = "o"; o.replace("p")
r = "r"; r.setbyte(0, 65); p r
s2 = "s"; s2[0] = "t"; p s2
3.times { |i| x = "loop"; x << i.to_s; p x }
arr = []; 2.times { arr << "el" }; arr[0] << "!"; p arr
buf = +""; 2.times { buf << "ab" }; p buf
Warning[:deprecated] = false
p "o".object_id == "o".object_id
p (+"a").frozen?
p "a".then { _1.frozen? }
p "x".freeze.frozen?, (-"x").frozen?
p eval("'abc'").frozen?, eval("# frozen_string_literal: true\n'abc'").frozen?
# receivers compiled as the frozen literal: their results are still new, unfrozen Strings
rs = [
  "abc".upcase, "ABC".upcase, "abc".downcase, "Abc".capitalize, "abc".swapcase, "abc".strip, "abc".lstrip, "abc".rstrip,
  "abc".chomp, "abc".chomp("x"), "abc".chop, "abc".squeeze, "abc".delete("x"), "abc".tr("x", "y"), "abc".tr_s("x", "y"),
  "abc".sub("x", "y"), "abc".gsub("x", "y"), "abc".center(2), "abc".ljust(2), "abc".rjust(2), "a".reverse, "abc".reverse,
  "abc".succ, "abc".b, "abc".scrub, "abc".unicode_normalize, "abc".inspect, "abc".dump, "abc".dup, +"abc",
  "abc".delete_prefix("x"), "abc".delete_suffix("x"), "abc".byteslice(0, 3), "abc".slice(0..), "abc"[0..], "abc"[0, 3],
  "abc" * 1, "abc" + "", "%s" % "abc", "abc".encode("UTF-8"), "abc".split(",")[0], "abc".partition("x")[0],
  "abc".rpartition("x")[2], "abc".lines[0], "abc".chars[0], "a".chars[0], "abc".unpack("a*")[0], "abc".encode,
]
p rs.map(&:frozen?).uniq, rs.size
p "abc".size, "abc" == "abc", "abc".start_with?("a"), "a-b".split("-"), "%05d" % 42, "abc".hash == "abc".hash
k = { "x" => 1 }; p k["x"], k.fetch("x"), k.key?("x"), k.dig("x")
k["y"] = 2; p k.keys.map(&:frozen?)
p [1, 2].join(", "), "a,b".split(","), "abc".include?("b")
p "abc".then(&:frozen?), "abc".itself.frozen?, "abc".to_s.frozen?, String("abc").frozen?
p "abc".frozen?, "abc".clone.frozen?, "abc".clone(freeze: true).frozen?
p __FILE__.frozen?
x = "abc".freeze; y = x.dup; y << "d"; p x, y
