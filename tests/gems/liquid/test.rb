require "bundler/setup"
require "liquid"

def r(src, assigns = {}, opts = {})
  Liquid::Template.parse(src, **opts).render(assigns)
end

puts r("Hello {{ name }}!", "name" => "World")
puts r("{{ name | upcase | append: '!' }} {{ 'x' | prepend: 'a' }}", "name" => "kenny")
puts r("{% if n > 2 %}big{% elsif n == 2 %}two{% else %}small{% endif %}", "n" => 2)
puts r("{% unless x %}no x{% endunless %} {% case c %}{% when 1 %}one{% when 2, 3 %}few{% else %}many{% endcase %}", "c" => 3)
puts r("{% for i in items %}{{ forloop.index }}:{{ i }}{% unless forloop.last %}, {% endunless %}{% endfor %}", "items" => %w[a b c])
puts r("{% for i in (1..5) limit:2 offset:1 reversed %}{{ i }}{% endfor %}")
puts r("{% for i in empty %}x{% else %}empty{% endfor %}", "empty" => [])
puts r("{% assign x = 'a,b,c' | split: ',' %}{{ x | join: '-' }} {{ x | size }} {{ x | first }} {{ x | last }}")
puts r("{% capture c %}captured {{ 1 | plus: 2 }}{% endcapture %}[{{ c }}]")
puts r("{{ 10 | minus: 3 | times: 2 | divided_by: 4 }} {{ 7 | modulo: 3 }} {{ 3.14159 | round: 2 }} {{ -5 | abs }}")
puts r("{{ 'hello world' | capitalize }} {{ 'Hello' | downcase }} {{ '  x ' | strip }}| {{ 'abc' | reverse }}")
puts r("{{ s | truncate: 8 }} {{ s | truncatewords: 2 }} {{ s | replace: 'o', '0' }} {{ s | remove_first: 'l' }}", "s" => "hello world foo")
puts r("{{ list | sort | join: ',' }} {{ list | uniq | size }} {{ list | map: 'x' }}", "list" => [3, 1, 2, 1])
puts r("{{ users | map: 'name' | join: ', ' }} {{ users | where: 'admin', true | map: 'name' }}",
  "users" => [{ "name" => "a", "admin" => true }, { "name" => "b", "admin" => false }])
puts r("{{ '<b>' | escape }} {{ 'a b&c' | url_encode }} {{ 'a\nb' | newline_to_br }} {{ '<p>x</p>' | strip_html }}")
puts r("{{ nil_var | default: 'dflt' }} {{ '' | default: 'empty' }} {{ false | default: 'f' }}")
puts r("{{ user.profile.name }} {{ user['profile']['age'] }} {{ arr[1] }} {{ arr.size }}", "user" => { "profile" => { "name" => "N", "age" => 3 } }, "arr" => [5, 6])
puts r("{% raw %}{{ not parsed }}{% endraw %} {% comment %}hidden{% endcomment %}ok")
puts r("{%- if true -%}  trimmed  {%- endif -%}|")
puts r("{% increment c %}{% increment c %}{% decrement d %} {% cycle 'a', 'b' %}{% cycle 'a', 'b' %}{% cycle 'a', 'b' %}")
puts r("{% tablerow i in (1..3) cols:2 %}{{ i }}{% endtablerow %}")
puts r("{% liquid\n  assign y = 5\n  echo y | plus: 1\n%}")
puts r("{{ 'now' | date: '%Y' | size }} {{ 1700000000 | date: '%Y-%m-%d' }}")
puts r("{{ 'a-b' | slice: 0, 2 }} {{ 'x' | append: 1 }} {{ 1.0 | plus: 1 }} {{ 'ab' | split: '' | size }}")

class Upper < Liquid::Tag
  def initialize(name, markup, ctx) = (super; @text = markup.strip)
  def render(_ctx) = @text.upcase
end
Liquid::Template.register_tag("shout", Upper)
module MyFilters
  def double(x) = x.to_s * 2
end
Liquid::Template.register_filter(MyFilters)
puts r("{% shout hey there %} {{ 'ab' | double }}")

class Prod < Liquid::Drop
  def initialize(n) = @n = n
  def name = "Prod #{@n}"
  def price = @n * 10
end
puts r("{{ p.name }} costs {{ p.price }} {{ p.missing }}", "p" => Prod.new(3))

t = Liquid::Template.parse("{{ x | nofilter }} {{ undefined }}")
puts t.render({ "x" => 1 }, strict_variables: true, strict_filters: true)
p t.errors.map(&:class)
begin
  Liquid::Template.parse("{% if %}", error_mode: :strict)
rescue Liquid::SyntaxError => e
  p e.class, e.message
end
begin
  Liquid::Template.parse("{% bogus %}")
rescue Liquid::SyntaxError => e
  p e.message
end
p Liquid::Template.parse("{{ 1 | divided_by: 0 }}").render
