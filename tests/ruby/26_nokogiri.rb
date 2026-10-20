# Nokogiri (native core in the runtime + Nokogiri's own Ruby layer).
# Output matches CRuby with nokogiri 1.15.7 (libxml2 2.11) for these inputs. HTML is parsed
# with html5ever (massaged towards libxml2's tree in HTML4 mode), so the cases stick to markup
# both parsers build the same tree for. Known differences elsewhere: whitespace-only text
# between <html>/<head>/<body> is dropped, libxml2's odd nesting of some misnested markup
# (<a>..<div>..</a>, <h1><h2>) isn't reproduced, a tag cut off by the end of input is dropped,
# `<p title>` serializes as title="", and a node moved between documents keeps its Ruby object.
require "nokogiri"

puts "== version"
p Nokogiri::VERSION
p Nokogiri::VERSION_INFO["nokogiri"]["version"]
p Nokogiri.uses_libxml?, Nokogiri.uses_gumbo?, Nokogiri.jruby?
p Nokogiri::XML::SyntaxError.ancestors.include?(Nokogiri::SyntaxError)

puts "== HTML4 fragments"
f = Nokogiri::HTML4::DocumentFragment.parse("<div><p>a</p><p>b</p></div>")
p f.to_html
p f.to_s
p f.children.length, f.children.first.name
p Nokogiri::HTML4.fragment("<div><span>a</span><span>b</span></div><table><tr><td>1</td></tr></table>").to_html
p Nokogiri::HTML4.fragment("<a href='a b\"c&d<>'>x</a><img src='x y'><input checked disabled=disabled value=\"a&b<c>\"><br/>").to_html
p Nokogiri::HTML4.fragment("<p>é   &lt;&gt;&amp;\"'</p>").to_html
p Nokogiri::HTML4.fragment("<script>if (a < b && c) {}</script><style>a > b {}</style><!-- c -->").to_html
p Nokogiri::HTML4.fragment("text <b>bold</b> more").children.map(&:class)
p Nokogiri::HTML4.fragment("<section><article>x</article></section><custom-el a=1>y</custom-el>").to_html
p Nokogiri::HTML4.fragment("<ul><li></li></ul><p></p><div></div>").to_html
p Nokogiri::HTML4.fragment("<div><!-- c --><p>x</p></div>").to_html
p Nokogiri::HTML4.fragment("<div>t<p>x</p>u</div>").to_html
p Nokogiri::HTML4.fragment("\n<p>x</p>\n<p>y</p>\n").to_html
p Nokogiri::HTML4.fragment("<a href=' \t x y '>z</a>").to_html

puts "== HTML4 documents"
d = Nokogiri::HTML4("<p>hi</p>")
p d.to_html
p d.encoding, d.html?, d.xml?, d.type
p d.root.name, d.root.parent.equal?(d)
d = Nokogiri::HTML4("<title>T</title><div><p>a</p><p>b</p></div><ul><li>x</li><li>y</li></ul>")
p d.to_html
p d.title
d = Nokogiri::HTML4("<!DOCTYPE html><p>x</p>")
p d.to_html
p d.internal_subset.name
p Nokogiri::HTML4::Document.new.to_html
d = Nokogiri::HTML4("<div>a</div><div>b</div><span>c</span>")
p d.at("div").to_html, d.at("body").to_html
p d.root.to_xml
p d.to_xhtml

puts "== traversal"
d = Nokogiri::HTML4("<div id='main' class='a b'><p class='x'>one</p><!--c--><p>two <b>three</b></p>tail</div>")
div = d.at_css("#main")
p div.name, div.node_name, div.type, div.element?, div.text?
p div["id"], div["class"], div[:id], div["missing"]
p div.key?("id"), div.has_attribute?("nope")
p div.attributes.keys, div.attributes["class"].value, div.keys, div.values
p div.attribute_nodes.map { |a| [a.name, a.value, a.class] }
p div.children.map(&:name)
p div.element_children.map(&:name), div.elements.length
p div.child.name, div.first_element_child.text, div.last_element_child.text
p div.children[1].comment?, div.children[1].content
p div.children.last.text?, div.children.last.content
p div.at("p").next_sibling.name, div.at("p").next_element.text, div.children.last.previous_sibling.name
p div.content, div.text, div.inner_text
p div.inner_html
p div.parent.name, div.ancestors.map(&:name)
p div.at("b").path, div.css_path rescue p $!
p div.document.equal?(d), div.at("b").document.equal?(d)
p div.classes, div.at("p").classes
p d.at("b").equal?(d.at("b"))
seen = []
div.traverse { |n| seen << n.name }
p seen
p div.at("p").matches?("p.x"), div.at("p").matches?("b")

