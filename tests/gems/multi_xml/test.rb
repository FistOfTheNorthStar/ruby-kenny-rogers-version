require "bundler/setup"
require "multi_xml"

MultiXml.parser = :rexml
p MultiXml.parser.name
xml = <<~XML
  <?xml version="1.0" encoding="UTF-8"?>
  <users type="array">
    <user id="1">
      <name>Alice</name>
      <age type="integer">30</age>
      <admin type="boolean">true</admin>
      <joined type="date">2020-01-02</joined>
      <tags type="array"><tag>a</tag><tag>b</tag></tags>
    </user>
    <user id="2">
      <name>Bob &amp; Co</name>
      <age type="integer">25</age>
      <admin type="boolean">false</admin>
      <score type="float">1.5</score>
      <note nil="true"/>
    </user>
  </users>
XML
h = MultiXml.parse(xml)
p h
p MultiXml.parse("<a><b>1</b><b>2</b><c x='y'>t</c></a>")
p MultiXml.parse("<a>  </a>"), MultiXml.parse("")
p MultiXml.parse("<a><b>1</b></a>", symbolize_keys: true)
begin
  MultiXml.parse("<a><b></a>")
rescue MultiXml::ParseError => e
  puts "ParseError"
end
MultiXml.parser = :rexml
p MultiXml.parse("<r><i type='integer'>7</i></r>")
