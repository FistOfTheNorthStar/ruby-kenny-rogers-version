require "bundler/setup"
require "domain_name"

d = DomainName("www.Example.co.uk")
p d.hostname, d.domain, d.tld, d.ipaddr?, d.canonical?
p DomainName("foo.github.io").domain
p DomainName("192.168.0.1").ipaddr?, DomainName("192.168.0.1").ipaddr.to_s
p DomainName("[::1]").ipaddr?
p d.cookie_domain?("example.co.uk"), d.cookie_domain?("co.uk"), d.cookie_domain?("other.co.uk")
p DomainName("example.com") == DomainName("EXAMPLE.com")
p DomainName("a.b.example.com") <=> DomainName("example.com")
p DomainName.normalize("BÜCHER.example")
p DomainName("xn--bcher-kva.example").hostname
p DomainName::Punycode.encode("bücher"), DomainName::Punycode.decode("bcher-kva")
p DomainName::Punycode.encode_hostname("www.bücher.de"), DomainName::Punycode.decode_hostname("www.xn--bcher-kva.de")
p DomainName("localhost").domain, DomainName("localhost").tld
p DomainName("www.city.kawasaki.jp").domain
p DomainName("example.com").superdomain&.to_s
p DomainName("a.b.example.com").superdomain.to_s
