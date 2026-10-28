require "bundler/setup"
require "minitest"
require "active_support"
require "active_support/testing/assertions"
require "active_support/core_ext/object/blank"
require "rails-dom-testing"

class DomCheck
  include Minitest::Assertions
  include ActiveSupport::Testing::Assertions
  include Rails::Dom::Testing::Assertions
  attr_accessor :assertions
  def initialize(html) = (@html = html; self.assertions = 0)
  def document_root_element = Nokogiri::HTML5::Document.parse(@html).root
end

html = <<~HTML
  <html><body><div id="main"><h1 class="title">Posts</h1>
  <ul><li class="post">One</li><li class="post">Two</li><li class="post draft">Three</li></ul>
  <form action="/x"><input name="q" value="hi"><select name="s"><option selected>b</option></select></form></div></body></html>
HTML
t = DomCheck.new(html)
t.assert_select "h1.title", "Posts"
t.assert_select "li.post", 3
t.assert_select "li.post", count: 3
t.assert_select "li.draft", text: "Three"
t.assert_select "li", minimum: 2, maximum: 5
t.assert_select "ul" do t.assert_select "li", 3 end
t.assert_select "input[name=?][value=?]", "q", "hi"
t.assert_select "table", false
t.assert_select "li", /^T/ rescue p [:regex, $!.class]
t.assert_select "#main h1", { text: /Pos/ }
els = t.css_select("li.post"); p els.size, els.map(&:text)
p t.assertions
begin
  t.assert_select "li.post", 4
rescue Minitest::Assertion => e
  puts e.message
end
begin
  t.assert_select "h2"
rescue Minitest::Assertion => e
  puts e.message
end
t.assert_dom_equal "<a href='x' class='c'>z</a>", "<a class='c' href='x'>z</a>"
t.assert_dom_not_equal "<a>z</a>", "<a>y</a>"
begin
  t.assert_dom_equal "<b>1</b>", "<b>2</b>"
rescue Minitest::Assertion => e
  puts e.message.lines.first
end
p t.assertions
