# define_method under module_function / private (Rack::Utils.escape_html is defined so)
module U
  module_function
  def a = 1
  define_method(:b) { 2 }
  define_method(:c, instance_method(:a))
  private
  define_method(:d) { 4 }
  def e = 5
end
p U.a, U.b, U.c, U.respond_to?(:d), U.respond_to?(:e)
p U.private_instance_methods.sort
class K
  private
  define_method(:x, Kernel.instance_method(:frozen?))
  public
  define_method(:y) { 1 }
end
p K.private_method_defined?(:x), K.public_method_defined?(:y)
