# kenny build examples/rails_html/config.ru -o rails_html $(ruby tools/gem-paths.rb actionpack 5.2.8.1) && ./rails_html
require_relative "app"

run ROUTES
