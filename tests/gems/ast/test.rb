require "bundler/setup"
require "ast"

include AST::Sexp
tree = s(:send, s(:int, 1), :+, s(:send, s(:int, 2), :*, s(:int, 3)))
p tree
puts tree.to_sexp
p tree.type, tree.children.size
p tree == s(:send, s(:int, 1), :+, s(:send, s(:int, 2), :*, s(:int, 3)))
p tree.updated(:csend).type
p tree.to_a.size
p tree.hash == s(:send, s(:int, 1), :+, s(:send, s(:int, 2), :*, s(:int, 3))).hash
p tree.frozen?

class Calc < AST::Processor
  def on_int(node) = node.children[0]
  def on_send(node)
    l, op, r = node.children
    process(l).send(op, process(r))
  end
end
p Calc.new.process(tree)

class Renamer < AST::Processor
  def on_int(node) = node.updated(:integer)
  def on_send(node) = node.updated(nil, node.children.map { |c| c.is_a?(AST::Node) ? process(c) : c })
end
puts Renamer.new.process(tree).inspect

class TypedNode < AST::Node
  def assign_properties(props)
    @line = props[:line]
  end
  attr_reader :line
end
n = TypedNode.new(:lvar, [:x], line: 7)
p n.line, n.to_sexp_array, n.inspect
p n.updated(nil, [:y]).line
