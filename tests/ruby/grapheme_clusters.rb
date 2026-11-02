# String#grapheme_clusters as CRuby (Oniguruma's \X): Bengali "য়া" is one cluster (Discourse's UserNameSuggester)
s = "য়া"
p s.codepoints.map { |c| c.to_s(16) }, s.grapheme_clusters.size, s.each_grapheme_cluster.to_a.size
p "য়া".unicode_normalize.codepoints.map { |c| c.to_s(16) }
p "য়া".unicode_normalize.grapheme_clusters.size
p "बहुत-लंबा".grapheme_clusters, "🇫🇮👨‍👩‍👧e\u0301".grapheme_clusters
p "a\r\nb".grapheme_clusters, "".grapheme_clusters, "🇫🇮🇸🇪".grapheme_clusters.size, "👍🏽x".grapheme_clusters, "각".unicode_normalize(:nfd).grapheme_clusters.size
