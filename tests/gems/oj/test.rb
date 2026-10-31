require "bundler/setup"
require "oj"
require "bigdecimal"

data = { "a" => [1, 2.5, nil, true], "b" => { "c" => "dé\n" }, "n" => 12345678901234567890 }
p Oj.dump(data, mode: :strict)
p Oj.dump(data, mode: :compat, indent: 2)
p Oj.load('{"x":[1,{"y":null}],"z":1.5e3}')
p Oj.load('{"x":1}', symbol_keys: true)
p Oj.load("[1.10, 2.0]", bigdecimal_load: :bigdecimal)
p Oj.dump(:sym, mode: :compat), Oj.dump(:sym, mode: :null), Oj.dump(nil)
Point = Struct.new(:x, :y)
p Oj.dump(Point.new(1, 2), mode: :compat)
class Pt
  def initialize(x) = @x = x
  def to_json(*) = %({"pt":#{@x}})
  def to_h = { "pt" => @x }
end
p Oj.dump(Pt.new(3), mode: :compat, use_to_json: true)
p Oj.dump(Pt.new(4), mode: :custom, use_to_hash: true) rescue p $!.class
p Oj.dump(Time.at(0).utc, mode: :compat)
begin
  Oj.load("{bad json")
rescue Oj::ParseError => e
  p e.class
end
w = Oj::StringWriter.new(mode: :compat)
w.push_object
w.push_value(1, "one")
w.push_array("list")
w.push_value("x")
w.pop
w.pop
p w.to_s
Oj.mimic_JSON
p JSON.parse('{"k":[1,2]}'), JSON.generate({ "q" => 1 }), [1, "a"].to_json
p Oj::Doc.open('{"a":[1,{"b":2}]}') { |d| [d.fetch("/a/2/b"), d.type("/a")] }
p Oj.safe_load('{"x":1}')
