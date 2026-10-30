require "bundler/setup"
require "haml"

def render(src, scope = Object.new, locals = {}, opts = {}, &blk)
  Haml::Template.new(**opts) { src }.render(scope, locals, &blk)
end

puts render("%p Hello")
puts render("%div#main.box.wide\n  %span= 1 + 2\n  %em text")
puts render("%a{href: '/x', class: ['a', 'b'], 'data-id': 7} link")
puts render("%input{type: 'checkbox', checked: true, disabled: false}")
puts render("%p= name\n%p= html", Object.new, name: "Kenny", html: "<b>&</b>")
puts render("%p!= raw", Object.new, raw: "<i>raw</i>")
puts render("- items.each do |i|\n  %li= i * 2", Object.new, items: [1, 2, 3])
puts render("- if flag\n  %p yes\n- else\n  %p no", Object.new, flag: false)
puts render("%p Hi \#{who}, you are \#{age + 1}", Object.new, who: "Ann", age: 40)
puts render("/ a comment\n%p after")
puts render("-# silent\n%p after silent")
puts render("%br/\n%meta{charset: 'utf-8'}")
puts render("!!! 5\n%html\n  %body")
puts render(":plain\n  some <plain> text\n  \#{1+1}")
puts render(":javascript\n  var x = 1;")
puts render(":css\n  p { color: red; }")
puts render("%p<\n  inner")
puts render("%ul\n  %li> a\n  %li> b")
puts render("%p= yield", Object.new) { "yielded" }

class Helper
  def greet(n) = "hey #{n}"
  def initialize; @title = "Title"; end
end
puts render("%h1= @title\n%p= greet('you')", Helper.new)
puts render("%p{class: klass} dyn\n%p(id='old' class=\"x\") html-style", Object.new, klass: "k1")
puts render("%section\n  = 'line one'\n  %p\n    nested\n    more")
puts render("%p= value", Object.new, { value: nil })
puts render("%p&= '<amp>'")
puts render("%p= '<x>'", Object.new, {}, escape_html: false)
puts render("%div{style: 'a'}= [1, [2, 3]].flatten.join(',')")
puts render("- [1, 2].each do |x|\n  = x * 2\n%p= [1, 2].map { |x| x * 3 }.inspect")
puts render("%span.a{class: 'b'}\n%span{id: ['x', 'y']}")

begin
  render("%p\n    %bad indent\n  %q")
rescue Haml::Error => e
  p e.class, e.message
end
begin
  render("%p= nope_undefined")
rescue NameError => e
  p e.class, e.name
end
begin
  render("%p{")
rescue Haml::Error => e
  p e.class
end
