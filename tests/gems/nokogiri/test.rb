require "bundler/setup"
require "nokogiri"

html = <<~HTML
  <!DOCTYPE html>
  <html><head><title>Test &amp; Page</title><meta charset="utf-8"></head>
  <body>
    <div id="main" class="content wide">
      <h1>Heading</h1>
      <p class="intro">First <b>bold</b> para.</p>
      <p>Second para with <a href="/one">link one</a> and <a href="http://x.example/two" rel="nofollow">two</a>.</p>
      <ul><li>a</li><li class="last">b</li><li>c</li></ul>
      <table><tr><td>1</td><td>2</td></tr></table>
    </div>
    <p>outside<br>break</p>
  </body></html>
HTML
doc = Nokogiri::HTML(html)
p doc.title, doc.at_css("h1").text, doc.css("p").size, doc.css("p.intro b").text
p doc.css("a").map { |a| [a["href"], a.text, a["rel"]] }
p doc.xpath("//li").map(&:text), doc.xpath("//li[@class='last']").first.text, doc.at_xpath("//ul/li[2]/following-sibling::li").text
p doc.xpath("count(//p)"), doc.xpath("string(//h1)"), doc.at_css("#main")["class"], doc.at_css("#main").classes
p doc.css("ul > li:first-child").text, doc.css("li:nth-child(2)").text, doc.css("td").map(&:content)
p doc.at_css("p.intro").inner_html, doc.at_css("p.intro").to_html
div = doc.at_css("#main")
p div.name, div.parent.name, div.children.select(&:element?).map(&:name), div.element_children.size
p doc.at_css("b").ancestors.map(&:name).first(3)
new = doc.create_element("span", "added", class: "new")
div.add_child(new)
doc.at_css("h1").content = "Changed"
doc.at_css("p.intro").remove
doc.at_css("li.last")["data-x"] = "1"
doc.at_css("ul").add_previous_sibling("<hr>")
p div.css("span.new").text, doc.at_css("h1").text, doc.css("p").size, doc.at_css("li.last").to_html, div.at_css("hr").nil?
p doc.at_css("br").parent.inner_html

frag = Nokogiri::HTML::DocumentFragment.parse("<p>one</p><p>two <i>x</i></p>text")
p frag.children.size, frag.css("p").map(&:text), frag.to_html

xml = <<~XML
  <?xml version="1.0" encoding="UTF-8"?>
  <catalog xmlns:dc="http://purl.org/dc/elements/1.1/">
    <book id="b1" lang="en"><title>Ruby</title><price>29.99</price><dc:creator>Matz</dc:creator></book>
    <book id="b2"><title>Rust</title><price>39.50</price><dc:creator>Graydon</dc:creator></book>
    <!-- a comment -->
    <note><![CDATA[<raw> & stuff]]></note>
  </catalog>
XML
x = Nokogiri::XML(xml)
p x.root.name, x.xpath("//book").size, x.xpath("//book/@id").map(&:value), x.at_xpath("//book[price>30]/title").text
p x.xpath("//dc:creator", "dc" => "http://purl.org/dc/elements/1.1/").map(&:text), x.namespaces
p x.xpath("sum(//price)"), x.xpath("//book[1]/title/text()").to_s, x.at("note").text, x.xpath("//comment()").first.text
p x.css("book[lang=en] title").text, x.at_xpath("//book[last()]")["id"], x.xpath("//book[position()=2]/price").text.to_f
p x.errors.empty?, Nokogiri::XML("<a><b></a>").errors.size > 0
b = Nokogiri::XML::Builder.new(encoding: "UTF-8") do |xx|
  xx.root(version: "1") do
    xx.item("one", id: 1)
    xx.item { xx.cdata("c&d") }
    xx.send(:"name-with-dash", "z")
    xx.comment("hi")
  end
end
puts b.to_xml
x.at_xpath("//book[@id='b2']").remove
x.root.add_child("<book id='b3'><title>Go</title></book>")
puts x.root.to_xml
p x.to_xml(save_with: Nokogiri::XML::Node::SaveOptions::AS_XML | Nokogiri::XML::Node::SaveOptions::NO_DECLARATION).lines.first.strip
p Nokogiri::XML("<r><a>1</a><a>2</a></r>").xpath("//a").map { |n| n.text.to_i }.sum
class Counter < Nokogiri::XML::SAX::Document
  attr_reader :names
  def start_element(name, attrs = []) = (@names ||= []) << name
end
c = Counter.new
Nokogiri::XML::SAX::Parser.new(c).parse("<a><b/><c><d/></c></a>")
p c.names
p Nokogiri::HTML5("<p>unclosed <b>bold<p>next").at("body").inner_html
p Nokogiri::XML::Document.parse("<a/>").root.to_s, Nokogiri::HTML.fragment("a &amp; b").text
