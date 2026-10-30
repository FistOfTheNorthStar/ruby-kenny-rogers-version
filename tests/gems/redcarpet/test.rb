require "bundler/setup"
require "redcarpet"
require "redcarpet/render_strip"

md = Redcarpet::Markdown.new(Redcarpet::Render::HTML, autolink: true, tables: true, fenced_code_blocks: true,
  strikethrough: true, superscript: true, highlight: true, footnotes: true, no_intra_emphasis: true)

doc = <<~MD
  # Title

  Some *emphasis*, **strong**, `code` and ~~strike~~ and ==mark== and 2^nd.

  * one
  * two
    1. nested

  > quote

  | a | b |
  |---|:-:|
  | 1 | 2 |

  ```ruby
  puts "hi"
  ```

  Visit http://example.com or [link](http://x.io "T").
  ![img](a.png)

  Footnote[^1].

  [^1]: The note.

  ---
  foo_bar_baz
MD
puts md.render(doc)

puts Redcarpet::Markdown.new(Redcarpet::Render::HTML.new(filter_html: true, hard_wrap: true)).render("a\nb <script>x</script>")
puts Redcarpet::Markdown.new(Redcarpet::Render::HTML.new(escape_html: true)).render("<b>x</b>")
puts Redcarpet::Markdown.new(Redcarpet::Render::HTML.new(with_toc_data: true)).render("# One\n## Two two")
puts Redcarpet::Markdown.new(Redcarpet::Render::HTML_TOC).render("# One\n## Two\n# Three")
puts Redcarpet::Markdown.new(Redcarpet::Render::XHTML).render("line  \nbreak\n\n---")
puts Redcarpet::Markdown.new(Redcarpet::Render::StripDown).render("# Head\n\n*a* [b](c)")
puts Redcarpet::Markdown.new(Redcarpet::Render::HTML.new(safe_links_only: true, link_attributes: { rel: "nofollow" })).render("[a](javascript:x) [b](http://y)")

class Custom < Redcarpet::Render::HTML
  def header(text, level) = "<h#{level} class=\"c\">#{text.upcase}</h#{level}>\n"
  def block_code(code, lang) = "<pre data-lang=\"#{lang}\">#{code.strip}</pre>\n"
  def emphasis(text) = "<i>#{text}</i>"
  def preprocess(doc) = doc.gsub("KENNY", "Kenny")
  def postprocess(doc) = doc + "<!-- end -->\n"
end
puts Redcarpet::Markdown.new(Custom, fenced_code_blocks: true).render("# hey KENNY\n\n*x*\n\n```sh\nls\n```")

class Base < Redcarpet::Render::Base
  def paragraph(text) = "[P:#{text}]"
  def double_emphasis(t) = "{#{t}}"
end
puts Redcarpet::Markdown.new(Base).render("one **two**\n\nthree")

p Redcarpet::Render::SmartyPants.render("\"quotes\" -- dash ... 'single'")
p Redcarpet::VERSION
p md.render("").empty?
begin
  md.render(nil)
rescue TypeError => e
  p e.class
end
