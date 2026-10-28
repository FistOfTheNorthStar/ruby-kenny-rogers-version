require "bundler/setup"
require "connection_pool"

class Conn
  @@n = 0
  @@lock = Mutex.new
  attr_reader :id
  def initialize = @@lock.synchronize { @id = (@@n += 1) }
  def query(x) = "conn#{@id > 0 ? "" : "?"}:#{x}"
  def close = nil
end
pool = ConnectionPool.new(size: 3, timeout: 1) { Conn.new }
p pool.size, pool.available
pool.with { |c| p c.query(1), pool.available }
results = Queue.new
threads = 10.times.map { |i| Thread.new { pool.with { |c| sleep 0.01; results << c.query(i) } } }
threads.each(&:join)
p results.size, Array.new(results.size) { results.pop }.sort.first(3)
p pool.available <= 3
ids = Mutex.new; seen = []
6.times.map { Thread.new { pool.with { |c| ids.synchronize { seen << c.id }; sleep 0.02 } } }.each(&:join)
p seen.uniq.size <= 3
small = ConnectionPool.new(size: 1, timeout: 0.05) { Conn.new }
small.with do
  t = Thread.new { begin; small.with { :never }; rescue ConnectionPool::TimeoutError => e; e.class; end }
  p t.value
end
wrapper = ConnectionPool::Wrapper.new(size: 2) { Conn.new }
p wrapper.query("w"), wrapper.respond_to?(:query)
wrapper.with { |c| p c.class }
pool.shutdown { |c| c.close }
begin
  pool.with { }
rescue ConnectionPool::PoolShuttingDownError => e
  p e.class
end
p ConnectionPool::VERSION.is_a?(String)
pl = ConnectionPool.new(size: 2) { Conn.new }
p pl.checkout.class, pl.available, pl.checkin, pl.available
p pl.with(timeout: 0.1) { |c| c.class }
pl.reload { |c| c.close }
p pl.available
