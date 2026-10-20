# kenny-run-flags: -I "$(ruby -e 'print Gem::Specification.find_by_name("sqlite3", "1.7.3").gem_dir')/lib"
# The sqlite3 gem (1.7.3): its Ruby layer from the installed gem, its C extension native in Kenny.
require "sqlite3"

db = SQLite3::Database.new(":memory:")
db.execute_batch <<-SQL
  create table t (id integer primary key, name text not null unique, score real, data blob);
  insert into t (name, score) values ('a', 1.5);
  -- a comment between statements
  insert into t (name, score) values ('b', 2);
SQL
db.execute("insert into t (name, score, data) values (?, ?, ?)", ["c", 3, SQLite3::Blob.new("\x00\x01")])
db.execute("insert into t (name, score) values (:n, :s)", n: "d", s: nil)
db.execute("insert into t (name, score) values (?, ?)", "e", 10**12)
db.execute("insert into t (name, score) values (:n, 7)", ":n" => "f")
puts "rowid=#{db.last_insert_row_id} changes=#{db.changes} total=#{db.total_changes}"
db.execute("select * from t order by id") { |row| p row }
rows = db.execute("select id, name, data from t where id = ?", 3)
p rows, rows[0][2].encoding, rows[0][1].encoding
p db.execute("select 9223372036854775807, -9223372036854775808, 4611686018427387904, 1.0 / 4, 'é', x'ff00', null")
p db.execute("select ?, ?, ?", [2**63 - 1, -2**62, 2**64]).first

# results_as_hash
db.results_as_hash = true
db.execute("select id, name from t order by id desc limit 2").each { |h| p h.to_a, h["name"], h[0] }
db.results_as_hash = false

p db.get_first_row("select count(*), max(score) from t")
p db.get_first_value("select name from t where id = ?", 4)
p db.get_first_value("select name from t where id = 99")
p db.execute2("select 1 as a, 'x' as b")

# prepared statements
stmt = db.prepare("select name from t where score > ? order by id")
stmt.bind_param(1, 1.0)
stmt.each { |r| p r }
p stmt.columns, stmt.types, stmt.column_count, stmt.bind_parameter_count, stmt.done?
stmt.reset!
p stmt.step, stmt.step
p stmt.execute(2).to_a
stmt.close
p stmt.closed?
begin
  stmt.step
rescue SQLite3::Exception => e
  p e.class, e.message
end
q = db.query("select id from t order by id")
p q.next, q.next.class, q.columns
q.close

# errors
begin
  db.execute("insert into t (name) values ('a')")
rescue SQLite3::ConstraintException => e
  p e.class, e.message, e.code
end
begin
  db.execute("select * from nope")
rescue SQLite3::SQLException => e
  p e.class, e.message, e.code
end
begin
  db.execute("selec 1")
rescue SQLite3::Exception => e
  p e.class, e.message
end
s2 = db.prepare("select :x")
begin
  s2.bind_param(:y, 1)
rescue SQLite3::Exception => e
  p e.message
ensure
  s2.close
end
begin
  db.execute("select ?", [Object.new])
rescue RuntimeError => e
  p e.message[0, 18]
end

# transactions
db.transaction { db.execute("insert into t (name) values ('x')") }
begin
  db.transaction do
    db.execute("insert into t (name) values ('y')")
    raise "boom"
  end
rescue => e
  p e.message
end
p db.execute("select name from t order by id").flatten
p db.transaction_active?
db.transaction(:immediate)
db.execute("delete from t where name = 'x'")
p db.transaction_active?
db.rollback
p db.get_first_value("select count(*) from t")

# functions and aggregates
db.create_function("twice", 1) { |f, x| f.result = x * 2 }
p db.execute("select twice(score) from t where score is not null order by id")
db.define_function("plus") { |a, b| a + b }
p db.get_first_value("select plus(2, 3)")
db.create_aggregate("mysum", 1) do
  step { |ctx, x| ctx[:s] = (ctx[:s] || 0) + (x || 0) }
  finalize { |ctx| ctx.result = ctx[:s] }
end
p db.get_first_value("select mysum(id) from t")
class Joiner
  def self.arity; 1; end
  def initialize; @parts = []; end
  def step(x); @parts << x; end
  def finalize; @parts.join("|"); end
end
db.define_aggregator("joined", Joiner.new)
p db.get_first_value("select joined(name) from (select name from t order by id)")
begin
  db.define_function("bad") { raise ArgumentError, "inner" }
  db.execute("select bad()")
rescue => e
  p e.class, e.message
end
db.collation("rev", Class.new { def compare(a, b) b <=> a end }.new)
p db.execute("select name from t order by name collate rev limit 3").flatten

# execute_batch2, pragmas, misc
p db.execute_batch2("select id, name from t order by id limit 2")
p db.user_version, db.foreign_keys
p db.complete?("select 1;"), db.complete?("select (")
p SQLite3::VERSION, SQLite3.libversion > 3_000_000, SQLite3.threadsafe?, SQLite3.sqlcipher?
p db.encoding
traced = []
db.trace { |sql| traced << sql }
db.execute("select 42")
db.trace(nil)
p traced.size, traced.last.end_with?("select 42")

# a database file, reopened read-only
path = "/tmp/kenny_sqlite_test_#{Process.pid}.db"
SQLite3::Database.open(path) do |f|
  f.execute("create table kv (k text primary key, v)")
  f.execute("insert into kv values ('one', 1), ('two', 2.5)")
end
ro = SQLite3::Database.new(path, readonly: true)
p ro.execute("select * from kv order by k"), ro.readonly?
begin
  ro.execute("insert into kv values ('three', 3)")
rescue SQLite3::ReadOnlyException => e
  p e.class, e.message
end
ro.close
File.delete(path)

db.close
p db.closed?
begin
  db.execute("select 1")
rescue => e
  p e.class, e.message
end