puts "== search"
d = Nokogiri::HTML4(<<~HTML)
  <html><body>
  <ul id="list"><li class="item first">1</li><li class="item">2</li><li class="item last" data-x="y">3</li></ul>
  <div class="box"><a href="/a">A</a><a href="http://b.example/" rel="nofollow">B</a></div>
  <p>para <span>s1</span></p><p lang="en-US">second</p>
  </body></html>
HTML
p d.css("li").map(&:text)
p d.css("li.item").length, d.css(".first").map(&:text), d.css("li.last").first["data-x"]
p d.css("ul > li:first-child").map(&:text), d.css("li:last-child").map(&:text)
p d.css("li:nth-child(2)").map(&:text), d.css("li:nth-child(odd)").map(&:text)
p d.css("a[href^='http']").map(&:text), d.css("a[href$='/a']").map(&:text), d.css("a[rel~=nofollow]").map(&:text)
p d.css("a[href*=example]").length, d.css("[data-x]").length, d.css("#list li").length
p d.css("div.box a, p span").map(&:text)
p d.css("li + li").map(&:text), d.css("li.first ~ li").map(&:text)
p d.css("p:contains('second')").map(&:name), d.css("li:not(.first)").map(&:text)
p d.at_css("p span").text, d.at_css("nothing")
p d.css("body *").length, d.css("p:empty").length, d.css("p:has(span)").length
p d.xpath("//li").length, d.xpath("//li[@class='item']").map(&:text)
p d.xpath("//li[1]").text, d.xpath("//li[last()]").text, d.xpath("(//li)[2]").text
p d.xpath("//ul/li[position() > 1]").map(&:text)
p d.xpath("count(//li)"), d.xpath("string(//li[2])"), d.xpath("boolean(//nope)"), d.xpath("1 + 2 * 3")
p d.xpath("//a/@href").map(&:value), d.xpath("//a/@href").first.class
p d.xpath("//li[contains(concat(' ', normalize-space(@class), ' '), ' last ')]").text
p d.xpath("//p[span]").length, d.xpath("//li/text()").map(&:to_s)
p d.xpath("//li | //a").map(&:name), d.xpath("//*[@lang]").first.name
p d.xpath("//p[lang('en')]").length
p d.xpath("//li[starts-with(., '2')]").text, d.xpath("//li[. = '3']").text
p d.xpath("name(//*[@id])"), d.xpath("local-name(/html)"), d.xpath("substring('abcdef', 2, 3)")
p d.xpath("translate('abc', 'ab', 'AB')"), d.xpath("concat('a', 'b', 'c')"), d.xpath("string-length('héllo')")
p d.xpath("sum(//li)"), d.xpath("floor(2.5)"), d.xpath("round(2.5)"), d.xpath("ceiling(2.1)")
p d.xpath("//li[1]/following-sibling::li").map(&:text), d.xpath("//li[3]/preceding-sibling::li").map(&:text)
p d.xpath("//span/ancestor::*").map(&:name), d.xpath("//span/..").map(&:name)
p d.at("ul").xpath("./li").length, d.at("ul").xpath(".//li").length, d.at("ul").search("li").length
p d.search("//a", "li.first").map(&:name)
p d.at("li").at_xpath("following::a[1]").text
ul = d.at("ul")
p ul.css("li").map(&:text), (ul > "li").length
p d.xpath("//li[@class=$c]", nil, "c" => "item").map(&:text)
handler = Class.new do
  def upcase(nodes)
    nodes.map { |n| n.text.upcase }.join(",")
  end

  def big(nodes)
    nodes.select { |n| n.text.to_i > 1 }
  end
end.new
p d.xpath("nokogiri:upcase(//li)", handler)
p d.xpath("//li[nokogiri:big(.)]", handler).map(&:text)
p d.css("li:big", handler).map(&:text)
begin
  d.xpath("//li[")
rescue Nokogiri::XML::XPath::SyntaxError => e
  p e.class
end
begin
  d.css("li[")
rescue Nokogiri::CSS::SyntaxError => e
  p e.class
end

