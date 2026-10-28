require "bundler/setup"
require "thor"
require "stringio"

class MyCLI < Thor
  def self.exit_on_failure? = false
  class_option :verbose, type: :boolean, aliases: "-v"

  desc "hello NAME", "say hello to NAME"
  method_option :shout, type: :boolean, desc: "uppercase"
  method_option :times, type: :numeric, default: 1
  def hello(name)
    msg = "Hello, #{name}!"
    msg = msg.upcase if options[:shout]
    options[:times].times { puts msg }
    puts "(verbose)" if options[:verbose]
  end

  desc "add A B", "add numbers"
  def add(a, b)
    puts a.to_i + b.to_i
  end

  desc "list [ITEMS...]", "list things"
  method_option :sep, type: :string, default: ","
  def list(*items)
    puts items.join(options[:sep])
  end

  class Sub < Thor
    desc "go", "sub go"
    def go = puts("sub go #{options.to_h}")
  end
  desc "sub SUBCOMMAND", "subcommands"
  subcommand "sub", Sub

  map "hi" => :hello
end

def capture
  old, $stdout = $stdout, StringIO.new
  oe, $stderr = $stderr, StringIO.new
  yield
  [$stdout.string, $stderr.string]
ensure
  $stdout, $stderr = old, oe
end

p capture { MyCLI.start(%w[hello World]) }
p capture { MyCLI.start(%w[hello World --shout --times 2 -v]) }
p capture { MyCLI.start(%w[hi there]) }
p capture { MyCLI.start(%w[add 2 40]) }
p capture { MyCLI.start(%w[list a b c --sep=-]) }
p capture { MyCLI.start(%w[sub go]) }
p capture { MyCLI.start(%w[add 1]) }
p capture { MyCLI.start(%w[bogus]) }
puts capture { MyCLI.start(%w[help]) }[0].gsub(/^\s*\S*test\.rb/, "  CLI")
puts capture { MyCLI.start(%w[help hello]) }[0].gsub(/\S*test\.rb/, "CLI")
p MyCLI.commands.keys.sort

class Gen < Thor::Group
  include Thor::Actions
  argument :name
  def one = say("one #{name}")
  def two = say("two #{name}", :green)
end
p capture { Gen.start(%w[x]) }
shell = Thor::Shell::Basic.new
p capture { shell.print_table([["a", "1"], ["bbb", "22"]]) }
p Thor::Util.snake_case("FooBar"), Thor::Util.camel_case("foo_bar")
