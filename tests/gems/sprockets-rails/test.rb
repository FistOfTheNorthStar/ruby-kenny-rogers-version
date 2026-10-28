require "bundler/setup"
require "action_controller/railtie"
require "sprockets/railtie"
require "tmpdir"
require "fileutils"
require "rack/mock"

ROOT = Dir.mktmpdir
FileUtils.mkdir_p(File.join(ROOT, "app/assets/config"))
FileUtils.mkdir_p(File.join(ROOT, "app/assets/stylesheets"))
FileUtils.mkdir_p(File.join(ROOT, "app/assets/javascripts"))
File.write(File.join(ROOT, "app/assets/config/manifest.js"), "//= link application.css\n//= link application.js\n")
File.write(File.join(ROOT, "app/assets/stylesheets/application.css"), "body { color: blue; }\n")
File.write(File.join(ROOT, "app/assets/javascripts/application.js"), "console.log('hi');\n")

class AssetsApp < Rails::Application
  config.root = ROOT
  config.eager_load = false
  config.secret_key_base = "s" * 64
  config.logger = Logger.new(nil)
  config.hosts.clear
  config.assets.debug = false
  config.assets.digest = true
  config.assets.compile = true
  config.assets.prefix = "/assets"
  routes.append { get "/" => "home#index" }
end
class HomeController < ActionController::Base
  def index
    render inline: "<%= stylesheet_link_tag 'application' %>\n<%= javascript_include_tag 'application' %>\n<%= asset_path('application.css') %>"
  end
end
AssetsApp.initialize!
res = Rack::MockRequest.new(Rails.application).get("/")
p res.status
puts res.body.gsub(/-\h{64}/, "-DIGEST")
asset = Rails.application.assets["application.css"]
p asset.logical_path, asset.to_s
r2 = Rack::MockRequest.new(Rails.application).get("/assets/application.css")
p r2.status, r2.body, r2.headers["content-type"]
p Rails.application.config.assets.paths.map { _1.sub(ROOT, "ROOT") }.select { _1.start_with?("ROOT") }.sort
FileUtils.rm_rf(ROOT)