puts "== mutation"
f = Nokogiri::HTML4.fragment("<div><p>one</p></div>")
div = f.at("div")
b = f.document.create_element("b", "bold", class: "x")
p b.to_html
div.add_child(b)
p f.to_html
div.add_child("<i>it</i>")
p f.to_html
f.at("p").add_next_sibling("<span>after</span>")
f.at("p").add_previous_sibling(Nokogiri::XML::Text.new("before ", f.document))
p f.to_html
f.at("span").replace("<em>rep</em>")
p f.to_html
f.at("em").swap(f.document.create_text_node("txt"))
p f.to_html
f.at("i").remove
p f.to_html
p f.at("b").unlink.parent
p f.to_html
f.at("p").before("<hr>").after("<br>")
p f.to_html
p f.at("p").wrap("<section></section>").parent.name
p f.to_html
f.at("p")["data-k"] = "v"
f.at("p").set_attribute("title", "t")
p f.at("p").to_html
f.at("p").remove_attribute("data-k")
p f.at("p").to_html
f.at("p").attributes["title"].value = "changed"
p f.at("p").to_html
f.at("p").attribute("title").remove
p f.at("p").to_html
f.at("p").content = "<new & content>"
p f.at("p").to_html, f.at("p").text
f.at("p").inner_html = "<b>x</b><i>y</i>"
p f.at("p").to_html
f.at("p").children = "plain"
p f.at("p").to_html
f.at("p").name = "h1"
p f.to_html
f.at("h1").add_class("big").add_class("red")
p f.at("h1")["class"]
f.at("h1").remove_class("big")
p f.at("h1")["class"], f.at("h1").classes
f.at("h1").append_class("big")
p f.at("h1")["class"]
p f.css("hr, br").remove.length, f.to_html
c = f.document.create_comment("note")
f.at("section").prepend_child(c)
p f.to_html
d2 = f.at("section").dup
p d2.to_html, d2.parent, d2.document.equal?(f.document)
p f.at("section").dup(0).to_html

puts "== cross-document moves"
src = Nokogiri::HTML4.fragment("<ul><li>a</li><li>b</li></ul>")
dst = Nokogiri::HTML4.fragment("<div></div>")
li = src.at("li")
li.instance_variable_set(:@mark, 42)
dst.at("div").add_child(li)
p dst.to_html, src.to_html
# (Kenny also keeps the moved node's object identity: dst.at("li").equal?(li); CRuby doesn't)
p li.document.equal?(dst.document), li.instance_variable_get(:@mark), li.parent.name
dst.at("div") << src.at("ul")
p dst.to_html, src.children.length

puts "== text merging"
f = Nokogiri::HTML4.fragment("<p>a<b>b</b>c</p>")
b = f.at("b")
b.before(b.children)
b.remove
p f.at("p").children.length, f.at("p").children.map(&:text), f.to_html

puts "== node sets"
d = Nokogiri::HTML4("<p>1</p><p>2</p><p>3</p><div>4</div>")
ps = d.css("p")
p ps.class, ps.length, ps.size, ps.first.text, ps.last.text, ps[1].text, ps[-1].text
p ps[0, 2].map(&:text), ps[1..].map(&:text), ps.slice(0).text, ps.first(2).map(&:text)
p ps.to_a.class, ps.map(&:text), ps.select { |n| n.text > "1" }.map(&:text)
divs = d.css("div")
both = ps + divs
p both.length, both.map(&:name), (both - divs).length, (both & divs).map(&:text), (ps | ps).length
p ps.include?(d.at("p")), divs.include?(d.at("p")), ps.index(ps[2]), ps.empty?, d.css("nope").empty?
p ps.text, ps.inner_text, ps.to_html, ps.to_s
p ps.attr("class", "c").map { |n| n["class"] }, ps.remove_attr("class").map { |n| n["class"] }
p ps.reverse.map(&:text), ps.children.length
p ps.filter("p:last-child").map(&:text)
removed = d.css("p").remove
p removed.length, d.css("p").length, d.at("body").to_html
ns = Nokogiri::XML::NodeSet.new(d, [d.at("div")])
ns << d.at("body")
ns.push(d.at("div"))
p ns.length, ns.map(&:name)
p ns.pop.name, ns.length, ns.shift.name, ns.length
p ps == d.css("nope"), ps.dup.length
p ps.inspect.start_with?("[#<Nokogiri::XML::Element")

