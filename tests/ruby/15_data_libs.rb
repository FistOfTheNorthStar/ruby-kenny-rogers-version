# Native replacements for C-extension stdlib: json, strscan, digest, zlib.
require "json"
require "strscan"
require "digest"
require "digest/md5"
require "digest/sha1"
require "digest/sha2"
require "zlib"
require "stringio"

# ---- JSON ----
doc = JSON.parse('{"name": "Kenny", "tags": ["a", "b"], "n": 1.5, "big": 12345678901234567890,
                  "ok": true, "none": null, "u": "é😀", "nested": {"x": [1, {"y": -0}]}}')
p doc["name"], doc["tags"], doc["n"], doc["big"], doc["ok"], doc["none"], doc["u"], doc["nested"]["x"]
puts JSON.generate(doc)
puts JSON.generate({ "q" => "quote\" back\\ slash/ nl\n ctl\u001f tab\t", :sym => :val, 3 => nil })
puts JSON.pretty_generate({ "a" => [1, 2, { "b" => nil }], "c" => {}, "d" => [] })
p JSON.parse('{"a": {"b": 1}}', symbolize_names: true)[:a][:b]
puts [1, "two", nil, true, 2.5].to_json, "hé".to_json, :sym.to_json, 42.to_json, nil.to_json
p JSON.generate(["é", " "], ascii_only: true), JSON.generate(["</x>"], script_safe: true)
p JSON.parse(JSON.generate(doc)) == doc

class Point
  def initialize(x, y)
    @x = x
    @y = y
  end

  def to_json(*args)
    { "x" => @x, "y" => @y }.to_json(*args)
  end
end
class Plain
  def to_s
    "plain!"
  end
end
class EscapedString < String
  def to_json(*)
    super.gsub("<", "\\u003c")
  end
end
puts JSON.generate([Point.new(1, 2), Plain.new, EscapedString.new("<b>")])
puts JSON.pretty_generate("pts" => [Point.new(3, [4])])
["[1,]", "{\"a\"}", "[01]", "tru", "{\"a\":1,}", "[1 2]", "", "nul"].each do |bad|
  JSON.parse(bad)
  puts "no error: #{bad}"
rescue JSON::ParserError
  puts "ParserError: #{bad}"
end
begin
  JSON.generate([0.0 / 0])
rescue JSON::GeneratorError => e
  puts e.message
end
p JSON.parse("[NaN]", allow_nan: true)[0].nan?
p JSON.dump({ "a" => [1, nil] }), JSON.load("[1,2]"), JSON.load(nil), JSON("[3]"), JSON([3])
p JSON::State.new(indent: "\t").indent, JSON::NestingError.superclass, JSON::ParserError.superclass
p JSON.parse('{"a":[1]}', freeze: true).frozen?

# ---- StringScanner ----
s = StringScanner.new("3.14 + foo_bar * (x1 - 2)")
toks = []
until s.eos?
  if s.skip(/\s+/) then next
  elsif (t = s.scan(/\d+(\.\d+)?/)) then toks << [:num, t, s[1]]
  elsif (t = s.scan(/(?<id>[a-z_]\w*)/)) then toks << [:id, t, s[:id]]
  elsif (t = s.scan(%r{[-+*/()]})) then toks << [:op, t]
  else toks << [:other, s.getch]
  end
end
p toks
s = StringScanner.new(+"test string here")
p s.scan(/\w+/), s.pos, s.matched, s.pre_match, s.post_match, s.matched_size
p s.check(/\s+/), s.pos, s.scan_until(/str/), s.pre_match, s.pos
p s.peek(3), s.rest, s.rest_size, s.skip_until(/her/), s.bol?, s.charpos
s.unscan
p s.pos, s.inspect
s.terminate
p s.eos?, s.inspect
s.pos = 5
p s.inspect, s.beginning_of_line?
s << " more"
p s.rest
s2 = StringScanner.new("key: value")
p s2.scan(/(\w+): (\w+)/), s2.captures, s2.values_at(0, 2), s2.size, s2[-1]
s3 = StringScanner.new("héllo wörld")
p s3.scan(/h./), s3.pos, s3.charpos, s3.getch, s3.get_byte
p s3.match?(/llo/), s3.exist?(/w/), s3.check_until(/w/)
x = StringScanner.new("abc")
x.pos = 1
p x.scan(/^b/), x.scan(/\Ac/), StringScanner.new("abc").scan("ab")
begin
  StringScanner.new("a").unscan
