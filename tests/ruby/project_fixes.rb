# Fixes from running Rails, Mastodon and Discourse under Kenny

# super through a module that appears twice in the ancestors
module Gen; def hi(_ = 0) = "Gen#hi"; end
class Object; include Gen; end
module Fwd
  def hi(depth = 0)
    return "loop!" if depth > 5
    "Fwd(" + super(depth + 1) + ")"
  end
end
[Enumerable, Object, Hash].reverse_each { _1.include(Fwd) }
puts({}.hi)

# protected methods
class PA
  def initialize(v) = @v = v
  def ==(o) = o.is_a?(PA) && v == o.v
  def <=>(o) = v <=> o.v
  def each_cmp(xs) = xs.map { |x| x.v }
  protected def v = @v
  class << self; protected def cp = :cp; end
end
class PB < PA; def peek(o) = o.v; end
class PC; def peek(a) = a.v; end
p PA.new(1) == PA.new(1), [PA.new(2), PA.new(1)].sort.map { |x| x.instance_variable_get(:@v) }, PA.new(1).each_cmp([PA.new(5)])
p PB.new(1).peek(PA.new(9)), PA.new(1).send(:v), PA.new(2).respond_to?(:v)
begin; PA.new(1).v; rescue NoMethodError => e; p e.message; end
begin; PC.new.peek(PA.new(1)); rescue NoMethodError => e; p e.message; end
begin; PA.cp; rescue NoMethodError => e; p e.message; end
[1].each { begin; PA.new(1).v; rescue NoMethodError => e; p e.message; end }
class PMM
  protected def secret = 1
  def method_missing(n, *a) = n == :secret ? super : :mm
  def respond_to_missing?(*) = true
end
begin; PMM.new.secret; rescue NoMethodError => e; p e.message; end

# remove_const of a pending autoload
module QueueAdapters
  autoload :SidekiqAdapter, "/nonexistent/sidekiq_adapter"
  p const_defined?(:SidekiqAdapter), remove_const(:SidekiqAdapter), const_defined?(:SidekiqAdapter), autoload?(:SidekiqAdapter)
end

# fcntl, ruby2_keywords, features listed by hand
require "fcntl"
p [Fcntl::F_GETFL, Fcntl::F_SETFL, Fcntl::O_NONBLOCK, Fcntl::FD_CLOEXEC, Fcntl::O_RDWR]
p require("ruby2_keywords")
$LOADED_FEATURES << "kenny_never_compiled.rb"
p require("kenny_never_compiled")

# interpolating a binary String
s = "\xc6\x2c\x12\xa3\xc3\xa9".b
t = "#{s}X"
p [t.encoding, t.bytesize, t.length]
u = "pre-#{s}"
p [u.encoding, u.length]
begin; "é#{"\xff".b}"; rescue => e; p e.class; end

# Regexp timeouts
p Regexp.timeout
begin; Regexp.timeout = -1; rescue => e; p e; end
Regexp.timeout = 1
p Regexp.timeout
Regexp.timeout = nil
p Regexp.new("a", timeout: nil).timeout, /a/.timeout, Regexp.new("a", timeout: 3).timeout
p Regexp.new("a", timeout: 1).casefold?, Regexp.new("a", timeout: 1).instance_variables
p Regexp::TimeoutError.ancestors.take(3)

# Range/Array subclasses, String == with to_str
class MyRange < Range; end
r = MyRange.new(1, 3)
p r.class, r.to_a, r.frozen?
begin; Range.new(1, "a"); rescue ArgumentError => e; p e.message; end
class MyArr < Array; def initialize(a, b, c) = super([a, b, c]); end
p MyArr.new(1, 2, 3)
begin; Array.new(1, 2, 3); rescue ArgumentError => e; p e.message; end
class StrLike; def to_str = "x"; def ==(o) = o == "x"; end
p "x" == StrLike.new, "x" === StrLike.new, "y" == StrLike.new

# Module.nesting
module NA; module NB; p Module.nesting; end; p Module.nesting; class << self; p Module.nesting; end; end
module NA::NB; p Module.nesting; [1].each { p Module.nesting }; def self.f = Module.nesting; end
p NA::NB.f, Module.nesting

# initialize returns self
class Cleaners < Hash
  def initialize(hash = {}) = super.replace(hash)
end
p Cleaners.new(a: 1), {}.send(:initialize).class, [].send(:initialize).class, "".send(:initialize).class

# grapheme cluster enumerator size
["\u{1F3F3}\u{FE0F}\u{200D}\u{26A7}\u{FE0F}", "❤️", "é"].each { |s| p [s.each_grapheme_cluster.size, s.each_grapheme_cluster.to_a.size] }

# runaway recursion fails fast
def runaway(a) = [a].map { |x| runaway(x) }
t = Time.now
begin; runaway(1); rescue SystemStackError; p :stack_error; end
p Time.now - t < 5

# `module A::B` doesn't have A in scope: the top-level autoload, not A's
d = __dir__
Object.autoload(:Trx, "#{d}/project_fixes/top_trx.rb")
module Adm; end
Adm.autoload(:Trx, "#{d}/project_fixes/adm_trx.rb")
module Adm::FH
  p Trx
end
module Adm
  p Trx
end

# frozen modules
m = Module.new.freeze
p m.frozen?, m.dup.equal?(m), m.dup.frozen?, Class.new.freeze.frozen?
begin; m.const_set(:X, 1); rescue => e; p e.class; end
begin; def m.x; end; rescue => e; p e.class; end
o = Object.new.freeze
p o.singleton_class.frozen?

# __callee__ in a block that becomes a method through &blk
class CK
  def self.repl(name, &blk) = define_method(name, &blk)
  def self.outer
    repl(:foo) { Kernel.__callee__ }
    repl(:bar) { __method__ }
  end
end
CK.outer
p CK.new.foo, CK.new.bar

# writing text into a binary StringIO
require "stringio"
b = StringIO.new.binmode
b << "abc" << "\xff\xfe".b
b.write("é")
p b.string.encoding, b.string.bytesize
