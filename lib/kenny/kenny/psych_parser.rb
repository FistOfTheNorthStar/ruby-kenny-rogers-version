# frozen_string_literal: true
# Kenny: Psych::Parser, the event parser behind Psych's tree API (Psych::TreeBuilder,
# Psych::Nodes, Psych.parse ...), as psych/parser.rb with the libyaml-based native part
# replaced by the runtime's YAML parser (Psych.__kn_events, crates/kenny-runtime/src/yaml.rs).
# It sends a Psych::Handler the events libyaml's parser would, each after `event_location`.
module Psych
  class Parser
    # (the encodings of libyaml's start_stream event)
    ANY = 0
    UTF8 = 1
    UTF16LE = 2
    UTF16BE = 3

    class Mark < Struct.new(:index, :line, :column)
    end

    attr_accessor :handler
    attr_writer :external_encoding

    def initialize(handler = Handler.new)
      @handler = handler
      @external_encoding = ANY
      @mark = Mark.new(0, 0, 0)
    end

    def parse(yaml, path = yaml.respond_to?(:path) ? yaml.path : "<unknown>")
      yaml = yaml.read if yaml.respond_to?(:read)
      Psych.__kn_events(yaml.to_str, path).each do |name, start_line, start_column, end_line, end_column, *args|
        # (a syntax error, after the events before it: file, line, column, offset, problem, context)
        raise SyntaxError.new(start_line, start_column, end_line, end_column, *args) if name == "__error"
        @mark = Mark.new(0, end_line, end_column)
        @handler.event_location(start_line, start_column, end_line, end_column)
        @handler.__send__(name, *args)
      end
      self
    end

    def mark
      @mark
    end
  end
end
