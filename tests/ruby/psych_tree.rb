# Psych's tree API: YAMLTree, Psych::Nodes, TreeBuilder, Parser events, Emitter, ToRuby
require "yaml"
require "stringio"

# YAMLTree -> nodes -> emitter
tree = Psych::Visitors::YAMLTree.create
tree << { "name" => "demo", "list" => [1, 2.5, nil, true, :sym], "text" => "a\nb\n", "q" => "=", "empty" => [], "h" => {} }
ast = tree.tree
p ast.class, ast.children.size, ast.children[0].class, ast.children[0].root.class
puts ast.yaml
puts ast.to_yaml(nil, line_width: 20, indentation: 4)

# a YAMLTree subclass (as rubygems' Gem::NoAliasYAMLTree)
class NoAlias < Psych::Visitors::YAMLTree
  def visit_String(str)
    return super unless str == "="
    @emitter.scalar str, nil, nil, false, true, Psych::Nodes::Scalar::SINGLE_QUOTED
  end

  def register(target, obj)
  end
end
shared = %w[x y]
t = NoAlias.create
t << { "a" => shared, "b" => shared, "eq" => "=" }
io = StringIO.new
Psych::Visitors::Emitter.new(io).accept(t.tree)
puts io.string
puts Psych.dump({ "a" => shared, "b" => shared })

# objects with encode_with / init_with
class Point
  attr_reader :x, :y
  def initialize(x, y) = (@x, @y = x, y)
  def encode_with(c) = (c["x"] = @x; c["y"] = @y)
  def init_with(c) = (@x, @y = c["x"], c["y"])
end
t = Psych::Visitors::YAMLTree.create
t << [Point.new(1, 2), Struct.new(:a).new(3).to_h]
yaml = t.tree.yaml
puts yaml
pt = Psych.unsafe_load(yaml)[0]
p [pt.class, pt.x, pt.y]

# building nodes by hand
doc = Psych::Nodes::Document.new([1, 1], [["!e!", "tag:example.com,2000:"]], false)
seq = Psych::Nodes::Sequence.new(nil, nil, true, Psych::Nodes::Sequence::FLOW)
seq.children << Psych::Nodes::Scalar.new("one") << Psych::Nodes::Scalar.new("tag:example.com,2000:x", nil, "tag:example.com,2000:t", false, false, Psych::Nodes::Scalar::ANY)
map = Psych::Nodes::Mapping.new("anc")
map.children << Psych::Nodes::Scalar.new("k") << seq
map.children << Psych::Nodes::Scalar.new("lit") << Psych::Nodes::Scalar.new("l1\nl2\n", nil, nil, false, true, Psych::Nodes::Scalar::LITERAL)
map.children << Psych::Nodes::Scalar.new("again") << Psych::Nodes::Alias.new("anc")
doc.children << map
stream = Psych::Nodes::Stream.new
stream.children << doc
puts stream.yaml
d2 = Psych::Nodes::Document.new([], [], true)
d2.children << Psych::Nodes::Scalar.new("implicit")
s2 = Psych::Nodes::Stream.new
s2.children << d2
p s2.yaml
d3 = Psych::Nodes::Document.new([], [], false)
d3.implicit_end = false
d3.children << Psych::Nodes::Scalar.new("x")
s2.children << d3
p s2.yaml

# parsing into nodes
src = <<~YAML
  %YAML 1.1
  ---
  base: &base
    a: 1
    b: [x, "y", 'z']
  derived:
    <<: *base
    c: !!str 3
    d: |
      block
  tagged: !ruby/object:Point
    x: 5
    y: 6
  ...
  --- second
YAML
docs = Psych.parse_stream(src)
p docs.class, docs.children.size
d = docs.children[0]
p [d.version, d.implicit, d.implicit_end, docs.children[1].implicit]
root = d.root
p root.class, root.children.size
base = root.children[1]
p [base.anchor, base.tag, base.style, base.children[3].style]
p base.children[3].children.map { |s| [s.value, s.style, s.plain, s.quoted] }
der = root.children[3]
p der.children[1].class, der.children[1].anchor
p der.children[3].tag, der.children[3].value, der.children[5].style
p root.children[5].tag
p [root.start_line, root.start_column, base.start_line]
p Psych.parse("--- foo").root.value
p Psych.parse("").class
ruby = Psych.unsafe_load(src)
p ruby["derived"], ruby["tagged"].x
p d.to_ruby.keys
p Psych.parse("[1, 2.0, true, ~, 0x1f]").to_ruby, Psych.parse("- :sym\n- 2001-02-03\n").to_ruby.map(&:to_s)
begin
  Psych.parse(src).to_yaml
rescue RuntimeError => e
  p e.message
end
puts Psych.parse_stream(src).to_yaml

# the each/ select of nodes
p Psych.parse("a: [1, {b: 2}]").root.each.map(&:class).uniq.size
p Psych.parse("- 1\n- two\n").root.children.map(&:value)

# parse events through a custom handler
class Rec < Psych::Handler
  attr_reader :events
  def initialize = @events = []
  %i[start_stream end_stream start_document end_document scalar start_sequence end_sequence start_mapping end_mapping alias].each do |m|
    define_method(m) { |*a| @events << [m, *a] }
  end
end
r = Rec.new
Psych::Parser.new(r).parse("--- &a\n- !foo 1\n- *a\n- {x: 'y'}\n")
r.events.each { |e| p e }

# syntax errors
begin
  Psych.parse("a: [1, 2\nb: 3", filename: "f.yml")
rescue Psych::SyntaxError => e
  p e.class, e.file
end

# Psych::Emitter directly
io = StringIO.new
em = Psych::Emitter.new(io)
em.start_stream(Psych::Parser::UTF8)
em.start_document([], [], false)
em.start_mapping(nil, nil, true, Psych::Nodes::Mapping::BLOCK)
em.scalar("key", nil, nil, true, false, Psych::Nodes::Scalar::ANY)
em.scalar("value with: colon", nil, nil, true, false, Psych::Nodes::Scalar::ANY)
em.end_mapping
em.end_document(true)
em.end_stream
p io.string

# Psych::Stream
io = StringIO.new
s = Psych::Stream.new(io)
s.start
s.push({ "a" => 1 }); s.push([2, 3])
s.finish
p io.string

# JSON-ish output and Psych.to_json
p Psych.parse_stream("a: 1").class
p Psych.to_json({ "a" => 1, "b" => [1, "x", nil, true], :c => 2.5 })
big = "word " * 30
p Psych.dump(big) == Psych.parse_stream(Psych.dump(big)).to_yaml, Psych::Visitors::YAMLTree.create.tap { |t| t << big }.tree.yaml == Psych.dump(big)
