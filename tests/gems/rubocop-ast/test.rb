require "bundler/setup"
require "rubocop-ast"

src = <<~RUBY
  module Shop
    class Cart
      TAX = 0.24
      def initialize(items = [])
        @items = items
      end

      def total
        @items.sum { |i| i.price * i.qty } * (1 + TAX)
      end

      def empty? = @items.empty?
      private def secret; end
    end
  end
  puts Shop::Cart.new.total if $DEBUG
RUBY
ps = RuboCop::AST::ProcessedSource.new(src, 3.3, "cart.rb")
p ps.valid_syntax?, ps.ast.class, ps.lines.size
root = ps.ast
root.each_node(:def) { |d| p [d.method_name, d.arguments.size, d.endless?, d.first_line, d.last_line] }
klass = root.each_descendant(:class).first
p klass.identifier.const_name, klass.parent_module_name rescue p $!
p root.each_descendant(:const).map(&:const_name).uniq
send = root.each_descendant(:send).find { |n| n.method?(:sum) }
p send.receiver.source, send.block_literal?, send.parent.type, send.parent.body.source
p root.each_descendant(:casgn).map { |c| [c.name, c.expression.value] }
p root.each_descendant(:if).map { |n| [n.modifier_form?, n.condition.source] }
p root.each_descendant(:ivasgn).map(&:name)

pat = RuboCop::AST::NodePattern.new("(send (ivar $_) ${:sum :empty?} ...)")
root.each_node(:send) { |n| (m = pat.match(n)) && p(m) }
pat2 = RuboCop::AST::NodePattern.new("(def $_ (args (optarg _ (array))) ...)")
root.each_node(:def) { |n| (m = pat2.match(n)) && p(m) }

module Matchers
  extend RuboCop::AST::NodePattern::Macros
  def_node_matcher :puts_call?, "(send nil? :puts $_)"
  def_node_search :all_ivars, "(ivar $_)"
end
include Matchers
root.each_node(:send) { |n| (r = puts_call?(n)) && p(r.source) }
p all_ivars(root).to_a.uniq

p ps.comments.size, ps.tokens.first(4).map { |t| [t.type, t.text] }
p ps.ast.each_node.count
p RuboCop::AST::ProcessedSource.new("def (", 3.3).valid_syntax?
p RuboCop::AST::ProcessedSource.new("def (", 3.3).diagnostics.map(&:message).first
