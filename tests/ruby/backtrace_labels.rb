# Backtrace labels as in Ruby 3.4: Class#method, Class.method, "block in ..."
def here = caller_locations(1, 1)[0]
def top_m = here
module Outer
  class Inner
    def inst = here
    def self.cls = here
    class << self
      def sc = here
    end
    def with_block = [1].each_with_object([]) { |_, a| a << here }[0]
    def with_proc
      pr = proc { here }
      pr.call
    end
  end
  def self.mod_m = here
end
class Outer::Path
  def m = here
end
class ::TopLevel
  def m = here
end
o = Object.new
def o.single = here
klass = Class.new { def anon = here }

[top_m, Outer::Inner.new.inst, Outer::Inner.cls, Outer::Inner.sc, Outer::Inner.new.with_proc,
 Outer.mod_m, Outer::Path.new.m, TopLevel.new.m, o.single, klass.new.anon, here].each do |l|
  p [l.label, l.base_label]
end
def boom = raise("x")
begin
  Outer::Inner.new.instance_eval { boom }
rescue => e
  puts e.backtrace[0][/'.*'/]
end
puts caller(0, 1)[0][/'.*'/]
