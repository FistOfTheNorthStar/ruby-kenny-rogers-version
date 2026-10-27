# Runs the same scaffold session against the Rails 8 blog served by Puma under Ruby and by
# the Kenny-compiled binary, each on a fresh copy of the databases, and compares status, key
# headers and bodies (CSRF tokens, session cookies and timestamps are normalized).
#
#   ruby examples/blog8/compare.rb [path/to/kenny-binary]
#   (COMPARE_PORT=n uses ports n and n+1 instead of 47321/47322)
#
# The app: `rails _8.1.4_ new examples/blog8 --skip-git --skip-docker --skip-kamal
# --skip-thruster --skip-ci --skip-test --skip-system-test`, then
# `rails g scaffold Post title:string body:text published:boolean`,
# `rails g scaffold Comment post:references author:string body:text` and `root "posts#index"`.
# config/database.yml names the production databases (the generator leaves them out).
# Before building or comparing, create the databases and the assets with Ruby:
#   cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=x bin/rails db:prepare assets:precompile
# Build:  kenny build examples/blog8/config.ru -o blog8
# Run:    cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=... ../../blog8
require "net/http"
require "uri"
require "json"
require "fileutils"

APP = __dir__
BIN = File.expand_path(ARGV[0] || File.join(APP, "../../blog8"))
DBS = %w[production production_cache production_queue production_cable].map { |d| File.join(APP, "storage/#{d}.sqlite3") }
# A database with its write-ahead log: a server stopped by TERM can leave rows in the -wal file,
# so it is saved and restored (or removed) along with the database.
DB_FILES = DBS.flat_map { |db| [db, "#{db}-wal", "#{db}-shm"] }
ENV_VARS = { "RAILS_ENV" => "production", "SECRET_KEY_BASE" => "compare" * 20, "RAILS_MAX_THREADS" => "1" }.freeze

def wait_for(port)
  150.times do
    return if Net::HTTP.get_response(URI("http://127.0.0.1:#{port}/up")).code == "200"
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
                                                "post[body]" => "Compiled & served.\nSecond line.", "post[published]" => "1" })
  show = URI(created["Location"].to_s).path
  post_id = show[%r{\d+}]
  s.request("get", show)
  s.request("get", "/posts")
  s.request("get", "/posts.json")
  s.request("get", "#{show}.json")
  edit = s.request("get", "#{show}/edit")
  s.request("post", show, form: { "_method" => "patch", "authenticity_token" => s.token(edit),
                                  "post[title]" => "Hello again", "post[published]" => "0" })
  s.request("get", show)
  # comments belong to a post
  cnew = s.request("get", "/comments/new")
  comment = s.request("post", "/comments", form: { "authenticity_token" => s.token(cnew), "comment[post_id]" => post_id,
                                                   "comment[author]" => "Kenny", "comment[body]" => "Know when to hold 'em" })
  cshow = URI(comment["Location"].to_s).path
  s.request("get", cshow)
  s.request("get", "/comments.json")
  bad = s.request("get", "/comments/new")
  s.request("post", "/comments", form: { "authenticity_token" => s.token(bad), "comment[post_id]" => "999999",
                                         "comment[author]" => "nobody" }) # belongs_to validation: 422
  page = s.request("get", "/posts/new")
  s.request("post", "/posts.json", json: { post: { title: "JSON", body: "via the API", published: true } },
                                   headers: { "X-CSRF-Token" => s.token(page), "Accept" => "application/json" })
  s.request("get", "/posts.json")
  cpage = s.request("get", cshow)
  s.request("post", cshow, form: { "_method" => "delete", "authenticity_token" => s.token(cpage) })
  index = s.request("get", show)
  s.request("post", show, form: { "_method" => "delete", "authenticity_token" => s.token(index) })
  s.request("get", "/")
  s.request("post", "/posts", form: { "post[title]" => "forged" }) # no token: 422
  s.request("get", "/posts/999999")                                 # RecordNotFound: 404
  s.request("get", "/no/such/route")                                # RoutingError: 404
  s.request("get", "/robots.txt")
  s.request("get", "/icon.png")
  html = s.log.first[2].body
  html.scan(%r{(?:href|src)="(/assets/[^"]+)"}).flatten.each { |a| s.request("get", a) }
  html.scan(%r{"(/assets/[^"]+\.js)"}).flatten.uniq.each { |a| s.request("get", a) } # the import map
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
  DB_FILES.each { |f| FileUtils.cp(f, "#{f}.compare-backup") if File.exist?(f) }
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
  DB_FILES.each do |f|
    if File.exist?("#{f}.compare-backup")
      FileUtils.mv("#{f}.compare-backup", f)
    else
      FileUtils.rm_f(f)
    end
  end
end

port = Integer(ENV.fetch("COMPARE_PORT", "47321"))
ruby = run("ruby (puma)", %W[bundle exec puma -t 1:1 -p #{port} config.ru], port)
kenny = run("kenny", [BIN], port + 1)
diffs = ruby.zip(kenny).reject { |a, b| a == b }
diffs.each do |a, b|
  puts "--- ruby\n#{a[0, 1500]}\n+++ kenny\n#{b.to_s[0, 1500]}\n"
end
puts diffs.empty? ? "identical (#{ruby.size} responses)" : "#{diffs.size} of #{ruby.size} responses differ"
exit(diffs.empty? ? 0 : 1)
