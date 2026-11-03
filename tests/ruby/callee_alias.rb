# `__callee__` is the name a method was called by: through an alias of a plain `def` too
# (Mustermann's `alias_method :&, :|`, Rails' `alias_method :each, :each_pair`).
class Pat
  def |(other) = [__callee__, __method__, other]
  alias_method :&, :|
  alias ^ |
  def each_pair(&b) = block_given? ? [__callee__].each(&b) : to_enum(__callee__)
  alias_method :each, :each_pair
  def in_block = [1].map { __callee__ }.first
  alias_method :in_block2, :in_block
  def plain = __callee__
  alias plain2 plain
  def chain = [__callee__, plain2, plain]
  alias_method :chain2, :chain
  def self.k(x) = [__callee__, x]
  class << self; alias_method :k2, :k; end
end
o = Pat.new
p o | 1, o & 2, o ^ 3
p o.each.next, o.each_pair.next, o.each { }
p o.in_block, o.in_block2, o.plain, o.plain2, o.chain, o.chain2
p o.send(:&, 4), o.method(:^).call(5), o.public_send(:plain2)
p Pat.k(1), Pat.k2(2)
p Pat.instance_method(:&).arity, Pat.instance_method(:plain2).source_location.class
# an alias of an alias, and the original name aliased back
class Pat
  alias_method :plain3, :plain2
  alias_method :orig, :plain
end
p o.plain3, o.orig
# module_function
module MF
  module_function
  def mf = __callee__
  alias_method :mf2, :mf
end
p MF.mf, MF.send(:mf2) rescue p $!.class
# raising through an alias
class Pat
  def boom = raise(ArgumentError, "#{__callee__} failed")
  alias_method :bang, :boom
end
p((o.bang rescue $!.message), (o.boom rescue $!.message))
