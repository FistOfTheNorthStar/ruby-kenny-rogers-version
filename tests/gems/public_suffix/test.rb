require "bundler/setup"
require "public_suffix"

d = PublicSuffix.parse("www.example.co.uk")
p d.tld, d.sld, d.trd, d.domain, d.subdomain, d.to_s
p PublicSuffix.domain("blog.google.com")
p PublicSuffix.domain("foo.github.io")
p PublicSuffix.valid?("example.com"), PublicSuffix.valid?("co.uk"), PublicSuffix.valid?("example.notarealtld")
p PublicSuffix.domain("www.example.notarealtld")
p PublicSuffix.domain("example.notarealtld", default_rule: nil) rescue p $!.class
p PublicSuffix.parse("a.b.kawasaki.jp").domain
p PublicSuffix.parse("city.kawasaki.jp").domain
p PublicSuffix.parse("WWW.Example.COM").to_a
begin
  PublicSuffix.parse("co.uk")
rescue PublicSuffix::DomainNotAllowed => e
  p e.class, e.message
end
list = PublicSuffix::List.parse("com\n*.foo.com\n!bar.foo.com\n")
p PublicSuffix.parse("x.y.foo.com", list: list).domain, PublicSuffix.parse("a.bar.foo.com", list: list).domain
p PublicSuffix::List.default.size > 5000
p PublicSuffix::List.default.find("example.com.au").value
p PublicSuffix.domain("www.blogspot.com", ignore_private: true)
p PublicSuffix.normalize("http://example.com") rescue p $!
