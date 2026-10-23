# `s = s + x` appended in place while the local is the only reference

def build(n)
  s = ""
  i = 0
  while i < n
    s = s + i.to_s
    i += 1
  end
  s
end
a = build(5)
b = build(5)
a << "!"
p a, b

# a copy kept before the next append
def keep
  s = ""
  s = s + "a"
  t = s
  s = s + "b"
  [t, s]
end
p keep

# kept by a call, in a loop
def pushes
  list = []
  s = ""
  i = 0
  while i < 4
    s = s + "x"
    list.push(s)
    i += 1
  end
  list
end
p pushes

# kept in one branch only
def branchy(flags)
  s = ""
  kept = []
  flags.each_index do |k|
    kept << k
  end
  i = 0
  while i < flags.size
    if flags[i]
      kept << s
    end
    s = s + i.to_s
    i += 1
  end
  [s, kept]
end
p branchy([false, true, false, true])

# a literal assigned again on each pass stays as it was
def lit
  out = []
  i = 0
  while i < 3
    s = "abc"
    s += "d"
    s += i.to_s
    out << s
    i += 1
  end
  out
end
p lit
p "abc"

# object identity is that of a new String each time (once looked at)
def ids
  s = ""
  s = s + "a"
  id1 = s.object_id
  s = s + "b"
  [id1 == s.object_id, s]
end
p ids

# errors and conversions
def bad
  s = ""
  s = s + 1
rescue TypeError => e
  e.message
end
p bad
class ToS
  def to_str = "conv"
end
def conv
  s = ""
  s = s + "x"
  s = s + ToS.new
  s
end
p conv

# a String subclass receiver: the result is a plain String, the receiver unchanged
class MyStr < String; end
def sub
  s = ""
  m = MyStr.new("m")
  s = m
  s = s + "x"
  [s, s.class, m, m.class]
end
p sub

# encodings
def enc
  s = ""
  s = s + "a"
  s = s + "é"
  t = ""
  t = t + "\xff".b
  [s, s.encoding, s.length, t.encoding, t.bytesize]
end
p enc

# rescue in the middle of building
def resc
  s = ""
  i = 0
  while i < 4
    begin
      s = s + (i == 2 ? raise("boom") : i.to_s)
    rescue => e
      s = s + "<#{e.message}>"
    end
    i += 1
  end
  s
end
p resc

# String#+ redefined, then restored
def plus_loop
  s = ""
  i = 0
  while i < 3
    s = s + "y"
    i += 1
  end
  s
end
p plus_loop
class String
  alias_method :orig_plus, :+
  def +(o) = orig_plus("[]").orig_plus(o)
end
p plus_loop
class String
  alias_method :+, :orig_plus
end
p plus_loop
module Loud
  def +(o) = super(o.upcase)
end
String.prepend(Loud)
p plus_loop

# frozen result kept and appended again
def frz
  s = ""
  s = s + "a"
  s.freeze
  s = s + "b"
  s
end
p frz, frz.frozen?
