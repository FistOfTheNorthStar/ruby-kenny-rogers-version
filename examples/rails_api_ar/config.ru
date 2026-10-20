# kenny build examples/rails_api_ar/config.ru -o rails_api_ar $(ruby tools/gem-paths.rb actionpack 5.2.8.1) \
#   $(ruby tools/gem-paths.rb activerecord 5.2.8.1) $(ruby tools/gem-paths.rb sqlite3 1.7.3) && ./rails_api_ar
#
# Data lives in a SQLite file ($DATABASE), shared by Kenny's worker processes.
ENV["DATABASE"] ||= "rails_api_ar.sqlite3"
require_relative "app"

# Workers are forked after boot: close the boot connection so each worker opens its own.
ActiveRecord::Base.connection_pool.disconnect!

run APP
