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
p [Fcntl::F_GETFL, Fcntl::F_SETFL, Fcntl::O_NONBLOCK == File::NONBLOCK, Fcntl::FD_CLOEXEC, Fcntl::O_RDWR]
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
# (the errno numbers differ between platforms: ECONNREFUSED is 61 on macOS, 111 on Linux)
refused = Errno::ECONNREFUSED::Errno
p SystemCallError.new(nil, refused).class, SystemCallError.new("x", refused).class, SystemCallError.new("x", refused).message, SystemCallError.new(nil, refused).message, SystemCallError.new(refused).class, SystemCallError.new("x").class, SystemCallError.new("x").errno, SystemCallError.new(nil, refused).errno == refused, SystemCallError.new(nil, 99999).class, SystemCallError.new(nil, 99999).message.start_with?("Unknown error")
p SystemCallError.new("x", refused, "func").message rescue p $!
p SystemCallError === Errno::ENOENT.new, Errno::ENOENT.new.message, Errno::ENOENT.new("a").message, Errno::ENOENT.new("a", "b").message, Errno::ENOENT::Errno

# MatchData's group accessors
md = /(?<a>h.)(x)?(é+)/.match("xxhé-héé!")
p md.match(0), md.match(:a), md.match("a"), md.match(1)
p md.match_length(0), md.match_length(:a), md.match_length(1)
p md.byteoffset(0), md.byteoffset(:a), md.byteoffset(1)
p md.bytebegin(0), md.byteend(0), md.bytebegin(:a), md.byteend("a"), md.bytebegin(1), md.byteend(1)
[[:match, 9], [:match_length, -1], [:byteoffset, :nope], [:bytebegin, "nope"], [:match, 1.5]].each do |m, k|
  begin
    md.send(m, k)
  rescue IndexError, TypeError => e
    puts "#{e.class}: #{e.message}"
  end
end

# Random's formatter methods on the class; rand over a range of objects that answer to_int
# only through respond_to_missing? (ActiveSupport::Duration); timezone objects in Time.new / .at
require "securerandom"
p Random.hex.size, Random.hex(4).size, Random.alphanumeric(5).size, Random.respond_to?(:uuid)
class LazyDur
  attr_reader :v
  def initialize(v) = @v = v
  def -(o) = LazyDur.new(v - (o.is_a?(LazyDur) ? o.v : o))
  def +(o) = LazyDur.new(v + (o.is_a?(LazyDur) ? o.v : o))
  def <=>(o) = v <=> (o.is_a?(LazyDur) ? o.v : o)
  include Comparable
  def respond_to_missing?(n, p = false) = n == :to_int || super
  def method_missing(n, *a) = n == :to_int ? v : super
end
r = rand(LazyDur.new(30)..LazyDur.new(600))
p r.class, (30..600).cover?(r.v)
class FixedZone
  def utc_to_local(t) = t + 3600
  def local_to_utc(t) = t - 3600
  def name = "Z1"
end
t = Time.new(2026, 6, 1, 12, 30, 0, FixedZone.new)
p t.utc_offset, t.hour, t.min, t.utc.hour, t.zone.class
p Time.at(0, in: FixedZone.new).hour, Time.at(0, in: FixedZone.new).utc_offset, Time.now(in: FixedZone.new).utc_offset

# clone keeps the singleton class, with the modules included into it (RSpec clones its DSL
# matchers when they are composed: `include("a" => a_matcher)`)
o = Object.new
class << o
  include(Module.new { def from_mod = :mod })
  def a = :a
end
c = o.clone
p c.from_mod, c.a, c.singleton_methods.sort, o.clone(freeze: false).from_mod, o.dup.respond_to?(:a)
def c.only_c = 1
p o.respond_to?(:only_c)
class SmA; def self.x = 1; class << self; private def hid = 1; protected def pro = 1; end; end
class SmB < SmA; def self.y = 1; end
p SmB.singleton_methods.sort, SmB.singleton_methods(false), String.singleton_methods.sort

