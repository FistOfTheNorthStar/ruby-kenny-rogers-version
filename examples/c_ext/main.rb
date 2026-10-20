require "fastmath"

p FastMath::VERSION, FastMath.fib(50), FastMath.sum(1, 2, 3.5), FastMath.sum(1, 2, scale: 10)
p FastMath.shout("hello from c"), FastMath.each_square(4) { |x| x + 1 }
p FastMath.call_back("ruby string")
s = FastMath::Stats.new
[2, 4, 9].each { |x| s.add(x) }
p s.mean
begin
  FastMath.fib(-1)
rescue ArgumentError => e
  p e.message
end
begin
  FastMath.fib("nope")
rescue TypeError => e
  p e.message
end
100_000.times { FastMath::Stats.new.add(1) }
GC.start
p :ok
