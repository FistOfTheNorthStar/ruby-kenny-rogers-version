# FFI in a Kenny binary. A Kenny program is one static executable: it doesn't load C
# libraries at runtime (C extensions are compiled in with `kenny build --ext`). This file
# stands in for the ffi gem so that a gem binding a C library through FFI still loads: the
# declarations (ffi_lib, typedef, enum, callback, attach_function, Struct/Union layouts) are
# recorded, and calling an attached function raises NotImplementedError — the same contract
# as Kenny's other dropped features in dependencies (docs/LANGUAGE.md). For example
# sass-rails loads `sassc` (an FFI binding of libsass) in every Rails 5.2 app, but production
# only serves precompiled assets and never compiles Sass.
module FFI
  VERSION = "1.17.4"

  class NotFoundError < LoadError; end
  class TypeError < ::TypeError; end
  class SignatureError < NotImplementedError; end

  module Platform
    OS = RbConfig::CONFIG["host_os"].to_s.include?("darwin") ? "darwin" : "linux"
    ARCH = RbConfig::CONFIG["host_cpu"].to_s
    NAME = "#{ARCH}-#{OS}"
    LIBPREFIX = "lib"
    LIBSUFFIX = OS == "darwin" ? "dylib" : "so"
    ADDRESS_SIZE = 64
    LONG_SIZE = 64
    BYTE_ORDER = :little_endian
    def self.mac? = OS == "darwin"
    def self.windows? = false
    def self.unix? = true
  end

  class Type
    attr_reader :name, :size
    def initialize(name, size = 8)
      @name = name
      @size = size
    end
    def alignment = @size
    def inspect = "#<FFI::Type #{@name}>"

    class Mapped < Type
      def initialize(converter)
        super(:mapped, 8)
        @converter = converter
      end
    end

    SIZES = {
      void: 0, bool: 1, char: 1, uchar: 1, int8: 1, uint8: 1, short: 2, ushort: 2, int16: 2, uint16: 2,
      int: 4, uint: 4, int32: 4, uint32: 4, float: 4, long: 8, ulong: 8, int64: 8, uint64: 8,
      long_long: 8, ulong_long: 8, double: 8, pointer: 8, string: 8, size_t: 8, ssize_t: 8, buffer_in: 8,
      buffer_out: 8, buffer_inout: 8, varargs: 8, intptr_t: 8, uintptr_t: 8
    }.freeze
    SIZES.each { |n, s| const_set(n.to_s.upcase, new(n, s)) }
  end

  TypeDefs = Type::SIZES.to_h { |n, _| [n, Type.const_get(n.to_s.upcase)] }

  def self.typedef(old, add)
    TypeDefs[add] = find_type(old)
  end

  def self.find_type(t, mod = nil)
    case t
    when Type, Enum, Class then t
    when Symbol, String then (mod&.ffi_typedefs&.[](t.to_sym)) || TypeDefs[t.to_sym] || raise(TypeError, "unable to resolve type '#{t}'")
    else t
    end
  end

  def self.map_library_name(lib)
    lib = lib.to_s
    return lib if lib.include?("/") || lib.include?(".")
    "#{Platform::LIBPREFIX}#{lib}.#{Platform::LIBSUFFIX}"
  end

  # FFI calls into C are not available in a Kenny binary.
  def self.unsupported(what)
    raise NotImplementedError, "#{what}: calling C libraries through FFI is not supported by Kenny " \
                               "(a Kenny binary doesn't load shared libraries; see docs/LANGUAGE.md)"
  end

  class Pointer
    SIZE = 8
    attr_reader :address
    def initialize(type = :void, address = 0)
      @address = address.is_a?(Pointer) ? address.address : address.to_i
    end
    def null? = @address.zero?
    def to_i = @address
    alias to_int to_i
    def ==(other) = other.is_a?(Pointer) ? other.address == @address : other.nil? && null?
    def +(off) = Pointer.new(:void, @address + off)
    def autorelease=(_v); end
    def autorelease? = false
    def free = nil
    def inspect = "#<#{self.class.name} address=0x#{@address.to_s(16)}>"
    def self.size = SIZE
    %w[read write get put].each do |op|
      %w[string pointer int uint long ulong int8 uint8 int16 uint16 int32 uint32 int64 uint64 double float bytes
         array_of_string array_of_pointer array_of_int].each do |t|
        define_method("#{op}_#{t}") { |*| FFI.unsupported("FFI::Pointer##{op}_#{t}") }
      end
    end
    NULL = new.freeze
  end

  class MemoryPointer < Pointer
    def initialize(type = :char, count = 1, clear = true)
      FFI.unsupported("FFI::MemoryPointer.new")
    end
    def self.from_string(_s) = FFI.unsupported("FFI::MemoryPointer.from_string")
  end
  class AutoPointer < Pointer; end
  class Buffer < Pointer; end

  class Enum
    attr_reader :tag, :symbols
    def initialize(info, tag = nil)
      @tag = tag
      @kv = {}
      value = 0
      info.each_with_index do |x, i|
        next if x.is_a?(Integer)
        value = info[i + 1] if info[i + 1].is_a?(Integer)
        @kv[x] = value
        value += 1
      end
      @symbols = @kv.keys
    end
    def [](q) = q.is_a?(Integer) ? @kv.key(q) : @kv[q]
    alias find []
    def to_h = @kv.dup
    alias to_hash to_h
    def size = 4
    def native_type = Type::INT
  end
  class Enums
    def initialize = @all = []
    def <<(e) = @all << e
    def find(q) = @all.find { |e| e.tag == q } || @all.find { |e| e.symbols.include?(q) }
  end

  class Function < Pointer
    def initialize(ret = nil, args = nil, *rest, &blk)
      super()
      @callable = rest.first || blk
    end
    def call(*) = FFI.unsupported("FFI::Function#call")
    def attach(mod, name)
      f = self
      mod.define_singleton_method(name) { |*a| f.call(*a) }
      self
    end
  end
  CallbackInfo = Struct.new(:result_type, :param_types)
  FunctionType = CallbackInfo

  class StructLayout
    Field = ::Struct.new(:name, :type, :offset, :size)
    attr_reader :members, :fields, :size
    def initialize(fields, size)
      @fields = fields
      @members = fields.map(&:name)
      @size = size
    end
    def [](name) = @fields.find { |f| f.name == name }
    def offset_of(name) = self[name]&.offset
    def offsets = @fields.map { |f| [f.name, f.offset] }
    def alignment = 8
  end

  class Struct
    class << self
      def layout(*spec)
        return @layout if spec.empty?
        spec = spec.first.to_a.flatten if spec.size == 1 && spec.first.is_a?(Hash)
        fields = []
        offset = 0
        union = self <= Union
        size = 0
        i = 0
        while i < spec.size
          name = spec[i]
          type = spec[i + 1]
          i += 2
          if spec[i].is_a?(Integer)
            offset = spec[i]
            i += 1
          end
          fsize = field_size(type)
          align = [[fsize, 8].min, 1].max
          offset = (offset + align - 1) / align * align unless union
          fields << StructLayout::Field.new(name, type, union ? 0 : offset, fsize)
          offset += fsize unless union
          size = [size, union ? fsize : offset].max
        end
        @layout = StructLayout.new(fields, (size + 7) / 8 * 8)
      end

      def field_size(type)
        case type
        when Class then type < Struct ? type.size : 8
        when Array then field_size(type[0]) * type[1]
        when Enum then 4
        when Type then type.size
        when Symbol then (FFI::TypeDefs[type]&.size || 8)
        else 8
        end
      end

      def size = (@layout ||= nil) ? @layout.size : 0
      def alignment = 8
      def members = layout.members
      def offset_of(name) = layout.offset_of(name)
      def offsets = layout.offsets
      def by_value = self
      def by_ref(*) = Type::POINTER
      def ptr(*) = Type::POINTER
      def val = self
      def in = Type::POINTER
      def out = Type::POINTER
    end

    attr_reader :pointer
    def initialize(pointer = nil, *_layout)
      @pointer = pointer || FFI.unsupported("#{self.class}.new without a pointer")
    end
    def [](_f) = FFI.unsupported("#{self.class}#[]")
    def []=(_f, _v)
      FFI.unsupported("#{self.class}#[]=")
    end
    def to_ptr = @pointer
    def null? = @pointer.null?
    def members = self.class.members
  end
  class Union < Struct; end
  class ManagedStruct < Struct; end

  module Library
    CURRENT_PROCESS = Object.new.freeze
    LIBC = "c"

    def ffi_lib(*names)
      @ffi_libs = names.flatten.map { |n| n.equal?(CURRENT_PROCESS) ? n : FFI.map_library_name(n) }
    end
    def ffi_libraries = @ffi_libs || []
    def ffi_lib_flags(*_flags) = 0
    def ffi_convention(c = nil) = (@ffi_convention = c || @ffi_convention || :default)

    def ffi_typedefs = (@ffi_typedefs ||= {})
    def find_type(t) = FFI.find_type(t, self)

    def typedef(old, add, _info = nil)
      ffi_typedefs[add] = old.is_a?(Symbol) ? (ffi_typedefs[old] || FFI::TypeDefs[old] || old) : old
    end

    def callback(*args)
      name = args.size == 3 ? args.shift : nil
      info = CallbackInfo.new(args[1], args[0])
      ffi_typedefs[name] = info if name
      info
    end

    def enum(*args)
      args.shift if args.first.is_a?(Type)
      tag = args[0].is_a?(Symbol) && args[1].is_a?(Array) ? args.shift : nil
      info = args.first.is_a?(Array) ? args.first : args
      e = Enum.new(info, tag)
      (@ffi_enums ||= Enums.new) << e
      ffi_typedefs[tag] = e if tag
      e
    end
    alias bitmask enum
    def enum_type(name) = ffi_typedefs[name]
    def enum_value(sym) = @ffi_enums&.find(sym)&.[](sym)

    # attach_function(name, [c_name,] params, returns, options = {})
    def attach_function(name, func, args, returns = nil, options = nil)
      mname = name
      cname = func
      unless returns && !returns.is_a?(Hash)
        cname = name
        returns = args
        args = func
      end
      libs = ffi_libraries.map(&:to_s).join(", ")
      body = proc { |*_a| FFI.unsupported("#{cname} (#{libs})") }
      define_singleton_method(mname, &body)
      define_method(mname, &body)
      nil
    end

    def attach_variable(mname, a1, a2 = nil)
      define_singleton_method(mname) { FFI.unsupported("#{a2 ? a1 : mname} (variable)") }
      define_singleton_method("#{mname}=") { |_v| FFI.unsupported("#{a2 ? a1 : mname} (variable)") }
    end

    def function_names(name, _arg_types) = [name.to_s]
  end
end
