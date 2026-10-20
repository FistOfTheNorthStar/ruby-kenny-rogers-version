# Kenny's Forwardable: same API as the standard library, built on define_method
# (the stdlib version generates code with eval + binding).
module Forwardable
  VERSION = "1.3.3"
  FORWARDABLE_VERSION = VERSION

  def self._target(obj, accessor)
    if accessor.start_with?("@")
      obj.instance_variable_get(accessor)
    else
      obj.__send__(accessor)
    end
  end

  def instance_delegate(hash)
    hash.each do |methods, accessor|
      Array(methods).each { |m| def_instance_delegator(accessor, m) }
    end
  end

  def def_instance_delegators(accessor, *methods)
    methods.each do |m|
      next if [:__send__, :__id__, "__send__", "__id__"].include?(m)
      def_instance_delegator(accessor, m)
    end
  end

  def def_instance_delegator(accessor, method, ali = method)
    accessor = accessor.to_s
    method = method.to_sym
    define_method(ali) do |*args, &block|
      Forwardable._target(self, accessor).__send__(method, *args, &block)
    end
    ali.to_sym
  end

  alias delegate instance_delegate
  alias def_delegators def_instance_delegators
  alias def_delegator def_instance_delegator
end

module SingleForwardable
  def single_delegate(hash)
    hash.each do |methods, accessor|
      Array(methods).each { |m| def_single_delegator(accessor, m) }
    end
  end

  def def_single_delegators(accessor, *methods)
    methods.each { |m| def_single_delegator(accessor, m) }
  end

  def def_single_delegator(accessor, method, ali = method)
    accessor = accessor.to_s
    method = method.to_sym
    define_singleton_method(ali) do |*args, &block|
      Forwardable._target(self, accessor).__send__(method, *args, &block)
    end
    ali.to_sym
  end

  alias delegate single_delegate
  alias def_delegators def_single_delegators
  alias def_delegator def_single_delegator
end
