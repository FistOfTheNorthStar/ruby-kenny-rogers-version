require "bundler/setup"
require "tilt"
require "tilt/erb"
require "tilt/string"
require "tilt/plain"
require "tmpdir"

class Scope
  def initialize(name) = @name = name
  def shout(s) = s.upcase
end

erb = Tilt::ERBTemplate.new { "Hello <%= @name %>! <%= shout(greeting) %>\n<% 2.times do |i| %>row <%= i %>\n<% end %>" }
puts erb.render(Scope.new("Kenny"), greeting: "hi there")
str = Tilt::StringTemplate.new { 'Sum: #{a + b}, #{self.class}' }
puts str.render(Object.new, a: 1, b: 2)
layout = Tilt::ERBTemplate.new { "<main><%= yield %></main>" }
puts layout.render { "inner content" }
p Tilt::PlainTemplate.new { "plain <%= x %>" }.render

Dir.mktmpdir do |dir|
  path = File.join(dir, "page.html.erb")
  File.write(path, "<p><%= items.map { |i| i * 2 }.join(',') %></p>")
  t = Tilt.new(path)
  p t.class, File.basename(t.file), t.basename, t.name, t.eval_file == path
  puts t.render(nil, items: [1, 2, 3])
  p Tilt.registered?("erb"), Tilt.registered?("nope"), Tilt["str"], Tilt.template_for("x.erb")
  sp = File.join(dir, "x.str"); File.write(sp, 'n=#{n}')
  puts Tilt.new(sp).render(nil, n: 42)
end

mapping = Tilt::Mapping.new
mapping.register(Tilt::StringTemplate, "txt")
p mapping.registered?("txt"), mapping["foo.txt"], mapping.extensions_for(Tilt::StringTemplate)
p Tilt::ERBTemplate.new(trim: "-") { "a\n<%- if true -%>\nb\n<%- end -%>\nc" }.render
p Tilt::ERBTemplate.new(outvar: "@buf") { "<% @buf << 'x' %>y" }.render
begin
  Tilt::ERBTemplate.new { "<%= raise 'boom' %>" }.render
rescue => e
  p e.message
end
