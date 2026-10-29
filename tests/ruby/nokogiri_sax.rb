# Nokogiri's SAX parsers. The expected output is CRuby's with nokogiri 1.19.4 (libxml2 2.13):
# the same callbacks with the same arguments, text split where libxml2 splits it, libxml2's
# error messages. (HTML SAX walks the parsed tree, so text isn't split at entities there.)
require "nokogiri"
require "stringio"
require "tmpdir"

class Log < Nokogiri::XML::SAX::Document
  attr_reader :ev

  def initialize
    @ev = []
  end

  %i[xmldecl start_document end_document start_element end_element characters comment warning error cdata_block processing_instruction reference].each do |m|
    define_method(m) { |*a| @ev << [m, *a] }
  end

  def start_element_namespace(name, attrs = [], prefix = nil, uri = nil, ns = [])
    @ev << [:start_ns, name, attrs.map(&:to_a), prefix, uri, ns]
    super
  end

  def end_element_namespace(name, prefix = nil, uri = nil)
    @ev << [:end_ns, name, prefix, uri]
    super
  end
end

def run(label, src, html: false, &blk)
  d = Log.new
  begin
    (html ? Nokogiri::HTML4::SAX::Parser : Nokogiri::XML::SAX::Parser).new(d).parse(src, &blk)
  rescue => e
    d.ev << [:raised, e.class, e.message]
  end
  puts "== #{label}"
  d.ev.each { |e| p e }
end

