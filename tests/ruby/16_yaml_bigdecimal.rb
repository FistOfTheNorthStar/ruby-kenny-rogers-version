# YAML (Psych API) and BigDecimal, both native in Kenny.
require "yaml"
require "bigdecimal"
require "bigdecimal/util"

def unsafe(src)
  YAML.respond_to?(:unsafe_load) ? YAML.unsafe_load(src) : YAML.load(src)
end

def show(h, indent = "")
  h.each do |k, v|
    if v.is_a?(Hash)
      puts "#{indent}#{k.inspect}:"
      show(v, indent + "  ")
    else
      puts "#{indent}#{k.inspect} = #{v.inspect}"
    end
  end
end

puts "== scalars"
p YAML.safe_load(<<~Y)
  - plain string
  - 'single quoted'
  - "double \\"quoted\\"\\tescape"
  - 42
  - -17
  - 0x1F
  - 0o17
  - 017
  - 1_000_000
  - 3.25
  - 6.02e+23
  - .inf
  - -.Inf
  - true
  - false
  - yes
  - No
  - on
  - OFF
  - ~
  - null
  -
  - '123'
  - "true"
  - 1e3
  - 12:30
  - 2024-10-19 x
Y
p YAML.safe_load("- .nan").first.nan?

puts "== symbols"
p YAML.safe_load("- :name\n- ':quoted'\n- :\"with space\"\n", permitted_classes: [Symbol])
begin
  YAML.safe_load("- :name")
rescue Psych::DisallowedClass => e
  puts "#{e.class}: #{e.message}"
end
sym = YAML.safe_load("a: 1\nb: {c: 2}\n", symbolize_names: true)
p sym.to_a.first, sym[:b].to_a

puts "== database.yml"
db = <<~Y
  # Rails-style config
  default: &default
    adapter: postgresql
    encoding: unicode
    pool: 5
    timeout: 5000
    host: localhost

  development:
    <<: *default
    database: app_development

  test:
    <<: *default
    database: app_test
    pool: 2

  production:
    <<: *default
    database: app_production
    host: db.example.com
    variables:
      statement_timeout: 5s
Y
config = unsafe(db)
show(config)
p YAML.safe_load(db, aliases: true)["test"]["pool"]
begin
  YAML.safe_load(db)
rescue Psych::BadAlias => e
  puts "BadAlias raised: #{e.is_a?(Psych::Exception)}"
end
multi = unsafe("base: &b {x: 1, y: 2}\nextra: &e {z: 3}\nall:\n  <<: [*b, *e]\n  y: 9\n")
p multi["all"].to_a
shared = unsafe("- &list [1, 2]\n- *list\n")
p shared, shared[0].equal?(shared[1])

puts "== collections and blocks"
doc = YAML.safe_load(<<~Y)
  flow_seq: [a, b, [c, d]]
  flow_map: {k: v, n: 1}
  nested:
    - name: first
      tags: [x, y]
    - name: second
      tags: []
  literal: |
    line one
      indented
    line three
  folded: >
    folded
    text

    new paragraph
  strip: |-
    no newline
  keep: |+
    kept

  empty_map: {}
  "quoted key": value # trailing comment
  ? complex key
  : complex value
Y
show(doc)
p YAML.load_stream("--- 1\n--- [2]\n---\nthree: 3\n").map { |d| d.is_a?(Hash) ? d.to_a : d }
p YAML.safe_load("")
p YAML.safe_load("# only a comment\n")

puts "== errors"
begin
  YAML.safe_load("key: [unclosed\nother: 1")
rescue Psych::SyntaxError => e
  puts "#{e.class} (#{e.is_a?(Psych::Exception)}) file=#{e.file.inspect}"
end
begin
  YAML.safe_load("a: !ruby/object:Object {}\n")
rescue Psych::DisallowedClass => e
  puts e.message
end

puts "== dump"
data = {
  "name" => "Kenny",
  "version" => 1,
  "ratio" => 0.5,
  "big" => 2**70,
  "enabled" => true,
  "nothing" => nil,
  :sym_key => :sym_value,
  "list" => [1, "two", 3.0, [4, 5], {"six" => 6}],
  "empty_list" => [],
  "empty_hash" => {},
  "nested" => {"a" => {"b" => {"c" => "deep"}}},
  "strings" => ["true", "123", "1.5", "", " padded ", "colon: here", "#hash", "- dash",
                "it's", "say \"hi\"", "tab\there", "multi\nline\n", "trailing\n", "null", "~", "café", "<<"],
}
puts YAML.dump(data)
puts [1, [2, [3]]].to_yaml
puts "plain".to_yaml
puts nil.to_yaml
puts YAML.dump_stream({"a" => 1}, [2])
puts YAML.dump({"long" => "word " * 25})
round = YAML.safe_load(YAML.dump(data), permitted_classes: [Symbol])
p round == data

