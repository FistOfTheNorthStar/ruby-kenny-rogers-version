require "bundler/setup"
require "action_controller"
require "action_dispatch"
require "rack/mock"

class ApplicationController < ActionController::Base
  before_action :set_user
  rescue_from ArgumentError, with: :bad
  private
  def set_user = (@user = request.headers["X-User"] || "anon")
  def bad(e) = render(plain: "bad: #{e.message}", status: 400)
end
class PostsController < ApplicationController
  protect_from_forgery with: :null_session
  def index = render(plain: "posts for #{@user} page=#{params[:page]}")
  def show = render(json: { id: params[:id].to_i, fmt: request.format.to_s })
  def create
    post = params.require(:post).permit(:title, tags: [])
    render json: post.to_h, status: :created
  end
  def boom = raise(ArgumentError, "nope")
  def go = redirect_to("/posts?page=2")
  def inline = render(inline: "<%= @user %>:<%= link_to 'p', post_path(7) %>")
  def head_only = head(:no_content)
  def cookie
    cookies[:flavor] = "choc"
    response.headers["X-Custom"] = "1"
    render plain: "ok"
  end
end

routes = ActionDispatch::Routing::RouteSet.new
routes.draw do
  resources :posts, only: [:index, :show, :create]
  get "boom" => "posts#boom"
  get "go" => "posts#go"
  get "inline" => "posts#inline"
  get "head" => "posts#head_only"
  get "cookie" => "posts#cookie"
  namespace :admin do get "stats", to: proc { [200, { "content-type" => "text/plain" }, ["stats"]] } end
end
PostsController.include routes.url_helpers
app = ActionDispatch::Cookies.new(routes)
req = Rack::MockRequest.new(app)
def show(r) = p([r.status, r.headers["content-type"], r.body])

show req.get("/posts?page=3", "HTTP_X_USER" => "kenny")
show req.get("/posts/42.json")
show req.post("/posts", params: { post: { title: "T", tags: ["a", "b"], evil: "x" } })
begin; req.post("/posts", params: { other: 1 }); rescue ActionController::ParameterMissing => e; p e.class, e.param; end
show req.get("/boom")
r = req.get("/go"); p r.status, r.headers["location"]
show req.get("/inline")
show req.get("/head")
r = req.get("/cookie"); p r.headers["set-cookie"], r.headers["x-custom"]
show req.get("/admin/stats")
show req.get("/nope") rescue p $!.class
p routes.url_helpers.post_path(5), routes.url_helpers.posts_url(host: "example.com", page: 2)
p routes.recognize_path("/posts/9"), routes.routes.map { |rt| rt.path.spec.to_s }.first(4)
params = ActionController::Parameters.new(a: 1, b: { c: 2 })
p params.permit(:a).to_h, params.permitted?, params.dig(:b, :c), params.slice(:a).to_unsafe_h
r = ActionDispatch::Request.new(Rack::MockRequest.env_for("/x?y=1", "HTTP_ACCEPT" => "application/json", "REMOTE_ADDR" => "1.2.3.4"))
p r.path, r.query_parameters, r.format.to_s, r.remote_ip, r.get?, r.ssl?
p Mime[:json].to_s, Mime::Type.lookup("text/html").symbol, ActionDispatch::Http::Headers.from_hash("CONTENT_TYPE" => "x")["Content-Type"]