run "simple", "<a><b/><c><d/></c></a>"
run "decl", %(<?xml version="1.0" encoding="UTF-8"?>\n<r a="1">x</r>\n)
run "decl standalone", %(<?xml version="1.0" standalone="yes"?><r/>)
run "decl bad", %(<?xml encoding="UTF-8"?><r/>)
run "ns", %(<root xmlns="urn:d" xmlns:p="urn:p"><p:item p:id="1" plain="2">t</p:item><other/></root>)
run "nested ns", %(<a xmlns:p="u1"><p:b xmlns:p="u2"><p:c/></p:b><p:d/></a>)
run "xml prefix", %(<r xml:lang="en" xmlns:a="u" a:x="1" b:y="2"/>)
run "undefined prefix", %(<p:a/>)
run "entities", "<r>a &amp; b &#65;&#x42; &lt;x&gt; &quot;&apos; end</r>"
run "declared entity", %(<!DOCTYPE r [<!ENTITY foo "bar &amp; baz"><!ENTITY ext SYSTEM "x.xml">]><r>a &foo; b</r>)
run("replace entities", %(<!DOCTYPE r [<!ENTITY foo "bar"><!ATTLIST r x CDATA "d">]><r>a &foo; b</r>)) { |c| c.replace_entities = true }
run "attlist", %(<!DOCTYPE r [<!ATTLIST r x CDATA "d" y CDATA #IMPLIED z CDATA #FIXED "f"><!ATTLIST s w CDATA 'q'>]><r a="1"><s w="given"/><s/></r>)
run "undefined entity", "<r>a &foo; b</r>"
run "ws", "<r>\n  <a>1</a>\n  <b>2</b>\n</r>"
run "cdata comment pi", "<!DOCTYPE r><r><![CDATA[<x> & y]]><!-- c --><?pi some data?><?bare?></r>"
run "top level", %(<!-- top --><?pi x?><r/><!-- after -->\n)
run "attribute values", %(<r a="x&#10;y" b="p\tq\nr" c='&lt;&amp;'/>)
run("attribute values replaced", %(<r c='&lt;&amp;&#38;'/>)) { |c| c.replace_entities = true }
run "non-ascii", "<r>héllo wörld ✓</r>"
run "long", "<r>#{"abcdefghij" * 40}</r>"
run "long non-ascii", "<r>#{"é" * 400}</r>"
run "mixed long", "<r>#{"✓" * 101}a#{"é" * 3}</r>"
run "newlines", "<r>a\r\nb\rc\td</r>"
run "latin1", "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><r>caf\xE9</r>".b
run "bom", "﻿<r/>"
run "charrefs", "<r>&#38;&#60;</r>"
run "gt in text", "<r>a > b</r>"
puts "== errors"
["<a><b></a>", "<a><b>text", "hello", "<a/><b/>", "<a/>junk", %(<a x="1" x="2"/>), %(<a x="<"/>),
 "<r>x]]>y</r>", "<r>c\u0001</r>", "<r>&#0;</r>", "<r a='1' b=2/>", "<r>a & b</r>", "<r><!-- a -- b --></r>",
 "<1r/>", "<r><!-- abc</r>", "<r><![CDATA[abc</r>", "<r></r x>", "<r a/>", "<r></>", "<r><?xml version='1.0'?></r>",
 " <?xml version=\"1.0\"?><r/>", "<r/><!DOCTYPE r>", ""].each do |src|
  d = Log.new
  begin
    Nokogiri::XML::SAX::Parser.new(d).parse(src)
  rescue => e
    d.ev << [:raised, e.class, e.message]
  end
  p [src, d.ev.reject { |e| %i[start_document end_document].include?(e[0]) }.last]
end
run "io", StringIO.new("<a>io</a>")
run "io empty", StringIO.new("")
run "aws", %(<?xml version="1.0" encoding="UTF-8"?>\n<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>alpha</Name><Contents><Key>a.txt</Key><Size>3</Size></Contents></ListBucketResult>)
run("recovery", "<a><b></a>") { |c| c.recovery = true }
f = File.join(Dir.tmpdir, "kenny_sax_test.xml")
File.write(f, "<f>file</f>")
d = Log.new
Nokogiri::XML::SAX::Parser.new(d).parse_file(f)
puts "== file"
d.ev.each { |e| p e }
File.delete(f)

puts "== html"
run "html", "<p>hi<br>there <b class=x>b</b>", html: true
run "html doc", "<!DOCTYPE html><html><head><title>T</title></head><body><p id=a>x y</p><!-- c --></body></html>", html: true
run "html text only", "just text", html: true

puts "== line and column"
class Where < Nokogiri::XML::SAX::Document
  attr_accessor :c

  def start_document = p([:sd, @c.line, @c.column])
  def start_element(n, a = []) = p([n, @c.line, @c.column])
  def end_element(n) = p(["/" + n, @c.line, @c.column])
  def characters(s) = p([s, @c.line, @c.column])
  def end_document = p([:ed, @c.line, @c.column])
end
w = Where.new
Nokogiri::XML::SAX::Parser.new(w).parse("<a>\n<b x='1'>hi</b>\n</a>") { |c| w.c = c }

puts "== push"
d = Log.new
pp = Nokogiri::XML::SAX::PushParser.new(d)
pp << "<a>te"
d.ev << :after1
pp << "xt<b/"
d.ev << :after2
pp << ">x</a>"
d.ev << :after3
pp.finish
d.ev.each { |e| p e }
d = Log.new
pp = Nokogiri::XML::SAX::PushParser.new(d)
["<", "a", "/>", " "].each { |c| pp << c; d.ev << :w }
pp.finish
d.ev.each { |e| p e }
[["<a>", nil], ["<a></b>", "<c/>"]].each do |a, b|
  d = Log.new
  pp = Nokogiri::XML::SAX::PushParser.new(d)
  [a, b].compact.each { |c| begin; pp << c; rescue => e; d.ev << [e.class, e.message, e.line, e.column, e.level]; end }
  begin
    pp.finish
  rescue => e
    d.ev << [e.class, e.message]
  end
  d.ev.each { |e| p e }
end
p Nokogiri::XML::SAX::PushParser.new.options

puts "== api"
p Nokogiri::XML::SAX::ParserContext.new("<a/>").class
ctx = Nokogiri::XML::SAX::ParserContext.memory("<a/>")
p [ctx.replace_entities, ctx.recovery, ctx.line, ctx.column]
p((ctx.parse_with(1) rescue [$!.class, $!.message]))
p Nokogiri::XML::SAX::Parser.new(Nokogiri::XML::SAX::Document.new).parse("<a/>")
p((Nokogiri::XML::SAX::Parser.new.parse(nil) rescue [$!.class, $!.message]))
class Bare
  def start_element_namespace(*a) = p(a)
end
p((Nokogiri::XML::SAX::Parser.new(Bare.new).parse("<a>t</a>") rescue [$!.class]))
class Boom < Nokogiri::XML::SAX::Document
  def start_element(n, a = []) = (raise "boom #{n}" if n == "b")
  def end_document = p(:never)
end
p((Nokogiri::XML::SAX::Parser.new(Boom.new).parse("<a><b/></a>") rescue [$!.class, $!.message]))
