# Runs the same scaffold session (index, new, create with the CSRF token, show, edit,
# update, the .json views, a JSON create, destroy, a forged POST, static assets, 404s)
# against the blog served by Puma under Ruby and by the Kenny-compiled binary, each on a
# fresh copy of the database, and compares status, key headers and bodies (CSRF tokens,
# session cookies, request ids and timestamps are normalized).
#
#   ruby examples/blog/compare.rb [path/to/kenny-binary]     (system Ruby, from anywhere)
#
# The app: `rails _5.2.8.1_ new examples/blog --skip-bundle --skip-git`,
# `rails g scaffold Post title:string body:text`, `root "posts#index"`. The only other
# changes make `bundle lock --local` resolve against the gems installed here: puma
# '~> 3.11' -> '>= 3.11', sass-rails '~> 5.0' -> '>= 5.0', and in the development group
# web-console/spring/spring-watcher-listen (not installed) removed, listen '< 3.2' relaxed.
# Before building or comparing, create the database and the assets with Ruby:
#   cd examples/blog && RAILS_ENV=production SECRET_KEY_BASE=x bundle exec rails db:migrate assets:precompile
# Build:  kenny build examples/blog/config.ru -o blog
# Run:    cd examples/blog && RAILS_ENV=production RAILS_SERVE_STATIC_FILES=1 SECRET_KEY_BASE=... ../../blog
require "net/http"
require "uri"
require "json"
require "fileutils"

APP = __dir__
BIN = File.expand_path(ARGV[0] || File.join(APP, "../../blog"))
DB = File.join(APP, "db/production.sqlite3")
ENV_VARS = { "RAILS_ENV" => "production", "RAILS_SERVE_STATIC_FILES" => "1",
             "SECRET_KEY_BASE" => "compare" * 20, "RAILS_MAX_THREADS" => "1" }.freeze

def wait_for(port)
  100.times do
    Net::HTTP.get_response(URI("http://127.0.0.1:#{port}/robots.txt"))
    return
  rescue SystemCallError, EOFError
    sleep 0.1
  end
  abort "server on #{port} did not start"
end

class Session
  attr_reader :log

  def initialize(port)
    @http = Net::HTTP.new("127.0.0.1", port)
    @cookie = nil
    @log = []
  end

  def request(verb, path, form: nil, json: nil, headers: {})
    req = Net::HTTP.const_get(verb.capitalize).new(path, headers)
    req["Cookie"] = @cookie if @cookie
    if form
      req.set_form_data(form)
    elsif json
      req["Content-Type"] = "application/json"
      req.body = JSON.generate(json)
    end
    res = @http.request(req)
    if (c = res["Set-Cookie"])
      @cookie = c.split(";").first
    end
    @log << [verb, path, res]
    res
  end

  def token(res)
    res.body[/name="authenticity_token" value="([^"]+)"/, 1] || res.body[/name="csrf-token" content="([^"]+)"/, 1]
  end
end

def scenario(port)
  s = Session.new(port)
  s.request("get", "/")
  new_page = s.request("get", "/posts/new")
  created = s.request("post", "/posts", form: { "authenticity_token" => s.token(new_page), "post[title]" => "Hello <Kenny>",
                                                "post[body]" => "Compiled & served.\nSecond line." })
  show = URI(created["Location"].to_s).path
  s.request("get", show)
  s.request("get", "/posts")
  s.request("get", "/posts.json")
  s.request("get", "#{show}.json")
  edit = s.request("get", "#{show}/edit")
  s.request("post", show, form: { "_method" => "patch", "authenticity_token" => s.token(edit), "post[title]" => "Hello again" })
  s.request("get", show)
  page = s.request("get", "/posts/new")
  s.request("post", "/posts.json", json: { post: { title: "JSON", body: "via the API" } },
                                   headers: { "X-CSRF-Token" => s.token(page), "Accept" => "application/json" })
  s.request("get", "/posts.json")
  index = s.request("get", "/posts")
  s.request("post", show, form: { "_method" => "delete", "authenticity_token" => s.token(index) })
  s.request("get", "/")
  s.request("post", "/posts", form: { "post[title]" => "forged" }) # no token: 422
  s.request("get", "/posts/999999")                                 # RecordNotFound: 404
  s.request("get", "/no/such/route")                                # RoutingError: 404
  s.request("get", "/robots.txt")
  s.request("get", "/favicon.ico")
  html = s.log.first[2].body
  html.scan(%r{(?:href|src)="(/assets/[^"]+)"}).flatten.each { |a| s.request("get", a) }
  s.request("get", "/assets/nope-0123.css")
  s.log
end

def normalize(text)
  text.to_s
      .gsub(/(name="authenticity_token" value=")[^"]+/, '\1TOKEN')
      .gsub(/(name="csrf-token" content=")[^"]+/, '\1TOKEN')
      .gsub(/"(created_at|updated_at)":"[^"]+"/, '"\1":"TIME"')
      .gsub(/127\.0\.0\.1:\d+/, "HOST")
end

def transcript(log)
  log.map do |verb, path, res|
    headers = %w[Content-Type Location Cache-Control Content-Length].map { |h| "#{h}: #{res[h]}" if res[h] }.compact
    headers.reject! { |h| h.start_with?("Content-Length") } if res["Content-Type"].to_s.include?("html") || res["Content-Type"].to_s.include?("json")
    normalize(["#{verb.upcase} #{path} -> #{res.code}", *headers, res.body].join("\n"))
  end
end

def run(name, cmd, port)
  FileUtils.mkdir_p(File.join(APP, "tmp/pids"))
  FileUtils.cp(DB, "#{DB}.compare-backup")
  pid = Process.spawn(ENV_VARS.merge("PORT" => port.to_s), *cmd, chdir: APP, out: File::NULL, err: File::NULL)
  wait_for(port)
  t = transcript(scenario(port))
  puts "#{name}: #{t.size} requests"
  t
ensure
  if pid
    Process.kill("TERM", pid)
    Process.wait(pid)
  end
  FileUtils.mv("#{DB}.compare-backup", DB)
end

ruby = run("ruby (puma)", %w[bundle exec puma -t 1:1 -p 47311 config.ru], 47311)
kenny = run("kenny", [BIN], 47312)
diffs = ruby.zip(kenny).reject { |a, b| a == b }
diffs.each do |a, b|
  puts "--- ruby\n#{a[0, 1500]}\n+++ kenny\n#{b.to_s[0, 1500]}\n"
end
puts diffs.empty? ? "identical (#{ruby.size} responses)" : "#{diffs.size} of #{ruby.size} responses differ"
exit(diffs.empty? ? 0 : 1)
