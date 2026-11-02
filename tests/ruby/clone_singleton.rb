# `clone` copies the singleton class (its methods, the modules it includes, its instance
# variables and constants); `dup` doesn't. rspec clones its DSL matchers when they are
# composed (`include("k" => custom_matcher)`), and their `matches?` is a singleton method.
class Matcher
  def initialize(&decl)
    klass = class << self
      include(@user = Module.new)
      self
    end
    klass.instance_variable_get(:@user).send(:define_method, :inner) { |v| v * 10 }
    klass.class_exec(&decl)
  end
end

m = Matcher.new { define_method(:matches?) { |v| inner(v) == 10 } }
c = m.clone
p c.matches?(1), c.matches?(2)
p c.singleton_class.include?(m.singleton_class.instance_variable_get(:@user))
p c.singleton_class.instance_variable_get(:@user).equal?(m.singleton_class.instance_variable_get(:@user))
p c.singleton_class.equal?(m.singleton_class)
p m.dup.respond_to?(:matches?)

o = Object.new
def o.hi = :hi
o.extend(Comparable)
o.singleton_class.const_set(:K, 42)
c = o.clone
p c.hi, c.is_a?(Comparable), c.singleton_class::K
def c.only_clone = 1
p o.respond_to?(:only_clone), c.respond_to?(:hi)
p o.dup.respond_to?(:hi), o.dup.is_a?(Comparable)
f = o.clone(freeze: true)
p f.frozen?, f.hi
s = "str"
def s.shout = upcase + "!"
p s.clone.shout, s.dup.respond_to?(:shout)