# String.new: BINARY without a String, `encoding:` wins (Mastodon's Request#truncated_body)
c = String.new(encoding: Encoding::BINARY)
p c.encoding, (c << "abc".dup).encoding
d = String.new(encoding: Encoding::BINARY); d << "é"
p d.encoding
p String.new.encoding, String.new("é").encoding, String.new("é", encoding: "BINARY").encoding, String.new(capacity: 9).encoding
k = Class.new(String); p k.new.encoding, k.new("x").encoding, k.new(encoding: "US-ASCII").encoding
h = String.new; h << "é"; p h.encoding
j = String.new; j << "\xff".b; p((j + "é" rescue $!.class))
p((String.new(encoding: "nope") rescue $!.message))
x = String.new; x << 233; p x.encoding, x.bytes

# an object made before its class had these ivars: setting a later one leaves the ones in
# between unset (ActiveRecord's `@inverse_name ||`-style memo read as computed nil, and a
# has_many :through join record lost its owner: "Collection reports is invalid")
class EarlyIv
  def initialize = (@a = 1)
  def memo = (defined?(@m) ? [:memo, @m] : (@m = :computed))
  def late = (@late ||= 2)
end
early = EarlyIv.new
other = EarlyIv.new
other.memo
other.late
early.late
p early.instance_variable_defined?(:@m), early.instance_variables, early.memo
early2 = EarlyIv.new.tap { |e| e.instance_variable_set(:@a, 0) }
early2.instance_variable_set(:@late, 3)
p early2.instance_variables, early2.memo

# Time#== goes through a redefined <=> (ActiveSupport compares a Time with a String or a
# DateTime that way: Mastodon's metrics specs)
class CmpTime < Time
  def <=>(o) = o.is_a?(String) ? (to_i.to_s <=> o) : super
end
ct = CmpTime.at(5)
p ct == "5", ct == "6", ct == CmpTime.at(5), ct == Time.at(5), ct == Object.new, Time.at(5) == Time.at(5)

