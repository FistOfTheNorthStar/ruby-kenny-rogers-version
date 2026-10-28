require "bundler/setup"
require "loofah"

dirty = %(<div onclick="evil()"><script>alert(1)</script><p style="color:red;position:fixed" class="x">Hi <a href="javascript:bad()">click</a> <a href="http://ok.example/">ok</a></p><img src="x.png" onerror="y()"><iframe src="http://evil"></iframe><unknown>u</unknown></div>)
p Loofah.fragment(dirty).scrub!(:strip).to_s
p Loofah.fragment(dirty).scrub!(:prune).to_s
p Loofah.fragment(dirty).scrub!(:escape).to_s
p Loofah.fragment(dirty).scrub!(:whitewash).to_s
p Loofah.fragment(%(<a href="http://x.example">x</a>)).scrub!(:nofollow).to_s
p Loofah.fragment(%(<a href="http://x.example">x</a>)).scrub!(:noopener).to_s
p Loofah.fragment(%(<a href="http://x.example">x</a>)).scrub!(:targetblank).to_s
p Loofah.fragment("<p>Hello<br>World</p><div>Two</div>").to_text
p Loofah.fragment("<p>Hello<br>World</p><div>Two</div>").text(encode_special_chars: false)
doc = Loofah.document("<html><body><h1>T</h1><script>x</script><p>keep</p></body></html>")
p doc.scrub!(:prune).to_html.gsub(/\s+/, " ")
spans = Loofah::Scrubber.new do |node|
  node.name = "span" if node.name == "b"
end
p Loofah.fragment("<b>bold</b> and <i>it</i>").scrub!(spans).to_s
stopper = Loofah::Scrubber.new(direction: :top_down) { |n| n.remove if n.name == "div"; Loofah::Scrubber::STOP if n.name == "div" }
p Loofah.fragment("<div><p>gone</p></div><p>stay</p>").scrub!(stopper).to_s
p Loofah::HTML5::Scrub.scrub_css("color: red; background: url(javascript:x); font-weight: bold")
p Loofah.xml_fragment("<a><b>x</b></a>").to_s, Loofah.xml_document("<r><s/></r>").root.name
p Loofah.html5_fragment("<p onmouseover='x'>p5</p>").scrub!(:strip).to_s
p Loofah.fragment("<p>a</p>").is_a?(Loofah::HTML4::DocumentFragment)
