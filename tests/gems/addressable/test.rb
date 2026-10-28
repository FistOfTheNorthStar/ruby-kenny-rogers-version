require "bundler/setup"
require "addressable/uri"
require "addressable/template"

u = Addressable::URI.parse("HTTP://User:Pw@Example.COM:80/a/./b/../c%7e?q=1&q=2&x=a+b#frag")
p u.scheme, u.user, u.password, u.host, u.port, u.path, u.query, u.fragment
p u.normalize.to_s
p u.query_values
p u.query_values(Array)
u2 = Addressable::URI.parse("http://example.com/")
u2.query_values = { "b" => "2", "a" => ["1", "x y"] }
p u2.to_s
p Addressable::URI.join("http://example.com/a/b/c", "../d?e=f").to_s
p Addressable::URI.parse("http://example.com/a/b").join("/root").to_s
p Addressable::URI.escape("http://example.com/ü ö"), Addressable::URI.unescape("%C3%BC%20")
p Addressable::URI.encode_component("a b&c", Addressable::URI::CharacterClasses::UNRESERVED)
p Addressable::URI.form_encode([["a", "1 2"], ["b", "&"]]), Addressable::URI.form_unencode("a=1+2&b=%26")
p Addressable::URI.parse("http://www.詹姆斯.com/").normalize.to_s
p Addressable::IDNA.to_ascii("bücher.de"), Addressable::IDNA.to_unicode("xn--bcher-kva.de")
p Addressable::URI.heuristic_parse("example.com/path").to_s
p Addressable::URI.parse("/relative/path").relative?, u.absolute?
p u.omit(:user, :password, :fragment).to_s
p Addressable::URI.parse("http://a.example.com/x").domain, u.origin

t = Addressable::Template.new("http://example.com/{user}/repos{?page,per_page}{&sort*}")
p t.expand(user: "kenny", page: 2, per_page: 10, sort: { by: "name" }).to_s
p t.variables
m = Addressable::Template.new("/users/{id}/posts/{post_id}").extract("/users/42/posts/7")
p m
p Addressable::Template.new("{+path}/here{#frag}").expand(path: "/foo/bar", frag: "x y").to_s
p Addressable::Template.new("/list{/items*}").expand(items: ["a", "b"]).to_s
p t.match("http://example.com/bob/repos?page=3").mapping rescue p $!
