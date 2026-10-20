# Code reloading in development mode: edits a view, a controller, a model and the routes of a
# running app, adds a new model file, raises from an action, and checks that each next request
# sees the change (Rails' development error pages included). Restores every file it touched.
#
#   ruby examples/blog/dev_session.rb APP_DIR PORT
#
# APP_DIR is a copy of examples/blog (the script edits its files) served in development mode on
# PORT, by the Kenny binary or by `bundle exec puma`. Prints one line per check and exits 1 if
# any check failed.
require "net/http"
require "uri"

APP = ARGV[0]
PORT = Integer(ARGV[1] || 3000)
abort "usage: dev_session.rb APP_DIR PORT" unless APP && File.directory?(File.join(APP, "app"))
WAIT = Float(ENV.fetch("DEV_SESSION_WAIT", "1.2")) # file watchers poll; give them time

def get(path)
  Net::HTTP.get_response(URI("http://127.0.0.1:#{PORT}#{path}"))
end

$saved = {}
def edit(rel)
  f = File.join(APP, rel)
  $saved[f] = (File.exist?(f) ? File.read(f) : nil) unless $saved.key?(f)
  File.write(f, yield(File.exist?(f) ? File.read(f) : ""))
  sleep WAIT
end

$failed = 0
def check(name, res, status, *needles)
  ok = res.code.to_i == status && needles.all? { |n| res.body.to_s.include?(n) }
  $failed += 1 unless ok
  puts "#{ok ? 'ok  ' : 'FAIL'} #{name} (#{res.code})"
  puts res.body.to_s[0, 800] unless ok
end

CONTROLLER = "app/controllers/posts_controller.rb"
# (re)writes PostsController#hello with the given body
def hello_action(body)
  edit(CONTROLLER) do |s|
    s = s.sub(/  # hello\n.*?  # \/hello\n/m, "")
    s.sub("class PostsController < ApplicationController\n",
          "class PostsController < ApplicationController\n  # hello\n  def hello\n    #{body}\n  end\n  # /hello\n")
  end
end

def asset(page, ext)
  page.body[%r{"(/assets/application[^"]*\.#{ext})"}, 1]
end

begin
  index = get("/posts")
  check "index", index, 200, "<h1>Posts</h1>"

  # assets compiled live (Sass, CoffeeScript), and again after an edit
  css, js = asset(index, "css"), asset(index, "js")
  check "stylesheet (Sass)", get(css.to_s), 200, "font-family"
  check "javascript (CoffeeScript)", get(js.to_s), 200, "Turbolinks"
  edit("app/assets/stylesheets/posts.scss") { |s| s + "\n$edited: #123456;\n.kenny-edit { color: $edited; }\n" }
  edit("app/assets/javascripts/posts.coffee") { |s| s + "\nwindow.kennyEdit = -> 42\n" }
  index = get("/posts")
  check "stylesheet after edit", get(asset(index, "css").to_s), 200, ".kenny-edit", "#123456"
  check "javascript after edit", get(asset(index, "js").to_s), 200, "kennyEdit"

  edit("app/views/posts/index.html.erb") { |s| s.sub("<h1>Posts</h1>", "<h1>Posts (edited view)</h1>") }
  check "view edit", get("/posts"), 200, "<h1>Posts (edited view)</h1>"

  edit("config/routes.rb") { |s| s.sub("root \"posts#index\"", "root \"posts#index\"\n  get \"hello\" => \"posts#hello\"") }
  hello_action 'render plain: "hello v1"'
  check "new route + action", get("/hello"), 200, "hello v1"

  hello_action 'render plain: "hello v2"'
  check "controller edit", get("/hello"), 200, "hello v2"

  edit("app/models/post.rb") { |s| s.sub("class Post < ApplicationRecord", "class Post < ApplicationRecord\n  def self.marker; \"model v1\"; end") }
  hello_action "render plain: Post.marker"
  check "model method", get("/hello"), 200, "model v1"
  edit("app/models/post.rb") { |s| s.sub("model v1", "model v2") }
  check "model edit", get("/hello"), 200, "model v2"

  edit("app/models/note.rb") { "class Note\n  def self.text; \"a new file\"; end\nend\n" }
  hello_action "render plain: Note.text"
  check "new model file", get("/hello"), 200, "a new file"

  hello_action 'raise "boom from the action"'
  check "exception page", get("/hello"), 500, "RuntimeError", "boom from the action", "posts_controller.rb"

  hello_action "render plain: undefined_thing"
  check "NameError page", get("/hello"), 500, "NameError", "undefined_thing"

  edit("config/routes.rb") { |s| s.sub("\n  get \"hello\" => \"posts#hello\"", "") }
  check "route removed", get("/hello"), 404, "Routing Error", "No route matches"

  check "index again", get("/posts"), 200, "<h1>Posts (edited view)</h1>"
ensure
  $saved.each { |f, s| s ? File.write(f, s) : File.delete(f) }
end
puts $failed.zero? ? "dev session: all checks passed" : "dev session: #{$failed} checks failed"
exit($failed.zero? ? 0 : 1)
