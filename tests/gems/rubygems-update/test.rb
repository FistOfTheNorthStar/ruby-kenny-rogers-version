require "bundler/setup"
# rubygems-update ships RubyGems' own sources; programs use them through the Gem::* APIs.
require "rubygems"

v = Gem::Version.new("1.2.3")
p v, v.segments, v.bump.to_s, v.release.to_s, v.prerelease?
p Gem::Version.new("2.0.0.pre1").prerelease?, Gem::Version.new("2.0.0.pre1").release.to_s
p Gem::Version.new("1.10") > Gem::Version.new("1.9"), Gem::Version.new("1.0") == Gem::Version.new("1")
p %w[1.0.10 1.0.2 1.0.0.rc1 1.0 0.9].map { Gem::Version.new(_1) }.sort.map(&:to_s)
p Gem::Version.correct?("1.a.b"), Gem::Version.correct?("not valid!")
p Gem::Version.new("3.4.1").approximate_recommendation, Gem::Version.new("1.2.3") <=> "1.2.4"
r = Gem::Requirement.new("~> 1.2", ">= 1.2.1")
p r.to_s, r.satisfied_by?(Gem::Version.new("1.5")), r.satisfied_by?(Gem::Version.new("2.0")), r.satisfied_by?(Gem::Version.new("1.2.0"))
p Gem::Requirement.default.to_s, Gem::Requirement.create(">= 0").none?, Gem::Requirement.parse("~> 3.0")
p Gem::Requirement.new("= 1.0").exact?, Gem::Requirement.new("> 1", "< 3").as_list
d = Gem::Dependency.new("rails", "~> 8.0", ">= 8.0.1")
p d.name, d.requirement.to_s, d.type, d.match?("rails", "8.0.2"), d.match?("rails", "7.0")
spec = Gem::Specification.new do |s|
  s.name = "demo"
  s.version = "0.1.0"
  s.summary = "A demo gem"
  s.authors = ["Kenny"]
  s.files = ["lib/demo.rb"]
  s.license = "MIT"
  s.add_dependency "rack", ">= 2"
  s.add_development_dependency "rake", "~> 13"
  s.required_ruby_version = ">= 3.1"
end
p spec.full_name, spec.runtime_dependencies.map(&:to_s), spec.development_dependencies.map(&:to_s)
p spec.required_ruby_version.to_s, spec.require_paths, spec.licenses
p spec.satisfies_requirement?(Gem::Dependency.new("demo", "~> 0.1"))
puts spec.to_ruby.lines.grep(/s\.(name|version|summary|add_)/).join
pl = Gem::Platform.new("x86_64-linux")
p pl.cpu, pl.os, pl.to_s, Gem::Platform.new("arm64-darwin-23").os, Gem::Platform::RUBY
p Gem::Platform.new("x86_64-linux") === Gem::Platform.new("x86_64-linux-gnu")
p Gem::NameTuple.new("foo", Gem::Version.new("1.0"), "ruby").full_name
p Gem::Requirement.new("~> 1.0").satisfied_by?(Gem::Version.new("1.0.0.pre")) 
begin
  Gem::Version.new("bad version!")
rescue ArgumentError => e
  puts e.message
end

# files that ship in rubygems-update and aren't loaded by default
require "rubygems/text"
include Gem::Text
p levenshtein_distance("kitten", "sitting"), format_text("a " * 20, 20).lines.size
require "rubygems/package"
require "tmpdir"
Dir.mktmpdir do |dir|
  Dir.chdir(dir) do
    FileUtils.mkdir_p("lib")
    File.write("lib/demo.rb", "module Demo; end\n")
    out = nil
    Gem::DefaultUserInteraction.use_ui(Gem::SilentUI.new) { out = Gem::Package.build(spec) }
    p out, File.exist?(out)
    pkg = Gem::Package.new(out)
    p pkg.spec.full_name, pkg.spec.dependencies.map(&:to_s), pkg.contents
  end
end
