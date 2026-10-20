# Bundler for Kenny binaries. A compiled program can't resolve gems at runtime: `kenny build`
# reads the Gemfile and Gemfile.lock, compiles the locked gems in, and writes what Bundler
# needs at runtime (groups, `require:` options, versions) into kenny/bundle.rb. This file
# implements the runtime part of Bundler's API on top of that.
require "kenny/bundle"
require "pathname"

module Bundler
  VERSION = "2.4.22"

  class BundlerError < StandardError; end
  class GemfileError < BundlerError; end
  class GemNotFound < BundlerError; end
  class GemRequireError < BundlerError
    attr_reader :orig_exception
    def initialize(orig_exception, msg)
      full_message = msg + "\nGem Load Error is: #{orig_exception.message}\n"\
        "Backtrace for gem load error is:\n#{orig_exception.backtrace&.join("\n")}\n"\
        "Bundler Error Backtrace:\n"
      super(full_message)
      @orig_exception = orig_exception
    end
  end

  # A Gemfile dependency (`gem "name", ...`) as captured at build time.
  class Dependency
    attr_reader :name, :groups, :autorequire

    def initialize(name, groups, autorequire, platform_ok, compiled)
      @name = name
      @groups = groups
      @autorequire = autorequire
      @platform_ok = platform_ok
      @compiled = compiled
    end

    def should_include? = @platform_ok
    def compiled? = @compiled
    def to_s = @name
  end

  # The parts of Gem::Specification that code asks Bundler / Gem.loaded_specs for.
  class StubSpecification
    attr_reader :name, :version, :full_gem_path, :full_require_paths

    def initialize(name, version, dir, paths)
      @name = name
      v, plat = version.split("-", 2)
      @version = Gem::Version.new(v)
      @platform = plat || "ruby"
      @full_gem_path = dir
      @full_require_paths = paths
    end

    def platform = @platform
    def gem_dir = @full_gem_path
    def require_paths = @full_require_paths
    def full_name = @platform == "ruby" ? "#{@name}-#{@version}" : "#{@name}-#{@version}-#{@platform}"
    def loaded_from = @full_gem_path.empty? ? nil : File.join(File.dirname(File.dirname(@full_gem_path)), "specifications", "#{full_name}.gemspec")
    def to_spec = self
    def inspect = "#<Bundler::StubSpecification #{full_name}>"
  end

  class Runtime
    def initialize
      @setup = false
    end

    def specs = Bundler.specs
    def dependencies = Bundler.dependencies
    def gems = specs

    def setup(*groups)
      return self if @setup
      @setup = true
      gem_dirs = specs.map { |s| File.dirname(File.dirname(s.full_gem_path)) unless s.full_gem_path.empty? }
      Gem.path = (gem_dirs.compact.uniq - ["."]).freeze
      specs.each do |s|
        Gem.loaded_specs[s.name] = s
        s.full_require_paths.each { |p| $LOAD_PATH << p unless $LOAD_PATH.include?(p) }
      end
      self
    end

    # Bundler::Runtime#require: require the default file of every dependency in `groups`.
    def require(*groups)
      groups.map!(&:to_sym)
      groups = [:default] if groups.empty?
      dependencies.each do |dep|
        next unless (dep.groups & groups).any? && dep.should_include?
        # an application server (Puma, ...): the binary's built-in HTTP server replaces it
        next if Kenny::REPLACED.include?(dep.name)
        unless dep.compiled?
          warn "kenny: gem '#{dep.name}' (groups: #{dep.groups.join(", ")}) was not compiled into this binary " \
               "(BUNDLE_WITHOUT=#{Kenny::WITHOUT.join(":")} at build time)"
          next
        end
        required_file = nil
        begin
          Array(dep.autorequire || dep.name).each do |file|
            file = dep.name if file == true
            required_file = file
            begin
              Kernel.require file
            rescue RuntimeError => e
              raise e if e.is_a?(LoadError)
              raise GemRequireError.new e, "There was an error while trying to load the gem '#{file}'."
            end
          end
        rescue LoadError => e
          raise if dep.autorequire || e.path != required_file
          if dep.autorequire.nil? && dep.name.include?("-")
            begin
              namespaced_file = dep.name.tr("-", "/")
              Kernel.require namespaced_file
            rescue LoadError => e
              raise if e.path != namespaced_file
            end
          end
        end
      end
    end
  end

  class << self
    def root
      @root ||= Pathname.new(Kenny::ROOT || Dir.pwd)
    end

    def default_gemfile = Pathname.new(Kenny::GEMFILE || File.join(Dir.pwd, "Gemfile"))
    def default_lockfile = Pathname.new(Kenny::LOCKFILE || File.join(Dir.pwd, "Gemfile.lock"))
    def bundle_path = root

    def dependencies
      @dependencies ||= Kenny::DEPENDENCIES.map { |row| Dependency.new(*row) }
    end

    def specs
      @specs ||= Kenny::SPECS.map { |row| StubSpecification.new(*row) }
    end

    def load
      @load ||= Runtime.new
    end
    alias environment load

    def setup(*groups)
      load.setup(*groups)
    end

    def require(*groups)
      setup(*groups).require(*groups)
    end

    def settings = {}
    def rubygems = Gem
    def frozen_bundle? = true

    # A Kenny binary has no Bundler environment variables to save and restore.
    def with_original_env = yield
    alias with_clean_env with_original_env
    alias with_unbundled_env with_original_env
    def original_env = ENV.to_hash
    def unbundled_env = ENV.to_hash
    alias clean_env unbundled_env
  end
end
