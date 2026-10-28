require "bundler/setup"
require "unicode/display_width"

samples = ["hello", "日本語", "한국어", "🤦‍♀️", "é", "­", "Ｆｕｌｌ", "\t", "a​b", "🇫🇮", "👍🏽", "ä", "·", "─", "\e[31mred\e[0m"]
samples.each do |s|
  p [s, Unicode::DisplayWidth.of(s), Unicode::DisplayWidth.of(s, 2), Unicode::DisplayWidth.of(s, emoji: false)]
end
dw = Unicode::DisplayWidth.new(ambiguous: 2, emoji: true)
p dw.of("·─ä"), dw.of("😀😀")
p Unicode::DisplayWidth.of("Ｗｉｄｅ", overwrite: { "Ｗ".ord => 1 })
p Unicode::DisplayWidth.of("x" * 1000)
p Unicode::DisplayWidth.of("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", emoji: :all)
p Unicode::DisplayWidth.of("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", emoji: :none)
p Unicode::DisplayWidth.of("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", emoji: :rqi)
p Unicode::DisplayWidth::UNICODE_VERSION
words = %w[apple 苹果 りんご 🍎]
width = words.map { |w| Unicode::DisplayWidth.of(w) }.max
words.each { |w| puts w + " " * (width - Unicode::DisplayWidth.of(w)) + "|" }
