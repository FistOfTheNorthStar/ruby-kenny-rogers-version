require "bundler/setup"
require "rexml/document"
require "rexml/xpath"
require "rexml/streamlistener"

xml = <<~XML
  <?xml version="1.0" encoding="UTF-8"?>
  <library name="city">
    <!-- a comment -->
    <book id="1" lang="en"><title>Ruby</title><price>29.99</price></book>
    <book id="2" lang="fi"><title>Kalevala</title><price>9.50</price></book>
    <book id="3" lang="en"><title>Rust &amp; You</title><price>39.00</price><![CDATA[<raw>]]></book>
  </library>
XML
doc = REXML::Document.new(xml)
root = doc.root
p root.name, root.attributes["name"], doc.version, doc.encoding
p root.elements.size, root.get_elements("book").size
root.each_element("book") { |b| puts "#{b.attributes["id"]}: #{b.elements["title"].text} #{b.elements["price"].text}" }
p REXML::XPath.match(doc, "//book[@lang='en']/title").map(&:text)
p REXML::XPath.first(doc, "//book[price > 20]/@id").value
p REXML::XPath.match(doc, "count(//book)"), REXML::XPath.each(doc, "//title/text()").map(&:value)
p REXML::XPath.first(doc, "string(/library/book[last()]/title)")
p doc.elements["//book[2]"].attributes.to_a.map { |a| [a.name, a.value] }
p root.elements[3].cdatas.map(&:value)
p root.comments.map(&:string)

new_book = root.add_element("book", { "id" => "4", "lang" => "de" })
new_book.add_element("title").text = "Faust <1>"
new_book.add_attribute("year", "1808")
root.delete_element("book[@id='2']")
out = +""
doc.write(out)
puts out
f = REXML::Formatters::Pretty.new(2)
f.compact = true
out = +""
f.write(root.elements["book[@id='4']"], out)
puts out

d2 = REXML::Document.new
d2 << REXML::XMLDecl.new("1.0", "UTF-8")
e = d2.add_element("config")
e.add_element("item", "k" => "a").add_text("x & y")
e.add_element("empty")
s = +""
d2.write(output: s, indent: 2)
puts s

listener = Class.new do
  include REXML::StreamListener
  attr_reader :events
  def initialize = @events = []
  def tag_start(n, a) = @events << [:start, n, a]
  def tag_end(n) = @events << [:end, n]
  def text(t) = (@events << [:text, t] unless t.strip.empty?)
end.new
REXML::Document.parse_stream("<a x='1'><b>hi</b><c/></a>", listener)
p listener.events
begin
  REXML::Document.new("<a><b></a>")
rescue REXML::ParseException => e
  puts "ParseException: #{e.message.lines.first.strip}"
end
p REXML::Text.new("a < b & c").to_s, REXML::Text.unnormalize("&lt;&amp;&#65;")
