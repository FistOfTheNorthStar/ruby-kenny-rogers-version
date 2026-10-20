# frozen_string_literal: true

# SassC in a Kenny binary: the sassc gem's Ruby API (SassC::Engine, SassC::Importer, the
# SassC::Script values and custom functions) on a native Sass compiler — grass, in the runtime
# (crates/kenny-runtime/src/assets.rs) — instead of libsass through FFI. The CSS is the same up
# to formatting. Imports are resolved by the compiler on the load paths (a custom importer such
# as sassc-rails' only learns the imported files afterwards, as dependencies), so the ERB and
# glob imports of sassc-rails' importer are not supported.

module SassC
  # The global load paths for Sass files (see the sassc gem).
  def self.load_paths
    @load_paths ||= if ENV['SASS_PATH']
                      ENV['SASS_PATH'].split(SassC::Util.windows? ? ';' : ':')
                    else
                      []
                    end
  end
end

require "sassc/version"
require "sassc/importer"
require "sassc/util"
require "sassc/util/normalized_map"
require "sassc/script"
require "sassc/script/value"
require "sassc/script/value/bool"
require "sassc/script/value/number"
require "sassc/script/value/color"
require "sassc/script/value/string"
require "sassc/script/value/list"
require "sassc/script/value/map"
require "sassc/script/functions"
require "sassc/dependency"
require "sassc/error"

begin
  require "kenny_assets"
rescue LoadError
  # not linked in: rendering raises NotImplementedError
end

module SassC
  class Sass2Scss
    def self.convert(_sass)
      raise NotImplementedError, "SassC::Sass2Scss is not available in a Kenny binary (grass compiles the indented syntax directly)"
    end
  end

  class Engine
    OUTPUT_STYLES = %i[
      sass_style_nested
      sass_style_expanded
      sass_style_compact
      sass_style_compressed
    ]

    attr_reader :template, :options

    def initialize(template, options = {})
      @template = template
      @options = options
      @functions = options.fetch(:functions, Script::Functions)
    end

    def render
      return @template.dup if @template.empty?
      unless defined?(::Kenny::Sass)
        raise NotImplementedError, "this binary was built without the Sass compiler (kenny build links it in when the sassc gem is used)"
      end

      names = Script.custom_functions(functions: @functions)
      specs = names.map { |n| [n.to_s, @functions.instance_method(n).parameters.map { |_, p| p.to_s }] }
      wrapper = functions_wrapper
      options = @options
      callback = lambda do |index, args|
        begin
          values = args.map { |a| Engine.from_kenny(a, options) }.compact
          result = wrapper.send(names[index], *values) || Script::Value::String.new("")
          result.options = options if result.respond_to?(:options=)
          Engine.to_kenny(result)
        rescue StandardError => e
          ["error", e.message]
        end
      end

      status, css, line_or_deps, file = ::Kenny::Sass.render(
        @template.to_s, filename&.to_s, sass? ? "sass" : "scss", output_style.to_s.sub("sass_style_", ""),
        load_path_list, specs, callback
      )
      raise SyntaxError.new(css, filename: file || filename, line: line_or_deps) if status == "error"

      @dependencies = line_or_deps
      @source_map = nil
      if (context = @options.dig(:sprockets, :context)) && context.respond_to?(:depend_on)
        @dependencies.each { |dep| context.depend_on(dep) }
      end

      css.force_encoding(@template.encoding)
      return css unless quiet?
    end

    def dependencies
      raise NotRenderedError unless @dependencies
      Dependency.from_filenames(@dependencies)
    end

    # Source maps are not generated.
    def source_map
      raise NotRenderedError unless @source_map
      @source_map
    end

    def filename
      @options[:filename]
    end

    # A tagged array from the native compiler -> a SassC::Script::Value (nil for null).
    def self.from_kenny(a, options)
      value = case a[0]
              when "s" then Script::Value::String.new(a[1], a[2] ? :string : :identifier)
              when "n"
                num, den = a[2].to_s.split("/", 2)
                Script::Value::Number.new(a[1], num.to_s.split("*"), den.to_s.split("*"))
              when "c" then Script::Value::Color.new(red: a[1], green: a[2], blue: a[3], alpha: a[4])
              when "b" then Script::Value::Bool.new(a[1])
              when "l"
                sep = a[2] == "comma" ? :comma : :space
                Script::Value::List.new(a[1].map { |x| from_kenny(x, options) }.compact, separator: sep, bracketed: a[3])
              when "m" then Script::Value::Map.new(a[1].to_h { |k, v| [from_kenny(k, options), from_kenny(v, options)] })
              end
      value.options = options if value
      value
    end

    # A SassC::Script::Value -> the native compiler's tagged array.
    def self.to_kenny(v)
      case v
      when nil then ["null"]
      when Script::Value::String then ["s", v.value.to_s, v.type != :identifier]
      when Script::Value::Number
        units = v.numerator_units.join("*")
        units += "/" + v.denominator_units.join("*") unless v.denominator_units.empty?
        ["n", v.value.to_f, units]
      when Script::Value::Color
        v.red ? ["c", v.red, v.green, v.blue, v.alpha] : ["s", v.to_s, false]
      when Script::Value::Bool then ["b", v.value]
      when Script::Value::List then ["l", v.to_a.map { |x| to_kenny(x) }, v.separator.to_s, v.bracketed ? true : false]
      when Script::Value::Map then ["m", v.to_h.map { |k, x| [to_kenny(k), to_kenny(x)] }]
      else raise UnsupportedValue, "Sass return type #{v.class.name.split('::').last} unsupported"
      end
    end

    private

    def functions_wrapper
      # an anonymous class, as in sassc's FunctionsHandler (no shared state between threads)
      functions = @functions
      wrapper = Class.new do
        attr_accessor :options
        include functions
      end.new
      wrapper.options = @options
      wrapper
    end

    def quiet?
      @options[:quiet]
    end

    def sass?
      @options[:syntax] && @options[:syntax].to_sym == :sass
    end

    def output_style
      @output_style ||= begin
        style = @options.fetch(:style, :sass_style_nested).to_s
        style = "sass_style_#{style}" unless style.include?("sass_style_")
        style = style.to_sym
        raise InvalidStyleError unless OUTPUT_STYLES.include?(style)
        style
      end
    end

    def load_path_list
      ((@options[:load_paths] || []) + SassC.load_paths).map(&:to_s)
    end
  end
end
