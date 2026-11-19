# frozen_string_literal: true

# Frozen literals with the same contents are one object, in every file (CRuby's fstring
# table): Mastodon's specs check `expect(helper.first_of_value(["a"])).to be "a"`.
require_relative "frozen_literals/other"
def first(v) = v.first
p first(["a"]).equal?("a"), "es".equal?(other_es), "a".frozen?
# -@ and dedup give the literal; a String with ivars or another encoding is not it
p (-"a".dup).equal?("a"), "a".dedup.equal?("a"), (-String.new("zz_unseen")).frozen?
s = String.new("q")
s.instance_variable_set(:@x, 1)
p (-s).equal?(s), (-s).frozen?
p "a".b.equal?("a"), (-"a".b).encoding
# without the comment, literals are chilled: a new String each time
p eval('a = "x"; b = "x"; [a.equal?(b), a.frozen?]')
