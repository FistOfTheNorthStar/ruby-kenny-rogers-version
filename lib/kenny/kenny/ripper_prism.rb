# frozen_string_literal: true
#
# Ripper's parser under Kenny (see ripper.so, lib/kenny/ripper_ext.rb): Prism parses, and
# Prism::Translation::Ripper walks the tree dispatching Ripper's events. `kenny build` uses this
# file when the program's prism Ruby library matches the libprism Kenny links.
require "prism"

class Ripper
  # A Prism::Translation::Ripper whose events go to a ::Ripper object (a SexpBuilder, a user's
  # subclass ...), which sees the event's position through its own `lineno` / `column`.
  class KennyParser < ::Prism::Translation::Ripper
    def initialize(target, source, filename, lineno)
      super(source, filename, lineno)
      @kenny_target = target
      @kenny_line = lineno
    end

    ::Prism::Translation::Ripper::EVENTS.each do |event|
      name = :"on_#{event}"
      # (a Ripper without the stdlib's ripper/core.rb has no handlers: the event's value is its
      # first argument, as Ripper's default handlers return)
      define_method(name) do |*args|
        t = @kenny_target
        t.respond_to?(name, true) ? t.__send__(name, *args) : args[0]
      end
      private name
    end

    def kenny_end_seen?
      !result.data_loc.nil?
    end

    private

    def warn(fmt, *args)
      @kenny_target.__send__(:warn, fmt, *args)
    end

    def warning(fmt, *args)
      @kenny_target.__send__(:warning, fmt, *args)
    end

    def compile_error(msg)
      @kenny_target.__send__(:compile_error, msg)
    end

    def dedent_string(string, width)
      @kenny_target.__send__(:dedent_string, string, width)
    end

    # (as Prism::Translation::Ripper's, with the file name and first line number)
    def result
      @result ||= ::Prism.parse(source, filepath: filename, line: @kenny_line, partial_script: true, version: "current")
    end
  end

  class Lexer
    private

    # Ripper::Lexer#parse: Prism.lex_compat's tokens as Elems; syntax errors as on_parse_error
    # Elems in `errors`.
    def __kenny_lex(raise_errors)
      @errors = []
      @stack = []
      @buf = []
      result = ::Prism.lex_compat(__kenny_source, filepath: filename, line: @__lineno, version: "current")
      result.errors.each do |e|
        loc = e.location
        @errors << Elem.new([loc.start_line, loc.start_column], :on_parse_error, loc.slice, State.new(EXPR_NONE), e.message)
      end
      if raise_errors && !@errors.empty?
        raise SyntaxError, @errors.map(&:message).join(" ;")
      end
      result.value.map do |t|
        Elem.new(t.location.dup, t.event, t.value, State.new(t.state.to_int))
      end
    end
  end
end
