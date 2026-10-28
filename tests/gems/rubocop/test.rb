require "bundler/setup"
require "rubocop"
require "stringio"

def capture
  out = StringIO.new
  old, $stdout = $stdout, out
  status = yield
  [status, out.string]
ensure
  $stdout = old
end

status, out = capture { RuboCop::CLI.new.run(%w[--cache false --no-color --format simple sample.rb]) }
p status
puts out

status, out = capture { RuboCop::CLI.new.run(%w[--cache false --no-color --format offenses sample.rb]) }
puts out

src = "def m( a )\n  x = 'b'\n  return a.length==0\nend\n"
$stdin = StringIO.new(src)
status, out = capture { RuboCop::CLI.new.run(%w[--cache false --no-color --autocorrect --format quiet --stdin fix.rb]) }
$stdin = STDIN
p status
puts out

config = RuboCop::ConfigStore.new.for_dir(".")
ps = RuboCop::ProcessedSource.new("x = [1,2]\nputs x .first\n", 3.4, "inline.rb")
team = RuboCop::Cop::Team.mobilize([RuboCop::Cop::Layout::SpaceAfterComma, RuboCop::Cop::Layout::SpaceAroundMethodCallOperator, RuboCop::Cop::Style::FrozenStringLiteralComment], config)
report = team.investigate(ps)
report.offenses.sort.each { |o| puts "#{o.line}:#{o.column} #{o.cop_name} #{o.message} [#{o.severity}]" }
p RuboCop::Cop::Registry.global.length > 400
p RuboCop::Cop::Registry.global.find_by_cop_name("Style/StringLiterals").department
