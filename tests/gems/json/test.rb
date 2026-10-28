require "bundler/setup"
require "json"

data = { "name" => "kenny", "n" => 42, "f" => 1.5, "list" => [1, nil, true, false, "x"], "nested" => { "a" => { "b" => [] } } }
s = JSON.generate(data)
puts s
p JSON.parse(s) == data
puts JSON.pretty_generate(data)
p JSON.parse('{"a":1,"b":[1,2]}', symbolize_names: true)
p JSON.parse('[1.25, 1e3, -0, "é\\u00e9", "\\ud83d\\ude00"]')
p JSON.dump("a" => " <script>")
p JSON.generate({ "a" => "/</script>\u2028" }, script_safe: true)
p({ a: 1, b: :sym, c: nil }.to_json)
p [1, "two", 3.0].to_json
p "café".to_json
p JSON.parse("123")
p JSON.parse('"str"')
p JSON.load('{"x": [1,2,3]}')
p JSON.parse('{"big": 123456789012345678901234567890}')["big"]
p JSON.parse('{"d": 1.1}', decimal_class: Float)

class Point
  attr_reader :x, :y
  def initialize(x, y) = (@x, @y = x, y)
  def to_json(*a) = { "type" => "point", "x" => x, "y" => y }.to_json(*a)
  def self.from_json(h) = new(h["x"], h["y"])
end
j = JSON.generate([Point.new(1, 2), Point.new(3, 4)])
puts j
p JSON.parse(j).map { |h| Point.from_json(h) }.map { |pt| [pt.x, pt.y] }
puts JSON.pretty_generate([Point.new(5, 6)])

begin
  JSON.parse("{bad json}")
rescue JSON::ParserError => e
  puts "ParserError: #{e.class.ancestors.include?(StandardError)}"
end
begin
  JSON.generate(Float::NAN)
rescue JSON::GeneratorError => e
  puts "GeneratorError raised"
end
p JSON.generate(Float::NAN, allow_nan: true)
p JSON.parse("[NaN]", allow_nan: true)[0].nan?
class Bag
  attr_reader :h
  def initialize = @h = {}
  def []=(k, v)
    @h[k.to_sym] = v
  end
end
p JSON.parse('{"a":1,"b":{"c":2}}', object_class: Bag).h.keys
p JSON.parse('[1,2]', array_class: Array)
p JSON::VERSION.is_a?(String)
p JSON.generate({ 1 => 2, nil => 3, :s => 4 })
puts JSON.pretty_generate({}, indent: "    ")
puts JSON.pretty_generate({ "a" => [] , "b" => {} })
puts JSON.generate([1, 2], array_nl: "\n")
p JSON.parse(JSON.generate("tab\tnl\nquote\"bs\\ctl\u0001"))
p JSON.parse('// comment
[1, /* c */ 2]', allow_trailing_comma: true) rescue p $!.class
p JSON.parse('[1,2,]', allow_trailing_comma: true) rescue p $!.class
p JSON.parse('{"json_class":"Range","a":[1,2,false]}')
coder = JSON::Coder.new { |obj| obj.is_a?(Time) ? obj.utc.strftime("%Y-%m-%d") : obj.to_s }
p coder.dump({ t: Time.at(0), s: :sym, r: 1..2 })
p coder.load('{"a":[1]}')
require "tmpdir"
Dir.mktmpdir do |d|
  File.write("#{d}/x.json", '{"file": true}')
  p JSON.load_file("#{d}/x.json"), JSON.load_file("#{d}/x.json", symbolize_names: true)
end
p JSON.generate("a" => 1).frozen?, JSON.parse('{"a":"b"}', freeze: true).frozen?
p JSON::State.new(indent: "  ").indent, JSON.generate([1, [2]], JSON::State.new(indent: "  ", array_nl: "\n"))
