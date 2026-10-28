require "bundler/setup"
require "unf"

s = "é Å ﬁ ①"
%i[nfc nfd nfkc nfkd].each do |form|
  r = UNF::Normalizer.normalize(s, form)
  p [form, r, r.length]
end
p "ｶﾞｷﾞ".to_nfkc, "é".to_nfd.length, "é".to_nfc.length
p UNF::Normalizer.instance.normalize("Ⅻ", :nfkd)
begin
  UNF::Normalizer.normalize("x", :bogus)
rescue ArgumentError => e
  p e.class
end
