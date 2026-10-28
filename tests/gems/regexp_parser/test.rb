require "bundler/setup"
require "regexp_parser"

re = /(?<year>\d{4})-(\d{2})(?:-(?<day>\d\d))?[a-z&&[^aeiou]]+\p{Alpha}*?\z|foo{2,3}+/ix
tree = Regexp::Parser.parse(re)
p tree.class, tree.to_s == re.source
def walk(node, depth = 0)
  node.each do |child|
    q = child.quantified? ? " #{child.quantifier.text}" : ""
    puts "#{"  " * depth}#{child.type}:#{child.token} #{child.text.inspect}#{q}"
    walk(child, depth + 1) if child.respond_to?(:each) && !child.is_a?(Regexp::Expression::CharacterSet::Range)
  end
end
walk(tree)
p Regexp::Scanner.scan(/a+[b-d]\w/).map { |t, k, txt, s, e| [t, k, txt, s, e] }
p Regexp::Lexer.lex("(a|b)*").map(&:to_s)
p tree.each_expression.count
p tree.each_expression.select { |e, _| e.is_a?(Regexp::Expression::Group::Named) }.map { |e, _| e.name }
p tree.strfre_tree("%a %t").lines.first(3)
p Regexp::Parser.parse("a{2,}", "ruby/3.1")[0].quantifier.then { |q| [q.min, q.max, q.mode] }
p tree.options
begin
  Regexp::Parser.parse("(abc")
rescue Regexp::Parser::Error => e
  p e.class, e.message
end
p Regexp::Parser.parse(/\h+\R\X/).map(&:token)
p Regexp::Parser.parse(/[[:alpha:]]/)[0][0].token
