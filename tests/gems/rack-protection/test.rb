require "bundler/setup"
require "rack/protection"
require "rack/mock"

app = ->(env) { [200, { "content-type" => "text/html" }, ["ok"]] }
def show(r) = p([r.status, r.body, r.headers.select { |k, _| k.start_with?("x-", "content-security", "strict") }.sort.to_h])

stack = Rack::Builder.new do
  use Rack::Lint
  use Rack::Protection, except: [:session_hijacking, :remote_token, :authenticity_token]
  run app
end.to_app
m = Rack::MockRequest.new(stack)
show m.get("/")
show m.get("/../etc/passwd")
show m.post("/", "HTTP_ORIGIN" => "http://evil.example", "HTTP_HOST" => "example.org")
show m.get("/", "HTTP_X_FORWARDED_FOR" => "1.1.1.1", "HTTP_CLIENT_IP" => "2.2.2.2")

pt = Rack::MockRequest.new(Rack::Protection::PathTraversal.new(->(e) { [200, {}, [e["PATH_INFO"]]] }))
p pt.get("/foo/../bar/./baz").body, pt.get("/a/%2e%2e/b").body

csp = Rack::Protection::ContentSecurityPolicy.new(app, default_src: "'self'", script_src: "'self' cdn.example", report_only: false)
p Rack::MockRequest.new(csp).get("/")["content-security-policy"]
hsts = Rack::Protection::StrictTransport.new(app, max_age: 100, include_subdomains: true)
p Rack::MockRequest.new(hsts).get("/")["strict-transport-security"]

esc = Rack::Protection::EscapedParams.new(->(e) { [200, {}, [Rack::Request.new(e).params["q"]]] })
p Rack::MockRequest.new(esc).get("/?q=%3Cscript%3E").body

# authenticity token with a session hash
tokapp = Rack::Protection::AuthenticityToken.new(->(e) { [200, {}, ["passed"]] })
sess = {}
tm = Rack::MockRequest.new(tokapp)
p tm.get("/", "rack.session" => sess).status
p tm.post("/", "rack.session" => sess).status
token = Rack::Protection::AuthenticityToken.token(sess)
p token.is_a?(String), token.size > 20
p tm.post("/", "rack.session" => sess, "HTTP_X_CSRF_TOKEN" => token).body
p tm.post("/", "rack.session" => sess, params: { "authenticity_token" => "bad" }).status

js = Rack::Protection::JsonCsrf.new(->(e) { [200, { "content-type" => "application/json" }, ["[1]"]] })
p Rack::MockRequest.new(js).get("/", "HTTP_REFERER" => "http://evil.example/").status
p Rack::MockRequest.new(js).get("/").status
p Rack::Protection::Base.new(app).respond_to?(:accepts?)
