require "bundler/setup"
require "crass"

css = <<~CSS
  /* comment */
  @import url("foo.css");
  body { color: #fff; margin: 0 auto !important; }
  a:hover, .nav > li { background: url(img.png) no-repeat; width: calc(100% - 10px); }
  @media screen and (max-width: 600px) { .x { display: none } }
CSS
tree = Crass.parse(css, preserve_comments: true)
p tree.map { |n| n[:node] }
rule = tree.find { |n| n[:node] == :style_rule }
p rule[:selector][:value], rule[:children].select { |c| c[:node] == :property }.map { |c| [c[:name], c[:value], c[:important]] }
media = tree.find { |n| n[:node] == :at_rule && n[:name] == "media" }
p media[:prelude].map { |t| t[:node] }.uniq.sort
p Crass::Parser.stringify(tree)
props = Crass.parse_properties("color: red; font-size: 12px; background:url( 'x.png' )")
p props.select { |n| n[:node] == :property }.map { |n| [n[:name], n[:value]] }
toks = Crass::Tokenizer.tokenize("a { b: 1.5em -2 'str' #id U+0025-00FF }")
p toks.map { |t| t[:node] }
p toks.select { |t| t[:node] == :dimension }.map { |t| [t[:value], t[:unit], t[:type]] }
p toks.find { |t| t[:node] == :hash }.slice(:value, :type)
p Crass.parse("a{}").first[:selector][:tokens].map { |t| t[:node] }
p Crass.parse("}{ bad").map { |n| n[:node] }
