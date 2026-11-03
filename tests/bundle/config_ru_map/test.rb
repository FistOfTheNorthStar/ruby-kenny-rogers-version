# config.ru's `map path do ... end` (Discourse's config.ru maps its app under "/"): the binary
# serves a Rack::URLMap of the mapped apps, the `run` app at "/", and `use` middleware around
# them all, as Rack::Builder does. (A config.ru written as a program: a thread requests the
# served app, prints the answers to stderr and exits; stdout, where the server announces its
# port, goes to /dev/null. Under CRuby: `rackup` with these lines.)
require "bundler/setup"
require "rack"
require "socket"

ENV["PORT"] = (38_000 + Process.pid % 1000).to_s
$stdout.reopen(File::NULL)

Thread.new do
  get = lambda do |path|
    s = nil
    50.times do
      s = TCPSocket.new("127.0.0.1", ENV["PORT"].to_i)
      break
    rescue SystemCallError
      sleep 0.1
    end
    s.write("GET #{path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
    r = s.read
    s.close
    status = r[/\AHTTP\/1\.1 (\d+)/, 1]
    tag = r[/^x-mw: (.*)\r$/i, 1]
    "#{path} -> #{status} #{r.split("\r\n\r\n", 2)[1]} (#{tag})"
  end
  %w[/ /x /admin /admin/users /api/v1/t /apix /api].each { |p| $stderr.puts get.(p) }
  $stderr.flush
  exit!(0)
end

class Tag
  def initialize(app, name) = (@app, @name = app, name)

  def call(env)
    s, h, b = @app.call(env)
    [s, h.merge("x-mw" => [h["x-mw"], @name].compact.join(",")), b]
  end
end

use Tag, "outer"

map "/admin" do
  run ->(env) { [200, { "content-type" => "text/plain" }, ["admin #{env["SCRIPT_NAME"]}|#{env["PATH_INFO"]}"]] }
end

map "/api/v1" do
  use Tag, "inner"
  run ->(env) { [200, { "content-type" => "text/plain" }, ["api #{env["SCRIPT_NAME"]}|#{env["PATH_INFO"]}"]] }
end

run ->(env) { [200, { "content-type" => "text/plain" }, ["root #{env["SCRIPT_NAME"]}|#{env["PATH_INFO"]}"]] }
