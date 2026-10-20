# More threads than cores, switching often (preemption, Thread.pass, queues, sleeps), so
# idle cores keep taking waiting threads from busy ones: a thread resumes on a different OS
# thread than it stopped on. Per-thread state must move with it: exceptions in flight
# through `ensure`, `$!`, `$~`, Fibers, thread locals, method definition targets.

T = 24

# ensure clauses that switch threads while an exception is in flight
res = T.times.map do |t|
  Thread.new do
    caught = 0
    200.times do |i|
      begin
        begin
          raise ArgumentError, "e#{t}-#{i}"
        ensure
          Thread.pass
          x = [i] * 3
          sleep 0 if i % 50 == 0
        end
      rescue ArgumentError => e
        caught += 1 if e.message == "e#{t}-#{i}" && $!.equal?(e)
      end
    end
    caught
  end
end.map(&:value)
p res.uniq

# break / non-local return through ensure while switching
def find_first(list)
  list.each do |x|
    begin
      return x if x > 5
    ensure
      Thread.pass
    end
  end
  nil
end
p T.times.map { Thread.new { (0..9).map { find_first([1, 3, 7, 9]) }.uniq } }.map(&:value).uniq

# $~ is per thread (and per frame)
p T.times.map { |t|
  Thread.new do
    ok = true
    300.times do |i|
      "a#{t}b#{i}c" =~ /a(\d+)b(\d+)c/
      Thread.pass
      ok &&= ($1 == t.to_s && $2 == i.to_s)
    end
    ok
  end
}.map(&:value).uniq

# Fibers inside threads that switch while a Fiber is suspended
p T.times.map { |t|
  Thread.new do
    f = Fiber.new { |x| loop { x = Fiber.yield(x * 2) } }
    s = 0
    100.times { |i| s += f.resume(i); Thread.pass }
    s
  end
}.map(&:value).uniq

# thread-locals and fiber-locals
p T.times.map { |t|
  Thread.new do
    Thread.current[:me] = t
    Thread.current.thread_variable_set(:tv, t * 2)
    50.times { Thread.pass; sleep 0 }
    Thread.current[:me] == t && Thread.current.thread_variable_get(:tv) == t * 2
  end
}.map(&:value).uniq

# class bodies and define_method running while threads move
T.times.map { |t|
  Thread.new do
    Class.new do
      Thread.pass
      def hello; :hi; end
      Thread.pass
      define_method(:n) { t }
    end.new.then { |o| [o.hello, o.n] }
  end
}.map(&:value).then { |v| p v.map(&:first).uniq, v.map(&:last).sort == (0...T).to_a }

# producer/consumer chains across many threads, Mutex + ConditionVariable
m = Mutex.new
cv = ConditionVariable.new
turn = 0
order = []
ths = 12.times.map do |t|
  Thread.new do
    m.synchronize do
      cv.wait(m) until turn == t
      order << t
      turn += 1
      cv.broadcast
    end
  end
end
ths.each(&:join)
p order == (0...12).to_a

q = Queue.new
out = Queue.new
workers = 8.times.map { Thread.new { while (x = q.pop) != :done; out << x * x; Thread.pass; end } }
1000.times { |i| q << i }
8.times { q << :done }
workers.each(&:join)
sum = 0
sum += out.pop until out.empty?
p sum

# Thread#raise and #kill reach threads wherever they run
victims = 8.times.map { Thread.new { begin; loop { Thread.pass; [1] * 10 }; rescue RuntimeError => e; e.message; end } }
sleep 0.05
victims.each_with_index { |v, i| v.raise(RuntimeError, "stop#{i}") }
p victims.map(&:value) == 8.times.map { |i| "stop#{i}" }
k = 8.times.map { Thread.new { begin; loop { Thread.pass }; ensure; Thread.current[:cleaned] = true; end } }
sleep 0.05
k.each(&:kill).each(&:join)
p k.map { |x| x[:cleaned] }.uniq, k.map(&:status).uniq

# CPU-bound threads finish (stolen work runs to completion)
p T.times.map { |t| Thread.new { s = 0; 200_000.times { |i| s += i % (t + 1) }; s } }.map(&:value).size
GC.start
p :done
