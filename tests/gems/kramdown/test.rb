require "bundler/setup"
require "kramdown"

doc = Kramdown::Document.new(<<~MD)
  # Title {#top}

  Some *emphasis*, **strong**, `code`, and a [link](http://x.io "T").
  Auto <http://example.com> and an abbr HTML.

  * one
  * two
    1. nested

  > quote

  | a | b |
  |---|--:|
  | 1 | 2 |

  ~~~ ruby
  puts "hi"
  ~~~

  Term
  : Definition

  Footnote[^1].
  {: .klass #pid}

  [^1]: The note.

  *[HTML]: Hyper Text

  ---
  <div markdown="1">*inside*</div>
MD
puts doc.to_html
p doc.warnings
p doc.root.children.map(&:type)

puts Kramdown::Document.new("# A\n\ntext _x_ \"q\" -- ...").to_kramdown
puts Kramdown::Document.new("<p>html <b>in</b></p>", input: "html").to_kramdown
puts Kramdown::Document.new("# H\n\npara", auto_ids: false).to_html
puts Kramdown::Document.new("# Héllo wörld!\n## Second one", auto_id_prefix: "x-").to_html
puts Kramdown::Document.new("{::options parse_block_html=\"true\" /}\n<div>\n*md*\n</div>").to_html
puts Kramdown::Document.new("* a\n* b", input: "kramdown").to_html
puts Kramdown::Document.new("# Toc\n* toc\n{:toc}\n\n## A\n## B").to_html
puts Kramdown::Document.new("$$x^2$$\n\nH~2~O").to_html
puts Kramdown::Document.new("line  \nbreak\n\n    indented code").to_html
puts Kramdown::Document.new("<script>x</script>\n\n<span>s</span>").to_html
puts Kramdown::Document.new("a\nb", input: "GFM").to_html rescue p $!.class
puts Kramdown::Document.new("# T\n\n\"Smart\" 'quotes'", smart_quotes: "lsquo,rsquo,ldquo,rdquo").to_html
puts Kramdown::Document.new("![img](a.png){: width=\"10\"}").to_html

el = Kramdown::Document.new("para *x*").root
p el.type, el.children.first.type, el.children.first.children.map(&:type)

class Kramdown::Converter::Shout < Kramdown::Converter::Base
  def convert(el)
    case el.type
    when :text then el.value.upcase
    else el.children.map { |c| convert(c) }.join
    end
  end
end
p Kramdown::Document.new("hello *there*").to_shout

begin
  Kramdown::Document.new("x", input: "nope")
rescue Kramdown::Error => e
  p e.class, e.message
end
begin
  Kramdown::Document.new("x").to_nothing_here
rescue NoMethodError => e
  p e.class
end
d = Kramdown::Document.new("[undefined][ref] and {:unknown}")
puts d.to_html
p d.warnings
