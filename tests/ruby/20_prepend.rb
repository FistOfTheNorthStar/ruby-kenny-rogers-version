module Loud
  def speak = super.upcase + "!"
end
module Polite
  def speak = "please, " + super
end
class Dog
  prepend Loud
  def speak = "woof"
end
class Puppy < Dog
  prepend Polite
  def speak = "yip and " + super
end
p Dog.new.speak, Puppy.new.speak, Dog.ancestors.first(3), Puppy.ancestors.first(5)
module JsonExt
  def to_json(*a) = "<" + super + ">"
end
require "json"
Hash.prepend(JsonExt)
p({ a: 1 }.to_json)
