# fork with other threads running: in the child only the forking thread exists (the others are
# dead, `join` returns), and the child can collect garbage and start threads of its own
# (ActiveSupport's parallel tests: a forked worker's DRb.stop_service joins the server thread)
busy = 4.times.map { |i| Thread.new { a = []; loop { a << "x#{i}" * 10; a.clear if a.size > 1000 } } }
sleeper = Thread.new { sleep }
q = Queue.new
waiter = Thread.new { q.pop }
sleep 0.2
r, w = IO.pipe
pid = fork do
  r.close
  out = []
  out << [busy.map(&:alive?), sleeper.alive?, sleeper.status, waiter.alive?, Thread.list.size]
  sleeper.join
  waiter.kill.join
  out << [sleeper.value.inspect, waiter.alive?]
  junk = 200_000.times.map { |i| [i, i.to_s * 3] }
  GC.start
  out << junk.size
  ts = 3.times.map { |i| Thread.new { (1..20_000).sum + i } }
  out << ts.map(&:value)
  q2 = Queue.new
  t = Thread.new { q2.pop * 2 }
  q2 << 21
  out << t.value
  m = Mutex.new
  out << m.synchronize { :locked }
  w.puts out.inspect
  w.close
end
w.close
child = r.read
Process.wait(pid)
puts child
p $?.success?, busy.all?(&:alive?), sleeper.alive?
# from a thread other than the main one
pid2 = nil
Thread.new { pid2 = fork { exit!(Thread.list.size == 1 ? 3 : 4) } }.join
Process.wait(pid2)
p $?.exitstatus
busy.each(&:kill)
sleeper.kill
waiter.kill

# `fork` goes through Process._fork, so a hook prepended to it runs in parent and child
# (ActiveSupport::ForkTracker: Rails' after-fork callbacks)
module ForkHook
  def _fork
    pid = super
    $hooked = pid.zero? ? :child : :parent
    pid
  end
end
Process.singleton_class.prepend(ForkHook)
r, w = IO.pipe
pid = fork { r.close; w.write($hooked.to_s); w.close }
w.close
p r.read, $hooked
Process.wait(pid)
pid = Process.fork { exit!(Process.respond_to?(:_fork) && $hooked == :child ? 5 : 6) }
Process.wait(pid)
p $?.exitstatus
