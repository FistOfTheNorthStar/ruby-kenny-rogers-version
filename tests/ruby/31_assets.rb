# kenny-run-flags: -I "$(ruby -e 'print Gem::Specification.find_by_name("sassc", "2.4.0").gem_dir')/lib" -I "$(ruby -e 'print Gem::Specification.find_by_name("execjs", "2.10.0").gem_dir')/lib" -I "$(ruby -e 'print Gem::Specification.find_by_name("coffee-script", "2.4.1").gem_dir')/lib" -I "$(ruby -e 'print Gem::Specification.find_by_name("coffee-script-source", "1.12.2").gem_dir')/lib"
# The asset pipeline's compilers without libsass or Node: the sassc gem's API on the runtime's
# Sass compiler (grass), with a custom function calling back into Ruby, and ExecJS on the
# runtime's JavaScript engine (QuickJS), running the CoffeeScript compiler.
require "sassc"
require "execjs"
require "coffee-script"

module SassC::Script::Functions
  def asset_path(path, options = {})
    SassC::Script::Value::String.new("/assets/#{path.value}", :string)
  end

  def double_it(n)
    SassC::Script::Value::Number.new(n.value * 2, n.numerator_units)
  end
end

puts SassC::Engine.new(<<~SCSS, style: :expanded).render
  $c: #336699;
  @mixin pad($w) { padding: $w; }
  .a { color: darken($c, 10%); .b { width: double-it(21px); @include pad(3px); } }
  .img { background: url(asset-path("x.png")); }
SCSS
puts SassC::Engine.new(".x { .y { color: red; } }", style: :compressed).render
begin
  SassC::Engine.new(".x { color: ; ", filename: "bad.scss").render
rescue SassC::SyntaxError => e
  puts "SassC::SyntaxError: #{e.message.lines.first}"
end

puts ExecJS.runtime.name
ctx = ExecJS.compile("var add = function(a, b) { return a + b }; var obj = {x: [1, 2.5, {y: 'z'}], n: null};")
p ctx.call("add", 1, 2)
p ctx.eval("obj")
p ctx.exec("return 'from exec'")
p ctx.eval("undefined")
p ExecJS.eval("'héllo'.toUpperCase()")
begin
  ctx.eval("nope()")
rescue ExecJS::ProgramError => e
  puts "ExecJS::ProgramError: #{e.message.lines.first}"
end
begin
  ctx.eval("1 +")
rescue ExecJS::RuntimeError => e
  puts "ExecJS::RuntimeError"
end
puts CoffeeScript.compile("square = (x) -> x * x\nconsole.log square 3", bare: true)
