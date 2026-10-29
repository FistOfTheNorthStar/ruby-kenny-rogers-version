# frozen_string_literal: true
# Kenny: JSON is implemented natively by the runtime (crates/kenny-runtime/src/json.rs);
# this replaces the stdlib json.rb, which loads the json C extension.
#
# The runtime follows the json version the program gets (JSON::VERSION: the Gemfile.lock's,
# else the Ruby's own). For json 2.10 and later, the Ruby-level parts of the gem's
# json/common.rb and json/ext.rb are below.
require "json/ext/parser.so"

json_version = JSON::VERSION.split(".").map(&:to_i)

module JSON
  class << self
    unless method_defined?(:load_file!)
      def load_file!(filespec, opts = nil)
        parse!(File.read(filespec, encoding: Encoding::UTF_8), opts)
      end
    end
  end
end

if (json_version <=> [2, 10]) >= 0
  module JSON
    JSON_VERSION3 = (JSON::VERSION.split(".").map(&:to_i) <=> [3, 0]) >= 0 # :nodoc:
    private_constant :JSON_VERSION3

    class GeneratorError < JSONError
      attr_reader :invalid_object

      def initialize(message, invalid_object = nil)
        super(message)
        @invalid_object = invalid_object
      end

      def detailed_message(...)
        super_message = defined?(super) ? super : message
        if @invalid_object.nil?
          super_message
        else
          "#{super_message}\nInvalid object: #{@invalid_object.inspect}"
        end
      end
    end

    Fragment = Struct.new(:json) do
      def initialize(json)
        unless string = String.try_convert(json)
          raise TypeError, "no implicit conversion of #{json.class} into String"
        end

        super(string)
      end

      def to_json(state = nil, *)
        json
      end
    end

    module ParserOptions # :nodoc:
      class << self
        def on_load(on_load, object_class, array_class)
          on_load = object_class_proc(object_class, on_load) if object_class
          on_load = array_class_proc(array_class, on_load) if array_class
          on_load
        end

        private

        def object_class_proc(object_class, on_load)
          ->(obj) do
            if Hash === obj
              object = object_class.new
              obj.each { |k, v| object[k] = v }
              obj = object
            end
            on_load.nil? ? obj : on_load.call(obj)
          end
        end

        def array_class_proc(array_class, on_load)
          ->(obj) do
            if Array === obj
              array = array_class.new
              obj.each { |v| array << v }
              obj = array
            end
            on_load.nil? ? obj : on_load.call(obj)
          end
        end
      end
    end
    private_constant :ParserOptions

    class Coder
      PARSER_OPTIONS = %i(
        max_nesting
        allow_nan
        allow_trailing_comma
        allow_comments
        allow_control_characters
        allow_invalid_escape
        symbolize_names
        freeze
        allow_duplicate_key
        decimal_class
      ).freeze
      private_constant :PARSER_OPTIONS

      EXCLUDED_GENERATOR_OPTIONS = (PARSER_OPTIONS - %i(
        max_nesting
        allow_nan
        allow_duplicate_key
      )).freeze
      private_constant :EXCLUDED_GENERATOR_OPTIONS

      def initialize(object_class: nil, array_class: nil, on_load: nil, strict: true, **options, &as_json)
        raise ArgumentError, "JSON::Coder requires strict mode" if !strict && JSON_VERSION3

        if object_class || array_class
          on_load = ParserOptions.on_load(on_load, object_class, array_class)
        end

        parser_options = options.slice(*PARSER_OPTIONS)
        parser_options[:on_load] = on_load if on_load
        @parser_config = Ext::Parser::Config.new(parser_options).freeze

        generator_options = options.reject { |k, _| EXCLUDED_GENERATOR_OPTIONS.include?(k) }
        @state = State.new(
          **generator_options,
          strict: strict,
          as_json: as_json || generator_options[:as_json],
        ).freeze
      end

      def dump(object, io = nil)
        @state.generate(object, io)
      end
      alias_method :generate, :dump

      def load(source)
        @parser_config.parse(source)
      end
      alias_method :parse, :load

      def load_file(path)
        load(File.read(path, encoding: Encoding::UTF_8))
      end
    end

    module Ext
      class << self
        def valid_encoding?(str) # :nodoc:
          str.ascii_only? || (str.encoding == Encoding::UTF_8 && str.valid_encoding?)
        end

        def coerce_key(key, as_json) # :nodoc:
          as_json_called = false

          unless String === key || Symbol === key
            key = as_json.call(key, true)
            as_json_called = true
          end

          if Symbol === key
            key = key.to_s
          end

          unless String === key
            raise GeneratorError.new("#{key.class} not allowed as object key in JSON", key)
          end

          unless valid_encoding?(key)
            if as_json_called
              raise GeneratorError.new("source sequence is illegal/malformed utf-8", key)
            end

            key = as_json.call(key, true)
            unless String === key
              raise GeneratorError.new("#{key.class} not allowed as object key in JSON", key)
            end

            unless Ext.valid_encoding?(key)
              raise GeneratorError.new("source sequence is illegal/malformed utf-8", key)
            end
          end

          key
        end

        def on_duplicate_key(hash, as_json) # :nodoc:
          set = {}
          hash.each_key do |key|
            key = coerce_key(key, as_json)
            if set[key]
              raise GeneratorError.new("detected duplicate key #{key.inspect}", key)
            else
              set[key] = true
            end
          end
        end
      end

      Generator::State.default_sort_keys_proc = ->(hash) { hash.sort.to_h }

      # RFC 8785 (JSON canonicalization): ECMAScript number formatting, keys in UTF-16 order
      Generator::State.rfc8785_number_formatter_proc = ->(num) {
        if Integer === num
          if num > Float::MAX || num < -Float::MAX
            raise GeneratorError.new("Integer out of range for RFC 8785", num)
          end
          num = num.to_f
        end
        if num.zero?
          "0"
        else
          if num < 0
            num, sign = -num, '-'
          end
          native_rep = num.to_s
          decimal, exponential = native_rep.split('e')
          exp_val = exponential.to_i
          exponential = exp_val > 0 ? ('+' + exp_val.to_s) : exp_val.to_s

          integral, fractional = decimal.split('.')
          fractional = fractional.sub(/0+$/, '')

          if exp_val > 0 && exp_val < 21
            while exp_val > 0
              integral += fractional.to_s[0] || '0'
              fractional = fractional.to_s[1..-1]
              exp_val -= 1
            end
            exponential = nil
          elsif exp_val == 0
            exponential = nil
          elsif exp_val < 0 && exp_val > -7
            fractional, integral, exponential = integral + fractional.to_s, '0', nil
            fractional = ("0" * (-exp_val - 1)) + fractional
          end

          fractional = nil if fractional.to_s.empty?
          sign.to_s + integral + (fractional ? ".#{fractional}" : '') + (exponential ? "e#{exponential}" : '')
        end
      }

      Generator::State.rfc8785_sort_keys_proc = ->(hash, as_json) {
        converted = if as_json
          hash.transform_keys { |k| Ext.coerce_key(k, as_json) }
        else
          hash.transform_keys(&:to_s)
        end

        if converted.size < hash.size
          Ext.on_duplicate_key(hash, as_json)
        end

        converted.sort_by { |k,| k.encode(Encoding::UTF_16) }.to_h
      }
    end
  end
