# Lines of a multi-line flow collection indented no further than its key: libyaml (Psych)
# accepts them (faker's locales/en/avatar.yml closes a `[` at the key's column)
require "yaml"
src = <<~Y
  en:
    faker:
      quotes: [
        "a",
        "b, c"
      ]
      dates: ['x', 'y']
    other: {
      k: 1,
    k2: [1,
  2]
    }
    after: done
Y
p YAML.load(src)
p YAML.load("a:\n  b: [\n    1,\n  2\n  ]\n")
p YAML.safe_load("- [\n 1,\n2]\n- x\n")
p Psych.parse("k: [\n1]\n").to_ruby
# still an error: a block mapping's key indented wrongly
begin
  YAML.load("a:\n    b: 1\n  c: 2\n")
rescue Psych::SyntaxError => e
  puts "SyntaxError"
end
