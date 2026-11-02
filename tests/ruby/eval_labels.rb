# Code eval'd from a string takes the label of the frame it is evaluated in (pry's
# backtraces: "from (pry):7:in '<main>'"), and its blocks are "block in" that frame.
class Foo
  def bar = eval("caller_locations(0, 1)[0].label")
  def baz = eval("[1].map { caller_locations(0, 1)[0].label }.first")
  def deep = eval("[1].map { [2].map { caller_locations(0, 1)[0].label }.first }.first")
  def qux = instance_eval("caller_locations(0, 1)[0].label")
  def blk
    [1].each { return eval("caller_locations(0, 1)[0].label") }
  end
  def nested = eval("eval('caller_locations(0, 1)[0].label')")
  def self.cm = class_eval("caller_locations(0, 1)[0].label")
  def bnd = binding
end
f = Foo.new
p f.bar, f.baz, f.deep, f.qux, f.blk, f.nested, Foo.cm
p eval("caller_locations(0, 1)[0].label")
p Foo.class_eval("caller_locations(0, 1)[0].label")
# through a Binding: where the binding was made
p f.bnd.eval("caller_locations(0, 1)[0].label")
p TOPLEVEL_BINDING.eval("caller_locations(0, 1)[0].label")
def run_in(b, src) = b.eval(src)
p run_in(TOPLEVEL_BINDING, "caller_locations(0, 1)[0].label")
p run_in(f.bnd, "[1].map { caller_locations(0, 1)[0].label }.first")
p eval("caller_locations(0, 1)[0].label", f.bnd)
# in an exception's backtrace
def raiser
  TOPLEVEL_BINDING.eval("raise 'x'", "(pry)", 7)
rescue => e
  e.backtrace.first
end
p raiser
def raiser2 = eval("1.then { raise 'y' }") rescue $!.backtrace_locations.map(&:label).grep(/raiser2/).first
p raiser2
