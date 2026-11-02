# kenny-run-flags: -I tests/ruby/require_runtime
# `require` as CRuby does it at runtime: $LOAD_PATH starts with the -I directories, required
# files are listed in $LOADED_FEATURES by their absolute paths, a directory added to
# $LOAD_PATH at runtime is searched, and Kernel#require wrappers see literal requires too.
dir = File.expand_path("require_runtime", __dir__)
show = ->(f) { f.sub(__dir__, "<dir>") }
p $LOAD_PATH.map(&show).include?("<dir>/require_runtime")
require "inc_lit"
name = "inc_comp"
require name
require_relative "require_runtime/rel_feat"
p $LOADED_FEATURES.grep(/inc_|rel_feat/).map(&show)

# a directory put on $LOAD_PATH at runtime (RSpec's and Rails' lib/): compiled-in files under
# it by their short name, and files from disk
$LOAD_PATH.unshift(File.join(dir, "lib"))
require "rt_lib"
p require("rt_lib")
n = "rt_nested/conf"
p require(n), require(n)
$LOAD_PATH.unshift(File.expand_path("require_runtime_disk", __dir__))
n = "disk_feat"
p require(n), require(n)
p $LOADED_FEATURES.grep(/rt_lib|rt_nested|disk_feat/).map(&show)

# Kernel#require wrappers chained by alias_method (mine, then Zeitwerk's core_ext pattern)
module Kernel
  alias_method :first_original_require, :require
  def require(name)
    puts "  first wrapper: #{name}"
    first_original_require(name)
  end
  module_function :require
end
module Kernel
  module_function
  alias_method :zw_original_require, :require
  class << self
    alias_method :zw_original_require, :require
  end
  def require(path)
    puts "  zeitwerk-style wrapper: #{path}"
    zw_original_require(path)
  end
end
puts "require 'inc_late' (compiled in, not loaded yet):"
p require("inc_late")
puts "require 'inc_lit' (loaded):"
p require("inc_lit")
puts "Kernel.require 'inc_comp':"
p Kernel.require("inc_comp")
puts "require_relative (not a require call):"
p require_relative("require_runtime/rel_feat")
begin
  require "kenny_not_there"
rescue LoadError
  puts "LoadError"
end
p $LOADED_FEATURES.grep(/inc_late/).map(&show)
