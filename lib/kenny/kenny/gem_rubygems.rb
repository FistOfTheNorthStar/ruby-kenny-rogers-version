# The rest of RubyGems for Kenny's Gem (kenny/gem.rb): its own autoloads, as `require
# "rubygems"` would set them up (it loads nothing more under Kenny), and Gem.load_yaml.
# `kenny build` compiles this file in only when the program's code names one of these classes
# (crates/kenny-compiler/src/gem_extras.rs): their files are about 150 of RubyGems' and Psych.
module Gem
  autoload :ConfigFile, "rubygems/config_file"
  autoload :CIDetector, "rubygems/ci_detector"
  autoload :DependencyList, "rubygems/dependency_list"
  autoload :Installer, "rubygems/installer"
  autoload :Licenses, "rubygems/util/licenses"
  autoload :NameTuple, "rubygems/name_tuple"
  autoload :PathSupport, "rubygems/path_support"
  autoload :RequestSet, "rubygems/request_set"
  autoload :Resolver, "rubygems/resolver"
  autoload :Source, "rubygems/source"
  autoload :SourceList, "rubygems/source_list"
  autoload :SpecFetcher, "rubygems/spec_fetcher"
  autoload :SpecificationPolicy, "rubygems/specification_policy"
  autoload :Util, "rubygems/util"

  def self.load_yaml
    return if @yaml_loaded
    require "psych"
    require "rubygems/psych_tree"
    require "rubygems/safe_yaml"
    @yaml_loaded = true
  end
end
