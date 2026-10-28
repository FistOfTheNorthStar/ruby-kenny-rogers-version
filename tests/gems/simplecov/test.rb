require "bundler/setup"
require "simplecov"

require "tmpdir"
SimpleCov.merging false
SimpleCov.coverage_dir File.join(Dir.tmpdir, "kenny-simplecov-test")
SimpleCov.at_exit {}
SimpleCov.start do
  enable_coverage :branch
  skip "/test.rb"
  group "Library", "lib"
end

require_relative "lib/shapes"
p Shapes.area(:square, 3)
p Shapes.area(:circle, 2)
p Shapes.describe(5)

result = SimpleCov.result
p result.class
result.files.each do |file|
  puts file.project_filename
  p [file.lines_of_code, file.covered_lines.size, file.missed_lines.size, file.covered_percent.round(2)]
  p file.missed_lines.map(&:line_number)
  p file.lines.map { |l| l.coverage }
  p [file.total_branches.size, file.covered_branches.size, file.missed_branches.size]
  p file.missed_branches.map { |b| [b.type, b.start_line] }
end
p result.groups.keys
p result.covered_percent.round(2)
p result.covered_lines, result.missed_lines
