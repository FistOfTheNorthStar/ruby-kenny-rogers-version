# Calls the Rails app directly (no HTTP): kenny run examples/rails_api/demo.rb $(ruby tools/gem-paths.rb actionpack 5.2.8.1)
require_relative "app"

def show(env)
  status, headers, body = ROUTES.call(env)
  text = +""
  body.each { |part| text << part }
  puts "#{status} #{headers['Content-Type']} #{text}"
end

show Rack::MockRequest.env_for("/hello/kenny")
show Rack::MockRequest.env_for("/items", method: "POST", params: { item: "guitar" })
show Rack::MockRequest.env_for("/nope")
