require "bundler/setup"
require "rails-html-sanitizer"

html = %(<div class="x" onclick="evil()"><script>alert(1)</script><a href="javascript:alert(2)">bad</a> <a href="https://ok.example/p" title="t">ok</a><img src="x.png" onerror="e()"><b>bold</b><style>p{}</style></div>)
[Rails::HTML4, Rails::HTML5].each do |mod|
  p mod::SafeListSanitizer.new.sanitize(html)
  p mod::SafeListSanitizer.new.sanitize(html, tags: %w[a b], attributes: %w[href])
  p mod::FullSanitizer.new.sanitize(html)
  p mod::LinkSanitizer.new.sanitize(html)
  p mod::SafeListSanitizer.new.sanitize_css("color: red; background: url(javascript:x); width: 10px")
end
p Rails::HTML::Sanitizer.html5_support?, Rails::HTML::Sanitizer.best_supported_vendor
p Rails::HTML::SafeListSanitizer.allowed_tags.include?("table"), Rails::HTML5::SafeListSanitizer.allowed_attributes.include?("href")
p Rails::HTML5::SafeListSanitizer.new.sanitize("<p>café &amp; &lt;tag&gt; &copy;</p>"), Rails::HTML5::FullSanitizer.new.sanitize("a &lt; b <i>c</i>")
scrubber = Loofah::Scrubber.new { |node| node.name = "span" if node.name == "b" }
p Rails::HTML5::SafeListSanitizer.new.sanitize("<b>x</b><i>y</i>", scrubber: scrubber)
class OnlyEm < Rails::HTML::PermitScrubber
  def initialize; super; self.tags = %w[em]; end
end
p Rails::HTML5::SafeListSanitizer.new.sanitize("<p><em>keep</em><strong>drop</strong></p>", scrubber: OnlyEm.new)
p Rails::HTML5::SafeListSanitizer.new.sanitize(nil), Rails::HTML5::FullSanitizer.new.sanitize("")
