# `def` inside a block defines into the block's own scope (CRuby's cref), also when the block
# is called from inside another object's instance_exec / class_eval / class body (Rails'
# after_initialize hooks, Discourse's config/initializers/101-lograge.rb)
$hooks = []
def on_load(&blk) = $hooks << blk

on_load do
  def top_helper(x) = "top_helper #{x}"
  [1].each { |s| puts top_helper(s) }
end

class App; end
class Runner
  def run = App.new.instance_exec { $hooks.each { |b| b.call(self) } }
  def run_in_class_eval = App.class_eval { $hooks.each { |b| b.call(self) } }
end
Runner.new.run
p [Object.private_method_defined?(:top_helper), App.public_method_defined?(:top_helper)]
Runner.new.run_in_class_eval
p App.instance_methods(false)

class Host
  $hooks.each(&:call)
  p instance_methods(false)
end

# instance_exec / class_eval / class bodies still define into their own target
o = Object.new
o.instance_exec { def single = :single }
p [o.single, o.singleton_methods]
App.class_eval { [1].each { def from_class_eval = :ce } }
p App.new.from_class_eval
m = Module.new { def from_module_new = :mn }
p m.instance_methods
class Host
  [1].each { def in_class_block = :icb }
end
p Host.new.in_class_block

# def inside a method: defines into the method's class
class Maker
  def self.make = (def made = :made)
end
class Other
  Maker.make
  p instance_methods(false)
end
p Maker.public_method_defined?(:made)

# alias in a hook block run from instance_exec
on_load { alias top_alias top_helper }
Runner.new.run
p [Object.private_method_defined?(:top_alias), App.method_defined?(:top_alias)]
