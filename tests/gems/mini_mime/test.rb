require "bundler/setup"
require "mini_mime"

%w[a.html b.JPG c.json d.tar.gz e.unknownext f.css report.pdf x.svg].each do |f|
  info = MiniMime.lookup_by_filename(f)
  p [f, info&.content_type, info&.encoding, info&.binary?]
end
i = MiniMime.lookup_by_extension("png")
p i.content_type, i.extension
c = MiniMime.lookup_by_content_type("application/json")
p c.extension, c.content_type
p MiniMime.lookup_by_content_type("text/plain").extension
p MiniMime.lookup_by_content_type("nope/nope")
