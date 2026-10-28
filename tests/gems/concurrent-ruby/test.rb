require "bundler/setup"
require "concurrent"

f = Concurrent::Promises.future(2) { |x| x * 21 }
p f.value, f.fulfilled?
chain = Concurrent::Promises.future { 1 }.then { |v| v + 1 }.then { |v| v * 10 }
p chain.value!
z = Concurrent::Promises.zip(*(1..5).map { |i| Concurrent::Promises.future(i) { |n| n * n } })
p z.value!
bad = Concurrent::Promises.future { raise ArgumentError, "boom" }
p bad.value, bad.reason.class, bad.rejected?, bad.rescue { |e| e.message }.value
p Concurrent::Promises.any(Concurrent::Promises.fulfilled_future(:a), Concurrent::Promises.future { sleep 0.2; :b }).value

m = Concurrent::Map.new
threads = 8.times.map { |i| Thread.new { 100.times { |j| m.compute_if_absent("k#{j % 10}") { 0 }; m.compute("c") { |v| (v || 0) + 1 } } } }
threads.each(&:join)
p m.size, m["c"], m.keys.sort.first(3), m.fetch("zz", :dflt), m.put_if_absent("k1", 99), m.key?("k1")

a = Concurrent::AtomicFixnum.new(0)
8.times.map { Thread.new { 1000.times { a.increment } } }.each(&:join)
p a.value
ar = Concurrent::AtomicReference.new([])
p ar.update { |v| v + [1] }, ar.compare_and_set([1], [2]), ar.get
ab = Concurrent::AtomicBoolean.new; ab.make_true; p ab.true?

pool = Concurrent::FixedThreadPool.new(4)
results = Concurrent::Array.new
20.times { |i| pool.post { results << i * 2 } }
pool.shutdown; pool.wait_for_termination
p results.sort.sum, pool.shutdown?

latch = Concurrent::CountDownLatch.new(3)
3.times { Thread.new { latch.count_down } }
p latch.wait(2)
ev = Concurrent::Event.new; Thread.new { ev.set }; p ev.wait(2)
d = Concurrent::Delay.new { :delayed }; p d.value
iv = Concurrent::IVar.new; iv.set(5); p iv.value
mv = Concurrent::MVar.new; Thread.new { mv.put(:v) }; p mv.take
ag = Concurrent::Agent.new(0); 5.times { ag.send { |v| v + 1 } }; ag.await_for(2); p ag.value
tv = Concurrent::TVar.new(1); Concurrent.atomically { tv.value += 1 }; p tv.value
sem = Concurrent::Semaphore.new(2); sem.acquire; p sem.available_permits
p Concurrent::Promises.schedule(0.05) { :scheduled }.value
tt = Concurrent::TimerTask.new(execution_interval: 0.02) { :tick }; p tt.class
p Concurrent::Hash.new.merge(a: 1), Concurrent.monotonic_time.class, Concurrent::ImmutableStruct.new(:x, :y).new(1, 2).values
