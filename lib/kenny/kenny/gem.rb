# The parts of RubyGems that libraries use at runtime (CRuby preloads RubyGems).
# Kenny resolves gems at build time, so there is no gem activation here: this file
# gives version checks (`Gem::Version`, `Gem::Requirement`) and a few queries.
# Loaded the first time a program refers to `Gem`.
module Gem
  VERSION = "3.5.22"

  class LoadError < ::LoadError
    attr_accessor :name, :requirement
  end
  class MissingSpecError < LoadError; end

  class Version
    include Comparable

    PATTERN = /\A\s*([0-9]+(?>\.[0-9a-zA-Z]+)*(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?)?\s*\z/

    def self.correct?(version)
      !version.nil? && PATTERN.match?(version.to_s)
    end

    def self.create(input)
      case input
      when Version then input
      when nil then nil
      else new(input)
      end
    end

    def initialize(version)
      unless self.class.correct?(version)
        raise ArgumentError, "Malformed version number string #{version}"
      end
      v = version.to_s.strip
      v = "0" if v.empty?
      @version = v.gsub("-", ".pre.")
    end

    def version = @version
    alias to_s version

    def inspect = "#<Gem::Version #{@version.inspect}>"

    def segments
      @segments ||= @version.scan(/[0-9]+|[a-z]+/i).map { |s| /\A\d+\z/.match?(s) ? s.to_i : s }
    end

    def canonical_segments
      segs = segments.dup
      segs.pop while segs.size > 1 && segs.last == 0
      segs
    end

    def prerelease? = @version.match?(/[a-zA-Z]/)

    def release
      return self unless prerelease?
      segs = segments.dup
      segs.pop while segs.any? { |s| s.is_a?(String) }
      self.class.new(segs.join("."))
    end

    def bump
      segs = segments.dup
      segs.pop while segs.any? { |s| s.is_a?(String) }
      segs.pop if segs.size > 1
      segs[-1] = segs[-1].succ
      self.class.new(segs.join("."))
    end

    def approximate_recommendation
      segs = segments.dup
      segs.pop while segs.any? { |s| s.is_a?(String) }
      segs.pop while segs.size > 2
      segs.push 0 while segs.size < 2
      "~> #{segs.join('.')}"
    end

    def <=>(other)
      other = self.class.new(other) if other.is_a?(String) && self.class.correct?(other)
      return unless other.is_a?(Version)
      return 0 if @version == other.version

      lhs = canonical_segments
      rhs = other.canonical_segments
      limit = [lhs.size, rhs.size].max - 1
      0.upto(limit) do |i|
        l = lhs[i] || 0
        r = rhs[i] || 0
        next if l == r
        return -1 if l.is_a?(String) && r.is_a?(Integer)
        return 1 if l.is_a?(Integer) && r.is_a?(String)
        return l <=> r
      end
      0
    end

    def eql?(other) = other.is_a?(Version) && @version == other.version
    def hash = canonical_segments.hash
  end

  class Requirement
    OPS = {
      "=" => ->(v, r) { v == r },
      "!=" => ->(v, r) { v != r },
      ">" => ->(v, r) { v > r },
      "<" => ->(v, r) { v < r },
      ">=" => ->(v, r) { v >= r },
      "<=" => ->(v, r) { v <= r },
      "~>" => ->(v, r) { v >= r && v.release < r.bump },
    }.freeze

    class BadRequirementError < ArgumentError; end

    def self.create(*inputs)
      inputs = inputs.flatten
      return inputs.first if inputs.size == 1 && inputs.first.is_a?(Requirement)
      new(*inputs)
    end

    def self.default = new(">= 0")

    def self.parse(obj)
      return ["=", obj] if obj.is_a?(Version)
      m = /\A\s*(=|!=|>|<|>=|<=|~>)?\s*([0-9][0-9a-zA-Z.\-]*)\s*\z/.match(obj.to_s)
      raise BadRequirementError, "Illformed requirement [#{obj.inspect}]" unless m
      [m[1] || "=", Version.new(m[2])]
    end

    attr_reader :requirements

    def initialize(*requirements)
      requirements = requirements.flatten.compact.uniq
      requirements = [">= 0"] if requirements.empty?
      @requirements = requirements.map { |r| self.class.parse(r) }
    end

    def satisfied_by?(version)
      raise ArgumentError, "Need a Gem::Version: #{version.inspect}" unless version.is_a?(Version)
      @requirements.all? { |op, rv| OPS.fetch(op).call(version, rv) }
    end
    alias === satisfied_by?
    alias =~ satisfied_by?

    def none? = @requirements == [[">=", Version.new("0")]]
    def to_s = @requirements.map { |op, v| "#{op} #{v}" }.join(", ")
    def as_list = @requirements.map { |op, v| "#{op} #{v}" }
    def ==(other) = other.is_a?(Requirement) && as_list.sort == other.as_list.sort
  end

  Dependency = Struct.new(:name, :requirement)

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
    def platforms = []
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
end
