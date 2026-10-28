require "bundler/setup"
require "bundler"

p Bundler.respond_to?(:require)
Bundler.require(:default)
p defined?(Base64) ? "Base64 loaded" : "not loaded"
p Base64.strict_encode64("bundler")
specs = Bundler.load.specs
p specs.map(&:name).sort
p Gem.loaded_specs.key?("base64"), Gem.loaded_specs["base64"].version.to_s
p Bundler.load.dependencies.map(&:name)
p Bundler.load.current_dependencies.map { |d| [d.name, d.requirement.to_s] }
p File.basename(Bundler.default_gemfile.to_s), File.basename(Bundler.default_lockfile.to_s)
p Bundler.root.to_s == Dir.pwd
p Bundler::VERSION.is_a?(String)
lock = Bundler::LockfileParser.new(File.read("Gemfile.lock"))
p lock.specs.map { |s| [s.name, s.version.to_s] }, lock.dependencies.keys
p Gem::Specification.find_by_name("base64").name
p Bundler.settings[:without].inspect.class
