# Raises LoadError the first two times it is required (like a gem whose native library is
# missing), then loads.
$optional_dep_runs = ($optional_dep_runs || 0) + 1
raise LoadError, "native library missing" if $optional_dep_runs < 3
module OptionalDep
  def self.ok? = true
end
