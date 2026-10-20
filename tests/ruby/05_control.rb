def classify(n)
  case n
  when 0 then "zero"
  when 1..9 then "small"
  when Integer then "big"
  when String, Symbol then "text"
  else "other"
  end
end
p [0, 5, 50, "x", :y, 1.5].map { |v| classify(v) }

i = 0
i += 1 while i < 10
p i
j = 10
until j <= 0
  j -= 3
end
p j
r = [1, 2, 3, 4, 5].each do |x|
  next if x.odd?
  break x * 100 if x == 4
end
p r
out = []
for k in 1..3 do out << k * k end
p out
p (1..20).find { |x| x % 7 == 0 }
res = loop do
  i -= 1
  break i if i < 5
end
p res

def risky(n)
  raise ArgumentError, "negative!" if n < 0
  raise "zero!" if n == 0
  Math.sqrt(n)
end
[4, 0, -1].each do |n|
  begin
    p risky(n)
  rescue ArgumentError => e
    puts "arg error: #{e.message}"
  rescue => e
    puts "error: #{e.message} (#{e.class})"
  else
    puts "no error"
  ensure
    puts "ensure #{n}"
  end
end

attempts = 0
begin
  attempts += 1
  raise "flaky" if attempts < 3
  puts "succeeded after #{attempts}"
rescue
  retry
end

class MyError < StandardError
  def initialize(msg = "custom default") = super
end
begin
  raise MyError
rescue MyError => e
  p e.message, e.class, e.is_a?(StandardError)
end
v = Integer("zz") rescue -1
p v

def early(x)
  [1, 2, 3].each { |y| return y * 10 if y == x }
  :none
end
p early(2), early(9)

def with_ensure
  return :from_body
ensure
  puts "ensure ran"
end
p with_ensure

def yielder
  return :no_block unless block_given?
  yield(1, 2) + yield(3, 4)
end
p yielder, yielder { |a, b| a * b }

def kw(a, b = 2, *rest, c:, d: 4, **opts, &blk)
  [a, b, rest, c, d, opts, blk&.call]
end
p kw(1, c: 3)
p kw(1, 9, 8, 7, c: 3, d: 5, e: 6) { :blk }
add = ->(x, y) { x + y }
p add.(1, 2), add[3, 4], add.call(5, 6)
sq = proc { |x| (x || 0) ** 2 }
p sq.call(4), sq.call, [1, 2].map(&sq)
counter = 0
incr = -> { counter += 1 }
3.times { incr.call }
p counter
adders = (1..3).map { |n| ->(x) { x + n } }
p adders.map { |f| f.(10) }
r = catch(:done) do
  10.times { |a| 10.times { |b| throw :done, [a, b] if a * b == 6 } }
  :never
end
p r
p [1, 2, 3].each_with_index.to_a
x = 5
msg = if x > 3 then "big" elsif x > 1 then "mid" else "small" end
p msg
p (x > 3 && "yes") || "no"
p defined?(x), defined?(@nope), defined?(puts), defined?(String)
p [1, 2, 3].frozen?, :a.frozen?
begin
  "frozen".upcase!
rescue FrozenError => e
  puts "FrozenError: #{e.message}"
end
