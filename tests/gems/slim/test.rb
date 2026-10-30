require "bundler/setup"
require "slim"

def render(src, scope = Object.new, locals = {}, opts = {}, &blk)
  Slim::Template.new(**opts) { src }.render(scope, locals, &blk)
end

puts render("p Hello")
puts render("div#main.box.wide\n  span = 1 + 2\n  em text")
puts render("a href='/x' class=['a', 'b'] data-id=7 link")
puts render("input type='checkbox' checked=true disabled=false")
puts render("p = name\np = html", Object.new, name: "Kenny", html: "<b>&</b>")
puts render("p == raw", Object.new, raw: "<i>raw</i>")
puts render("ul\n  - items.each do |i|\n    li = i * 2", Object.new, items: [1, 2, 3])
puts render("- if flag\n  p yes\n- else\n  p no", Object.new, flag: false)
puts render("p Hi \#{who}, you are \#{age + 1}", Object.new, who: "Ann", age: 40)
puts render("/! html comment\np after")
puts render("/ code comment\np after code comment")
puts render("br\nmeta charset='utf-8'\nimg src='a.png' /")
puts render("doctype html\nhtml\n  head\n    title T\n  body")
puts render("p\n  | piped text\n    continues here\n  ' with trailing space")
puts render("javascript:\n  var x = 1;")
puts render("css:\n  p { color: red; }")
puts render("p: a href='#' nested inline")
puts render("p = yield", Object.new) { "yielded" }
puts render("p data={a: 1, b: 'two'} attrs")
puts render("p(id='paren' class=\"x\") wrapped")
puts render("*{class: 'splat', id: 'sp'} splatted")
puts render("p class=klass dyn", Object.new, klass: nil)
puts render("<div>inline html</div>\np after")
puts render("p\n  = 'a'\n  = 'b'")
puts render("p = '<x>'", Object.new, {}, disable_escape: true)
puts render("div\n  p one\n  p two", Object.new, {}, pretty: true)
puts render("ruby:\n  x = 5\n  y = x * 2\np = y")

class Helper
  def greet(n) = "hey #{n}"
  def initialize; @title = "Title"; end
end
puts render("h1 = @title\np = greet('you')", Helper.new)
puts render("== '<b>bold</b>'\n= '<i>esc</i>'")

begin
  render("p\n    deep\n  shallow")
rescue Slim::Parser::SyntaxError => e
  p e.class
  puts e.message.lines.first
end
begin
  render("p = nope_undefined")
rescue NameError => e
  p e.class, e.name
end
begin
  render("p(id='x'")
rescue Slim::Parser::SyntaxError => e
  p e.class
end
