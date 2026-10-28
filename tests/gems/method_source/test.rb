require "bundler/setup"
require "method_source"

# Adds two numbers.
# Returns the sum.
def add(a, b)
  a + b
end

class Greeter
  # Says hello
  def hello(name) = "hello #{name}"

  define_method(:dynamic) do |x|
    x * 2
  end
end

m = method(:add)
puts m.source
puts m.comment
p m.source_location.map { |v| v.is_a?(String) ? File.basename(v) : v }
puts Greeter.instance_method(:hello).source
puts Greeter.instance_method(:hello).comment
puts Greeter.instance_method(:dynamic).source
blk = proc { |x|
  x + 1
}
puts blk.source
p MethodSource.complete_expression?("def foo")
p MethodSource.complete_expression?("def foo; end")
p MethodSource.expression_at(["x = 1\n", "def y\n", "  2\n", "end\n"], 2)
begin
  method(:puts).source
rescue MethodSource::SourceNotFoundError => e
  p e.class, e.message
end
