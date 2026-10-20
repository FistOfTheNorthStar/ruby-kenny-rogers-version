class Model
  %w[name email age].each do |attr|
    class_eval <<~RUBY, __FILE__, __LINE__ + 1
      def #{attr}
        @attributes[:#{attr}]
      end

      def #{attr}=(value)
        @attributes[:#{attr}] = value
      end

      def #{attr}?
        !@attributes[:#{attr}].nil?
      end
    RUBY
  end

  def initialize(**attrs) = @attributes = attrs
end

m = Model.new(name: "Kenny", age: 3)
m.email = "k@example.com"
p m.name, m.email, m.age, m.age?, Model.instance_methods(false).sort.first(4)

p eval("1 + 2 * 3"), eval("[1, 2, 3].map { |x| x * 10 }.sum")
code = "def self.dyn_greet(n) = \"hello \#{n}\""
eval(code)
p dyn_greet("eval")

str = "secret"
p str.instance_eval("upcase + '!'"), str.instance_eval("size")
String.class_eval("def shout = upcase + '!!'")
p "hey".shout

module Helpers; end
Helpers.module_eval("def self.twice(x) = x * 2; CONST_FROM_EVAL = 99")
p Helpers.twice(21), Helpers::CONST_FROM_EVAL

1000.times { |i| eval("i_val = 1") }
begin
  eval("1 +")
rescue SyntaxError => e
  p e.class
end
