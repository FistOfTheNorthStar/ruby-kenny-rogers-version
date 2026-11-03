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

  # Gem::Specification on first reference (not an autoload: RubyGems' own `class
  # Specification < BasicSpecification` must define it when RubyGems itself is loaded)
  def self.const_missing(name)
    if name == :Specification
      require "kenny/gem_specification"
      return const_get(:Specification, false)
    end
    super
  end

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

    # a GEM_REQUIREMENT_<NAME> environment variable's extra requirement
    def env_requirement(gem_name)
      @env_requirements_by_name ||= {}
      @env_requirements_by_name[gem_name] ||= begin
        req = ENV["GEM_REQUIREMENT_#{gem_name.upcase}"] || ">= 0"
        Gem::Requirement.create(req)
      end
    end

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

    # (Gem::Specification#date, #to_ruby)
    def source_date_epoch_string
      e = ENV["SOURCE_DATE_EPOCH"]
      e = nil if e && e.strip.empty?
      (e || DEFAULT_SOURCE_DATE_EPOCH.to_s).strip
    end
    def source_date_epoch = Time.at(source_date_epoch_string.to_i).utc.freeze

    # (Gem::Package, Gem::ConfigFile: as RubyGems' rubygems.rb and rubygems/defaults.rb)
    def configuration = (@configuration ||= Gem::ConfigFile.new([]))
    def configuration=(c)
      @configuration = c
    end
    def binary_mode = "rb"
    def user_home = (@user_home ||= (Dir.home.dup rescue File.expand_path("/")))
    def config_home = (@config_home ||= ENV["XDG_CONFIG_HOME"] || File.join(user_home, ".config"))
    def config_file
      @config_file ||= begin
        rc = File.join(user_home, ".gemrc")
        File.exist?(rc) ? rc : File.join(config_home, "gem", "gemrc")
      end
    end
    def cache_home = (@cache_home ||= ENV["XDG_CACHE_HOME"] || File.join(user_home, ".cache"))
    def data_home = (@data_home ||= ENV["XDG_DATA_HOME"] || File.join(user_home, ".local", "share"))
    def state_home = (@state_home ||= ENV["XDG_STATE_HOME"] || File.join(user_home, ".local", "state"))
    def state_file = (@state_file ||= File.join(state_home, "gem", "last_update_check"))
    def default_key_path = gem_home_file("gem-private_key.pem")
    def default_cert_path = gem_home_file("gem-public_cert.pem")
    def gem_home_file(name)
      f = File.join(user_home, ".gem", name)
      File.exist?(f) ? f : File.join(data_home, "gem", name)
    end
    private :gem_home_file
    def read_binary(path) = File.binread(path)
    def marshal_version = "#{Marshal::MAJOR_VERSION}.#{Marshal::MINOR_VERSION}"
    def operating_system_defaults = {}
    def platform_defaults = {}
    def default_sources = %w[https://rubygems.org/]
    def sources = (@sources ||= Gem::SourceList.from(configuration.sources || default_sources))
    def sources=(list)
      @sources = list && Gem::SourceList.from(list)
    end
  end
  DEFAULT_SOURCE_DATE_EPOCH = 315_619_200

  # RubyGems' own Gem::Deprecate (addressable extends it) and Gem::Platform (Platform.local:
  # the AWS SDK's user agent), from the stdlib when first used
  autoload :Deprecate, "rubygems/deprecate"
  autoload :Platform, "rubygems/platform"

  # The rest of RubyGems (Gem::ConfigFile, Gem::Installer, Gem::SourceList ..., Gem.load_yaml):
  # `kenny build` compiles kenny/gem_rubygems.rb in only when the program's code names one of
  # those classes (see crates/kenny-compiler/src/gem_extras.rs), so it isn't required by a
  # literal path here: it pulls in about 150 of RubyGems' files and Psych.
  EXTRAS = "kenny/gem_rubygems"
  private_constant :EXTRAS
  begin
    require EXTRAS
  rescue ::LoadError # (not Gem::LoadError)
  end
end
