require "bundler/setup"
require "http-cookie"
require "uri"

origin = URI("https://www.example.com/app/page")
cookies = HTTP::Cookie.parse("sid=abc123; Domain=example.com; Path=/; Secure; HttpOnly, theme=dark; Path=/app; Max-Age=3600", origin)
cookies.each { |c| p [c.name, c.value, c.domain, c.path, c.secure?, c.httponly?, c.for_domain?, c.session?] }
c = HTTP::Cookie.new("lang", "en", domain: "example.com", path: "/", for_domain: true)
p c.to_s, c.cookie_value
p c.set_cookie_value
p c.valid_for_uri?("https://sub.example.com/x"), c.valid_for_uri?("https://other.com/")

jar = HTTP::CookieJar.new
jar.parse("a=1; Path=/", URI("http://example.com/"))
jar.parse("b=2; Path=/admin", URI("http://example.com/admin/"))
jar.parse("c=3; Domain=example.com", URI("http://www.example.com/"))
jar.add(HTTP::Cookie.new("d", "4", domain: "other.org", path: "/"))
p jar.cookies(URI("http://example.com/admin/x")).map(&:name).sort
p jar.cookies(URI("http://www.example.com/")).map(&:name).sort
p HTTP::Cookie.cookie_value(jar.cookies(URI("http://example.com/admin/")).sort_by(&:name))
p jar.count, jar.empty?
jar.delete(HTTP::Cookie.new("a", "", domain: "example.com", path: "/"))
p jar.map(&:name).sort
p HTTP::Cookie.cookie_value_to_hash("x=1; y=2")
bad = HTTP::Cookie.parse("x=1; Domain=evil.com", URI("http://example.com/"))
p bad
require "stringio"
io = StringIO.new
jar.save(io, format: :cookiestxt, session: true)
p io.string.lines.grep_v(/^#/).reject { |l| l.strip.empty? }.map { |l| l.split("\t").values_at(0, 2, 5, 6).join("|").strip }.sort
