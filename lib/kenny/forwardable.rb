# Kenny's Forwardable: same API as the standard library, built on define_method
# (the stdlib version generates code with eval + binding).
module Forwardable
  VERSION = "1.3.3"
  FORWARDABLE_VERSION = VERSION

  # The accessor as the stdlib accepts it (it evals it): "@ivar", "@@cvar", "method",
  # "Const::Path", "self", and chains of those with "." ("self.class", "@a.b"). Parsed
  # once, when the delegator is defined.
  def self._path(accessor)
    accessor.split(".").map.with_index do |part, i|
      if part == "self" && i == 0 then [:self]
      elsif part.start_with?("@@") then [:cvar, part.to_sym]
      elsif part.start_with?("@") then [:ivar, part.to_sym]
      elsif i == 0 && part.match?(/\A(::)?[A-Z]/) then [:const, part.delete_prefix("::")]
      else [:call, part.to_sym]
      end
    end
  end

  def self._target(obj, path)
    path = _path(path) if path.is_a?(String)
    path.inject(obj) do |cur, (kind, name)|
      case kind
      when :self then obj
      when :ivar then cur.instance_variable_get(name)
      when :cvar then (cur.is_a?(Module) ? cur : cur.class).class_variable_get(name)
      when :const then Object.const_get(name)
      else cur.__send__(name)
      end
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
    accessor = Forwardable._path(accessor.to_s)
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
    accessor = Forwardable._path(accessor.to_s)
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
