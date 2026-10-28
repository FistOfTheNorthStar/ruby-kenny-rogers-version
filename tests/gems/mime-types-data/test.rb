require "bundler/setup"
require "mime/types/data"
require "json"

p MIME::Types::Data::VERSION.is_a?(String)
dir = MIME::Types::Data::PATH
p File.basename(dir), File.directory?(dir)
files = Dir.children(dir).sort
p files
cols = File.read(File.join(dir, "mime.content_type.column"), encoding: "UTF-8").lines(chomp: true)
p cols.size > 1000, cols.first(2), cols.grep(/\Atext\/html /)
ext = File.readlines(File.join(dir, "ext_mime.db"), chomp: true).map(&:split)
p ext.find { |e| e[0] == "png" }, ext.size > 500
db = File.foreach(File.join(dir, "content_type_mime.db")).map(&:split).to_h { |e, t, enc| [e, [t, enc]] }
p db["atom"], db["json"]
data = JSON.parse(File.read(File.join(dir, "mime-types.json")))
p data.size > 1000, data.find { |d| d["content-type"] == "application/json" }&.slice("content-type", "extensions", "encoding")
