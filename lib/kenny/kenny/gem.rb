# The parts of RubyGems that libraries use at runtime (CRuby preloads RubyGems).
# Kenny resolves gems at build time, so there is no gem activation here: this file
# gives a few queries, and loads RubyGems' version checks (`Gem::Version`,
# `Gem::Requirement`) when first used.
# Loaded the first time a program refers to `Gem`.
module Gem
  VERSION = "3.5.22"

  class LoadError < ::LoadError
    attr_accessor :name, :requirement
  end
  class MissingSpecError < LoadError; end

  # RubyGems' own classes, loaded from the stdlib when first used
  autoload :Version, "rubygems/version"
  autoload :Requirement, "rubygems/requirement"
  autoload :Dependency, "rubygems/dependency"

  @loaded_specs = {}
  # the gem directories the program's gems were compiled from (set by Bundler.setup)
  @path = []

  class << self
    attr_reader :loaded_specs
    attr_writer :path

    def ruby_version = Version.new(RUBY_VERSION)
    def rubygems_version = Version.new(VERSION)
    def win_platform? = false
    def java_platform? = false
    def platforms = (@platforms ||= [Platform::RUBY, Platform.local])
    def platforms=(list)
      @platforms = list
    end
    def target_rbconfig = RbConfig::CONFIG

    # [file, line] of a caller (Gem::Deprecate's notes)
    def location_of_caller(depth = 1)
      loc = caller_locations(depth + 1, 1)&.first
      loc ? [loc.path, loc.lineno] : [nil, 0]
    end
    def path = @path
    def paths = @path
    def dir = @path.first || ""
    def default_dir = dir
    def user_dir = dir
    def bindir(_dir = nil) = ""
    def ruby = "kenny"
    def try_activate(_path) = false
    def suffixes = ["", ".rb"]
  end

  # RubyGems' own Gem::Deprecate (addressable extends it) and Gem::Platform (Platform.local:
  # the AWS SDK's user agent), from the stdlib when first used
  autoload :Deprecate, "rubygems/deprecate"
  autoload :Platform, "rubygems/platform"
end
