require "bundler/setup"
require "rack"
require "rack/mock"
require "tmpdir"
require "stringio"

app = ->(env) { [200, { "content-type" => "text/plain" }, ["hi #{Rack::Request.new(env).params["name"]}"]] }
res = Rack::MockRequest.new(Rack::Lint.new(app)).get("/?name=kenny")
p res.status, res.headers["content-type"], res.body
p Rack::Utils.parse_nested_query("a[b][]=1&a[b][]=2&c=3")
p Rack::Utils.build_query(x: 1, y: "a b")
p Rack::Utils.escape_html("<a href='x'>&</a>")

# Utils
p Rack::Utils.build_nested_query(user: { name: "a b", tags: ["x", "y"] })
p Rack::Utils.parse_query("a=1&a=2&b=%20")
p Rack::Utils.escape("a b&c/d"), Rack::Utils.unescape("a+b%26c"), Rack::Utils.escape_path("a b/c")
p Rack::Utils.status_code(:not_found), Rack::Utils::HTTP_STATUS_CODES[418]
p Rack::Utils.byte_ranges({ "HTTP_RANGE" => "bytes=0-4,10-" }, 20)
p Rack::Utils.parse_cookies_header("a=1; b=two%20words")
p Rack::Utils.set_cookie_header("sid", { value: "abc", path: "/", httponly: true, same_site: :lax })
p Rack::Utils.best_q_match("text/html;q=0.5, application/json", %w[text/html application/json])
h = Rack::Headers.new
h["Content-Type"] = "x"; h["X-Foo"] = "1"
p h.to_h, h["content-type"]

# Builder with middleware
class Upcase
  def initialize(app) = @app = app
  def call(env)
    s, h, b = @app.call(env)
    out = +""; b.each { |c| out << c }; b.close if b.respond_to?(:close)
    [s, h, [out.upcase]]
  end
end
built = Rack::Builder.new do
  use Rack::ContentLength
  use Rack::ETag
  use Upcase
  map "/api" do
    run ->(env) { [200, { "content-type" => "application/json" }, [%({"path":"#{env["PATH_INFO"]}","script":"#{env["SCRIPT_NAME"]}"})]] }
  end
  run ->(env) { [404, { "content-type" => "text/plain" }, ["nope"]] }
end.to_app
m = Rack::MockRequest.new(built)
r = m.get("/api/users")
p r.status, r.body, r["content-length"], r["etag"]
r = m.get("/other")
p r.status, r.body

# Request / Response
env = Rack::MockRequest.env_for("http://example.com:8080/path/x?q=1&arr[]=a&arr[]=b",
  method: "POST", input: "f=1&g=2", "CONTENT_TYPE" => "application/x-www-form-urlencoded",
  "HTTP_COOKIE" => "c1=v1; c2=v2", "HTTP_USER_AGENT" => "kenny", "HTTP_X_FORWARDED_PROTO" => "https")
req = Rack::Request.new(env)
p req.post?, req.request_method, req.path, req.fullpath, req.host, req.port, req.scheme, req.ssl?
p req.GET, req.POST, req.params, req.cookies, req.user_agent, req.media_type, req.base_url, req.url
resp = Rack::Response.new
resp.status = 201
resp.write "hello "
resp.write "world"
resp.set_cookie("s", { value: "1", path: "/" })
resp.delete_cookie("old")
resp["x-custom"] = "yes"
st, hd, bd = resp.finish
p st, hd.sort.to_h, bd.to_enum(:each).to_a
p Rack::Response.new(["x"], 302, { "location" => "/y" }).redirect?

# Multipart
boundary = "AaB03x"
data = [
  "--#{boundary}", %(Content-Disposition: form-data; name="title"), "", "Hello",
  "--#{boundary}", %(Content-Disposition: form-data; name="file"; filename="a.txt"), "Content-Type: text/plain", "", "file contents\nline2",
  "--#{boundary}--", ""
].join("\r\n")
menv = Rack::MockRequest.env_for("/up", method: "POST", input: data,
  "CONTENT_TYPE" => "multipart/form-data; boundary=#{boundary}", "CONTENT_LENGTH" => data.bytesize.to_s)
mp = Rack::Multipart.parse_multipart(menv)
p mp["title"], mp["file"][:filename], mp["file"][:type], mp["file"][:name], mp["file"][:tempfile].read
upload = Rack::Multipart::UploadedFile.new(io: StringIO.new("abc"), filename: "u.txt", content_type: "text/plain")
p upload.original_filename, upload.content_type, upload.read

# Cookies round trip (sessions live in the rack-session gem since Rack 3)
capp = ->(env) {
  req = Rack::Request.new(env)
  n = req.cookies["n"].to_i + 1
  res = Rack::Response.new(["n=#{n}"])
  res.set_cookie("n", { value: n.to_s, path: "/", httponly: true })
  res.finish
}
cr = Rack::MockRequest.new(capp).get("/", "HTTP_COOKIE" => "n=41")
p cr.body, cr["set-cookie"], cr.cookies.keys rescue p $!

# Static / Files from a tmp dir
Dir.mktmpdir do |dir|
  File.write(File.join(dir, "hello.txt"), "static hello")
  Dir.mkdir(File.join(dir, "css"))
  File.write(File.join(dir, "css", "site.css"), "body{}")
  files = Rack::MockRequest.new(Rack::Files.new(dir))
  r = files.get("/hello.txt")
  p r.status, r.body, r["content-type"], r["content-length"]
  r = files.get("/hello.txt", "HTTP_RANGE" => "bytes=0-5")
  p r.status, r.body, r["content-range"]
  p files.get("/missing").status
  static = Rack::Static.new(->(e) { [200, {}, ["dynamic"]] }, urls: ["/css"], root: dir)
  sm = Rack::MockRequest.new(static)
  p sm.get("/css/site.css").body, sm.get("/css/site.css")["content-type"], sm.get("/other").body
end

# Other middleware
p Rack::MockRequest.new(Rack::Head.new(app)).head("/").body
cascade = Rack::Cascade.new([->(e) { [404, {}, []] }, ->(e) { [200, {}, ["second"]] }])
p Rack::MockRequest.new(cascade).get("/").body
urlmap = Rack::URLMap.new("/a" => ->(e) { [200, {}, ["A #{e["PATH_INFO"]}"]] }, "/b" => ->(e) { [200, {}, ["B"]] })
p Rack::MockRequest.new(urlmap).get("/a/x").body
mo = Rack::MethodOverride.new(->(e) { [200, {}, [e["REQUEST_METHOD"]]] })
p Rack::MockRequest.new(mo).post("/", params: { "_method" => "DELETE" }).body
deflate = Rack::Deflater.new(->(e) { [200, { "content-type" => "text/plain" }, ["x" * 100]] })
dr = Rack::MockRequest.new(deflate).get("/", "HTTP_ACCEPT_ENCODING" => "gzip")
p dr["content-encoding"], Zlib::GzipReader.new(StringIO.new(dr.body)).read.size
cond = Rack::ConditionalGet.new(->(e) { [200, { "etag" => '"abc"' }, ["body"]] })
p Rack::MockRequest.new(cond).get("/", "HTTP_IF_NONE_MATCH" => '"abc"').status
st = Rack::ShowExceptions.new(->(e) { raise ArgumentError, "boom" })
r = Rack::MockRequest.new(st).get("/", "HTTP_ACCEPT" => "text/plain")
p r.status, r.body.lines.first.chomp
