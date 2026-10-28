require "bundler/setup"
require "multi_json"

p MultiJson.adapter
s = MultiJson.dump({ "a" => 1, "b" => [true, nil, 2.5], "c" => { "d" => "e" } })
puts s
p MultiJson.load(s)
p MultiJson.load(s, symbolize_keys: true)
puts MultiJson.dump({ "x" => [1, 2] }, pretty: true)
begin
  MultiJson.load("{oops")
rescue MultiJson::ParseError => e
  puts "ParseError: #{e.data.inspect}"
end
MultiJson.use(:json_gem)
p MultiJson.adapter
p MultiJson.load('[1,"two"]')
p MultiJson.load('{"k":[1,2,{"z":null}]}')
p MultiJson.dump([1, "é", nil])
MultiJson.with_adapter(:json_gem) { p MultiJson.current_adapter }
p MultiJson.load("") rescue p $!.class
