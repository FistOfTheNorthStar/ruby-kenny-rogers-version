require "bundler/setup"
require "rails"
require "action_controller/railtie"
require "rack/mock"

$order = []
class MyRailtie < Rails::Railtie
  railtie_name :my_railtie
  config.my_railtie = ActiveSupport::OrderedOptions.new
  config.my_railtie.greeting = "hello"
  initializer "my_railtie.setup" do |app|
    $order << :railtie
    app.config.x.from_railtie = config.my_railtie.greeting
  end
end
module Blorgh
  class Engine < ::Rails::Engine
    isolate_namespace Blorgh
    initializer("blorgh.init") { $order << :engine }
  end
end

class MiniApp < Rails::Application
  config.root = __dir__
  config.eager_load = false
  config.logger = Logger.new(nil)
  config.secret_key_base = "s" * 64
  config.hosts.clear
  config.x.payment.retries = 3
  config.middleware.use(Class.new {
    def initialize(app) = @app = app
    def call(env) = @app.call(env).tap { |r| r[1]["x-mini"] = "1" }
  })
  initializer("mini.init", after: "my_railtie.setup") { $order << :app }
  config.before_initialize { $order << :before_initialize }
  config.after_initialize { $order << :after_initialize }
  routes.append do
    get "/" => "pages#home"
    get "/boom" => "pages#boom"
    mount Blorgh::Engine => "/blorgh"
  end
end
class PagesController < ActionController::Base
  def home = render(plain: "home #{Rails.application.config.x.from_railtie} #{Rails.configuration.x.payment.retries}")
  def boom = raise("kaboom")
end
MiniApp.initialize!
Blorgh::Engine.routes.draw { get "/hello" => proc { [200, { "content-type" => "text/plain" }, ["engine hello"]] } }

p $order
p Rails.application.initialized?, Rails.env, Rails.env.production?, Rails.groups, Rails.application.class.module_parent_name
r = Rack::MockRequest.new(Rails.application)
res = r.get("/"); p res.status, res.body, res.headers["x-mini"]
res = r.get("/blorgh/hello"); p res.status, res.body
res = r.get("/boom"); p res.status
res = r.get("/missing"); p res.status
p Rails.application.railties.map(&:class).map(&:name).include?("Blorgh::Engine"), Blorgh::Engine.engine_name, Blorgh.table_name_prefix
p Rails::Railtie.subclasses.map(&:name).compact.include?("MyRailtie"), MiniApp.config.my_railtie.greeting
p Rails.application.middleware.map(&:name).first(5)
p Rails.application.config.paths["app/controllers"].to_a.map { File.basename(_1) }, Rails.application.config.paths["config/routes.rb"].first.then { File.basename(_1) }
p Rails.application.env_config.keys.grep(/secret|cookies/).sort.first(3)
p Rails.backtrace_cleaner.class, Rails.application.key_generator.generate_key("salt", 8).bytesize
