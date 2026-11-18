# eval of a string runs in the class body its call site is in (CRuby: the caller's cref):
# `class X` in it defines X there, and Module.nesting and constants see that scope, also
# from a proc run with instance_exec (ActiveSupport's DescendantsTrackerTest)
class EvT
  BLK = proc { eval "class Inner1; end; Module.nesting" }
  def run = instance_exec(&BLK)
  def run2 = BLK.call
  def m = eval("class Inner3; end; Module.nesting")
  def self.blk = proc { eval "class Inner4; end; Module.nesting" }
end
p EvT.new.run, EvT.new.run2, EvT.new.m, EvT.blk.call
p EvT.new.instance_eval { eval "Module.nesting" }
p EvT.constants.sort, Object.const_defined?(:Inner1)
module Outer
  K = 1
  class In
    def m = eval("[K, Module.nesting, (class Deep; self; end)]")
    def n = eval("def evdef = Module.nesting; evdef")
  end
end
p Outer::In.new.m, Outer::In.new.n, Outer::In.new.m
p eval("Module.nesting"), (eval "class TopE; end; TopE.name")
