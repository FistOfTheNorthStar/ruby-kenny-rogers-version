# The ways of reaching a file through `require`.
# a cycle the build reaches from one side and the program from the other
require_relative "require_forms/cycle/a" if ARGV[0] == "never"
require_relative "require_forms/cycle/b"
p B_VAL, A_VAL

# a directory added to $LOAD_PATH at runtime
$LOAD_PATH.unshift File.join(__dir__, "require_forms/lib")
require "rf_thing"
p RF_THING
p require("rf_thing")

# listed in $LOADED_FEATURES, under the path, after it has run
require_relative "require_forms/listed"
p $LOADED_FEATURES.any? { |f| f.end_with?("require_forms/listed.rb") }
p LISTED_DURING
require "erb"
p $LOADED_FEATURES.grep(/\/erb\.rb\z/).size

# literal requires reach an overridden Kernel#require, with the name as written
module Kernel
  alias_method :rf_orig_require, :require
  def require(name)
    puts "hook #{name}" if name.start_with?("base64", "rf_")
    rf_orig_require(name)
  end
end
require "base64"
p Base64.strict_encode64("x")
p require("base64")
require "rf_thing"
# (require_relative isn't `require`, in CRuby either)
require_relative "require_forms/quiet"

# a class's own `require` method
class Own
  def require(n) = "own #{n}"
  def go = require "base64"
end
p Own.new.go

# a removed feature is loaded again by its path
load_me = File.join(__dir__, "require_forms/again.rb")
AGAIN = []
require load_me
$LOADED_FEATURES.delete_if { |f| f.end_with?("require_forms/again.rb") }
p require(load_me)
p AGAIN
