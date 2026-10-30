require "bundler/setup"
require "rouge"

code = <<~RUBY
  # comment
  def greet(name, n = 1)
    puts "hi \#{name}" * n
    :sym if n > 2
  end
  class Foo < Bar; @x = [1, 2.5, 0x1f]; end
RUBY

lexer = Rouge::Lexers::Ruby.new
tokens = lexer.lex(code).to_a
p tokens.size
tokens.first(12).each { |t, v| p [t.qualname, v] }

puts Rouge::Formatters::HTML.new.format(lexer.lex("x = 1 + 2"))
puts Rouge::Formatters::HTMLInline.new(Rouge::Themes::Github.new).format(Rouge::Lexers::Ruby.new.lex("def a; end"))
puts Rouge::Formatters::HTMLTable.new(Rouge::Formatters::HTML.new).format(lexer.lex("a\nb\n"))
puts Rouge::Formatters::HTMLLinewise.new(Rouge::Formatters::HTML.new, class: "line-%i").format(lexer.lex("a\nb\n"))
puts Rouge::Formatters::HTMLPygments.new(Rouge::Formatters::HTML.new, "py").format(lexer.lex("nil"))
p Rouge::Formatters::Terminal256.new(Rouge::Themes::Monokai.new).format(lexer.lex("x = 'y'"))
puts Rouge.highlight("{\"a\": [1, true, null]}", "json", "html")
puts Rouge.highlight("SELECT a FROM t WHERE b = 1;", "sql", "html")
puts Rouge.highlight("<div class=\"x\">hi</div>", "html", "html")
puts Rouge.highlight("function f(a) { return a * 2; }", "javascript", "html")
puts Rouge.highlight("def f(x):\n    return x  # hi", "python", "html")
puts Rouge.highlight("key: value\nlist:\n  - 1", "yaml", "html")
puts Rouge.highlight("fn main() { let x: u8 = 5; }", "rust", "html")
puts Rouge.highlight("#include <stdio.h>\nint main(){return 0;}", "c", "html")
puts Rouge.highlight("echo $HOME | grep x", "shell", "html")
puts Rouge.highlight("plain <text>", "plaintext", "html")
p Rouge::Lexer.find("ruby").tag, Rouge::Lexer.find("rb")&.tag, Rouge::Lexer.find("nope")
p Rouge::Lexer.find_fancy("python").tag
p Rouge::Lexer.guess(filename: "foo.rb").tag
p Rouge::Lexer.guess(filename: "x.py").tag
p Rouge::Lexer.guess(mimetype: "application/json").tag
p Rouge::Lexer.guess(source: "#!/usr/bin/env ruby\nputs 1").tag
p Rouge::Lexers::Ruby.aliases, Rouge::Lexers::Ruby.filenames.first(3)
p Rouge::Lexer.all.size > 100
css = Rouge::Themes::ThankfulEyes.render(scope: ".hl")
p css.lines.size > 10, css.lines.first.strip
puts Rouge::Themes::Base16.mode(:dark).render(scope: ".x").lines.first(3)
p Rouge::Lexers::ERB.new.lex("<%= x %> y").map { |t, v| [t.qualname, v] }
p Rouge::Token::Tokens::Name::Function.qualname, Rouge::Token::Tokens::Keyword.shortname
p Rouge::Lexers::Diff.new.lex("+a\n-b\n c\n").map { |t, v| [t.shortname, v] }

class MyLexer < Rouge::RegexLexer
  tag "mine"
  state :root do
    rule %r/\d+/, Num
    rule %r/[a-z]+/, Name
    rule %r/\s+/, Text
    rule %r/./, Punctuation
  end
end
p MyLexer.new.lex("abc 12 !").to_a.map { |t, v| [t.shortname, v] }
p Rouge::Lexer.find("mine") == MyLexer
begin
  Rouge.highlight("x", "nolexer", "html")
rescue => e
  p e.class
end
