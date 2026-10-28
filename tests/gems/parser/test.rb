require "bundler/setup"
require "parser/current"

src = <<~RUBY
  class Greeter < Base
    attr_reader :name
    def initialize(name, greeting: "Hello")
      @name = name
      @greeting = greeting
    end

    def greet(*others, &blk)
      list = [name, *others].map { |n| "\#{@greeting}, \#{n}!" }
      list.each(&blk) if blk
      list.size > 1 ? list : list.first
    end
  end
  x = { a: 1, "b" => 2.5, c: :sym, d: 1..3, e: /re/i, f: nil }
  case x
  in { a: Integer => n } then puts n
  end
RUBY

ast, comments = Parser::CurrentRuby.parse_with_comments(src + "# trailing comment\n")
puts ast.to_sexp.lines.first(12).join
p comments.map(&:text)
p ast.type, ast.children.size

class Counter < Parser::AST::Processor
  attr_reader :counts
  def initialize = (@counts = Hash.new(0); super)
  def on_send(node) = (@counts[node.children[1]] += 1; super)
  def on_def(node) = (@counts[:"def #{node.children[0]}"] += 1; super)
end
c = Counter.new
c.process(ast)
p c.counts.sort_by { |k, _| k.to_s }

defn = ast.children[0].children[2].children[2]
p defn.type, defn.loc.line, defn.loc.keyword.source, defn.loc.name.source, defn.loc.expression.source.lines.size

buffer = Parser::Source::Buffer.new("(example)", source: "foo(1, 2)\nbar.baz = 3\n")
tree = Parser::CurrentRuby.new.parse(buffer)
rewriter = Parser::Source::TreeRewriter.new(buffer)
tree.children.each do |n|
  rewriter.replace(n.loc.selector, "FOO") if n.children[1] == :foo
  rewriter.insert_after(n.loc.expression, " # was a setter") if n.type == :send && n.children[1] == :baz=
end
puts rewriter.process

begin
  Parser::CurrentRuby.parse("def broken(")
rescue Parser::SyntaxError => e
  p e.message, e.diagnostic.location.line
end
diags = []
pr = Parser::CurrentRuby.new
pr.diagnostics.all_errors_are_fatal = false
pr.diagnostics.consumer = ->(d) { diags << [d.level, d.reason] }
pr.parse(Parser::Source::Buffer.new("w", source: "a = 1\nif (b = 2) then end\n"))
p diags
p Parser::CurrentRuby.parse("1 + 2 * 3") == Parser::AST::Node.new(:send, [Parser::AST::Node.new(:int, [1]), :+, Parser::AST::Node.new(:send, [Parser::AST::Node.new(:int, [2]), :*, Parser::AST::Node.new(:int, [3])])])
