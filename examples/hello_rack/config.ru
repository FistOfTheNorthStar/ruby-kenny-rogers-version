# A plain Rack app. `kenny build config.ru -o server && ./server`
class Hello
  def initialize
    @hits = 0
  end

  def call(env)
    @hits += 1
    case env["PATH_INFO"]
    when "/"
      [200, { "content-type" => "text/plain" }, ["Hello from Kenny! (#{@hits} requests served by this worker)\n"]]
    when "/json"
      body = %({"path":"#{env["PATH_INFO"]}","query":"#{env["QUERY_STRING"]}","method":"#{env["REQUEST_METHOD"]}"})
      [200, { "content-type" => "application/json" }, [body]]
    when "/echo"
      [200, { "content-type" => "text/plain" }, [env["rack.input"].read]]
    else
      [404, { "content-type" => "text/plain" }, ["Not found: #{env["PATH_INFO"]}\n"]]
    end
  end
end

class Timing
  def initialize(app) = @app = app
  def call(env)
    status, headers, body = @app.call(env)
    headers["x-served-by"] = "kenny"
    [status, headers, body]
  end
end

use Timing
run Hello.new
