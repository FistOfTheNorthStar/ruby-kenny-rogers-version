# getbyte/setbyte, split and to_i edge cases (inline and fast paths)
def bytes_of(s, idxs) = idxs.map { |i| s.getbyte(i) }
s = "hello"
p bytes_of(s, [0, 4, 5, -1, -5, -6, 100])
t = +"abc"
i = 0
while i < 3
  t.setbyte(i, t.getbyte(i) + 1)
  i += 1
end
p t
t.setbyte(-1, 0x7a)
p t
t.setbyte(0, 321)
p t.bytes
begin
  t.setbyte(3, 1)
rescue IndexError => e
  puts "IndexError #{e.message}"
end
begin
  "frozen".freeze.setbyte(0, 65)
rescue => e
  puts e.class
end
begin
  t.getbyte("x")
rescue TypeError => e
  puts "TypeError"
end
class MyStr < String
  def getbyte(i) = 42
end
p bytes_of(MyStr.new("zz"), [0])
p bytes_of("é!", [0, 1, 2])

p "a,b,,c,,".split(","), "a,b,,c,,".split(",", -1), "a,b,c".split(",", 2)
p "a--b----c".split("--"), ",a".split(","), "".split(","), "abc".split("abcd")
p "  a  b c ".split, "a b".split(" "), "abc".split(""), "x::y".split("::")
p "12".to_i, "-12".to_i, "+7".to_i, "007".to_i, "-0".to_i, " 42 ".to_i, "1_000".to_i
p "12abc".to_i, "abc".to_i, "".to_i, "-".to_i, "999999999999999999".to_i, "9999999999999999999".to_i
p "-9999999999999999999".to_i, "123456789012345678".to_i, "0x1A".to_i
