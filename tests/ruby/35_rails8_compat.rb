# kenny-run-flags: -I tests/ruby/35_rails8_compat
# Behavior Rails 8 (Zeitwerk, Solid Cache/Queue, Jbuilder, Active Storage) depends on.
require "json"

# a multiple assignment's value is its right-hand side
a = (x, y = 1, 2)
b = (q, = [3, 4])
p a, b, [x, y, q]

# `...` forwarded from a block, a lambda and an operator method
class Fwd
  def target(*a, **k, &b) = [a, k, b&.call]
  def via_block(...) = [1].map { target(...) }.first
  def via_lambda(...) = -> { target(...) }.call
  def +(...) = target(...)
end
f = Fwd.new
p f.via_block(1, k: 2) { 3 }, f.via_lambda(4), f + 5

# **{} passes nothing (BroadcastLogger#dispatch forwarding to Logger#warn(msg))
def one_opt(msg = :none) = msg
def splat_to(m, *args, **kw) = send(m, *args, **kw)
p splat_to(:one_opt, "hi"), splat_to(:one_opt)

# Set without require, Enumerable#to_set
p Set.new([1, 1, 2]).size, (1..3).to_set.include?(2)

# Module#const_added fires for constants, classes, modules
class Watch
  @added = []
  def self.const_added(name) = (@added << name)
  def self.added = @added
  A = 1
  class B; end
  module C; end
  const_set(:D, 2)
end
p Watch.added

# Module#dup copies class-level instance variables and singleton methods
module Orig
  @setting = :on
  def self.setting = @setting
end
Copy = Orig.dup
p Copy.setting

# super from a method whose `def` runs again for another class (route_set's included hook)
module Base
  def hello = "base"
end
2.times do |i|
  k = Class.new { include Base; def hello = "k:" + super }
  p k.new.hello
end

# Data.define
Point = Data.define(:x, :y) do
  def dist2 = x * x + y * y
end
pt = Point.new(x: 3, y: 4)
p pt.dist2, pt.with(y: 0).to_h, Point.new(1, 2).x, pt == Point.new(3, 4), Point.members
begin
  Point.new(1)
rescue ArgumentError => e
  p e.message
end

# Range#step with Rational and Float steps
p (0r..1r).step(1/2r).to_a, (1..2).step(0.5).to_a

# reads with a length are binary (TZInfo's zoneinfo reader)
File.write("/tmp/kenny_35_bin", "\xFF\x00abc".b)
p File.read("/tmp/kenny_35_bin", 2).encoding, File.binread("/tmp/kenny_35_bin").encoding
File.delete("/tmp/kenny_35_bin")

# Thread.each_caller_location
def where = Thread.each_caller_location { |l| break l.lineno }
p where.is_a?(Integer)

# every Encoding constant, dummy? and lookups
p Encoding::ISO_2022_JP.dummy?, Encoding::UTF_16LE.ascii_compatible?, Encoding.find("cp1252") == Encoding::Windows_1252
p Encoding.list.size > 100, "x".force_encoding("US-ASCII").encoding, "x".b.encoding

# a require that raised can be retried (rescue LoadError around an optional dependency)
3.times do
  require "optional_dep"
  p OptionalDep.ok?
rescue LoadError => e
  p e.message
end
p require("optional_dep")

# Fiber storage, inherited by new fibers and threads
Fiber[:request_id] = 7
p Fiber[:request_id], Fiber.new { Fiber[:request_id] }.resume, Thread.new { Fiber[:request_id] }.value
Fiber[:request_id] = nil
p Fiber.current.storage

# Enumerable methods with a block run it during each yield (Jbuilder's scoped collections)
class Scoped
  include Enumerable
  attr_reader :inside
  def each
    [1, 2].each do |v|
      @inside = true
      yield v
      @inside = false
    end
  end
end
sc = Scoped.new
p sc.map { sc.inside }, sc.select { sc.inside }, sc.each_slice(1).map { |s| [s, sc.inside] }
p sc.sort_by { -_1 }, sc.min_by { _1 }, sc.inject { |s, v| s + v }, sc.each_with_object([]) { |v, m| m << v }
nat = Enumerator.new { |y| n = 0; loop { y << (n += 1) } }
p nat.each_slice(2).first(2), nat.map.class

# a module included into a class and later into one of its ancestors (Rails 8.1's to_json)
module EncodeAll
  def to_json(state = nil, *) = state.is_a?(JSON::State) ? super : "encoded:" + JSON.generate(self)
end
[Enumerable, Object, Array, Hash, String].reverse_each { |k| k.include(EncodeAll) }
p({ "a" => ["b"] }.to_json, "s".to_json, JSON.generate({ "a" => 1 }), Hash.ancestors[1])

# erb/escape (Rails' html_escape): returns the string itself when nothing needs escaping
require "erb"
require "erb/escape"
plain = +"plain"
p ERB::Escape.html_escape(plain).equal?(plain), ERB::Util.html_escape("<a href='x'>&\"</a>"), ERB::Escape.html_escape(12)

# match? with a start position, and String#blank?'s whitespace regexp (Unicode spaces too)
p /b/.match?("abc", 2), "héllo".match?(/l/, 3), /c/.match?("abc", -1)
p [0x20, 0x85, 0xa0, 0x3000, 0x200b].map { /\A[[:space:]]*\z/.match?(_1.chr(Encoding::UTF_8)) }, 0xe9.chr.encoding

# gsub leaves $~ at the last match; a String pattern expands \0 and \\ in the replacement
p "abab".gsub(/b/, "x"), $~.begin(0), "abc".gsub("b", '<\0>'), "a b".gsub(" ", " " => "%20")
