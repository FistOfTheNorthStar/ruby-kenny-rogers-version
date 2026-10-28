# frozen_string_literal: true
# Kenny: Gem::Specification is RubyGems' own, but a compiled program's gems are the ones
# compiled in, so looking a gem up by name (find_by_name, find_all_by_name) answers from
# Gem.loaded_specs instead of scanning gemspec files on disk.
require "rubygems/specification"

class Gem::Specification
  class << self
    def find_all_by_name(name, *requirements)
      req = Gem::Requirement.create(requirements.flatten)
      Gem.loaded_specs.values.select { |s| s.name == name && req.satisfied_by?(s.version) }
    end

    def find_by_name(name, *requirements)
      found = find_all_by_name(name, *requirements).first
      return found if found
      req = Gem::Requirement.create(requirements.flatten)
      raise Gem::MissingSpecError, "Could not find '#{name}' (#{req}) among #{Gem.loaded_specs.size} total gem(s)"
    end
  end
end
