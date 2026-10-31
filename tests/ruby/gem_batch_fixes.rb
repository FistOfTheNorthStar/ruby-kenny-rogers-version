# Fixes found running gems (sequel, webmock, pg, faker, factory_bot): checked against CRuby.

# String#match / match? compile a String pattern as a regexp (webmock: uri.match('^https?://'))
p "http://x".match("^https?://")&.to_s, "abc".match?("b."), "a.c".match(".")&.to_s, "xay".match("a", 2)
begin
  "a".match("+")
rescue RegexpError => e
  p e.class
end
p "a.c".index("."), "a.c".sub(".", "!"), "a+b".split("+")

# KeyError.new(msg, receiver:, key:)
e = KeyError.new("boom", receiver: { a: 1 }, key: :b)
p e.message, e.receiver, e.key
begin
  KeyError.new("x").key
rescue ArgumentError => x
  p x.message
end
begin
  KeyError.new("x").receiver
rescue ArgumentError => x
  p x.message
end
begin
  KeyError.new("x", foo: 1)
rescue ArgumentError => x
  p x.message
end
begin
  {}.fetch(:zz)
rescue KeyError => x
  p x.key, x.receiver
end

# Random#rand / Kernel#rand with a Range of non-Integer ends (faker: rand(Date..Date))
require "date"
r = Random.new(42)
d = r.rand(Date.new(2024, 1, 1)..Date.new(2024, 1, 31))
p d.class, (Date.new(2024, 1, 1)..Date.new(2024, 1, 31)).cover?(d)
t0 = Time.at(0)
t = r.rand(t0..(t0 + 10))
p t.class, t >= t0 && t <= t0 + 10
q = r.rand(Rational(1, 2)..Rational(7, 2))
p q.class
begin
  r.rand("a".."c")
rescue TypeError => x
  p x.message
end

# Module#set_temporary_name (sequel)
m = Module.new
p m.set_temporary_name("fake<1>").equal?(m), m.name, m.to_s
c = Class.new
c.set_temporary_name("my_class")
p c.name, c.new.class.name, Class.new(c).name
[["Foo"], ["A::B"], [""]].each do |a|
  x = Module.new
  x.set_temporary_name(*a)
rescue ArgumentError => err
  p err.message
end
m.set_temporary_name(nil)
p m.name
c.set_temporary_name("again")
TempNamedClass = c
p c.name
begin
  c.set_temporary_name("zz")
rescue RuntimeError => err
  p err.message
end
o = Module.new
o.set_temporary_name("outer<x>")
o.const_set(:Inner, Module.new)
p o::Inner.name
TempNamedOuter = o
p TempNamedOuter.name

# anonymous modules
an = Module.new
p an.inspect.sub(/\h{16}/, "X"), an.to_s.sub(/\h{16}/, "X")
an.const_set(:I, Class.new)
p an::I.name.sub(/\h{16}/, "X")
begin
  Module.new.new
rescue NoMethodError => err
  p err.message.sub(/\h{16}/, "X")
end

# IO#wait(events, timeout), IO::READABLE ... (pg)
p IO::READABLE, IO::PRIORITY, IO::WRITABLE
rd, wr = IO.pipe
p wr.wait(IO::WRITABLE, 0), rd.wait(IO::READABLE, 0), rd.wait(IO::READABLE | IO::PRIORITY, 0)
wr.write "x"
p rd.wait(IO::READABLE, 0), rd.wait(IO::READABLE | IO::PRIORITY, 0)
p rd.wait(0).equal?(rd), rd.wait(0.1, :read).equal?(rd), wr.wait(:write, 0).equal?(wr)
p rd.wait_priority(0)
