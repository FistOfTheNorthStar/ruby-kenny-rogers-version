# Ripper's native part (ripper.so), as Kenny provides it: Ruby 3.4's event tables and lexer
# state constants, and a parser built on Prism. `Ripper#parse` has Prism::Translation::Ripper
# (the prism gem's drop-in Ripper) walk Prism's tree and send Ripper's events to this Ripper
# object, so the stdlib's subclasses (Ripper::SexpBuilder for `Ripper.sexp`, ActionView's render
# parser ...) work; `Ripper.lex` and Ripper::Lexer use Prism.lex_compat, Prism's Ripper-compatible
# token stream (the tree walk doesn't produce every scanner event). The backend is
# kenny/ripper_prism.rb, which `kenny build` writes: Prism's Ruby library must be the version
# of the libprism Kenny links (see crates/kenny/src/main.rs), else parsing raises
# NotImplementedError.
class Ripper
  Version = "0.1.0"
  PARSER_EVENT_TABLE = {
    :BEGIN => 1, :END => 1, :alias => 2, :alias_error => 2, :aref => 2, :aref_field => 2,
    :arg_ambiguous => 1, :arg_paren => 1, :args_add => 2, :args_add_block => 2, :args_add_star => 2, :args_forward => 0,
    :args_new => 0, :array => 1, :aryptn => 4, :assign => 2, :assign_error => 2, :assoc_new => 2,
    :assoc_splat => 1, :assoclist_from_args => 1, :bare_assoc_hash => 1, :begin => 1, :binary => 3, :block_var => 2,
    :blockarg => 1, :bodystmt => 4, :brace_block => 2, :break => 1, :call => 3, :case => 2,
    :class => 3, :class_name_error => 2, :command => 2, :command_call => 4, :const_path_field => 2, :const_path_ref => 2,
    :const_ref => 1, :def => 3, :defined => 1, :defs => 5, :do_block => 2, :dot2 => 2,
    :dot3 => 2, :dyna_symbol => 1, :else => 1, :elsif => 3, :ensure => 1, :excessed_comma => 0,
    :fcall => 1, :field => 3, :fndptn => 4, :for => 3, :hash => 1, :heredoc_dedent => 2,
    :hshptn => 3, :if => 3, :if_mod => 2, :ifop => 3, :in => 3, :kwrest_param => 1,
    :lambda => 2, :magic_comment => 2, :massign => 2, :method_add_arg => 2, :method_add_block => 2, :mlhs_add => 2,
    :mlhs_add_post => 2, :mlhs_add_star => 2, :mlhs_new => 0, :mlhs_paren => 1, :module => 2, :mrhs_add => 2,
    :mrhs_add_star => 2, :mrhs_new => 0, :mrhs_new_from_args => 1, :next => 1, :nokw_param => 1, :opassign => 3,
    :operator_ambiguous => 2, :param_error => 2, :params => 7, :paren => 1, :parse_error => 1, :program => 1,
    :qsymbols_add => 2, :qsymbols_new => 0, :qwords_add => 2, :qwords_new => 0, :redo => 0, :regexp_add => 2,
    :regexp_literal => 2, :regexp_new => 0, :rescue => 4, :rescue_mod => 2, :rest_param => 1, :retry => 0,
    :return => 1, :return0 => 0, :sclass => 2, :stmts_add => 2, :stmts_new => 0, :string_add => 2,
    :string_concat => 2, :string_content => 0, :string_dvar => 1, :string_embexpr => 1, :string_literal => 1, :super => 1,
    :symbol => 1, :symbol_literal => 1, :symbols_add => 2, :symbols_new => 0, :top_const_field => 1, :top_const_ref => 1,
    :unary => 2, :undef => 1, :unless => 3, :unless_mod => 2, :until => 2, :until_mod => 2,
    :var_alias => 2, :var_field => 1, :var_ref => 1, :vcall => 1, :void_stmt => 0, :when => 3,
    :while => 2, :while_mod => 2, :word_add => 2, :word_new => 0, :words_add => 2, :words_new => 0,
    :xstring_add => 2, :xstring_literal => 1, :xstring_new => 0, :yield => 1, :yield0 => 0, :zsuper => 0
  }.freeze
  SCANNER_EVENT_TABLE = {
    :CHAR => 1, :__end__ => 1, :backref => 1, :backtick => 1, :comma => 1, :comment => 1,
    :const => 1, :cvar => 1, :embdoc => 1, :embdoc_beg => 1, :embdoc_end => 1, :embexpr_beg => 1,
    :embexpr_end => 1, :embvar => 1, :float => 1, :gvar => 1, :heredoc_beg => 1, :heredoc_end => 1,
    :ident => 1, :ignored_nl => 1, :imaginary => 1, :int => 1, :ivar => 1, :kw => 1,
    :label => 1, :label_end => 1, :lbrace => 1, :lbracket => 1, :lparen => 1, :nl => 1,
    :op => 1, :period => 1, :qsymbols_beg => 1, :qwords_beg => 1, :rational => 1, :rbrace => 1,
    :rbracket => 1, :regexp_beg => 1, :regexp_end => 1, :rparen => 1, :semicolon => 1, :sp => 1,
    :symbeg => 1, :symbols_beg => 1, :tlambda => 1, :tlambeg => 1, :tstring_beg => 1, :tstring_content => 1,
    :tstring_end => 1, :words_beg => 1, :words_sep => 1, :ignored_sp => 1
  }.freeze
  EXPR_ARG = 16
  EXPR_ARG_ANY = 48
  EXPR_BEG = 1
  EXPR_BEG_ANY = 577
  EXPR_CLASS = 512
  EXPR_CMDARG = 32
  EXPR_DOT = 256
  EXPR_END = 2
  EXPR_ENDARG = 4
  EXPR_ENDFN = 8
  EXPR_END_ANY = 14
  EXPR_FITEM = 4096
  EXPR_FNAME = 128
  EXPR_LABEL = 1024
  EXPR_LABELED = 2048
  EXPR_MID = 64
  EXPR_NONE = 0
  EXPR_VALUE = 1
  
  def initialize(src, filename = "(ripper)", lineno = 1)
    @__src = src
    @__filename = filename
    @__lineno = lineno
    @__kenny_parser = nil
    @__kenny_error = false
    @__kenny_end_seen = false
  end

  def filename
    @__filename
  end

  def parse
    require "kenny/ripper_prism"
    parser = KennyParser.new(self, __kenny_source, @__filename, @__lineno)
    @__kenny_parser = parser
    begin
      parser.parse
    ensure
      @__kenny_error = parser.error?
      @__kenny_end_seen = parser.kenny_end_seen?
      @__kenny_parser = nil
    end
  end

  def error?
    @__kenny_error
  end

  def end_seen?
    @__kenny_end_seen
  end

  def encoding
    @__src.is_a?(String) ? @__src.encoding : Encoding::UTF_8
  end

  # (the position of the event being dispatched, while parsing)
  def lineno
    @__kenny_parser&.lineno
  end

  def column
    @__kenny_parser&.column
  end

  def state
    nil
  end

  def token
    nil
  end

  attr_accessor :yydebug, :debug_output

  # The source as a String (Ripper also takes an IO, anything with `gets`, or with `to_str`).
  def __kenny_source
    src = @__src
    return src if src.is_a?(String)
    if src.respond_to?(:gets)
      out = +""
      while (line = src.gets)
        out << line
      end
      @__src = out
    elsif src.respond_to?(:to_str)
      @__src = src.to_str
    else
      raise TypeError, "no implicit conversion of #{src.class} into String"
    end
  end
  private :__kenny_source

  LEX_STATE_NAMES = %w[BEG END ENDARG ENDFN ARG CMDARG MID FNAME DOT CLASS LABEL LABELED FITEM].freeze
  private_constant :LEX_STATE_NAMES

  def self.lex_state_name(state)
    names = []
    LEX_STATE_NAMES.each_with_index { |n, i| names << n if state & (1 << i) != 0 }
    names.empty? ? "NONE" : names.join("|")
  end

  # Removes up to `width` columns of leading whitespace from `input` (in place, tabs to the next
  # multiple of 8), returning the number of characters removed: the dedent of a `<<~` heredoc.
  def self.dedent_string(input, width)
    whitespace = 0
    cursor = 0
    while cursor < input.length && input[cursor].match?(/\s/) && whitespace < width
      if input[cursor] == "\t"
        whitespace = ((whitespace / 8 + 1) * 8)
        break if whitespace > width
      else
        whitespace += 1
      end
      cursor += 1
    end
    input.replace(input[cursor..])
    cursor
  end

  private def dedent_string(input, width)
    Ripper.dedent_string(input, width)
  end

  # Ripper::Lexer (defined by the stdlib's ripper/lexer.rb) collects scanner events as the C
  # parser emits them; under Kenny it takes Prism.lex_compat's tokens instead.
  class Lexer < ::Ripper
    module KennyLex
      def parse(raise_errors: false)
        require "kenny/ripper_prism"
        __kenny_lex(raise_errors)
      end
    end
    prepend KennyLex
  end
end
