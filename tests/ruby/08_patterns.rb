def describe(x)
  case x
  in Integer => n if n > 100 then "big int #{n}"
  in Integer | Float => n then "number #{n}"
  in String => s unless s.empty? then "string #{s}"
  in "" then "empty string"
  in [] then "empty array"
  in [1, *rest] then "starts with 1, rest #{rest}"
  in [Integer => a, Integer => b] then "pair #{a},#{b}"
  in [*, :needle, *post] then "found needle, then #{post}"
  in {name: String => name, age: Integer => age} if age >= 18 then "adult #{name}"
  in {name:, **rest} then "person #{name} with #{rest.keys}"
  in {type: :circle, r:} then "circle r=#{r}"
  in nil then "nil!"
  else "something else: #{x.inspect}"
  end
end
[500, 7, 2.5, "hi", "", [], [1, 2, 3], [4, 5], [:a, :needle, :b, :c], {name: "Ann", age: 30},
 {name: "Kid", age: 9, city: "Oslo"}, {type: :circle, r: 2}, nil, :sym].each { |v| puts describe(v) }

Point = Struct.new(:x, :y)
case Point.new(1, 2)
in [x, y] then puts "struct as array #{x},#{y}"
end
case Point.new(3, 4)
in {x:, y:} then puts "struct as hash #{x},#{y}"
end
expected = 5
case 5
in ^expected then puts "pinned match"
end
config = {db: {user: "admin", port: 5432}}
config => {db: {user:, port:}}
puts "#{user}@#{port}"
p((1 in Integer))
p(("x" in Integer))
begin
  case 42
  in String then nil
  end
rescue NoMatchingPatternError => e
  puts "NoMatchingPatternError: #{e.message}"
end
result = case [1, [2, [3, 4]]]
         in [a, [b, [c, d]]] then a + b + c + d
         end
puts result
