# Calls the app directly (no HTTP), on an in-memory database:
#   kenny run examples/rails_api_ar/demo.rb $(ruby tools/gem-paths.rb actionpack 5.2.8.1) \
#     $(ruby tools/gem-paths.rb activerecord 5.2.8.1) $(ruby tools/gem-paths.rb sqlite3 1.7.3)
require_relative "app"

def show(method, path, params = nil)
  env = Rack::MockRequest.env_for(path, method: method, params: params)
  status, headers, body = APP.call(env)
  text = +""
  body.each { |part| text << part }
  puts "#{method} #{path} -> #{status} #{headers['Content-Type']} #{text}"
end

show "POST", "/posts", post: { title: "Hello Kenny", body: "Compiled Rails" }
show "POST", "/posts", post: { title: "Second" }
show "POST", "/posts", post: { title: "" }
show "POST", "/posts/1/comments", body: "nice"
show "POST", "/posts/1/comments", body: "fast"
show "POST", "/posts/2/comments", body: "hmm"
show "POST", "/posts/1/like"
show "POST", "/posts/1/like"
show "POST", "/posts/2/like"
show "GET", "/posts"
show "GET", "/posts?popular=1"
show "GET", "/posts/1"
show "GET", "/stats"
show "DELETE", "/posts/2"
show "GET", "/posts/2"
show "GET", "/stats"
