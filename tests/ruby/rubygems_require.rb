# The real RubyGems, as `require "rubygems"` gives a program: its Kernel#require would look for
# gems again at run time, which a binary resolved at build time (bundler isn't even a gem in
# it). Discourse's config/boot.rb does this, then requires bundler/setup.
require "rubygems"
require "bundler/setup"
require "json"
require "time"

p Gem::Version.new("1.2") < Gem::Version.new("1.10")
p Gem::Specification.name
p JSON.generate(a: 1)
n = "strscan"
p require(n).class
p Kernel.instance_method(:require).owner
