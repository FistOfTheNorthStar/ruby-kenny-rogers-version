# `require "rubygems"` loads nothing: RubyGems is already there (CRuby preloads it; Kenny's
# Gem is the runtime's), not the stdlib's rubygems.rb compiled in, whose Kernel#require then
# tried to activate gems for a failing require (Discourse's config/boot.rb, google-protobuf)
p require("rubygems")
p require("rubygems")
p Gem::Version.new("1.2") < Gem::Version.new("1.10")
begin
  require "nonexistent/#{RUBY_VERSION.sub(/\.\d+$/, "")}/thing_c"
rescue LoadError => e
  puts e.message.sub(/\d+\.\d+/, "X.Y")
end
# (a RubyGems file that loads rubygems.rb by require_relative)
require "rubygems/local_remote_options"
p Gem::LocalRemoteOptions.class
begin
  require "nonexistent/#{RUBY_VERSION}/other_c"
rescue LoadError => e
  puts e.class
end
