require "bundler/setup"
require "pg"

conn = PG.connect(dbname: "postgres")
conn.exec("SET client_min_messages TO WARNING")
conn.exec("DROP TABLE IF EXISTS kenny_pg_test")
conn.exec("CREATE TEMP TABLE kenny_pg_test (id serial PRIMARY KEY, name text NOT NULL UNIQUE, score numeric(6,2), tags text[], data jsonb, at timestamptz)")
conn.exec_params("INSERT INTO kenny_pg_test (name, score, tags, data, at) VALUES ($1, $2, $3, $4, $5)",
                 ["alice", 12.5, "{a,b}", '{"x": 1}', "2024-01-02 03:04:05+00"])
conn.prepare("ins", "INSERT INTO kenny_pg_test (name, score) VALUES ($1, $2) RETURNING id")
p conn.exec_prepared("ins", ["bob", 7]).getvalue(0, 0)
r = conn.exec("SELECT name, score, tags, data, at FROM kenny_pg_test ORDER BY id")
p r.ntuples, r.nfields, r.fields, r.values
p r.map { |row| row["name"] }, r.column_values(1)
conn.type_map_for_results = PG::BasicTypeMapForResults.new(conn)
r2 = conn.exec("SELECT 1::int AS i, 2.5::float8 AS f, true AS b, 'x'::text AS t, '{1,2,3}'::int[] AS a, NULL AS n")
p r2.to_a
begin
  conn.exec("INSERT INTO kenny_pg_test (name) VALUES ('alice')")
rescue PG::UniqueViolation => e
  p e.class, e.result.error_field(PG::PG_DIAG_SQLSTATE)
end
begin
  conn.exec("SELECT * FROM nope")
rescue PG::UndefinedTable => e
  p e.class.ancestors.include?(PG::Error)
end
conn.transaction do |c|
  c.exec("INSERT INTO kenny_pg_test (name) VALUES ('tx')")
  raise "rollback"
rescue RuntimeError
  raise PG::Error, "x" rescue nil
end rescue p $!.class
p conn.exec("SELECT count(*) FROM kenny_pg_test WHERE name = 'tx'").getvalue(0, 0)
p conn.escape_string("O'Reilly"), conn.quote_ident("we\"ird"), PG::Connection.escape_bytea("\x00\xff".b)
conn.copy_data("COPY kenny_pg_test (name, score) FROM STDIN") do
  conn.put_copy_data("carol\t3.25\n")
  conn.put_copy_data("dave\t\\N\n")
end
p conn.exec("SELECT name, score FROM kenny_pg_test WHERE name IN ('carol','dave') ORDER BY name").values
out = []
conn.copy_data("COPY (SELECT name FROM kenny_pg_test ORDER BY id) TO STDOUT") { while (l = conn.get_copy_data); out << l; end }
p out
conn.send_query("SELECT pg_sleep(0.05), 42")
conn.block
p conn.get_result.getvalue(0, 1)
conn.get_result
p conn.status == PG::CONNECTION_OK, conn.transaction_status, conn.server_version > 100000
conn.close
p conn.finished?
