# kenny run examples/rails_html/demo.rb $(ruby tools/gem-paths.rb actionpack 5.2.8.1)
require_relative "app"

status, _headers, body = ROUTES.call(Rack::MockRequest.env_for("/hi/kenny"))
text = +""
body.each { |part| text << part }
puts status, text
