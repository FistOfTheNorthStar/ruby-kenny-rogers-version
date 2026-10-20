# `require "bundler/setup"` in a Kenny binary: the gems were resolved from Gemfile.lock and
# compiled in by `kenny build`, so setting up means recording them (see lib/kenny/bundler.rb).
require "bundler"
Bundler.setup
