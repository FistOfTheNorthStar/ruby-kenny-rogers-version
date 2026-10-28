require "bundler/setup"
require "builder"

xml = Builder::XmlMarkup.new(indent: 2)
xml.instruct!
xml.declare! :DOCTYPE, :html, :PUBLIC, "-//W3C//DTD XHTML 1.0 Strict//EN"
xml.comment! "generated"
xml.feed(xmlns: "http://www.w3.org/2005/Atom") do
  xml.title "Kenny & <friends>"
  xml.link href: "http://example.com/", rel: "self"
  %w[one two].each_with_index do |t, i|
    xml.entry(id: i) do
      xml.title t
      xml.content(type: "html") { xml.cdata!("<p>#{t}</p>") }
      xml.tag!("dc:creator", "me")
    end
  end
  xml.empty
  xml.raw { xml << "<pre>unescaped</pre>" }
end
puts xml.target!

x = Builder::XmlMarkup.new
x.p("text", class: "c") ; x.br ; x.span { x.text! "a < b" }
p x.target!
s = +""
y = Builder::XmlMarkup.new(target: s, indent: 1, margin: 1)
y.root { y.child "v" }
p s
p Builder::XChar.encode("a&b<\u00e9")
p Builder::XmlMarkup.new.tag!(:x, "y", "z" => 1)
x2 = Builder::XmlMarkup.new
x2.instruct! :xml, version: "1.0", encoding: "UTF-8"
x2.ns :item, "v"
x2.tag!("data") { x2.number 3.5; x2.nil_val nil; x2.sym :abc }
p x2.target!
