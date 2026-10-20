# Print `-I <dir>` flags for a gem and its runtime dependencies (using installed gems).
# usage: ruby tools/gem-paths.rb rails 5.2.8.1
name, version = ARGV
seen = {}
add = lambda do |n, req|
  spec = Gem::Specification.find_all_by_name(n, req).max_by(&:version) rescue nil
  next unless spec
  next if seen[spec.name]
  seen[spec.name] = spec
  spec.runtime_dependencies.each { |d| add.(d.name, d.requirement) }
end
add.(name, version ? Gem::Requirement.new(version) : Gem::Requirement.default)
puts seen.values.flat_map { |s| s.full_require_paths.map { |p| "-I #{p}" } }.join(" ")
