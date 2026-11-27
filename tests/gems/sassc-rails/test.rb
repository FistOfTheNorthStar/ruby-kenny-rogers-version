require "bundler/setup"
require "action_controller/railtie"
require "sprockets/railtie"
require "sassc-rails"
require "tmpdir"
require "fileutils"

# What sassc-rails' importer adds to Sass's imports: ERB files, `x.css.scss`, and globs.
# (The sprockets context here is only what the importer uses: the ERB's `self`, and depend_on.)
class Context
  attr_reader :deps

  def initialize = @deps = []
  def depend_on(path) = @deps << path
  def brand = "#c0ffee"
end

Dir.mktmpdir do |dir|
  dir = File.realpath(dir)
  w = ->(path, src) { FileUtils.mkdir_p(File.join(dir, File.dirname(path))); File.write(File.join(dir, path), src) }
  w["app/application.scss", <<~SCSS]
    @import "vars";
    @import "theme";
    @import "legacy";
    @import "indented";
    @import "plain";
    @import "partials/*";
    @import "tree/**/*";
    @import "shared";
    .app { color: $brand; width: $w; }
  SCSS
  w["app/_vars.scss.erb", "$brand: <%= brand %>;\n$w: <%= 6 * 7 %>px;\n"]
  w["app/theme.scss.erb", ".theme { background: darken($brand, <%= 5 + 5 %>%); }\n"]
  w["app/legacy.css.scss", ".legacy { .inner { margin: $w; } }\n"]
  w["app/indented.sass.erb", ".indented\n  padding: <%= 1 + 1 %>px\n"]
  w["app/plain.css.erb", ".plain { order: <%= 3 %>; }\n"]
  w["app/partials/_b.scss", ".b { color: blue; }\n"]
  w["app/partials/a.scss", ".a { color: red; @import \"nested/*\"; }\n"]
  w["app/partials/c.scss.erb", ".c { z-index: <%= 10 %>; }\n"]
  w["app/partials/d.css", ".d { color: green; }\n"]
  w["app/partials/e.sass", ".e\n  color: pink\n"]
  w["app/partials/notes.txt", "not sass\n"]
  w["app/partials/nested/n.scss", ".n { top: 0; }\n"]
  w["app/tree/one.scss", ".one { left: 1px; }\n"]
  w["app/tree/deep/two.scss", ".two { left: 2px; }\n"]
  w["app/tree/deep/er/_three.scss", ".three { left: 3px; }\n"]
  w["lib/shared.scss.erb", ".shared { content: \"<%= self.class %>\"; }\n"]
  # a file that globs its own directory imports the others, not itself
  w["app/self/index.scss", "@import \"*\";\n.index { color: $x; }\n"]
  w["app/self/_x.scss", "$x: teal;\n"]
  w["app/bad.scss.erb", "<%= raise ArgumentError, 'in the ERB' %>\n"]

  render = lambda do |file, style: :expanded|
    context = Context.new
    engine = SassC::Engine.new(File.read(File.join(dir, file)),
      filename: File.join(dir, file), syntax: :scss, style: style, load_paths: [File.join(dir, "lib")],
      importer: SassC::Rails::Importer, sprockets: { context: context })
    css = engine.render
    [css, context.deps.uniq.map { _1.sub(dir, "") }.sort, engine.dependencies.map { _1.filename.sub(dir, "") }.sort]
  end

  css, deps, engine_deps = render["app/application.scss"]
  puts css.gsub(/\s+/, " ").gsub("; }", " }").gsub("} ", "}\n")
  puts deps
  p engine_deps - deps, (deps - engine_deps).all? { File.directory?(dir + _1) }
  puts render["app/self/index.scss", style: :compressed][0].strip.delete_suffix(";}") 
  w["app/broken.scss", "@import \"bad\";\n"]
  begin
    render["app/broken.scss"]
  rescue ArgumentError => e
    puts "#{e.class}: #{e.message}"
  end
  w["app/missing.scss", "@import \"nowhere/*\";\n"]
  begin
    render["app/missing.scss"]
  rescue SassC::SyntaxError => e
    puts e.class
  end
end

# The same through a Rails app's asset pipeline: the ERB runs in the Sprockets context (its
# asset helpers).
ROOT = File.realpath(Dir.mktmpdir)
css_dir = File.join(ROOT, "app/assets/stylesheets")
FileUtils.mkdir_p(File.join(ROOT, "app/assets/config"))
FileUtils.mkdir_p(File.join(css_dir, "widgets"))
FileUtils.mkdir_p(File.join(ROOT, "app/assets/images"))
File.write(File.join(ROOT, "app/assets/config/manifest.js"), "//= link application.css\n//= link logo.svg\n")
File.write(File.join(ROOT, "app/assets/images/logo.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>")
File.write(File.join(css_dir, "application.scss"), "@import \"settings\";\n@import \"widgets/*\";\nbody { color: $ink; }\n")
File.write(File.join(css_dir, "_settings.scss.erb"), "$ink: #<%= '1' * 3 %>;\n$logo: \"<%= asset_path('logo.svg') %>\";\n")
File.write(File.join(css_dir, "widgets/_card.scss"), ".card { background: url($logo); border: 1px solid $ink; }\n")
File.write(File.join(css_dir, "widgets/list.scss.erb"), ".list { columns: <%= 1 + 2 %>; background: image-url(\"logo.svg\"); }\n")

class AssetsApp < Rails::Application
  config.root = ROOT
  config.eager_load = false
  config.secret_key_base = "s" * 64
  config.logger = Logger.new(nil)
  config.assets.debug = false
  config.assets.digest = true
  config.assets.compile = true
  # (Kenny's Sass compiler writes no `/* line 1, file */` comments)
  config.sass.line_comments = false
end
AssetsApp.initialize!
squeeze = ->(css) { css.gsub(/-\h{64}/, "-DIGEST").gsub(/\s+/, " ").gsub("; }", " }").gsub("} ", "}\n") }
puts squeeze[Rails.application.assets["application.css"].to_s]
FileUtils.rm_rf(ROOT)

# Sprockets' own Sass processor (a Sprockets environment without sassc-rails) asks the
# compiler for a source map and combines it with its own: each rule maps back to the file and
# line it was written on.
Dir.mktmpdir do |dir|
  File.write(File.join(dir, "site.scss"), "@import \"colors\";\n\n.site {\n  width: 1px;\n  .title { color: $ink; }\n}\n")
  File.write(File.join(dir, "_colors.scss"), "$ink: red;\n.c {\n  color: $ink;\n}\n")
  env = Sprockets::Environment.new(dir)
  env.append_path(dir)
  asset = env["site.css"]
  map = asset.metadata[:map]
  section = map["sections"][0]["map"]
  p section.slice("file", "sources")
  mappings = Sprockets::SourceMapUtils.decode_vlq_mappings(section["mappings"], sources: section["sources"])
  asset.to_s.lines.each_with_index do |line, i|
    # (a rule's selector, at the start of its line: Sprockets' default style is nested,
    # which libsass indents and Kenny's compiler writes as expanded)
    next unless line =~ /\A( *)(\.[^{]*)\{/
    selector = $2.strip
    m = mappings.select { _1[:generated] == [i + 1, $1.size] }.last
    puts "#{selector} <- #{m && [m[:source], m[:original][0]].inspect}"
  end
end
