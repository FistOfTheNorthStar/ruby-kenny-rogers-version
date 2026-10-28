require "bundler/setup"
require "coderay"

ruby = <<~RUBY
  # A comment
  def hello(name = "world")
    puts "Hello, \#{name}!" if name =~ /\\w+/
    [1, 2.5, :sym, nil, 0x1F].each { |x| @count += x }
  end
RUBY
tokens = CodeRay.scan(ruby, :ruby)
p tokens.count
p tokens.each_slice(2).first(12).map { |t, k| [t, k] }
puts CodeRay.scan(ruby, :ruby).term
puts CodeRay.scan(ruby, :ruby).html
puts CodeRay.scan('{"a": [1, true, null], "b": "x"}', :json).html(css: :style)
puts CodeRay.scan("SELECT id, name FROM users WHERE id = 3;", :sql).term
puts CodeRay.highlight("<p class='x'>Hi &amp; bye</p>", :html, line_numbers: :inline, wrap: :div)
p CodeRay.scan(ruby, :ruby).statistic.split("\n").first(8)
p CodeRay::Scanners.list.sort.first(10)
p CodeRay.scan("x = 1", :ruby).text
p CodeRay.encode("a = [1]", :ruby, :yaml).lines.first(4) rescue p $!.class
p CodeRay.encode("a = [1]", :ruby, :json)