class Point
  attr_reader :x, :y
  def initialize(x, y)
    @x = x
    @y = y
  end
end
pt_yaml = Point.new(3, [4, "five"]).to_yaml
puts pt_yaml
pt = unsafe(pt_yaml)
p [pt.class, pt.x, pt.y]
p YAML.safe_load(pt_yaml, permitted_classes: [Point]).y
twice = [1, 2]
puts YAML.dump("a" => twice, "b" => twice)

puts "== BigDecimal"
a = BigDecimal("123.456")
b = BigDecimal("0.0001")
p a, b, BigDecimal("-0.5"), BigDecimal("1e20"), BigDecimal("  42  "), BigDecimal(7), BigDecimal(0.1, 5), BigDecimal("1_000.25")
p [a + b, a - b, a * b, a / 4, -a, a.abs]
p [BigDecimal("0.1") + BigDecimal("0.2") == BigDecimal("0.3"), 0.1 + 0.2 == 0.3]
p [a.to_s, a.to_s("F"), BigDecimal("1234567.891").to_s("3F"), BigDecimal("100").to_s("F"), b.to_s, b.to_s("F")]
p [a.to_i, a.to_f, a.to_int, a.truncate, a.floor, a.ceil, a.round]
p [a.round(2), a.round(-1), a.round(1, :half_even), BigDecimal("2.5").round(0, :half_even), BigDecimal("2.5").round(0, BigDecimal::ROUND_HALF_UP), BigDecimal("-2.5").round]
p [a.floor(1), a.ceil(1), a.truncate(2), BigDecimal("1.25").round(1, :banker), BigDecimal("1.35").round(1, :half_down)]
p [a.precision, a.scale, a.exponent, a.sign, b.sign, BigDecimal("0").sign, a.split]
p [a.zero?, BigDecimal("0").zero?, a.nonzero?.nil?, BigDecimal("-1").negative?, a.positive?, a.finite?]
inf = BigDecimal("Infinity")
nan = BigDecimal("NaN")
p [inf, -inf, nan, inf.infinite?, (-inf).infinite?, a.infinite?, nan.nan?, inf.finite?]
p [BigDecimal("1") / 0, BigDecimal("0") / 0, inf - inf, BigDecimal("1") / inf]
p [(BigDecimal("1") / 3).round(20), BigDecimal("2").div(3, 10), BigDecimal("10").div(3), BigDecimal("7.5") % 2, BigDecimal("-7.5").divmod(2).map(&:to_f)]
p [BigDecimal("1.5") ** 2, BigDecimal("2") ** -2, BigDecimal("1.1").mult(BigDecimal("1.1"), 2), BigDecimal("1.15").add(BigDecimal("1.1"), 2)]
p [a <=> b, a > b, b < 1, 1 < a, a == BigDecimal("123.4560"), a.eql?(BigDecimal("123.456")), BigDecimal("1.0") == 1, 1 == BigDecimal("1.0"), BigDecimal("0.5") == 0.5]
p [1 + a, a + 1, 2 * b, 0.5 + BigDecimal("1"), a.coerce(2), [BigDecimal("3"), 1, 2.5].sort, [a, b].max]
p [3.to_d, 0.25.to_d, (0.1 + 0.2).to_d, "1.5".to_d, "12abc".to_d, "abc".to_d, nil.to_d, a.to_d.equal?(a), BigDecimal("1.50").to_digits]
h = {BigDecimal("1.5") => "x"}
p h[BigDecimal("1.50")], [BigDecimal("1"), BigDecimal("1.0")].uniq.size
p format("%.2f", BigDecimal("1.2345")), "#{BigDecimal("3.14")}"
begin
  BigDecimal("12abc")
rescue ArgumentError => e
  puts e.message
end
p BigDecimal("junk", exception: false)
begin
  BigDecimal(nil)
rescue TypeError => e
  puts e.message
end
begin
  inf.to_i
rescue FloatDomainError => e
  puts e.message
end
p [BigDecimal::ROUND_HALF_UP, BigDecimal::ROUND_HALF_EVEN, BigDecimal::SIGN_NEGATIVE_FINITE, BigDecimal::BASE]
p [a.is_a?(Numeric), a.frozen?, BigDecimal.ancestors.include?(Comparable)]
price = BigDecimal("19.99")
total = price * 3
p [total, total.to_s("F"), (total * BigDecimal("1.08")).round(2).to_s("F")]
