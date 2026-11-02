# The rest of RubyGems (kenny/gem_rubygems.rb) is compiled in when the code names one of its
# classes, as here; programs that don't (most) leave out its ~150 files and Psych.
p Gem::Version.new("1.2") < Gem::Version.new("1.10")
p Gem::NameTuple.new("rake", Gem::Version.new("13.0"), "ruby").full_name
p Gem::Util.respond_to?(:gunzip)
p Gem.respond_to?(:load_yaml)
