# s[i] == "x" / s[i] != "x", String#[] with an Integer, String#length

def checks(s, i, lit)
  [s[i] == lit, s[i] != lit]
end
p checks("abc", 0, "a"), checks("abc", 1, "a"), checks("abc", -1, "c"), checks("abc", -4, "a")
p checks("abc", 3, "a"), checks("abc", 99, "a"), checks("abc", 0, "ab"), checks("abc", 0, "")
p checks("héllo", 1, "é"), checks("héllo", 2, "l"), checks("héllo", -4, "é"), checks("héllo", 9, "o")
p checks("\xff\x00".b, 1, "\x00"), checks("ab".b, 0, "a")

# not a String receiver, or not an Integer index
p [1, "x"][1] == "x", {0 => "x"}[0] != "x", "abc"[1.5] == "b", "abc"[1..] == "bc"
begin
  nil[0] == "x"
rescue NoMethodError => e
  p e.message
end

# the index is evaluated once
k = 0
p "abc"[(k += 1)] == "b", k

# String subclasses
class Sub < String
  def [](i) = "z"
end
class Plain < String; end
p Sub.new("abc")[0] == "z", Plain.new("abc")[0] == "a", Plain.new("abc")[0].class

# String#length / #size / #slice
s = +"abc"
p s.length, s.size, s.slice(1), s.slice(-1), s.slice(5), "héllo".length, "".length
s << "é"
p s.length, s[3] == "é", s[3]
f = "frozen".freeze
p f.length, f[0] == "f", f.length

# redefinitions
def first_is_a(s) = s[0] == "a"
p first_is_a("abc")
class String
  alias_method :orig_eq, :==
  def ==(o) = true
end
p first_is_a("xyz")
class String
  alias_method :==, :orig_eq
end
p first_is_a("xyz")
class NilClass
  def ==(o) = o == "q"
end
p "abc"[9] == "q"
class NilClass
  remove_method :==
end
p "abc"[9] == "q"
class String
  alias_method :orig_aref, :[]
  def [](*a) = "a"
end
p first_is_a("xyz"), "xyz".slice(0)
class String
  alias_method :[], :orig_aref
end
p first_is_a("xyz")
class String
  def !=(o) = :neq
end
p "abc"[0] != "a"
class String
  remove_method :!=
end
p "abc"[0] != "a"
class String
  def length = -1
end
p "abc".length, "abc".size
