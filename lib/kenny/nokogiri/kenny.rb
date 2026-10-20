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
