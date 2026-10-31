require "bundler/setup"
require "ox"

xml = <<~XML
  <?xml version="1.0" encoding="UTF-8"?>
  <catalog xmlns:x="urn:x">
    <!-- a comment -->
    <book id="b1" lang="en"><title>Gambler</title><price currency="USD">9.99</price></book>
    <book id="b2"><title>Islands &amp; Streams</title><price>12.50</price><x:note><![CDATA[<raw>]]></x:note></book>
  </catalog>
XML
doc = Ox.parse(xml)
p doc.class, doc.root.name, doc.root.nodes.size
cat = doc.locate("catalog").first
books = cat.locate("book")
p books.map { |b| b[:id] }, books.map { |b| b.title.text }, books[0].price[:currency]
p doc.locate("catalog/book/@id"), doc.locate("*/book/price").map(&:text)
p cat.book.attributes, books[1].locate("x:note").first.nodes.first.class
puts Ox.dump(doc, indent: 2)

d = Ox::Document.new(version: "1.0")
top = Ox::Element.new("top")
top[:a] = "1"
kid = Ox::Element.new("kid")
kid << "text <&>" << Ox::Element.new("empty")
top << kid << Ox::Comment.new(" c ") << Ox::CData.new("cd")
d << top
puts Ox.dump(d)
puts Ox.dump(top, indent: -1)

h = Ox.load('<r><a x="1">one</a><a>two</a><b/></r>', mode: :hash)
p h
p Ox.load('<r><a x="1">one</a><b><c>3</c></b></r>', mode: :hash_no_attrs)
p Ox.load("<r a='q'>x</r>", mode: :generic).attributes

Thing = Struct.new(:name, :nums, :flag)
class Obj
  attr_accessor :a, :b
  def initialize(a = nil, b = nil) = (@a, @b = a, b)
end
o = Obj.new([1, 2.5, "s", :sym, nil, true], { "k" => Obj.new(3, nil) })
s = Ox.dump(o)
puts s
back = Ox.load(s, mode: :object)
p back.class, back.a, back.b["k"].a
p Ox.load(Ox.dump([1, { x: 2 }, "y"]), mode: :object)

class Sax < Ox::Sax
  attr_reader :events
  def initialize = @events = []
  def start_element(name) = @events << [:start, name]
  def end_element(name) = @events << [:end, name]
  def attr(name, value) = @events << [:attr, name, value]
  def text(value) = @events << [:text, value]
  def cdata(value) = @events << [:cdata, value]
end
sax = Sax.new
Ox.sax_parse(sax, StringIO.new('<a k="v"><b>hi</b><![CDATA[x]]></a>'))
p sax.events
begin
  Ox.parse("<a><b></a>")
rescue Ox::ParseError => e
  puts "ParseError: #{e.message}"
end
p Ox.load("<a><b></a>", mode: :generic, effort: :tolerant).nodes.map(&:value) rescue p $!.class
p Ox.default_options[:mode], Ox::VERSION.class
