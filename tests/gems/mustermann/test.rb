require "bundler/setup"
require "mustermann"

pat = Mustermann.new("/:name/posts/:id")
p pat.class, pat.to_s
p pat.params("/kenny/posts/42")
p pat.match("/kenny/posts/42")&.captures
p pat === "/a/posts/1", pat === "/a/b/c"
p pat.names
p pat.expand(name: "bob", id: 7)
p pat.to_templates rescue p $!.class
p pat.peek("/kenny/posts/42/more"), pat.peek_size("/kenny/posts/42/more")

s = Mustermann.new("/files/*path.:ext")
p s.params("/files/a/b/c.txt")
p Mustermann.new("/files/*").params("/files/x/y")
p Mustermann.new("/opt(/:x)?").params("/opt"), Mustermann.new("/opt(/:x)?").params("/opt/9")
p Mustermann.new("/:a-:b").params("/foo-bar")
p Mustermann.new("/users/:id(.:format)", type: :rails).params("/users/3.json")
p Mustermann.new("/users/:id(.:format)", type: :rails).expand(id: 1, format: "json")
p Mustermann.new("/:id", capture: /\d+/).params("/12"), Mustermann.new("/:id", capture: /\d+/).params("/ab")
p Mustermann.new("/:id", capture: { id: :digit }).match("/ab").nil?
p Mustermann.new("/hello world").params("/hello%20world")
p Mustermann.new("/foo", uri_decode: false) === "/foo"
p Mustermann.new("/:x", type: :sinatra).regexp.source
p Mustermann.new("/a/:b", except: "/a/x").params("/a/x"), Mustermann.new("/a/:b", except: "/a/x").params("/a/y")

r = Mustermann.new(%r{/(?<year>\d{4})/(?<slug>[^/]+)})
p r.params("/2024/hello"), r.class

id = Mustermann.new("/exact", type: :identity)
p id.class, id === "/exact", id.params("/exact")

combo = Mustermann.new("/a/:x", "/b/:y")
p combo.class, combo.params("/b/5")
p (Mustermann.new("/foo/:x") | Mustermann.new("/bar/:x")).params("/bar/1")
p (Mustermann.new("/foo") + Mustermann.new("/:x")).params("/foo/zz")
p (Mustermann.new("/:a") & Mustermann.new("/b")).params("/b")

ex = Mustermann::Expander.new("/:a", "/:a/:b")
p ex.expand(a: 1), ex.expand(a: 1, b: 2)
p ex.expand(:append, a: 1, c: 3)
p Mustermann.new("/:name").to_proc.call("/z")
p Mustermann.new("/foo") == Mustermann.new("/foo"), Mustermann.new("/foo").eql?(Mustermann.new("/foo", type: :rails))
p Mustermann.new("/:x").hash == Mustermann.new("/:x").hash
p Mustermann.new("/:x/:y").named_captures rescue p $!.class
p "/search/q".sub(Mustermann.new("/search/:q")) { "found" } rescue p $!.class
p Mustermann.new("/:x") =~ "/zz"

[%w[/:x( rails], %w[/{a sinatra], ["/:x/:x", "sinatra"]].each do |src, type|
  Mustermann.new(src, type: type.to_sym)
rescue Mustermann::Error => e
  p e.class, e.message
end
begin
  Mustermann.new("/:a").expand(b: 1)
rescue Mustermann::ExpandError => e
  p e.class, e.message
end
begin
  Mustermann.new("/x", type: :nonexistent)
rescue ArgumentError => e
  p e.class, e.message
end
