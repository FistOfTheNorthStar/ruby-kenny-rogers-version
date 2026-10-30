# frozen_string_literal: true
# Kenny: Psych's tree API (Psych::Nodes, Psych::Visitors, TreeBuilder, Parser, Emitter,
# Stream, ...). As in CRuby, it is psych's own Ruby code (that of the Ruby, or psych gem, the
# program is built against) on two native pieces, which here are Kenny's: Psych::Parser's
# events come from the native YAML parser and Psych::Emitter's documents go through the native
# libyaml emitter port (crates/kenny-runtime/src/yaml.rs). YAML.load / YAML.dump don't come
# here (they are native end to end); lib/kenny/psych.rb autoloads this file on first use of
# one of these constants.
require "date"
require "psych/handler"

module Psych
  # psych.so's Psych::Emitter (libyaml's emitter): collects a document's events and emits it,
  # written to the IO, at its end (when libyaml flushes).
  class Emitter < Handler
    attr_reader :line_width, :indentation, :canonical

    def initialize(io, options = nil)
      @__io = io
      @__state = :stream
      @__open = []
      @__events = []
      @__first = true
      @__open_ended = 0
      @line_width = 0
      @indentation = 2
      @canonical = false
      if options
        self.line_width = options.line_width
        self.indentation = options.indentation
        self.canonical = options.canonical
      end
    end

    def line_width=(width)
      width = Integer(width)
      @line_width = width >= 0 ? width : -1
    end

    def indentation=(level)
      level = Integer(level)
      @indentation = level > 1 && level < 10 ? level : 2
    end

    def canonical=(style)
      @canonical = style == true
    end

    def start_stream(encoding)
      Integer(encoding)
      __kn_expect(:stream_start)
      @indentation = 2 unless @indentation > 1 && @indentation < 10
      @line_width = 80 if @line_width >= 0 && @line_width <= @indentation * 2
      @line_width = 2147483647 if @line_width < 0
      @__first = true
      @__open_ended = 0
      @__state = :doc
      self
    end

    def start_document(version, tag_directives, implicit)
      raise TypeError, "wrong argument type #{__kn_type(version)} (expected Array)" unless Array === version
      raise TypeError, "wrong argument type #{__kn_type(tag_directives)} (expected Array)" unless Array === tag_directives
      __kn_expect(:doc_start)
      @__events = [[0, version, tag_directives, implicit ? true : false]]
      @__state = :root
      self
    end

    def end_document(implicit_end)
      __kn_expect(:doc_end)
      @__events << [1, implicit_end ? true : false]
      out, @__open_ended = Psych.__kn_emit(@__events, [@indentation, @line_width, @__first, @__open_ended])
      @__events = []
      @__first = false
      @__state = :doc
      @__io.write(out) unless out.empty?
      self
    end

    def scalar(value, anchor, tag, plain, quoted, style)
      raise TypeError, "wrong argument type #{__kn_type(value)} (expected String)" unless String === value
      __kn_expect(:node)
      @__events << [2, value, anchor, tag, plain ? true : false, quoted ? true : false, Integer(style)]
      __kn_added
      self
    end

    def start_sequence(anchor, tag, implicit, style)
      __kn_expect(:node)
      @__events << [3, anchor, tag, implicit ? true : false, Integer(style)]
      @__open << 0
      self
    end

    def end_sequence
      __kn_expect(:seq_end)
      @__events << [4]
      @__open.pop
      __kn_added
      self
    end

    def start_mapping(anchor, tag, implicit, style)
      __kn_expect(:node)
      @__events << [5, anchor, tag, implicit ? true : false, Integer(style)]
      @__open << 1
      self
    end

    def end_mapping
      __kn_expect(:map_end)
      @__events << [6]
      @__open.pop
      __kn_added
      self
    end

    def alias(anchor)
      __kn_expect(:node)
      @__events << [7, anchor]
      __kn_added
      self
    end

    def end_stream
      __kn_expect(:stream_end)
      # (a block scalar with trailing empty lines left the stream open-ended)
      @__io.write("...\n") if @__open_ended == 2
      @__open_ended = 0
      @__state = :end
      self
    end

    private

    def __kn_type(v)
      v.nil? ? "nil" : v.class.to_s
    end

    # libyaml's checks of the event order (its emitter raises them as RuntimeErrors).
    # @__open: per open collection, 0 for a sequence, else 1 + the mapping's items so far.
    def __kn_expect(event)
      @__state ||= :stream
      @__open ||= []
      problem =
        case @__state
        when :stream then "expected STREAM-START" unless event == :stream_start
        when :doc then "expected DOCUMENT-START or STREAM-END" unless event == :doc_start || event == :stream_end
        when :end then "expected nothing"
        when :doc_end then "expected DOCUMENT-END" unless event == :doc_end
        else
          top = @__open.last
          ok = event == :node || (event == :seq_end && top == 0) || (event == :map_end && top && top.odd?)
          "expected SCALAR, SEQUENCE-START, MAPPING-START, or ALIAS" unless ok
        end
      raise RuntimeError, problem if problem
    end

    # a node is complete: the document's root, or an item of the collection it is in
    def __kn_added
      if @__open.empty?
        @__state = :doc_end
      elsif @__open.last > 0
        @__open[-1] += 1
      end
    end
  end

  class Parser
    ANY = 0
    UTF8 = 1
    UTF16LE = 2
    UTF16BE = 3
  end

  class ClassLoader
    private

    # rb_path_to_class
    def path2class(path)
      path = path.to_s
      path.split("::").reduce(Object) do |mod, name|
        raise ArgumentError, "undefined class/module #{path}" if name.empty? || !mod.const_defined?(name, false)
        c = mod.const_get(name, false)
        raise TypeError, "#{path} does not refer to class/module" unless Module === c
        c
      end
    end
  end
end

require "psych/class_loader"
require "psych/scalar_scanner"
require "psych/nodes"
require "psych/streaming"
require "psych/visitors"
require "psych/tree_builder"
require "psych/parser"
require "psych/omap"
require "psych/set"
require "psych/stream"
require "psych/json/tree_builder"
require "psych/json/stream"
require "psych/handlers/document_stream"

module Psych
  class Parser
    # The current position: the end of the last event handed to the handler.
    def mark
      Mark.new(*(@__mark || [0, 0, 0]))
    end

    private

    def _native_parse(handler, yaml, path)
      yaml = yaml.read if yaml.respond_to?(:read)
      events, err = Psych.__kn_events(yaml.to_str, path)
      events.each do |name, args, start_line, start_column, end_line, end_column|
        @__mark = [0, end_line, end_column]
        handler.event_location(start_line, start_column, end_line, end_column)
        handler.__send__(name, *args)
      end
      raise Psych::SyntaxError.new(*err) if err
      self
    end
  end

  module Visitors
    class ToRuby
      private

      def build_exception(klass, message)
        Psych.__kn_build_exception(klass, message)
      end

      # (psych 5.3's Data objects: rb_struct_initialize with the members)
      def init_struct(data, attrs = {}, **kw)
        data.__send__(:initialize, **attrs, **kw)
        data
      end
    end
  end
end
