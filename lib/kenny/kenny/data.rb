# Ruby 3.2's Data (immutable value objects), loaded the first time a program calls
# Data.define (the Data class itself and its `define` are native, so code that reopens
# `class Data`, as ActiveSupport's JSON extensions do, finds it).
class Data
  class << self
    undef_method :new

    def __kenny_define(*names, &block)
      names = names.map do |n|
        case n
        when Symbol then n
        when String then n.to_sym
        else raise TypeError, "#{n.inspect} is not a symbol nor a string"
        end
      end
      names.each do |n|
        raise ArgumentError, "invalid data member: #{n}" if n.end_with?("=")
      end
      if (dup = names.find { |n| names.count(n) > 1 })
        raise ArgumentError, "duplicate member: #{dup}"
      end
      klass = Class.new(self)
      klass.instance_variable_set(:@__members, names.freeze)
      names.each do |n|
        klass.define_method(n) { @__values[n] }
      end
      class << klass
        def new(*args, **kwargs, &block)
          members = self.members
          unless args.empty?
            if args.size > members.size
              raise ArgumentError, "wrong number of arguments (given #{args.size}, expected 0..#{members.size})"
            end
            kwargs = members.first(args.size).zip(args).to_h.merge(kwargs)
          end
          obj = allocate
          obj.send(:initialize, **kwargs, &block)
          obj.freeze
        end
        alias_method :[], :new
      end
      klass.class_eval(&block) if block
      klass
    end

    def members
      @__members || (superclass.respond_to?(:members) ? superclass.members : [])
    end
  end

  def initialize(**kwargs)
    members = self.class.members
    missing = members - kwargs.keys
    unless missing.empty?
      raise ArgumentError, "missing keyword#{"s" if missing.size > 1}: #{missing.map(&:inspect).join(", ")}"
    end
    extra = kwargs.keys - members
    unless extra.empty?
      raise ArgumentError, "unknown keyword#{"s" if extra.size > 1}: #{extra.map(&:inspect).join(", ")}"
    end
    @__values = members.to_h { |m| [m, kwargs[m]] }
  end

  def initialize_copy(other)
    @__values = other.instance_variable_get(:@__values)
    super
  end

  def members
    self.class.members
  end

  def to_h(&block)
    block ? @__values.to_h(&block) : @__values.dup
  end

  def with(**kwargs)
    return self if kwargs.empty?
    self.class.new(**@__values.merge(kwargs))
  end

  def deconstruct
    @__values.values
  end

  def deconstruct_keys(keys)
    return @__values.dup if keys.nil?
    raise TypeError, "wrong argument type #{keys.class} (expected Array or nil)" unless keys.is_a?(Array)
    h = {}
    keys.each do |k|
      break unless @__values.key?(k)
      h[k] = @__values[k]
    end
    h
  end

  def ==(other)
    other.class == self.class && other.to_h == @__values
  end

  def eql?(other)
    other.class == self.class && other.to_h.eql?(@__values)
  end

  def hash
    [self.class, @__values].hash
  end

  def inspect
    name = self.class.name
    fields = @__values.map { |k, v| "#{k}=#{v.inspect}" }.join(", ")
    "#<data #{[name, fields].reject { |x| x.nil? || x.empty? }.join(" ")}>"
  end
  alias_method :to_s, :inspect
end
