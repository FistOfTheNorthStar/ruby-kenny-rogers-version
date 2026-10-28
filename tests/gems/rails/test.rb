require "bundler/setup"
require "rails/all"
require "rack/mock"

ENV["DATABASE_URL"] = "sqlite3::memory:"
class BlogApp < Rails::Application
  config.load_defaults Rails::VERSION::STRING.to_f
  config.root = __dir__
  config.eager_load = false
  config.logger = Logger.new(nil)
  config.secret_key_base = "s" * 64
  config.hosts.clear
  config.active_storage.service = :test rescue nil
  config.action_mailer.delivery_method = :test
  config.active_job.queue_adapter = :inline
  config.active_storage.service_configurations = { test: { service: "Disk", root: Dir.mktmpdir } }
  routes.append do
    resources :articles, only: [:index, :show, :create]
    root "articles#index"
  end
end
BlogApp.initialize!
ActiveRecord::Migration.verbose = false
ActiveRecord::Schema.define { create_table(:articles) { |t| t.string :title; t.integer :likes, default: 0 } }

class Article < ActiveRecord::Base
  validates :title, presence: true
end
class ApplicationController < ActionController::Base; end
class ArticlesController < ApplicationController
  skip_forgery_protection
  def index
    @articles = Article.order(:title)
    render inline: "<h1>Articles</h1><% @articles.each do |a| %><%= link_to a.title, article_path(a) %>;<% end %>"
  end
  def show
    article = Article.find(params[:id])
    render json: article.as_json(only: [:title, :likes])
  end
  def create
    a = Article.new(params.require(:article).permit(:title))
    if a.save then redirect_to article_path(a) else render json: a.errors, status: :unprocessable_entity end
  end
end
class Notifier < ActionMailer::Base
  def hi = mail(to: "x@y.z", from: "a@b.c", subject: "hi", body: "b")
end
class LikeJob < ActiveJob::Base
  def perform(article) = article.increment!(:likes)
end

Article.create!(title: "Zebra"); Article.create!(title: "Apple")
app = Rack::MockRequest.new(Rails.application)
r = app.get("/"); p r.status, r.body
r = app.get("/articles/1"); p r.status, r.body
r = app.post("/articles", params: { article: { title: "New" } }); p r.status, r.headers["location"]
r = app.post("/articles", params: { article: { title: "" } }); p r.status, r.body
r = app.get("/articles/999"); p r.status
LikeJob.perform_later(Article.first)
p Article.first.likes
Notifier.hi.deliver_now; p ActionMailer::Base.deliveries.size
p Rails.env, Rails.version == Rails::VERSION::STRING, Rails.application.class.name, Rails.root.basename.to_s
p Rails.application.config.load_defaults_version rescue p Rails.application.config.loaded_config_version
p Rails.application.routes.url_helpers.articles_path, Rails.application.config.filter_parameters.first(3)
p ActiveSupport::OrderedOptions === Rails.application.config.x, Rails.application.message_verifier(:x).verify(Rails.application.message_verifier(:x).generate(1))
