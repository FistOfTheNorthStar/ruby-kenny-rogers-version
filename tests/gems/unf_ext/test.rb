require "bundler/setup"
require "unf_ext"

n = UNF::Normalizer.new
s = "é Å ﬁ ①"
%i[nfc nfd nfkc nfkd].each do |form|
  r = n.normalize(s, form)
  p [form, r, r.codepoints.map { |c| c.to_s(16) }, r.encoding]
end
p n.normalize("Ｒｕｂｙ", :nfkc)
p n.normalize("가", :nfd).codepoints, n.normalize("가", :nfc)
p UNF::Normalizer.instance_method(:normalize).arity
