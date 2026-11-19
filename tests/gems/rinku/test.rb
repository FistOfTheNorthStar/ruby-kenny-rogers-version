require "bundler/setup"
require "rinku"
require "redcarpet"

puts Rinku.auto_link("see https://example.com and mail me@example.com now")
puts Rinku.auto_link("www.example.org/x?y=1", :urls)
puts Redcarpet::Markdown.new(Redcarpet::Render::HTML).render("*hi* [a](http://b)\n\n- one\n- two\n")
