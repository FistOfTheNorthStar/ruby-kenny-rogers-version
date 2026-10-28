require "bundler/setup"
require "erubi"
require "erubi/capture_block"

src = Erubi::Engine.new("Hi <%= name %>!\n<% list.each do |x| %>\n- <%== x %>\n<% end %>\n<%# comment %>done\n").src
puts src
name = "Kenny"
list = ["<b>", "a&b"]
puts eval(src)

puts Erubi::Engine.new("<%= v %>|<%== v %>", escape: true).src
v = "<i>"
puts eval(Erubi::Engine.new("<%= v %>|<%== v %>", escape: true).src)
puts eval(Erubi::Engine.new("a <%= 1 + 1 %> b", bufvar: "@out", freeze: true).src)
p @out
puts Erubi.h("<script>alert('x' & \"y\")</script>")
p Erubi::Engine.new("x\n  <% if true %>\n  y\n  <% end %>\nz", trim: false).src
e = Erubi::Engine.new("<%= a %>", bufval: "String.new", literal_prefix: "{%", literal_postfix: "%}")
p e.bufvar, e.filename
a = 5
p eval(e.src)

class Ctx
  def initialize = @items = %w[x y]
  def render(tpl) = instance_eval(Erubi::Engine.new(tpl).src)
end
puts Ctx.new.render("<% @items.each do |i| %>[<%= i %>]<% end %>")
class Cap
  def upcase_form(&block) = "<form>#{@_buf.capture(&block).upcase}</form>"
  def render(tpl) = instance_eval(Erubi::CaptureBlockEngine.new(tpl, bufvar: "@_buf").src)
end
puts Cap.new.render("<%= upcase_form do %>inner <%= 1 %> <%== '<x>' %><% end %>!")
