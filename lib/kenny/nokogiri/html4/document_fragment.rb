# frozen_string_literal: true

module Nokogiri
  module HTML4
    class DocumentFragment < Nokogiri::XML::DocumentFragment
      ####
      # Create a Nokogiri::XML::DocumentFragment from +tags+, using +encoding+
      def self.parse(tags, encoding = nil, options = XML::ParseOptions::DEFAULT_HTML, &block)
        doc = HTML4::Document.new

        encoding ||= if tags.respond_to?(:encoding)
          encoding = tags.encoding
          if encoding == ::Encoding::ASCII_8BIT
            "UTF-8"
          else
            encoding.name
          end
        else
          "UTF-8"
        end

        doc.encoding = encoding

        new(doc, tags, nil, options, &block)
      end

      KENNY_TABLE_PART = /\A\s*<(tr|td|th|tbody|thead|tfoot|caption|colgroup|col)[\s>\/]/i # :nodoc:
      KENNY_TABLE_CONTEXT = { # :nodoc:
        "tr" => "tbody", "td" => "tr", "th" => "tr", "tbody" => "table", "thead" => "table",
        "tfoot" => "table", "caption" => "table", "colgroup" => "table", "col" => "colgroup",
      }.freeze

      def initialize(document, tags = nil, ctx = nil, options = XML::ParseOptions::DEFAULT_HTML) # rubocop:disable Lint/MissingSuper
        return self unless tags

        options = Nokogiri::XML::ParseOptions.new(options) if Integer === options
        yield options if block_given?

        if ctx
          preexisting_errors = document.errors.dup
          # Kenny: html5ever parses in the context element directly (no <div> wrapper, which
          # would lose table parts like <tr> outside a table)
          node_set = ctx.parse(tags, options)
          node_set.each { |child| child.parent = self }
          self.errors = document.errors - preexisting_errors
        elsif (m = KENNY_TABLE_PART.match(tags))
          # Kenny: table parts (<tr>, <td>...) outside a table only survive the HTML5 parsing
          # algorithm in a table context (libxml2 keeps them anywhere)
          context = document.create_element(KENNY_TABLE_CONTEXT[m[1].downcase])
          context.parse(tags, options).each { |child| child.parent = self }
        else
          # This is a horrible hack, but I don't care
          path = if /^\s*?<body/i.match?(tags)
            "/html/body"
          else
            "/html/body/node()"
          end

          temp_doc = HTML4::Document.parse("<html><body>#{tags}", nil, document.encoding, options)
          temp_doc.xpath(path).each { |child| child.parent = self }
          self.errors = temp_doc.errors
        end
        children
      end
    end
  end
end
