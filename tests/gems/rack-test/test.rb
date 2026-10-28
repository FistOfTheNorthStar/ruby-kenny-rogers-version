require "bundler/setup"
require "rack/test"

class App
  def call(env)
    req = Rack::Request.new(env)
    case req.path
    when "/hello" then [200, { "content-type" => "text/plain" }, ["Hello #{req.params["name"] || "world"}"]]
    when "/echo"
      [200, { "content-type" => "application/json" }, [%({"method":"#{req.request_method}","body":#{req.body.read.inspect},"ct":"#{req.media_type}","x":"#{env["HTTP_X_TOKEN"]}"})]]
    when "/login"
      r = Rack::Response.new(["logged in"]); r.set_cookie("user", { value: req.params["u"], path: "/" }); r.finish
    when "/me" then [200, {}, ["me: #{req.cookies["user"]}"]]
    when "/redirect" then [302, { "location" => "/hello?name=redirected" }, []]
    when "/upload"
      f = req.params["file"]
      [200, {}, ["#{f[:filename]} #{f[:type]} #{f[:tempfile].read.bytesize} #{req.params["note"]}"]]
    when "/auth" then [200, {}, [env["HTTP_AUTHORIZATION"].to_s]]
    else [404, {}, ["not found"]]
    end
  end
end

class Spec
  include Rack::Test::Methods
  def app = App.new
  def run
    get "/hello"; p last_response.status, last_response.body, last_response.ok?, last_response.content_type
    get "/hello", name: "kenny"; p last_response.body
    post "/echo", "raw body", "CONTENT_TYPE" => "text/plain"; p last_response.body
    put "/echo", { a: 1, b: [2, 3] }; p last_response.body
    header "X-Token", "t0k"; delete "/echo"; p last_response.body
    get "/login", u: "bob"; get "/me"; p last_response.body
    p rack_mock_session.cookie_jar["user"]
    clear_cookies; get "/me"; p last_response.body
    set_cookie "user=alice"; get "/me"; p last_response.body
    get "/redirect"; p last_response.status, last_response.redirect?
    follow_redirect!; p last_response.body, last_request.url
    path = File.join(__dir__, "Gemfile")
    post "/upload", file: Rack::Test::UploadedFile.new(path, "text/plain"), note: "hi"
    p last_response.body
    post "/upload", file: Rack::Test::UploadedFile.new(StringIO.new("abcdef"), "application/octet-stream", original_filename: "data.bin")
    p last_response.body
    basic_authorize "user", "secret"; get "/auth"; p last_response.body
    get "/nowhere"; p last_response.not_found?, last_response.body
    p last_request.env["REQUEST_METHOD"], last_request.path
    p build_rack_test_session(:other).class
  end
end
Spec.new.run
p Rack::Test::Utils.build_nested_query(a: { b: [1, 2] }, c: "d e")
