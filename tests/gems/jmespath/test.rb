require "bundler/setup"
require "jmespath"

data = {
  "people" => [
    { "name" => "a", "age" => 30, "tags" => ["x", "y"] },
    { "name" => "b", "age" => 25, "tags" => ["y"] },
    { "name" => "c", "age" => 41, "tags" => [] },
  ],
  "meta" => { "count" => 3, "nested" => { "deep" => [1, 2, 3, 4] } },
}
p JMESPath.search("people[*].name", data)
p JMESPath.search("people[?age > `28`].name", data)
p JMESPath.search("people[0]", data)
p JMESPath.search("meta.nested.deep[1:3]", data)
p JMESPath.search("meta.nested.deep[::-1]", data)
p JMESPath.search("sort_by(people, &age)[*].name", data)
p JMESPath.search("max_by(people, &age).name", data)
p JMESPath.search("length(people)", data)
p JMESPath.search("people[*].tags[]", data)
p JMESPath.search("{names: people[*].name, total: meta.count}", data)
p JMESPath.search("people[?contains(tags, 'y')] | [*].name", data)
p JMESPath.search("join(', ', people[*].name)", data)
p JMESPath.search("sum(meta.nested.deep)", data)
p JMESPath.search("missing.key", data)
p JMESPath.search("people[*].[name, age]", data)
p JMESPath.search("to_string(meta.count)", data)
p JMESPath.search("foo.bar", foo: { bar: "symbol keys" })
begin
  JMESPath.search("people[", data)
rescue JMESPath::Errors::SyntaxError => e
  p e.class
end
