require "bundler/setup"
require "racc/parser"
require_relative "calc.tab"

p Racc::Parser::Racc_Runtime_Core_Version_R rescue p $!.class
p Racc::VERSION
calc = Calc.new
["1 + 2 * 3", "(1 + 2) * 3", "-4 * 2 - 10 / 5", "2 * (3 + 4) * 5", ""].each do |src|
  puts "#{src.inspect} = #{calc.parse(src)}"
end
begin
  calc.parse("1 + * 2")
rescue Racc::ParseError => e
  puts "ParseError: #{e.message.strip}"
end
# yyparse-style: tokens from a block-driven lexer
class Calc
  def parse_tokens(tokens)
    yyparse(tokens, :each)
  end
end
p Calc.new.parse_tokens([[:NUMBER, 6], ["*", "*"], [:NUMBER, 7], [false, "$end"]])
