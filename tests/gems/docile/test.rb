require "bundler/setup"
require "docile"

class Pizza
  attr_reader :toppings, :size
  def initialize = (@toppings = []; @size = :medium)
  def cheese(kind = :mozzarella) = (@toppings << kind; self)
  def pepperoni = @toppings << :pepperoni
  def large = @size = :large
end

def chef_name = "Kenny"

pizza = Docile.dsl_eval(Pizza.new) do
  cheese
  cheese :parmesan
  pepperoni
  large
  @note = "from outer"
  toppings << chef_name.downcase.to_sym
end
p pizza.toppings, pizza.size
p @note

class PizzaBuilder
  def initialize = @t = []
  def topping(x) = @t << x
  def build = "pizza with #{@t.join(", ")}"
end
p Docile.dsl_eval_with_block_return(PizzaBuilder.new) { topping :ham; topping :olives; 42 }
p Docile.dsl_eval(PizzaBuilder.new) { topping :ham; topping :olives }.build

Config = Struct.new(:name, :port, :tags)
cfg = Docile.dsl_eval_immutable(Config.new("app", 80, [].freeze)) do
  self.class
end
p cfg

nested = Docile.dsl_eval([]) do
  push 1
  Docile.dsl_eval({}) { store(:inner, chef_name) }.then { |h| push h }
  push 3
end
p nested

begin
  Docile.dsl_eval(Pizza.new) { no_such_method_here }
rescue NameError => e
  p e.class, e.name
end

local = 10
p Docile.dsl_eval([]) { push(local * 2); push(chef_name) }
