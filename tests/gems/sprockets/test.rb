require "bundler/setup"
require "sprockets"
require "tmpdir"
require "fileutils"

Dir.mktmpdir do |dir|
  w = ->(path, src) { FileUtils.mkdir_p(File.join(dir, File.dirname(path))); File.write(File.join(dir, path), src) }
  w["javascripts/application.js", "//= require util\n//= require_tree ./lib\nvar app = util(1);\n"]
  w["javascripts/util.js", "function util(x) { return x + 1; }\n"]
  w["javascripts/lib/a.js", "var a = 'a';\n"]
  w["javascripts/lib/b.js", "var b = 'b';\n"]
  w["stylesheets/application.css", "/*\n *= require base\n */\nbody { color: red; }\n"]
  w["stylesheets/base.css", "html { margin: 0; }\n"]
  w["stylesheets/erb.css.erb", "p { width: <%= 2 * 5 %>px; }\n"]
  w["images/logo.svg", "<svg xmlns='http://www.w3.org/2000/svg'/>"]
  w["data/info.txt", "plain text"]

  env = Sprockets::Environment.new(dir)
  %w[javascripts stylesheets images data].each { |d| env.append_path(File.join(dir, d)) }
  env.cache = Sprockets::Cache::MemoryStore.new
  js = env["application.js"]
  puts js.to_s
  p js.logical_path, js.content_type, js.digest_path.sub(/-\h{64}/, "-DIGEST"), js.length, js.hexdigest.size
  p js.to_a.map(&:logical_path) rescue p env.find_asset("application.js", pipeline: :self).logical_path
  p js.metadata.keys.sort, js.metadata[:included].map { |u| File.basename(u.split("?").first) }
  css = env["application.css"]
  puts css.to_s
  puts env["erb.css"].to_s
  p env["logo.svg"].content_type, env["info.txt"].source, env["nope.js"]
  p env.find_asset("util.js").source
  manifest = Sprockets::Manifest.new(env, File.join(dir, "public/assets"))
  manifest.compile("application.js", "application.css", "logo.svg")
  p manifest.assets.keys.sort, manifest.files.size
  p Dir.children(File.join(dir, "public/assets")).map { _1.sub(/-\h{64}/, "-D") }.reject { _1.start_with?(".sprockets-manifest") }.sort
  p Sprockets::VERSION.class, env.mime_types.key?("application/javascript")
end
