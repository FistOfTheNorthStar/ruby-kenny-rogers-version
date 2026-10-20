class Acct
  attr_reader :bal
  def initialize(b) = @bal = b
  def show = "#{secret}/#{self.secret}"
  def >(o) = bal > o.bal
  private
  def secret = "s#{bal}"
  attr_writer :bal
  public
  def open = 1
  def bump = (self.bal = bal + 1)
  protected def prot = 2
  def cmp(o) = o.prot
  private def helper = 3
end
a = Acct.new(5)
p a.show, a.open, a.bump, a.cmp(a)
begin
  a.secret
rescue NoMethodError => e
  puts e.message
end
begin
  a.bal = 9
rescue NoMethodError => e
  puts e.message
end
p a.respond_to?(:secret), a.respond_to?(:secret, true), a.send(:secret), a.public_send(:open)
begin
  a.public_send(:helper)
rescue NoMethodError => e
  puts e.message
end
p Acct.private_instance_methods(false).sort, Acct.public_instance_methods(false).sort, Acct.protected_instance_methods(false)
p Acct.private_method_defined?(:secret), Acct.method_defined?(:secret), Acct.public_method_defined?(:open)
class Acct
  def later = 4 # visibility resets when the class is reopened
end
p a.later
class Acct
  public :secret
  private :open
end
p a.secret
begin
  a.open
rescue NoMethodError => e
  puts e.message
end
module Util
  module_function
  def twice(x) = x * 2
end
p Util.twice(4)
class UsesUtil
  include Util
  def go = twice(5)
end
p UsesUtil.new.go
begin
  UsesUtil.new.twice(1)
rescue NoMethodError => e
  puts e.message
end
def top_helper = :top
p top_helper
begin
  1.top_helper
rescue NoMethodError => e
  puts e.message
end
p Kernel.format("%03d", 7), Kernel.respond_to?(:puts)
class Ghost
  def method_missing(n, *a) = "mm:#{n}"
  def respond_to_missing?(n, p = false) = n.to_s.start_with?("g")
  private def hidden = 1
end
p Ghost.new.ghost, Ghost.new.hidden, Ghost.new.respond_to?(:gx), Ghost.new.respond_to?(:x)
class Single
  class << self
    def make = new
    private :new
  end
  private_class_method def self.inner = 9
end
p Single.make.class
begin
  Single.new
rescue NoMethodError => e
  puts e.message
end
