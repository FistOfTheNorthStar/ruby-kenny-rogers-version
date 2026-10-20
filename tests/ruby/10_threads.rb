# producer / consumer
q = Queue.new
producer = Thread.new do
  5.times { |i| q << i * 10 }
  q << :done
end
items = []
while (v = q.pop) != :done
  items << v
end
producer.join
p items

# values and join
threads = (1..4).map { |n| Thread.new(n) { |x| x * x } }
p threads.map(&:value)

# mutex protects a counter across preemption
count = 0
m = Mutex.new
ws = 4.times.map do
  Thread.new do
    1000.times { m.synchronize { count += 1 } }
  end
end
ws.each(&:join)
p count

# sleep overlaps instead of adding up
t0 = Time.now
3.times.map { |i| Thread.new { sleep 0.2; i } }.each(&:join)
p (Time.now - t0) < 0.5

# condition variable
mutex = Mutex.new
cv = ConditionVariable.new
ready = false
waiter = Thread.new do
  mutex.synchronize do
    cv.wait(mutex) until ready
    :woke
  end
end
sleep 0.05
mutex.synchronize { ready = true; cv.signal }
p waiter.value

# exceptions propagate through join
bad = Thread.new { raise ArgumentError, "boom in thread" }
bad.report_on_exception = false
begin
  bad.join
rescue ArgumentError => e
  p [:joined, e.message]
end

# Thread#raise and kill
sleeper = Thread.new do
  begin
    sleep 10
  rescue RuntimeError => e
    "rescued: #{e.message}"
  end
end
sleep 0.02
sleeper.raise("wake up")
p sleeper.value
looper = Thread.new { loop { Thread.pass } }
looper.kill
looper.join
p looper.alive?

# thread locals and Thread.current
t = Thread.new { Thread.current[:name] = "worker"; Thread.current[:name] }
p t.value, Thread.current[:name], Thread.main == Thread.current

# CPU-bound threads interleave (preemption)
order = []
a = Thread.new { 3.times { |i| order << "a#{i}"; x = 0; 300_000.times { x += 1 } } }
b = Thread.new { 3.times { |i| order << "b#{i}"; x = 0; 300_000.times { x += 1 } } }
[a, b].each(&:join)
p order.sort, order.size

# SizedQueue backpressure
sq = SizedQueue.new(2)
prod = Thread.new { 6.times { |i| sq.push(i) }; sq.close }
got = []
while (x = sq.pop)
  got << x
end
p got

# Fibers
fib = Fiber.new do
  a, b = 0, 1
  loop { Fiber.yield a; a, b = b, a + b }
end
p 10.times.map { fib.resume }
f2 = Fiber.new { |x| y = Fiber.yield(x * 2); y + 1 }
p f2.resume(5), f2.resume(100), f2.alive?

# external enumerators (infinite)
e = (1..Float::INFINITY).lazy.map { |x| x * 3 }.each_entry rescue nil
en = [1, 2, 3].each
p en.next, en.next, en.next
begin
  en.next
rescue StopIteration
  p :stop
end
nat = Enumerator.new { |y| n = 0; loop { y << n; n += 1 } }
p nat.take(5), nat.next, nat.next, nat.lazy.map { _1 * 2 }.first(3)
gen = (1..).each
p gen.next, gen.next, gen.peek, gen.next

# deadlock detection
begin
  Queue.new.pop
rescue Exception => e
  p e.class, e.message
end
