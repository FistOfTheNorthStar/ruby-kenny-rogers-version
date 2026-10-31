require "bundler/setup"
require "sinatra/base"
require "rack/test"
require "json"

class App < Sinatra::Base
  set :environment, :test
  set :show_exceptions, false
  set :raise_errors, false
  enable :sessions
  set :session_secret, "x" * 64

  before { headers "X-Before" => "yes" }
  after { response.headers["X-Path"] = request.path_info }

  helpers do
    def shout(s) = s.upcase + "!"
  end

  get "/" do
    "hello"
  end
  get "/hi/:name" do |name|
    shout("hi #{name}")
  end
  get "/splat/*.*" do
    params["splat"].inspect
  end
  get %r{/num/(\d+)} do
    (params["captures"].first.to_i * 2).to_s
  end
  post "/json" do
    content_type :json
    data = JSON.parse(request.body.read)
    { got: data, n: data.size }.to_json
  end
  get "/erb" do
    erb "<ul><% items.each do |i| %><li><%= i %></li><% end %></ul>", locals: { items: %w[a b <c>] }
  end
  get "/halt" do
    halt 403, "nope"
  end
  get "/redir" do
    redirect to("/")
  end
  get "/count" do
    session[:n] = (session[:n] || 0) + 1
    session[:n].to_s
  end
  get "/boom" do
    raise ArgumentError, "kaboom"
  end
  get "/q" do
    params.to_h.sort.inspect
  end
  not_found do
    "custom 404"
  end
  error ArgumentError do
    status 422
    "handled #{env["sinatra.error"].message}"
  end
end

include Rack::Test::Methods
def app = App

get "/"
p [last_response.status, last_response.body, last_response.headers["X-Before"], last_response.headers["X-Path"]]
get "/hi/kenny"
p last_response.body
get "/splat/path/to/file.txt"
p last_response.body
get "/num/21"
p last_response.body
post "/json", JSON.generate({ "a" => [1, 2], "b" => nil }), "CONTENT_TYPE" => "application/json"
p [last_response.content_type, JSON.parse(last_response.body)]
get "/erb"
p last_response.body
get "/halt"
p [last_response.status, last_response.body]
get "/redir"
p [last_response.status, last_response.location]
3.times { get "/count" }
p last_response.body
get "/boom"
p [last_response.status, last_response.body]
get "/missing"
p [last_response.status, last_response.body]
get "/q", "z" => "1", "a" => %w[x y], "h" => { "k" => "v" }
p last_response.body
head "/"
p [last_response.status, last_response.body]
options "/"
p last_response.status
