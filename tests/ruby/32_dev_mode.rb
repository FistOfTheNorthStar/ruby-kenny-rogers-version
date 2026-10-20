# What Rails' development mode and Action Cable rely on: `load` of a file that changed (or is
# new) runs the new source, const_defined? semantics, global variable aliases (English.rb),
# bare super with anonymous params inside a block, super falling through to method_missing,
# include/prepend order, recursive inspect, String subclasses, Struct.new(nil), IO.select.
require "tmpdir"

# --- load / require of changed and new files
dir = Dir.mktmpdir("kenny-load")
lib = File.join(dir, "reloaded.rb")
File.write(lib, "class Reloaded; def v = 1; end\n")
load lib
p Reloaded.new.v
File.write(lib, "class Reloaded; def v = 2; end\nrequire_relative 'helper_new'\n")
File.write(File.join(dir, "helper_new.rb"), "module HelperNew; def self.where = File.basename(__FILE__); end\n")
load lib
p Reloaded.new.v, HelperNew.where
Object.send(:remove_const, :Reloaded)
load lib
p Reloaded.new.v
p require(File.join(dir, "helper_new")) # already loaded by require_relative
File.write(lib, "class Reloaded; def v = 3; end; end\n")
begin
  load lib
rescue SyntaxError
  p :syntax_error
end

# --- const_defined? never calls const_missing; inherit flag; autoload counts
class Module
  alias __kn_cm const_missing
  def const_missing(n) = raise("const_missing #{n}")
end
module Mixin; FROM_MIXIN = 1; end
class Base; include Mixin; OWN = 2; end
class Sub < Base; end
p Object.const_defined?(:String), Object.const_defined?(:NoSuchThing)
p Sub.const_defined?(:OWN), Sub.const_defined?(:OWN, false), Sub.const_defined?(:FROM_MIXIN)
p Mixin.const_defined?(:String), Mixin.const_defined?(:String, false), Object.const_defined?("Base::OWN")
autoload :LaterLoaded, File.join(dir, "later_loaded")
p Object.const_defined?(:LaterLoaded)
class Module
  alias const_missing __kn_cm
end

# --- English.rb style global aliases
alias $MY_MATCH $&
alias $MY_ERROR $!
"hello world" =~ /o w/
p $MY_MATCH
begin
  raise "boom"
rescue => e
  p $MY_ERROR.message
end
alias $new_name $old_name
$old_name = 5
$new_name += 1
p $old_name

# --- bare super inside a block with anonymous * and **
class Renderer
  def render(*a, **kw, &b) = [a, kw, b&.call]
end
class Quiet < Renderer
  def silence = yield
  def render(*, **)
    silence { super }
  end
end
p Quiet.new.render(1, 2, layout: "x")

# --- super with no superclass method goes to method_missing
require "delegate"
class Target; def path = "/x"; end
class Wrapper < SimpleDelegator
  def path = super + "!"
end
p Wrapper.new(Target.new).path
class Lonely; def foo = super; end
begin
  Lonely.new.foo
rescue NoMethodError => e
  p e.message
end

# --- include skips modules the superclass already has; prepend doesn't
module Logs; end
module Service; include Logs; def post = :abstract; end
module Deprecates; include Logs; end
class AbstractService; include Service; include Deprecates; end
class RealService < AbstractService; def post = :real; end
module Serial; include Service; end
class SerialService < RealService; include Serial; end
p AbstractService.ancestors.take(4), SerialService.new.post
module Enc; end
class Object; prepend Enc; end
class String; prepend Enc; end
p String.ancestors.take(2)

# --- inspect of objects that contain themselves
class Node
  def initialize = (@me = self; @list = [self])
end
p Node.new.inspect.gsub(/0x\h+/, "X")

# --- String subclasses
class Inquirer < String; end
s = Inquirer.new("development")
p s.to_s.class, s.to_str.class, String.new("\xff".b).encoding

# --- Struct.new(nil)
Empty = Struct.new(nil)
p Empty.new, Struct.new(nil, :a).new(1).a

# --- threads referencing a constant another thread is autoloading wait for the whole file
autoload :SlowClass, "32_dev_mode/slow_class"
p 4.times.map { Thread.new { SlowClass.ready } }.map(&:value)

# --- IO.select: timeouts, ready pipes, a reader thread next to a sleeping one
r, w = IO.pipe
p IO.select([r], nil, nil, 0.05)
ticks = []
sleeper = Thread.new { 3.times { sleep 0.02; ticks << 1 } }
reader = Thread.new { IO.select([r]); r.gets }
sleep 0.2
w.puts "late"
p reader.value, ticks.size
sleeper.join