rescue StringScanner::Error => e
  p e.class
end

# ---- Digest ----
p Digest::MD5.hexdigest("hello"), Digest::SHA1.hexdigest("hello")
p Digest::SHA256.hexdigest("hello"), Digest::SHA384.hexdigest("hello")[0, 32]
p Digest::SHA512.hexdigest("")[0, 32], Digest::MD5.base64digest("hello")
d = Digest::SHA256.new
d.update("hel")
d << "lo"
p d.hexdigest == Digest::SHA256.hexdigest("hello"), d == Digest::SHA256.hexdigest("hello")
e = d.dup
e << "!"
p d.hexdigest[0, 16], e.hexdigest[0, 16]
p d.hexdigest![0, 16], d.hexdigest[0, 16]
p d.block_length, d.digest_length, Digest::SHA512.new.block_length, Digest::SHA256.digest("abc").bytesize
p Digest.hexencode("\x01\xff"), Digest::SHA2.new(512).update("hi").hexdigest == Digest::SHA512.hexdigest("hi")
p Digest(:MD5), Digest::MD5.new.inspect, Digest::MD5.superclass, Digest::Base.superclass

# ---- Zlib ----
p Zlib.crc32("hello"), Zlib.adler32("hello"), Zlib.crc32, Zlib.adler32
p Zlib.crc32("lo", Zlib.crc32("hel")) == Zlib.crc32("hello")
data = "hello hello hello hello " * 20
z = Zlib.deflate(data)
p z.bytesize < data.bytesize, Zlib.inflate(z) == data, Zlib.inflate(Zlib.deflate(""))
p Zlib::Inflate.inflate(Zlib::Deflate.deflate(data, Zlib::BEST_COMPRESSION)) == data
g = Zlib.gzip(data)
p g.bytes[0, 4], Zlib.gunzip(g) == data
io = StringIO.new
gz = Zlib::GzipWriter.new(io)
gz.write("abc")
gz << "def"
gz.puts "ghi"
gz.close
p Zlib.gunzip(io.string), Zlib::GzipReader.new(StringIO.new(io.string)).read
r = Zlib::GzipReader.new(StringIO.new(io.string))
p r.gets, r.read(2), r.read, r.eof?
p Zlib::GzipReader.wrap(StringIO.new(Zlib.gzip("wrapped")), &:read)
dd = Zlib::Deflate.new
c1 = dd.deflate("streamed ", Zlib::SYNC_FLUSH)
c2 = dd.deflate("data", Zlib::FINISH)
inf = Zlib::Inflate.new
p inf.inflate(c1) + inf.inflate(c2), inf.finished?
begin
  Zlib.inflate("garbage")
rescue Zlib::DataError => e
  p e.class, e.message
end
begin
  Zlib.gunzip("not gzip at all")
rescue Zlib::GzipFile::Error => e
  p e.class, e.message
end
big = (0...5000).map { |n| (n * 7919 % 256).chr }.join
p Zlib.inflate(Zlib.deflate(big)) == big, Zlib.gunzip(Zlib.gzip(big, level: 9)) == big
p Zlib::Inflate.new(Zlib::MAX_WBITS + 32).inflate(Zlib.gzip("auto"))
p Zlib::BEST_COMPRESSION, Zlib::DEFAULT_COMPRESSION
# Zlib streams with a block: the output is yielded and nil returned (Net::HTTP's inflater)
zi = Zlib::Inflate.new(32 + Zlib::MAX_WBITS)
got = +""
p zi.inflate(Zlib.gzip("abc" * 100)) { |c| got << c }, got.bytesize
zd = Zlib::Deflate.new
parts = []
zd.deflate("x" * 1000) { |c| parts << c }
p zd.finish { |c| parts << c }, Zlib::Inflate.inflate(parts.join).bytesize
