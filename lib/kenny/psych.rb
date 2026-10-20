# frozen_string_literal: true
# Kenny: Psych (YAML 1.1 for Ruby). Parsing, scalar resolution, Ruby object revival and the
# libyaml-compatible emitter are native (crates/kenny-runtime/src/yaml.rs); this file is the
# public API on top of Psych.__kn_load / Psych.__kn_dump, following psych 5.x.
require "kenny/psych.so"
# Psych::VERSION: the psych of the Ruby (or Gemfile.lock) the program is built against,
# written by `kenny build`. Before psych 4 (Ruby <= 3.0), `load` is a full load.
require "kenny/psych_version"

module Psych
  LIBYAML_VERSION = "0.2.5"

  class Exception < ::RuntimeError
  end

  class BadAlias < Exception
  end

  class AliasesNotEnabled < BadAlias
    def initialize
      super "Alias parsing was not enabled. To enable it, pass `aliases: true` to `Psych::load` or `Psych::safe_load`."
    end
  end

  class AnchorNotDefined < BadAlias
    def initialize(anchor_name)
      super "An alias referenced an unknown anchor: #{anchor_name}"
    end
  end

  class DisallowedClass < Exception
    def initialize(action, klass_name)
      super "Tried to #{action} unspecified class: #{klass_name}"
    end
  end

  class SyntaxError < Exception
    attr_reader :file, :line, :column, :offset, :problem, :context

    def initialize(file, line, col, offset, problem, context)
      err = [problem, context].compact.join(" ")
      filename = file || "<unknown>"
      message = "(#{filename}): #{err} at line #{line} column #{col}"
      @file = file
      @line = line
      @column = col
      @offset = offset
      @problem = problem
      @context = context
      super(message)
    end
  end

  module Nodes
    class Node
    end

    class Scalar < Node
      ANY = 0
      PLAIN = 1
      SINGLE_QUOTED = 2
      DOUBLE_QUOTED = 3
      LITERAL = 4
      FOLDED = 5
    end

    class Sequence < Node
      ANY = 0
      BLOCK = 1
      FLOW = 2
    end

    class Mapping < Node
      ANY = 0
      BLOCK = 1
      FLOW = 2
    end
  end

  class Coder
    attr_accessor :tag, :style, :implicit, :object
    attr_reader :type, :seq

    def initialize(tag)
      @map = {}
      @seq = []
      @implicit = false
      @type = :map
      @tag = tag
      @style = Psych::Nodes::Mapping::BLOCK
      @scalar = nil
      @object = nil
    end

    def scalar(*args)
      if args.length > 0
        @tag, @scalar, _ = args
        @type = :scalar
      end
      @scalar
    end

    def map(tag = @tag, style = @style)
      @tag = tag
      @style = style
      yield self if block_given?
      @map
    end

    def represent_scalar(tag, value)
      self.tag = tag
      self.scalar = value
    end

    def represent_seq(tag, list)
      @tag = tag
      self.seq = list
    end

    def represent_map(tag, map)
      @tag = tag
      self.map = map
    end

    def represent_object(tag, obj)
      @tag = tag
      @type = :object
      @object = obj
    end

    def scalar=(value)
      @type = :scalar
      @scalar = value
    end

    def map=(map)
      @type = :map
      @map = map
    end

    def []=(k, v)
      @type = :map
      @map[k] = v
    end
    alias add []=

    def [](k)
      @type = :map
      @map[k]
    end

    def seq=(list)
      @type = :seq
      @seq = list
    end
  end

  @load_tags = {}
  @dump_tags = {}
  @domain_types = {}

  class << self
    attr_accessor :load_tags, :dump_tags, :domain_types

    def unsafe_load(yaml, filename: nil, fallback: false, symbolize_names: false, freeze: false, strict_integer: false)
      docs = __kn_load(__kn_source(yaml), { filename: filename, unsafe: true, first_only: true,
                                            symbolize_names: symbolize_names, freeze: freeze, strict_integer: strict_integer })
      docs.empty? ? fallback : docs[0]
    end

    def safe_load(yaml, permitted_classes: [], permitted_symbols: [], aliases: false, filename: nil, fallback: nil,
                  symbolize_names: false, freeze: false, strict_integer: false)
      docs = __kn_load(__kn_source(yaml), { filename: filename, first_only: true,
                                            permitted_classes: permitted_classes, permitted_symbols: permitted_symbols,
                                            aliases: aliases, symbolize_names: symbolize_names, freeze: freeze,
                                            strict_integer: strict_integer })
      docs.empty? ? fallback : docs[0]
    end

    def load(yaml, permitted_classes: [Symbol], permitted_symbols: [], aliases: false, filename: nil, fallback: nil,
             symbolize_names: false, freeze: false, strict_integer: false)
      safe_load(yaml, permitted_classes: permitted_classes, permitted_symbols: permitted_symbols, aliases: aliases,
                      filename: filename, fallback: fallback, symbolize_names: symbolize_names, freeze: freeze,
                      strict_integer: strict_integer)
    end

    def load_stream(yaml, filename: nil, fallback: [], **kwargs)
      opts = { filename: filename, unsafe: true }
      opts[:symbolize_names] = kwargs[:symbolize_names] if kwargs.key?(:symbolize_names)
      opts[:freeze] = kwargs[:freeze] if kwargs.key?(:freeze)
      opts[:strict_integer] = kwargs[:strict_integer] if kwargs.key?(:strict_integer)
      docs = __kn_load(__kn_source(yaml), opts)
      if block_given?
        docs.each { |d| yield d }
        return nil
      end
      docs.empty? ? fallback : docs
    end

    def unsafe_load_file(filename, **kwargs)
      unsafe_load(__kn_read(filename), filename: filename, **kwargs)
    end

    def safe_load_file(filename, **kwargs)
      safe_load(__kn_read(filename), filename: filename, **kwargs)
    end

    def load_file(filename, **kwargs)
      load(__kn_read(filename), filename: filename, **kwargs)
    end

    def dump(o, io = nil, options = {})
      if Hash === io
        options = io
        io = nil
      end
      yaml = __kn_dump([o], options)
      return yaml unless io
      io.write(yaml)
      io
    end

    def safe_dump(o, io = nil, options = {})
      if Hash === io
        options = io
        io = nil
      end
      yaml = __kn_dump([o], options.merge(__restricted: true))
      return yaml unless io
      io.write(yaml)
      io
    end

    def dump_stream(*objects)
      __kn_dump(objects, {})
    end

    def to_json(object)
      require "json"
      JSON.generate(object)
    end

    if VERSION.to_i < 4
      # psych 3: `load` revives any class and allows aliases (what e.g. Rails 5.2 expects of
      # database.yml with `<<: *default`); `safe_load` also takes its options positionally.
      def load(yaml, legacy_filename = nil, filename: nil, fallback: false, symbolize_names: false, freeze: false, **)
        unsafe_load(yaml, filename: filename || legacy_filename, fallback: fallback, symbolize_names: symbolize_names, freeze: freeze)
      end

      alias __kn_safe_load5 safe_load
      def safe_load(yaml, legacy_permitted_classes = nil, legacy_permitted_symbols = nil, legacy_aliases = nil, legacy_filename = nil,
                    permitted_classes: [], permitted_symbols: [], aliases: false, filename: nil, fallback: nil,
                    symbolize_names: false, freeze: false, strict_integer: false)
        __kn_safe_load5(yaml, permitted_classes: legacy_permitted_classes || permitted_classes,
                              permitted_symbols: legacy_permitted_symbols || permitted_symbols,
                              aliases: legacy_aliases.nil? ? aliases : legacy_aliases,
                              filename: legacy_filename || filename, fallback: fallback,
                              symbolize_names: symbolize_names, freeze: freeze, strict_integer: strict_integer)
      end

      def load_file(filename, fallback: false, **kwargs)
        load(__kn_read(filename), filename: filename, fallback: fallback, **kwargs)
      end
    end

    def add_tag(tag, klass)
      load_tags[tag] = klass.name
      dump_tags[klass] = tag
    end

    # Registered like psych does; the native loader doesn't call these blocks yet (a
    # document using such a tag loads as the plain mapping/sequence/scalar).
    def add_domain_type(domain, type_tag, &block)
      key = ["tag", domain, type_tag].join(":")
      domain_types[key] = [key, block]
      domain_types["tag:#{type_tag}"] = [key, block]
    end

    def add_builtin_type(type_tag, &block)
      key = ["tag", "yaml.org,2002", type_tag].join(":")
      domain_types[key] = [key, block]
    end

    def remove_type(type_tag)
      domain_types.delete(type_tag)
    end

    private

    def __kn_source(yaml)
      yaml = yaml.read if yaml.respond_to?(:read)
      yaml.to_str
    end

    def __kn_read(filename)
      File.read(filename)
    end
  end
end

class Object
  def self.yaml_tag(url)
    Psych.add_tag(url, self)
  end

  def to_yaml(options = {})
    Psych.dump(self, options)
  end
end
