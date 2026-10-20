# kenny build examples/rails_api/config.ru -o rails_api $(ruby tools/gem-paths.rb actionpack 5.2.8.1) && ./rails_api
require_relative "app"

run ROUTES
