# kenny-run-flags: -I tests/ruby/loaded_features_by_hand
# A compiled-in file whose path the program put in $LOADED_FEATURES itself (a stand-in loaded
# in its place, as Discourse's rails_helper does for capybara) is not run by `require`
dir = File.expand_path("loaded_features_by_hand", __dir__)
$LOAD_PATH.unshift(dir)
$LOADED_FEATURES << File.join(dir, "standin.rb")
p require("standin")
p Kernel.require("standin")
p require(File.join(dir, "standin"))
p require("other")
p require("other")
# taken out again: it loads
$LOADED_FEATURES.delete(File.join(dir, "standin.rb"))
p require("standin")
p $LOADED_FEATURES.count { |f| f.end_with?("/standin.rb") }
