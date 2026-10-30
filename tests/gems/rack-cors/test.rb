require "bundler/setup"
require "rack/cors"
require "rack/test"
require "rack/builder"

App = Rack::Builder.new do
  use Rack::Cors do
    allow do
      origins "http://good.example", %r{\Ahttps://.*\.trusted\.io\z}
      resource "/api/*", headers: :any, methods: [:get, :post, :put, :delete, :options],
        expose: ["X-Total"], max_age: 600, credentials: true
      resource "/public/*", headers: %w[X-Custom], methods: :get, if: proc { |env| env["HTTP_X_SKIP"].nil? }
    end
    allow do
      origins "*"
      resource "/open", headers: :any, methods: [:get, :head]
    end
    allow do
      origins { |source, _env| source.end_with?(".dyn") }
      resource "/dyn", methods: :get
    end
  end
  run ->(env) { [200, { "content-type" => "text/plain", "x-total" => "3" }, ["#{env["REQUEST_METHOD"]} #{env["PATH_INFO"]}"]] }
end.to_app

include Rack::Test::Methods
def app = App

def show(label)
  h = last_response.headers
  keys = h.keys.select { |k| k.downcase.start_with?("access-control") || k.downcase == "vary" }.sort
  puts "#{label}: #{last_response.status} #{last_response.body.inspect}"
  keys.each { |k| puts "  #{k.downcase}=#{h[k]}" }
end

get "/api/items", {}, "HTTP_ORIGIN" => "http://good.example"
show "simple allowed"
get "/api/items", {}, "HTTP_ORIGIN" => "http://evil.example"
show "simple denied"
get "/api/items", {}, "HTTP_ORIGIN" => "https://a.trusted.io"
show "regex origin"
options "/api/items", {}, "HTTP_ORIGIN" => "http://good.example", "HTTP_ACCESS_CONTROL_REQUEST_METHOD" => "PUT",
  "HTTP_ACCESS_CONTROL_REQUEST_HEADERS" => "X-Foo, Content-Type"
show "preflight ok"
options "/api/items", {}, "HTTP_ORIGIN" => "http://evil.example", "HTTP_ACCESS_CONTROL_REQUEST_METHOD" => "PUT"
show "preflight bad origin"
options "/public/x", {}, "HTTP_ORIGIN" => "http://good.example", "HTTP_ACCESS_CONTROL_REQUEST_METHOD" => "POST"
show "preflight bad method"
options "/public/x", {}, "HTTP_ORIGIN" => "http://good.example", "HTTP_ACCESS_CONTROL_REQUEST_METHOD" => "GET",
  "HTTP_ACCESS_CONTROL_REQUEST_HEADERS" => "X-Other"
show "preflight bad header"
get "/public/x", {}, "HTTP_ORIGIN" => "http://good.example", "HTTP_X_SKIP" => "1"
show "if: false"
get "/open", {}, "HTTP_ORIGIN" => "http://anyone.example"
show "wildcard"
get "/dyn", {}, "HTTP_ORIGIN" => "http://x.dyn"
show "block origin"
get "/api/items"
show "no origin"
post "/api/items", {}, "HTTP_ORIGIN" => "http://good.example"
show "post"
get "/elsewhere", {}, "HTTP_ORIGIN" => "http://good.example"
show "unmatched path"
p Rack::Cors::Resources.new.respond_to?(:allow_origin?)
