# frozen_string_literal: true

# Ruby 4.0's Set. It is a core class there (set.c), no longer the standard library's set.rb,
# so a Kenny build against Ruby 4.0 has this in place of set.rb: loaded on first use of `Set`,
# or by `require "set"` (the build's overlay set.rb). It follows set.c: `Set[1, 2]` as the
# inspect form, set.c's error messages, and subclasses of Set get set.rb's old behavior from
# the standard library's set/subclass_compatible.rb (Set::CoreSet is the class without it).
#
# The elements are the keys of @hash, each mapped to true, as in set.rb. That is also how
# Ruby 4.0 marshals a Set (an object whose @hash is that Hash), so Marshal and YAML output
# stay the same.

class Set
  include Enumerable

  def self.[](*ary)
    h = Hash.new(false)
    ary.each { |o| h[o] = true }
    set = allocate
    set.instance_variable_set(:@hash, h)
    set
  end

  # (the elements' Hash exists before `initialize` runs, as set.c allocates its table: a
  # subclass's initialize, such as set/subclass_compatible.rb's, needn't call super)
  def self.new(...)
    set = allocate
    set.instance_variable_set(:@hash, Hash.new(false))
    set.__send__(:initialize, ...)
    set
  end

  def initialize(enum = nil, &block)
    return if enum.nil?
    if block
      __kenny_each_of(enum) { |o| @hash[block.call(o)] = true }
    else
      __kenny_merge(enum)
    end
  end

  def initialize_copy(other)
    return self if equal?(other)
    @hash = other.instance_variable_get(:@hash).dup
    self
  end

  def freeze
    @hash.freeze
    super
  end

  def size = @hash.size
  alias length size

  def empty? = @hash.empty?

  def include?(o) = @hash.include?(o)
  alias member? include?
  alias === include?

  def each(&block)
    return enum_for(:each) { size } unless block
    @hash.each_key(&block)
    self
  end

  def to_a = @hash.keys

  def to_set(*args, &block)
    klass = if args.empty?
      Set
    else
      warn "passing arguments to Set#to_set is deprecated", uplevel: 1, category: :deprecated
      args.shift
    end
    return self if klass == Set && instance_of?(Set) && args.empty? && !block
    klass.new(self, *args, &block)
  end

  def join(separator = nil) = to_a.join(separator)

  def add(o)
    __kenny_modify
    @hash[o] = true
    self
  end
  alias << add

  def add?(o)
    __kenny_modify
    return nil if @hash.include?(o)
    @hash[o] = true
    self
  end

  def delete(o)
    __kenny_modify
    @hash.delete(o)
    self
  end

  def delete?(o)
    __kenny_modify
    return nil unless @hash.include?(o)
    @hash.delete(o)
    self
  end

  def delete_if
    return enum_for(:delete_if) { size } unless block_given?
    __kenny_modify
    @hash.delete_if { |o, _| yield o }
    self
  end

  def reject!(&block)
    return enum_for(:reject!) { size } unless block
    n = size
    delete_if(&block)
    n == size ? nil : self
  end

  def keep_if
    return enum_for(:keep_if) { size } unless block_given?
    __kenny_modify
    @hash.keep_if { |o, _| yield o }
    self
  end

  def select!(&block)
    return enum_for(:select!) { size } unless block
    n = size
    keep_if(&block)
    n == size ? nil : self
  end
  alias filter! select!

  def collect!
    return enum_for(:collect!) { size } unless block_given?
    __kenny_modify
    h = __kenny_new_hash
    @hash.each_key { |o| h[yield(o)] = true }
    @hash = h
    self
  end
  alias map! collect!

  def clear
    __kenny_modify
    @hash.clear
    self
  end

  def replace(enum)
    __kenny_modify
    if enum.is_a?(Set)
      @hash = enum.instance_variable_get(:@hash).dup
      @hash.compare_by_identity if enum.compare_by_identity? && !@hash.compare_by_identity?
    else
      __kenny_enum_method(enum)
      @hash.clear
      __kenny_merge(enum)
    end
    self
  end

  def merge(*enums, **kw)
    raise ArgumentError, "no keywords accepted" unless kw.empty?
    __kenny_modify
    enums.each { |e| __kenny_merge(e) }
    self
  end

  def subtract(enum)
    __kenny_modify
    __kenny_each_of(enum) { |o| @hash.delete(o) }
    self
  end

  def |(enum)
    dup.merge(enum)
  end
  alias + |
  alias union |

  def -(enum)
    dup.subtract(enum)
  end
  alias difference -

  def &(enum)
    n = self.class.allocate
    h = __kenny_new_hash
    if enum.is_a?(Set)
      small, big = size < enum.size ? [self, enum] : [enum, self]
      small.each { |o| h[o] = true if big.include?(o) }
    else
      __kenny_each_of(enum) { |o| h[o] = true if @hash.include?(o) }
    end
    n.instance_variable_set(:@hash, h)
    n
  end
  alias intersection &

  def ^(enum)
    n = dup
    other = enum.is_a?(Set) ? enum : Set.new(enum)
    h = n.instance_variable_get(:@hash)
    other.each { |o| h.include?(o) ? h.delete(o) : h[o] = true }
    n
  end

  def subset?(set)
    __kenny_check_set(set)
    size <= set.size && all? { |o| set.include?(o) }
  end
  alias <= subset?

  def proper_subset?(set)
    __kenny_check_set(set)
    size < set.size && all? { |o| set.include?(o) }
  end
  alias < proper_subset?

  def superset?(set)
    __kenny_check_set(set)
    size >= set.size && set.all? { |o| include?(o) }
  end
  alias >= superset?

  def proper_superset?(set)
    __kenny_check_set(set)
    size > set.size && set.all? { |o| include?(o) }
  end
  alias > proper_superset?

  def <=>(set)
    return nil unless set.is_a?(Set)
    case size <=> set.size
    when -1 then -1 if all? { |o| set.include?(o) }
    when 1 then 1 if set.all? { |o| include?(o) }
    else 0 if all? { |o| set.include?(o) }
    end
  end

  def intersect?(set)
    case set
    when Set
      small, big = size < set.size ? [self, set] : [set, self]
      small.any? { |o| big.include?(o) }
    when Enumerable
      set.any?(self)
    else
      raise ArgumentError, "value must be enumerable"
    end
  end

  def disjoint?(set) = !intersect?(set)

  def ==(other)
    return true if equal?(other)
    return false unless other.is_a?(Set) && size == other.size
    return true if empty?
    return false if compare_by_identity? != other.compare_by_identity?
    other.all? { |o| @hash.include?(o) }
  end
  alias eql? ==

  def hash
    h = size
    @hash.each_key { |o| h ^= o.hash }
    [Set, h].hash
  end

  def classify
    return enum_for(:classify) { size } unless block_given?
    h = {}
    @hash.each_key { |o| (h[yield(o)] ||= self.class.new).add(o) }
    h
  end

  def divide(&func)
    return enum_for(:divide) { size } unless func
    if func.arity == 2
      items = to_a
      parent = (0...items.size).to_a
      root = ->(i) { i = parent[i] while parent[i] != i; i }
      items.each_with_index do |a, i|
        ((i + 1)...items.size).each do |j|
          b = items[j]
          if func.call(a, b) && func.call(b, a)
            ri, rj = root.(i), root.(j)
            parent[rj] = ri if ri != rj
          end
        end
      end
      groups = {}
      items.each_with_index { |o, i| (groups[root.(i)] ||= self.class.new).add(o) }
      Set.new(groups.values)
    else
      Set.new(classify(&func).values)
    end
  end

  def flatten
    n = self.class.new
    n.__send__(:__kenny_flatten_into, self, {})
    n
  end

  def flatten!
    return nil unless any? { |o| o.is_a?(Set) }
    __kenny_modify
    replace(flatten)
  end

  def compare_by_identity
    return self if compare_by_identity?
    __kenny_modify
    @hash.compare_by_identity
    self
  end

  def compare_by_identity? = @hash.compare_by_identity?

  def reset
    __kenny_modify
    @hash.rehash
    self
  end

  def inspect
    ids = (Thread.current[:__kenny_set_inspect] ||= {}.compare_by_identity)
    return "#{self.class}[...]" if ids.include?(self)
    begin
      ids[self] = true
      +"#{self.class}[#{@hash.keys.map(&:inspect).join(", ")}]"
    ensure
      ids.delete(self)
    end
  end
  alias to_s inspect

  # (Ruby 4.0's pp.rb leaves Set's pretty-printing to it when Set comes from a set.rb)
  def pretty_print(pp)
    pp.group(1, "#{self.class.name}[", "]") {
      pp.seplist(self) { |o| pp.pp o }
    }
  end

  def pretty_print_cycle(pp)
    name = self.class.name
    pp.text(empty? ? "#{name}[]" : "#{name}[...]")
  end

  private

  def __kenny_modify
    raise FrozenError.new("can't modify frozen #{self.class}: #{inspect}", receiver: self) if frozen?
  end

  def __kenny_new_hash
    h = Hash.new(false)
    h.compare_by_identity if @hash.compare_by_identity?
    h
  end

  def __kenny_check_set(set)
    raise ArgumentError, "value must be a set" unless set.is_a?(Set)
  end

  def __kenny_enum_method(enum)
    if enum.respond_to?(:each_entry) then :each_entry
    elsif enum.respond_to?(:each) then :each
    else raise ArgumentError, "value must be enumerable"
    end
  end

  def __kenny_each_of(enum, &block)
    if enum.is_a?(Set) || enum.is_a?(Array)
      enum.each(&block)
    else
      enum.__send__(__kenny_enum_method(enum), &block)
    end
  end

  def __kenny_merge(enum)
    h = @hash
    if enum.is_a?(Set)
      enum.instance_variable_get(:@hash).each_key { |o| h[o] = true }
    elsif enum.is_a?(Array)
      enum.each { |o| h[o] = true }
    else
      enum.__send__(__kenny_enum_method(enum)) { |o| h[o] = true }
    end
  end

  def __kenny_flatten_into(set, seen)
    set.each do |e|
      if e.is_a?(Set)
        case seen[e.object_id]
        when true then raise ArgumentError, "tried to flatten recursive Set"
        when false then next
        end
        seen[e.object_id] = true
        __kenny_flatten_into(e, seen)
        seen[e.object_id] = false
      else
        @hash[e] = true
      end
    end
  end

  # (made before `inherited` is defined: Set::CoreSet is the subclass without set.rb's ways)
  class CoreSet < Set
  end

  class << self
    private

    def inherited(subclass)
      super
      if equal?(Set)
        require "set/subclass_compatible"
        subclass.include(SubclassCompatible)
        subclass.extend(SubclassCompatible::ClassMethods)
      end
    end
  end
end

module Enumerable
  # Makes a set from the enumerable object with given arguments.
  # Passing arguments to this method is deprecated.
  def to_set(*args, &block)
    klass = if args.empty?
      Set
    else
      warn "passing arguments to Enumerable#to_set is deprecated", uplevel: 1
      args.shift
    end
    klass.new(self, *args, &block)
  end
end
