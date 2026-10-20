# Threads running Ruby code on several cores at once: shared objects stay consistent
# (single operations on Array/Hash/String are atomic), Mutex/Queue/ConditionVariable work
# across cores, and the GC stops every thread safely while they allocate.

N = 8

# Mutex-protected counter
count = 0
m = Mutex.new
N.times.map { Thread.new { 2000.times { m.synchronize { count += 1 } } } }.each(&:join)
p count

# Array#<< from many threads: no element lost, no crash
arr = []
N.times.map { |t| Thread.new { 1000.times { |i| arr << t * 1000 + i } } }.each(&:join)
p arr.size, arr.sort == (0...N * 1000).to_a

# Hash#[]= with distinct keys from many threads
h = {}
N.times.map { |t| Thread.new { 500.times { |i| h["k#{t}-#{i}"] = i } } }.each(&:join)
p h.size, h["k3-499"]

# String#<< from many threads
s = +""
N.times.map { Thread.new { 1000.times { s << "x" } } }.each(&:join)
p s.size

# different ivars of one shared object
class Box; end
box = Box.new
N.times.map { |t| Thread.new { 100.times { |i| box.instance_variable_set(:"@v#{t}", i) } } }.each(&:join)
p box.instance_variables.size, box.instance_variable_get(:@v5)

# lots of allocation on every thread (collections stop all of them)
res = N.times.map do |t|
  Thread.new do
    acc = []
    20_000.times { |i| acc << "s#{i}" if i % 100 == 0; [i, i.to_s, { a: i }] }
    acc.size + t
  end
end.map(&:value)
p res

# Queue across threads: every item consumed exactly once
q = Queue.new
consumers = 4.times.map { Thread.new { got = []; while (x = q.pop) != :stop; got << x; end; got } }
producers = 4.times.map { |t| Thread.new { 1000.times { |i| q << t * 1000 + i } } }
producers.each(&:join)
4.times { q << :stop }
all = consumers.flat_map(&:value)
p all.size, all.uniq.size, all.sort == (0...4000).to_a

# SizedQueue applies back-pressure across threads
sq = SizedQueue.new(3)
prod = Thread.new { 200.times { |i| sq << i }; sq << nil }
sum = 0
while (x = sq.pop)
  sum += x
end
prod.join
p sum

# ConditionVariable handoff between threads
mx = Mutex.new
cv = ConditionVariable.new
turn = 0
log = []
ths = 3.times.map do |me|
  Thread.new do
    10.times do
      mx.synchronize do
        cv.wait(mx) until turn % 3 == me
        log << me
        turn += 1
        cv.broadcast
      end
    end
  end
end
ths.each(&:join)
p log.first(7), log.size

# defining methods and constants while other threads call them
class Calc
  def base; 1; end
end
calc = Calc.new
callers = 4.times.map { Thread.new { r = 0; 2000.times { r += calc.base }; r } }
definer = Thread.new { 50.times { |i| Calc.define_method(:"m#{i}") { i } } }
definer.join
p callers.map(&:value), calc.m49

# Thread#raise and #kill reach threads running on other cores
spinner = Thread.new do
  begin
    x = 0
    loop { x += 1 }
  rescue RuntimeError => e
    e.message
  end
end
sleep 0.05
spinner.raise("stop spinning")
p spinner.value
looper = Thread.new { loop { [1, 2, 3].map { |i| i * 2 } } }
sleep 0.02
looper.kill
looper.join
p looper.alive?

# Thread-local state stays with its thread
vals = N.times.map { |t| Thread.new { Thread.current[:me] = t; sleep 0.001; Thread.current[:me] } }.map(&:value)
p vals == (0...N).to_a

# exceptions inside threads stay with their thread
errs = 4.times.map { |t| Thread.new { begin; raise ArgumentError, "e#{t}"; rescue => e; e.message; end } }.map(&:value)
p errs
