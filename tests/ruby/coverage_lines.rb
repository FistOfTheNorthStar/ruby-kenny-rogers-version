# Coverage of lines, branches and methods (the coverage library; the program is compiled
# with counters because it requires "coverage"): the files under coverage/ as CRuby counts
# them.
require "coverage"

p Coverage.running?
Coverage.start(lines: true, branches: true, methods: true)
p Coverage.running?, Coverage.state
require_relative "coverage/control"
require_relative "coverage/shapes"
result = Coverage.result
p Coverage.running?, Coverage.state, result.frozen?

result.keys.sort.each do |path|
  next unless path.include?("/coverage/")
  cov = result[path]
  puts "== #{File.basename(path)} #{cov.keys}"
  src = File.readlines(path)
  cov[:lines].each_with_index do |n, i|
    puts format("%4s %3d: %s", n.inspect, i + 1, src[i].rstrip)
  end
  p cov[:lines].frozen?
  cov[:branches].each do |base, targets|
    p base
    targets.each { |t, n| puts "  #{t.inspect} => #{n}" }
  end
  cov[:methods].sort_by { |k, _| [k[2], k[3], k[1].to_s] }.each do |(owner, name, *loc), n|
    p [owner.to_s, name, *loc, n]
  end
end