# \w \d \s are ASCII in Ruby, \b and the POSIX brackets Unicode (Sanitize drops a link whose
# href starts with a no-break space: Mastodon's sanitize config spec)
p "é" =~ /\w/, "é" =~ /\W/, "aé_1".scan(/\w+/), "ab-c" =~ /\A[\w-]+\z/, "é" =~ /[^\w]/, "é" =~ /[[:word:]]/
p "\\w" =~ /\\w/, /\w+/.source, /[\w.]+@[\w.]+/.match("bob.x@ex.com")[0], "A_z" =~ /\A[\w&&[^_]]+\z/
p " " =~ /\s/, "٣" =~ /\d/, " " =~ /[[:space:]]/, "٣" =~ /[[:digit:]]/, "a b".split(/\s/)
p "über" =~ /\bber/, "Straße".gsub(/\w/, "*"), " https://x" =~ /\A\s*([^\/#]*?):/ && $1
p "a b\tc" =~ /a\sb\sc/x, "a b" =~ /a [\s] b/x, "1٣" =~ /\A\d\D\z/, "é" =~ /[\s\d\w]/

# Fractional seconds survive Time.gm/new with Rational seconds, the usec argument and
# Time <-> DateTime, which keeps its offset (Mastodon: scheduled_at round-trips)
require "date"
p Time.utc(2026, 1, 1, 0, 0, 22, 5).usec, Time.utc(2026, 1, 1, 0, 0, 22, Rational(11, 2)).nsec, Time.gm(2026, 1, 1, 0, 0, Rational(45, 2)).nsec
p Time.utc(2026, 1, 1, 0, 0, 22.5).nsec, Time.new(2026, 1, 1, 0, 0, Rational(22740494506, 1000000000), "+00:00").nsec, Time.local(2026, 1, 1, 0, 0, 1, 250).usec
p Time.new(2026, 1, 1, 0, 0, 22.25, "+02:00").subsec, Time.utc(2026, 1, 1, 0, 0, 22, 740494.506r).nsec
t = Time.at(1_800_000_000, 740494506, :nsec).utc
p t.to_datetime.sec_fraction, t.to_datetime.to_time.nsec, DateTime.new(2026, 11, 3, 19, 11, Rational(45, 2)).to_time.nsec
p DateTime.parse("2026-11-03T19:11:22.740494Z").to_time.usec, DateTime.parse("2026-11-03T19:11:22.740494Z").sec_fraction
p Time.at(Rational(3, 2)).nsec, Time.at(1, 500, :usec).nsec, t.to_date.to_s
dt = DateTime.new(2026, 1, 1, 10, 0, Rational(1, 3), "+05:00")
p dt.to_time.utc_offset, dt.to_time.hour, DateTime.new(2026, 1, 1).to_time.utc_offset, Date.new(2026, 1, 1).to_time.hour

# A binary `String.new` buffer takes the encoding of what `replace`, `prepend` and `insert`
# put in it, as with `<<`
b = String.new; b.replace("é"); p b.encoding, b == "é"
b = String.new; b.prepend("é"); p b.encoding
b = String.new; b.insert(0, "é"); p b.encoding
b = String.new("ab"); b.insert(1, "é"); p b, b.encoding
b = "x".b; b.replace("ab"); p b.encoding

# `public :m` of an inherited method that is already public does nothing (rspec-mocks'
# reset of `allow(K).to receive(:new)` does that to K's singleton class)
class VisBase; def call(a, b) = :base; def pv = 1; private :pv; end
class VisSub < VisBase; end
VisBase.singleton_class.__send__(:public, :new)
p VisBase.singleton_class.instance_methods(false), VisSub.method(:new).owner
VisSub.send(:public, :call, :to_s); p VisSub.instance_methods(false)
VisSub.send(:public, :pv); p VisSub.instance_methods(false), VisSub.instance_method(:pv).owner, VisSub.new.pv

# IO::TimeoutError (Ruby 3.2+; Discourse rescues it)
p IO::TimeoutError.superclass, IO::TimeoutError.name, (raise IO::TimeoutError, "late" rescue $!.message)

# a push waiting for room in a SizedQueue raises ClosedQueueError when the queue is closed
sq = Thread::SizedQueue.new(1); sq << 1
pusher = Thread.new { sq.push(2) rescue $!.class }
sleep 0.1 until pusher.status == "sleep"
sq.close
p pusher.join(2)&.value, sq.pop, sq.pop

# SIGPIPE is ignored: writing to a closed pipe raises Errno::EPIPE (ActiveSupport's tests)
pr, pw = IO.pipe
pr.close
p((pw.write("x") rescue $!.class))
p IO.popen(["sh", "-c", "yes | head -c 3"], &:read)

# YAML: an !omap loads as a Psych::Omap (ActiveSupport::OrderedHash dumps as one); a quoted
# scalar's continuation lines may be indented as little as its key, as libyaml allows
# (Discourse's locale files)
require "yaml"
p YAML.unsafe_load("--- !omap\n- :a: 1\n- :b: [2]\n").then { [_1, _1.class] }
p YAML.load("a:\n  b: \"x\n  y\"\n  c: 'p\nq'\n  d: \"m\\\n  n\"\n"), Psych.parse("a:\n  b: \"x\n  y\"\n").to_ruby

# gsub! without a block or replacement is an Enumerator; Hash[[[k]]] is k => nil; flatten
# converts elements with to_ary (ActiveSupport's Inflections::Uncountables)
s = +"ab"; p s.gsub!(/a/).to_a, s
p Hash[[[1, 2], [3]]], (Hash[[[1, 2, 3]]] rescue $!), (Hash[[1]] rescue $!)
class FlatAry; def initialize(m) = @m = m; def to_ary = @m; end
p [FlatAry.new(%w[a b]), "c"].flatten, [[FlatAry.new([1, [2]])]].flatten(1).size, ([FlatAry.new(1)].flatten rescue $!)

# local times: an ambiguous one (clocks going back) is the later instant unless the
# ten-argument form's isdst says otherwise, a skipped one is moved forward; Date#to_time of a
# Julian date is that day
old_tz = ENV["TZ"]
ENV["TZ"] = "US/Eastern"
p Time.local(2005, 10, 30, 1, 0, 0), Time.local(2005, 4, 3, 2, 30), Time.local(1950, 6, 1)
p Time.local(0, 30, 1, 30, 10, 2005, 0, 0, true, nil), Time.local(0, 30, 1, 30, 10, 2005, 0, 0, false, nil)
require "date"
p Date.new(2005, 10, 30).to_time, Date.new(1582, 10, 4).to_time.to_date == Date.new(1582, 10, 4)
ENV["TZ"] = "Europe/Helsinki"
p Time.local(2024, 10, 27, 3, 30), Time.local(2024, 3, 31, 3, 30), Time.new(2024, 10, 27, 3, 30, 0)
ENV["TZ"] = old_tz

# DateTime#== goes through <=> (ActiveSupport compares a DateTime with a Time that way)
class DateTime; def <=>(o) = o.is_a?(Time) ? super(o.to_datetime) : super; end
p DateTime.new(2038, 1, 20, 11, 59, 59) == Time.utc(2038, 1, 20, 11, 59, 59), DateTime.new(2000, 1, 1) == Date.new(2000, 1, 1)

# Hash: default(key) runs the default proc; count/entries (Enumerable's) go through a
# redefined each (ActiveSupport's InheritableOptions); transform_keys(non-Hash) is a TypeError
p Hash.new { |_, k| k.to_s * 2 }.default(1), Hash.new(5).default(:x)
class EachH < Hash; def each(&b) = { a: 1, b: 2, c: 3 }.each(&b); end
eh = EachH.new; eh[:z] = 9
p eh.count, eh.entries, eh.to_a, eh.map { _1 }, eh.size
p((Hash[a: 1].transform_keys(nil) rescue $!), (Hash[a: 1].transform_keys!(nil) rescue $!), Hash[a: 1].transform_keys({ a: :b }))
# Date.new takes at most four arguments; Date arithmetic with a Rational of Bignums
p((Date.new(2013, 11, 12, 2, 11) rescue $!.class), (DateTime.new(2000, 1, 1) + Rational(0.1, 86400)).strftime("%T.%N"))
# pp prints a Hash as Ruby 3.4 inspects it
require "pp"
PP.pp({ one: "x", "two" => 2, "a b": 3, :+ => 4 }, $stdout, 20)

# Range#cover?(range) by an endless range; Time field range errors as CRuby's
p((1..).cover?(2..4), (1..).cover?(1..), (1...).cover?(1..), (1..).cover?(1...), (1.0..).cover?(2..3), (2..4).cover?(1..))
p [[2000, 13, 1], [2000, 90, 1], [2000, 12, 32], [2000, 1, 1, 25], [2000, 1, 1, 0, 64]].map { |a| (Time.utc(*a) rescue $!.message) }
# a Time in a timezone object: %Z is its abbr, and the object stays through +
class FixedZone
  def utc_to_local(t) = Time.at(t.to_i + 3600, in: "UTC").then { Time.utc(_1.year, _1.month, _1.day, _1.hour, _1.min, _1.sec) }
  def local_to_utc(t) = Time.utc(t.year, t.month, t.day, t.hour, t.min, t.sec) - 3600
  def abbr(_t) = "FXT"
  def name = "Fixed/Zone"
end
fz = FixedZone.new
tz = Time.new(2000, 1, 1, 1, 0, 0, fz)
p tz.strftime("%Z %z"), tz.zone.equal?(fz), (tz + 60).zone.equal?(fz), (tz + 60).strftime("%H:%M %Z")
