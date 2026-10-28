require "bundler/setup"
require "diff/lcs"
require "diff/lcs/hunk"
require "diff/lcs/string"

a = %w[a b c e h j l m n p]
b = %w[b c d e f j k l m r s t]
p Diff::LCS.lcs(a, b)
diffs = Diff::LCS.diff(a, b)
diffs.each { |hunk| p hunk.map(&:to_a) }
sdiff = Diff::LCS.sdiff(a, b)
puts sdiff.map { |c| "#{c.action} #{c.old_position}:#{c.old_element.inspect} #{c.new_position}:#{c.new_element.inspect}" }
patched = Diff::LCS.patch(a, diffs)
p patched == b
p Diff::LCS.unpatch!(b.dup, diffs) == a

p Diff::LCS.lcs("abcdefgh", "axcyegh")
p "kenny".lcs("penny")

events = []
Diff::LCS.traverse_sequences("abcd", "acbd",
  Class.new {
    define_method(:match) { |e| events << "=#{e.old_element}" }
    define_method(:discard_a) { |e| events << "-#{e.old_element}" }
    define_method(:discard_b) { |e| events << "+#{e.new_element}" }
  }.new)
p events.join(" ")

old_text = "one\ntwo\nthree\nfour\nfive\nsix\n".lines
new_text = "one\n2\nthree\nfour\nfive\nsix\nseven\n".lines
out = +""
file_length_difference = 0
Diff::LCS.diff(old_text, new_text).each do |piece|
  hunk = Diff::LCS::Hunk.new(old_text, new_text, piece, 3, file_length_difference)
  file_length_difference = hunk.file_length_difference
  out << hunk.diff(:unified) << "\n"
end
puts out
p Diff::LCS::Change.new("+", 0, "x").to_a
