class Animal
  attr_reader :name
  @@count = 0
  def initialize(name)
    @name = name
    @@count += 1
  end
  def self.count = @@count
  def speak = "..."
  def to_s = "#{self.class.name}(#{@name})"
  def <=>(other) = name <=> other.name
  include Comparable
end

class Dog < Animal
  def initialize(name, breed)
    super(name)
    @breed = breed
  end
  def speak = "Woof"
  def describe = "#{name} the #{@breed} says #{speak}"
end

module Loud
  def speak = super.upcase + "!"
end

class Puppy < Dog
  include Loud
  def speak = "yip"
end

d = Dog.new("Rex", "beagle")
puts d.describe
puts d
puts Animal.count
p Puppy.new("Bit", "pug").speak
puts Puppy.ancestors.take(4).inspect
puts Dog.superclass, Dog.new("a", "b").is_a?(Animal), Dog.instance_method(:speak).inspect
animals = [Dog.new("Zed", "x"), Dog.new("Amy", "y")]
puts animals.min.name, animals.sort.map(&:name).inspect
puts d.respond_to?(:speak), d.respond_to?(:fly)

class Point
  attr_accessor :x, :y
  def initialize(x = 0, y = 0)
    @x, @y = x, y
  end
  def +(o) = Point.new(x + o.x, y + o.y)
  def ==(o) = o.is_a?(Point) && x == o.x && y == o.y
  def to_s = "(#{x}, #{y})"
  def inspect = "#<Point #{x},#{y}>"
end
pt = Point.new(1, 2) + Point.new(3, 4)
puts pt, pt == Point.new(4, 6)
pt.x += 10
p pt

class Meta
  [:alpha, :beta].each do |n|
    define_method("#{n}?") { n == :alpha }
  end
  def method_missing(name, *args)
    name.to_s.start_with?("get_") ? "got #{name.to_s.sub('get_', '')}" : super
  end
  def respond_to_missing?(name, include_private = false) = name.to_s.start_with?("get_") || super
end
m = Meta.new
p m.alpha?, m.beta?, m.get_thing, m.respond_to?(:get_x), m.send(:alpha?)
begin
  m.unknown
rescue NoMethodError => e
  puts "NoMethodError: #{e.message}"
end

S = Struct.new(:a, :b) do
  def total = a + b
end
s = S.new(1, 2)
p s, s.total, s.to_a, S.new(1, 2) == s
s.a = 10
p s.a

class Stack
  include Enumerable
  def initialize(*items) = @items = items
  def each(&block)
    @items.each(&block)
    self
  end
end
st = Stack.new(3, 1, 2)
p st.sort, st.map { _1 * 2 }, st.include?(2), st.min_by { -it }, st.first, st.to_a

module Greeter
  def self.hello(n) = "hello #{n}"
  PREFIX = ">>"
  def greet = "#{PREFIX} hi from #{self.class}"
end
class Person; include Greeter; end
puts Greeter.hello("you"), Person.new.greet, Greeter::PREFIX

class Counter
  class << self
    def create = new
    attr_accessor :instances
  end
  self.instances = 0
end
Counter.instances += 1
p Counter.instances, Counter.create.class
p 5.frozen?, "lit".frozen?, String.new("x").frozen?
obj = Object.new
def obj.special = "singleton!"
p obj.special, obj.singleton_methods
