# frozen_string_literal: true

# Kenny: Ruby replacements for parts of Nokogiri's C extension that are simplest in Ruby.
# The rest of lib/kenny/nokogiri is Nokogiri 1.15.7's own Ruby layer (MIT License,
# Copyright 2008-2023 Mike Dalessio, Aaron Patterson et al.), lightly adapted where marked
# "Kenny:".

module Nokogiri
  module HTML4
    class ElementDescription
      class << self
        # The description of an HTML 4 element, or nil.
        def [](name)
          desc = DefaultDescriptions[name.to_s.downcase]
          desc && new(desc)
        end
      end

      def initialize(desc) # :nodoc:
        @desc = desc
      end

      def name
        @desc.name
      end

      def inline?
        !!@desc.isinline
      end

      def empty?
        !!@desc.empty
      end

      def sub_elements
        @desc.subelts || []
      end
    end
  end

  module XML
    class Document < Nokogiri::XML::Node
      # The namespace axis isn't supported by Kenny's XPath engine; walk the tree instead.
      def collect_namespaces
        result = {}
        stack = [root].compact
        until stack.empty?
          node = stack.shift
          next unless node.element?

          node.namespace_definitions.each do |ns|
            result[["xmlns", ns.prefix].compact.join(":")] = ns.href if ns.prefix != "xml"
          end
          stack.concat(node.element_children.to_a)
        end
        result
      end
    end
  end
end

# SAX parsing. The native scanner (crates/kenny-runtime/src/nokogiri/sax.rs) keeps its
# state in a ParserContext and calls the handler's methods as it reads.
module Nokogiri
  module XML
    module SAX
      class ParserContext
        attr_accessor :recovery, :replace_entities

        class << self
          def memory(input, encoding = nil)
            unless input.is_a?(String)
              raise TypeError, "wrong argument type #{input.nil? ? "nil" : input.class} (expected String)"
            end
            raise "input string cannot be empty" if input.empty?

            allocate.__kenny_init(input)
          end

          def io(input, encoding = nil)
            allocate.__kenny_init(input)
          end

          def file(filename, encoding = nil)
            allocate.__kenny_init(File.binread(filename))
          end
        end

        def __kenny_init(input) # :nodoc:
          @input = input
          @recovery = false
          @replace_entities = false
          self
        end

        def line
          @__line || 1
        end

        def column
          @__column || 1
        end

        def parse_with(parser)
          raise ArgumentError, "argument must be a Nokogiri::XML::SAX::Parser" unless parser.is_a?(Nokogiri::XML::SAX::Parser)

          handler = parser.document
          if is_a?(Nokogiri::HTML4::SAX::ParserContext)
            src = @input.is_a?(String) ? @input : @input.read.to_s
            __kenny_html(handler, src) unless src.empty?
          elsif @input.is_a?(String)
            __kenny_feed(handler, @input, true)
          else
            while (chunk = @input.read(4096)) && !chunk.empty?
              __kenny_feed(handler, chunk, false)
            end
            __kenny_feed(handler, nil, true)
          end
          nil
        end
      end

      class PushParser
        def initialize_native(sax_parser, file_name) # :nodoc:
          @__ctx = ParserContext.allocate.__kenny_init(nil)
          @__ctx.instance_variable_set(:@__push, true)
          @__options = 0
        end

        def native_write(chunk, last_chunk) # :nodoc:
          chunk = chunk.to_str unless chunk.nil? || chunk.is_a?(String)
          @__ctx.__kenny_feed(@sax_parser.document, chunk, last_chunk)
          if !@__ctx.recovery && (e = @__ctx.__kenny_error)
            err = Nokogiri::XML::SyntaxError.new(e[0])
            code = e[0].start_with?("Opening and ending tag mismatch") ? 76 : 1
            { :@level => e[3], :@line => e[1], :@column => e[2], :@domain => 1, :@code => code }.each do |k, v|
              err.instance_variable_set(k, v)
            end
            raise err
          end
          self
        end

        def options
          @__options
        end

        def options=(opts)
          @__options = opts
          @__ctx.recovery = (opts & Nokogiri::XML::ParseOptions::RECOVER) != 0
        end

        def replace_entities
          @__ctx.replace_entities
        end

        def replace_entities=(v)
          @__ctx.replace_entities = v
        end
      end
    end
  end

  module HTML4
    module SAX
      class PushParser
        def initialize_native(sax_parser, file_name, encoding = nil) # :nodoc:
          @__ctx = ParserContext.allocate.__kenny_init(nil)
          @__buf = +""
        end

        def native_write(chunk, last_chunk) # :nodoc:
          @__buf << chunk if chunk
          if last_chunk && !@__buf.empty?
            src = @__buf
            @__buf = +""
            @__ctx.__kenny_html(@sax_parser.document, src)
          end
          self
        end
      end
    end
  end
end