end

if (json_version <=> [2, 12]) >= 0
  class JSON::ParserError
    attr_reader :line
    attr_reader :column
  end
end

if (json_version <=> [3, 0]) >= 0
  module JSON
    class ParserError
      # The location of the error in the document, as a JSONPath-style string ("$.a[1]").
      def json_path
        return @json_path if String === @json_path

        if Array === @json_path
          path = build_json_path(@json_path)
          @json_path = path unless frozen?
          return path
        end
      end

      private

      def build_json_path(segments)
        error = false
        path = segments.filter_map do |segment|
          next if error

          case segment
          when Integer
            "[#{segment}]"
          when String, Symbol
            if !segment.valid_encoding?
              segment = segment.b.gsub(/["\\]/n, { '"' => '\\"', '\\' => '\\\\' }).force_encoding(Encoding::UTF_8)
              %{["#{segment}"]}
            elsif segment.match?(/\A[a-zA-Z\$\_][a-zA-Z\$\_0-9]*\z/)
              ".#{segment}"
            else
              segment = segment.to_s.gsub(/["\\]/, { '"' => '\\"', '\\' => '\\\\' })
              %{["#{segment}"]}
            end
          else
            error = true
            nil
          end
        end.join
        "$#{path}".freeze
      end
    end

    class << self
      def load_file(filespec, **options)
        parse(File.read(filespec, encoding: Encoding::UTF_8), **options)
      end

      def load_file!(filespec, **options)
        parse!(File.read(filespec, encoding: Encoding::UTF_8), **options)
      end

      def unsafe_load(source, proc = nil, **options)
        load(source, proc, max_nesting: false, **options)
      end

      def load(source, proc = nil, allow_blank: true, **options)
        unless source.is_a?(String)
          if source.respond_to? :to_str
            source = source.to_str
          elsif source.respond_to? :read
            source = source.read
          elsif source.respond_to? :to_io
            source = source.to_io.read
          end
        end

        if allow_blank && (source.nil? || (String === source && source.empty?))
          source = 'null'
        end

        if proc
          parse(source, allow_nan: true, on_load: proc.to_proc, **options)
        else
          parse(source, allow_nan: true, **options)
        end
      end

      def dump(obj, anIO = nil, _deprecated_limit = nil, kwargs = nil)
        if kwargs.nil?
          if _deprecated_limit.nil?
            if anIO.is_a?(Hash)
              kwargs = anIO
              anIO = nil
            end
          elsif _deprecated_limit.is_a?(Hash)
            kwargs = _deprecated_limit
            _deprecated_limit = nil
          end
        end

        unless anIO.nil?
          if anIO.respond_to?(:to_io)
            anIO = anIO.to_io
          elsif _deprecated_limit.nil? && !anIO.respond_to?(:write)
            anIO, _deprecated_limit = nil, anIO
          end
        end

        opts = {
          allow_nan: !(kwargs && kwargs[:rfc8785]),
        }
        opts[:max_nesting] = _deprecated_limit if _deprecated_limit
        opts.merge!(kwargs) if kwargs

        State.generate(obj, opts, anIO)
      end
    end
  end
end
