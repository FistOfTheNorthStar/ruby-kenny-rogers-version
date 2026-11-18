# A Method or UnboundMethod keeps its body after the method leaves its class (define_method
# then remove_method: Sinatra's routes), through collections and compaction.
class K
  define_method(:foo) { |x = 1| [:foo, x] }
  UM = instance_method(:foo)
  remove_method(:foo)
  define_method(:bar) { :bar }
  UB = instance_method(:bar)
  alias_method :baz, :bar
  remove_method(:bar)
end
def garbage = 200_000.times.map { |i| proc { |a, b| i } }.size
3.times { garbage; GC.start }
k = K.new
p K::UM.bind(k).call, K::UM.bind(k).call(2)
p K::UB.bind(k).call, k.baz
class K
  define_method(:temp) { :temp }
  M2 = new.method(:temp)
  remove_method(:temp)
end
3.times { garbage; GC.start }
p K::M2.call
p K::M2.unbind.bind(K.new).call
# Hash of them, as Sinatra keeps
routes = 2000.times.map do |i|
  K.send(:define_method, :"r#{i}") { i }
  m = K.instance_method(:"r#{i}")
  K.send(:remove_method, :"r#{i}")
  m
end
3.times { garbage; GC.start }
p routes.each_with_index.all? { |m, i| m.bind(k).call == i }
# replaced rather than removed
K.send(:define_method, :rep) { :old }
old = K.instance_method(:rep)
K.send(:define_method, :rep) { :new }
3.times { garbage; GC.start }
p old.bind(k).call, k.rep
GC.compact if GC.respond_to?(:compact)
p routes.each_with_index.all? { |m, i| m.bind(k).call == i }
K.send(:define_method, :back, routes[7])
3.times { garbage; GC.start }
p k.back