puts "== XML"
x = Nokogiri::XML("<root><a x='1'>t</a><b><c/><d>q</d></b><!-- c --><?pi data?><e><![CDATA[x<y]]></e></root>")
p x.class, x.encoding, x.version, x.root.name, x.xml?, x.html?, x.type
p x.to_xml
p x.root.to_xml
p x.at("b").to_s
p x.at("a")["x"], x.at("d").text, x.at("e").children.first.class, x.at("e").text
p x.xpath("/root/*").map(&:name), x.css("b > d").map(&:text)
p x.at("//comment()").content, x.at("//processing-instruction()").name
x2 = Nokogiri::XML("<?xml version='1.0' encoding='UTF-8'?><r a='é &lt; &quot; t'>é &gt; &#65;&#x42;</r>")
p x2.encoding, x2.to_xml, x2.root["a"], x2.root.text
p Nokogiri::XML("<r>é</r>").to_xml
ns = Nokogiri::XML("<r xmlns='urn:x' xmlns:p='urn:p'><p:a p:b='1'>x</p:a><c/></r>")
p ns.to_xml
p ns.root.namespace.href, ns.root.namespace.prefix, ns.at_xpath("//p:a", "p" => "urn:p").name
p ns.xpath("//xmlns:c").length, ns.root.namespaces.sort, ns.collect_namespaces.sort
pa = ns.at_xpath("//p:a", "p" => "urn:p")
p pa.namespace.prefix, pa.attribute_nodes.map { |a| [a.name, a.namespace&.prefix] }, pa["p:b"]
p pa.namespace_scopes.map(&:prefix).sort_by(&:to_s), pa.namespace_definitions
p Nokogiri::XML("<r>  <a>  </a>\n</r>").to_xml
p Nokogiri::XML("<r>").errors.length > 0
p Nokogiri::XML::Document.new.to_xml
x = Nokogiri::XML("<r><a/></r>")
x.root << "<b>t</b>"
x.root.add_child(x.create_element("c", "d"))
p x.to_xml
begin
  Nokogiri::XML("<r><a></r>", nil, nil, Nokogiri::XML::ParseOptions::STRICT)
rescue Nokogiri::XML::SyntaxError => e
  p e.class
end
frag = Nokogiri::XML.fragment("<a>1</a><b>2</b>")
p frag.children.map(&:name), frag.to_xml
p Nokogiri::XML("<r><a>1</a></r>").root.to_html

puts "== builder"
builder = Nokogiri::XML::Builder.new do |xml|
  xml.root do
    xml.item(id: "1") { xml.text "one" }
    xml.item("two", id: "2")
    xml.empty
  end
end
p builder.to_xml
b = Nokogiri::HTML4::Builder.new { |h| h.div(class: "x") { h.span "hi" } }
p b.doc.root.to_html

puts "== HTML5"
h5 = Nokogiri::HTML5("<!DOCTYPE html><p>x<br>y</p>")
p h5.class, h5.to_html, h5.encoding
p Nokogiri::HTML5.fragment("<p>a<br>b</p><template><b>x</b></template>").to_html
p Nokogiri::HTML5.fragment("<p title='a&b\" '>&lt; &amp;</p><script>a<b</script>").to_html
s = Nokogiri::HTML5("<svg viewBox='0 0 1 1'><path xlink:href='#a'/></svg><p>x")
p s.to_html, s.at_css("svg").namespace.href, s.at_css("p").namespace
p s.at_css("path").attribute_nodes.map { |a| [a.name, a.namespace&.prefix] }
p Nokogiri::HTML5("<table><tr><td>1</td></tr></table>").at_css("tbody").name
p Nokogiri::HTML5.fragment("<li>a</li><li>b</li>").css("li").map(&:text)

puts "== misc"
p Nokogiri::HTML4::NamedCharacters["nbsp"], Nokogiri::HTML4::NamedCharacters["copy"]
p Nokogiri::HTML4::ElementDescription["br"].empty?, Nokogiri::HTML4::ElementDescription["span"].inline?
d = Nokogiri::HTML4("<p>x</p>")
p d.at("p").encode_special_chars("<a & \"b\">")
p d.at("p") == d.at("p"), d.at("p") <=> d.at("body"), d.at("body") <=> d.at("p")
p d.at("p").line
p d.decorators(Nokogiri::XML::Node).class
p d.at("p").blank?, Nokogiri::HTML4.fragment(" ").children.first.blank?
p d.fragment("<b>f</b>").children.first.name
p d.at("p").fragment("<i>z</i>").to_html
p d.at("p").parse("<em>e</em>").map(&:name)
p Nokogiri::XML::Text.new("t", d).to_html, Nokogiri::XML::Comment.new(d, "c").to_html
p Nokogiri::XML::CDATA.new(d, "x<y").to_xml
p d.at("p").to_html(encoding: "UTF-8"), d.at("p").serialize

puts "== decorators"
module Shout
  def shout
    name.upcase
  end
end
d = Nokogiri::HTML4("<p><b>x</b><i>y</i></p>")
d.decorators(Nokogiri::XML::Node) << Shout
d.decorate!
p d.at("b").shout, d.at("b").class, d.at("i").is_a?(Shout), d.css("i").first.shout
p d.at("b").singleton_class.include?(Shout), d.at("p").instance_of?(Nokogiri::XML::Element)
# the decorated-node class is cached: it must survive a full collection with no node using it
d = nil
GC.start
d = Nokogiri::HTML4("<p><b>again</b></p>")
d.decorators(Nokogiri::XML::Node) << Shout
d.decorate!
p d.at("b").shout
s = Nokogiri::Slop("<html><body><p class='c'>t</p></body></html>")
p s.html.body.p.text, s.html.body.p["class"]
