# kenny-run-flags: -I "$(ruby -e 'print Gem::Specification.find_by_name("sqlite3", "1.7.3").gem_dir')/lib"
# SQLite under threads and compaction.
#
# Threads writing to one database through their own connections wait for each other's locks
# in a busy handler: a Ruby block that sleeps (Rails 8's busy_handler_timeout) or the native
# busy_timeout. SQLite holds the connection's mutex meanwhile, which only its OS thread can
# unlock, so the waiting thread must resume there (it used to move to another one, leave the
# mutex locked, and the next query on that connection hung).
#
# And what SQLite's callbacks reach through pointers the collector can't rewrite (the
# Database object, an aggregator and its instances, a collation's comparer) must stay put
# when a collection compacts (they used to move, and a callback then read a stale slot).
# (Not poison mode: it only evacuates pages that empty completely, and these don't. CRuby's
# sqlite3 1.7.3 C extension loses the trace block that way: this output is CRuby's with
# sqlite3 2.x, or without GC.compact.)
require "sqlite3"
require "tmpdir"

def churn = 3.times { Array.new(3000) { |i| "junk #{i}" * 2 }; GC.compact }

Dir.mktmpdir do |dir|
  path = File.join(dir, "t.db")
  SQLite3::Database.new(path) { |db| db.execute("create table t (id integer primary key, n integer)") }
  threads = 24.times.map do |i|
    Thread.new do
      db = SQLite3::Database.new(path)
      if i.even?
        deadline = nil
        db.busy_handler do |count|
          now = Process.clock_gettime(Process::CLOCK_MONOTONIC)
          deadline = now + 10 if count.zero?
          now < deadline && (sleep(0.001); true)
        end
      else
        db.busy_timeout = 10_000
      end
      60.times do |j|
        db.transaction(:immediate) do
          db.execute("insert into t (n) values (?)", [i * 1000 + j])
          x = 0
          3000.times { x += 1 } # (holds the write lock a moment)
        end
      end
      db.close
      :done
    end
  end
  watchdog = Thread.new { sleep 60; puts "stuck"; exit! 1 }
  p threads.map(&:value).tally
  watchdog.kill
  SQLite3::Database.new(path) { |db| p db.execute("select count(*), count(distinct n) from t") }
end

class Joiner
  def initialize = @parts = []
  def initialize_copy(_) = @parts = [] # (each group gets a copy)
  def step(x) = @parts << x.to_s
  def finalize = @parts.join("+")
end
class Rev
  def compare(a, b) = b <=> a
end

# (most of the Databases opened here are closed and dropped, so the pages of the ones kept
# are nearly empty and a compaction moves them)
dbs = []
90.times do |i|
  db = SQLite3::Database.new(":memory:")
  Array.new(40) { |k| "garbage #{k}" }
  unless i % 30 == 0
    db.close
    next
  end
  db.execute("create table w (g integer, s text)")
  db.execute("insert into w values (1, 'a'), (1, 'b'), (2, 'c'), (2, 'd'), (2, 'e')")
  db.define_aggregator("joined", Joiner.new)
  db.collation("rev", Rev.new)
  traced = []
  db.trace { |sql| traced << sql[0, 6] }
  db.busy_handler { |_| false }
  dbs << [db, traced]
end
churn
dbs.each do |db, traced|
  db.execute("select g, joined(s) from w group by g order by g") { churn }
  p db.execute("select g, joined(s) from w group by g order by g")
  p db.execute("select s from w order by s collate rev").flatten
  churn
  p traced.uniq
end

# Statement#step is audited (stackmap.rs): a compaction inside a callback goes ahead
db = SQLite3::Database.new(":memory:")
ran = []
db.create_function("churned", 1) do |f, x|
  Array.new(500) { |k| "g#{k}" }
  ran << !GC.compact.nil?
  f.result = x.to_s * 2
end
p db.execute("select churned(?), ?, 1.5, churned(?)", ["ab", "cd", "ef"]), ran.uniq
