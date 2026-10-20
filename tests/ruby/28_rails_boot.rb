# Runtime behaviours a whole Rails 5.2 app (railties boot) depends on.
# kenny-run-flags: -I tests/ruby/28_rails_boot

# symbols with NUL bytes don't shift the compiled-in symbol table
p [:"a\0b", :after_nul, :"\0"].map(&:size)

# backtraces: caller / caller_locations / Exception#backtrace
def inner = caller_locations(0).map { |l| [File.basename(l.path), l.lineno, l.label] }
def outer = [1].map { inner }.first
p outer.first(3)
def boom = raise("x")
begin
  boom
rescue => e
  p e.backtrace.first(2).map { |l| l.sub(/\A.*\//, "") }
end
class Base0
  def self.inherited(k) = (@from = caller_locations.map(&:path).uniq.size)
  def self.from = @from
end
class Sub0 < Base0; end
p Base0.from >= 1

# autoload in a class wins over a constant inherited from its superclass
class Railtie
  class Configuration
    def initialize = (@r = :railtie)
  end
end
class Engine < Railtie
  autoload :Configuration, "engine_configuration"
  def self.config = Engine::Configuration.new(:engine_root)
end
p Engine.config.root

# an autoload that defines the constant in a nested scope still resolves (ActionCable)
module Server
  autoload :Mgmt, "server_worker_mgmt"
  class Worker
    p Mgmt.hi
  end
end

# lexical scope: `module A::B` does not search A
module Rails0; def self.env = "rails0"; end
module Sass0; module Rails0; end; end
module Sass0::Rails0
  class Railtie
    def self.env = Rails0.env
  end
end
p Sass0::Rails0::Railtie.env rescue p $!.class
module Outer0
  X0 = :outer
  module Inner0; end
end
module Outer0::Inner0
  def self.x = defined?(X0)
end
p Outer0::Inner0.x

# Mod::Name doesn't fall back to Object's constants
module AttrAcc
  def self.struct
    const_set(:Struct, Struct.new(:a)) unless defined?(self::Struct)
    self::Struct
  end
end
p AttrAcc.struct.name, defined?(AttrAcc::String)

# a `def` in a class_eval string is public even after a bare `private`
class Vis
  private
  class_eval "def from_eval = 1"
  def priv = 2
end
p Vis.public_method_defined?(:from_eval), Vis.private_method_defined?(:priv)

# dup / clone call initialize_dup / initialize_clone (Set#| on a frozen set)
require "set"
p((Set[1, 2].freeze | [3]).to_a)
class Copy
  def initialize_dup(o) = (@how = :dup)
  def initialize_clone(o, freeze: nil) = (@how = :clone)
  attr_reader :how
end
p Copy.new.dup.how, Copy.new.clone.how

# Array subclass#to_a is a plain Array (Rails::Initializable::Collection#+)
class Coll < Array
  def +(other) = Coll.new(to_a + other.to_a)
end
p (Coll.new([1]) + Coll.new([2])).class

# defined?(super) in a module extended onto an object
module Flush
  def flush = (defined?(super) ? super : :no_super)
end
o = Object.new.extend(Flush)
p o.flush

# ::Kernel.block_given? in a BasicObject subclass (Jbuilder)
class BO < BasicObject
  def blk? = ::Kernel.block_given?
end
p BO.new.blk? {}, BO.new.blk?

# String#to_r reads the longest rational prefix
p "0.4.25".to_r, "1/3x".to_r, "abc".to_r

# /n regexps with byte escapes, and `#$+` interpolation in a regexp literal
p(/[\xC2-\xDF][\x80-\xBF]/n.source)
p(/[^ A-Za-z0-9!#$+.^_`|~-]/.source)

# implicit to_hash conversion
class Opts0; def to_hash = { a: 1 }; end
p({ b: 2 }.merge!(Opts0.new))

# Fiber.current, exception classes, RbConfig::MAKEFILE_CONFIG
p Fiber.current.equal?(Fiber.current), Fiber.current.alive?
p Interrupt.superclass, NoMemoryError.superclass, NoMatchingPatternError.superclass
p RbConfig::MAKEFILE_CONFIG["DLEXT"].is_a?(String)

# Object#eql? called through super is identity, not a re-dispatch (Mime::Type#eql?)
class MimeT
  def initialize(s) = (@s = s)
  attr_reader :s
  def eql?(o) = super || (o.class == self.class && o.s == @s)
end
p MimeT.new("a").eql?(MimeT.new("a")), MimeT.new("a").eql?(MimeT.new("b"))
p({ MimeT.new("x") => 1 }.size)
