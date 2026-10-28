require "bundler/setup"
require "pry"
require "stringio"

ENV["COLUMNS"] = "100"
ENV["LINES"] = "40"
Pry.config.input = StringIO.new("")
Pry.config.color = false
Pry.config.pager = false
Pry.config.correct_indent = false
Pry.config.history_load = false
Pry.config.history_save = false
Pry.config.should_load_rc = false
Pry.config.should_load_local_rc = false

class Robot
  # Beeps n times.
  def beep(n = 1) = "beep " * n
  def self.build = new
  attr_accessor :name
end

def run_pry(input, target = TOPLEVEL_BINDING)
  out = StringIO.new
  Pry.start(target, input: StringIO.new(input), output: out, hooks: Pry::Hooks.new)
  out.string.gsub(/0x\h+/, "0x")
end

puts run_pry(<<~IN)
  1 + 2
  [1, 2, 3].map { |x| x * 10 }
  def twice(x) = x * 2
  twice(21)
  "kenny".upcase
  Robot.new.beep(2)
  raise "oops"
  exit
IN

robot = Robot.new
robot.name = "R2"
puts run_pry(<<~IN, robot)
  self.class
  name
  @name
  ls -m
  exit
IN

puts run_pry(<<~IN)
  show-source Robot#beep
  show-doc Robot#beep
  cd Robot
  self
  ls
  cd ..
  exit
IN

out = StringIO.new
Pry.run_command("show-source Robot#beep", output: out)
puts out.string

p Pry::Code.new("def a\n  1\nend\n").with_line_numbers.to_s
p Pry::Method.from_str("Robot#beep").source_line
p Pry::VERSION
