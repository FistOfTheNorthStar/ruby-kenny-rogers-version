# frozen_string_literal: true

# `rails assets:precompile` inside a Kenny binary. `kenny build` of a Rails app with the asset
# pipeline runs the freshly built binary with KENNY_ASSETS_PRECOMPILE=1: config.ru loads the app
# as usual, and then, instead of serving, the binary requires this file. The assets compile on
# the binary's own Sass compiler and JavaScript engine (lib/kenny/sassc.rb, lib/kenny/execjs.rb)
# into public/assets, with the manifest the app reads at runtime — what sprockets-rails'
# assets:precompile task does.
require "logger"

app = Rails.application
config = app.config
env = app.assets || Sprockets::Railtie.build_environment(app, true)
logger = Logger.new($stderr)
logger.level = Logger::INFO
logger.formatter = proc { |_severity, _time, _progname, msg| "#{msg}\n" }
env.logger = logger
output = File.join(config.paths["public"].first, config.assets.prefix)
manifest = Sprockets::Manifest.new(env.cached, output, config.assets.manifest)
manifest.compile(config.assets.precompile)
