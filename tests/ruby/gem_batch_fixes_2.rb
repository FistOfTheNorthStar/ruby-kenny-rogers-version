# Kenny bugs found running gems' test suites (slim, liquid, dry-*, sorbet-runtime, hashie,
# prawn, Zeitwerk), second batch.

# $~ / $1 are local to a method frame: a match in a called method leaves the caller's alone
def split_ext(s) = s =~ /\.(\w+)\z/ ? $1 : nil
def failing(s) = s =~ /nomatch/
def subbing(s) = s.sub(/b/, "c")
if "app.js:rest" =~ /(\w+)\.(\w+):(\w+)/
  split_ext("x.rb")
  failing("y")
  subbing("abc")
  p [$1, $2, $3], $~[0], $'
end
def fresh = $~
"q" =~ /q/
p fresh, $~[0]
# blocks share their method's
def in_block
  [1].each { "zz" =~ /z/ }
  $~ && $~[0]
end
p in_block
# split, partition and rpartition set $~ too (split leaves it nil)
"a,b" =~ /a/
"a,b".split(/,/)
p $~
"a-b" =~ /zzz/
"a-b".partition(/-/)
p $~[0]
"a-b-c".rpartition(/-/)
p $~.pre_match

# a bare rescue (and the rescue modifier) is ::StandardError, not a lexical one
module Liquidish
  class StandardError < ::StandardError; end
  def self.go
    begin
      raise ArgumentError, "bare"
    rescue => e
      e.message
    end
  end
  def self.mod = (raise TypeError, "mod" rescue $!.message)
end
p Liquidish.go, Liquidish.mod

# Integer#[] with to_int indexes, [offset, len] and [range]
p 6[1.5], 6[1, 2], 6[0..1], 6[1..], 6[2..1], 6[1...1], -6[1..], 6[-3, 2], 6[1, -1], (2**80 + 5)[0..1]
[["x"], [:x], [nil]].each do |a|
  6[*a]
rescue TypeError => e
  p e.message
end
begin
  6[..2]
rescue ArgumentError => e
  p e.message
end
o = Object.new
def o.to_int = 1
p 6[o], 6[...1]

# Kernel#respond_to_missing? (private, Kernel's)
m = Kernel.instance_method(:respond_to_missing?)
p m.name, m.owner, Kernel.private_instance_methods(false).include?(:respond_to_missing?)
class Bo < BasicObject
  define_method(:respond_to_missing?, ::Kernel.instance_method(:respond_to_missing?))
end
p Bo.instance_method(:respond_to_missing?).owner

# $LOADED_FEATURES lists a library once it's loaded (a C extension's as such)
lib = "stringio"
before = $LOADED_FEATURES.size
p require(lib), $LOADED_FEATURES.size - before, File.basename($LOADED_FEATURES.last.to_s).sub(/\.so\z/, ".bundle")

# a string module_eval'd on an anonymous module sees the caller's constants
module Outer
  Result = :outer_result
  class Builder < ::Module
    def initialize
      super()
      module_eval("def call = Result")
    end
  end
  M = Module.new.tap { |mm| mm.module_eval("def call3 = Result") }
end
p Object.new.extend(Outer::Builder.new).call, Object.new.extend(Outer::M).call3

# Hash subclasses: merge keeps the class; [] on a missing key calls #default(key)
class Opts < Hash; end
oo = Opts.new
oo[:a] = 1
p oo.merge(b: 2).class, oo.select { true }.class, oo.to_h.class
class IH < Hash
  def default(key = nil) = key.is_a?(Symbol) ? self[key.to_s] : "dflt(#{key.inspect})"
end
h = IH.new
h["foo"] = 1
p h[:foo], h[:bar], h.dig(:foo), h.fetch("foo")

# Module#const_source_location
module T
  module Private; end
  X = 2
end
CLine = __LINE__ - 3
p T.const_source_location(:Private).then { [File.basename(_1), _2 - CLine] }
p Object.const_source_location("T::X")[1] - CLine, T.const_source_location(:Nope), Object.const_source_location(:String)

# __method__ / __callee__ in define_method bodies, and Kernel.__callee__
class Cm
  define_method(:orig) { [__callee__, __method__] }
  alias_method :aka, :orig
  define_method(:nested) { [1].map { __method__ } }
  def k = [Kernel.__callee__, Kernel.__method__]
end
p Cm.new.orig, Cm.new.aka, Cm.new.nested, Cm.new.k

# Windows-1252 (and other single-byte) strings keep their encoding
s = "Hello, café\nbye".encode("windows-1252")
p s.scan(/[^\n]+|\n/).map(&:encoding).uniq, s[0, 5].encoding, s.split(",").map(&:encoding).uniq
p s.lines.map(&:encoding).uniq, s.match(/caf./)[0].encoding, s.chars.first.encoding, s.byteslice(0, 2).encoding
shy = "­".encode("windows-1252")
i = "[^#{shy}]+"
p i.encoding, i.valid_encoding?, [shy, "b"].join.encoding, i.encode("windows-1252").bytesize
b = " caf\xE9 ".b
p [b.upcase, b.strip, b.reverse, b * 2, b.tr("c", "C"), b.split("")[-2]].map { [_1.bytes.last(2), _1.encoding] }
