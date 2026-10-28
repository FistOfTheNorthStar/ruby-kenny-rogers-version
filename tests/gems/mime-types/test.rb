require "bundler/setup"
require "mime/types"

t = MIME::Types["text/html"].first
p t.content_type, t.media_type, t.sub_type, t.extensions.sort, t.preferred_extension, t.ascii?, t.binary?, t.encoding
p MIME::Types.type_for("photo.jpeg").map(&:content_type)
p MIME::Types.type_for(%w[a.css b.js]).map(&:to_s).sort
p MIME::Types.of("x.pdf").first.to_s
j = MIME::Types["application/json"].first
p j.registered?, j.obsolete?, j.extensions, j.friendly
p MIME::Types[/^image\/png/].map(&:to_s)
p MIME::Type.new("content-type" => "application/x-kenny").simplified, MIME::Type.simplified("Application/X-Foo"), MIME::Type.i18n_key("text/plain")
custom = MIME::Type.new("content-type" => "application/x-kenny") { |ty| ty.extensions = %w[kny]; ty.add_extensions("ken") }
MIME::Types.add(custom)
p MIME::Types.type_for("prog.kny").map(&:to_s), custom.extensions
tp = MIME::Types["text/plain"].first
p tp == "text/plain", (tp <=> MIME::Types["text/html"].first), tp.eql?(MIME::Type.new("content-type" => "text/plain"))
p MIME::Types["text/plain"].first.like?("text/plain"), MIME::Types.type_for("noext").empty?
p MIME::Types["audio/mpeg"].first.extensions.include?("mp3")
p MIME::Types::VERSION.is_a?(String)
