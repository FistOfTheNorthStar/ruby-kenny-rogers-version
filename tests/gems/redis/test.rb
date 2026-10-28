require "bundler/setup"
require "redis"
require "redis/distributed"
require "socket"

# A closed local port: bind, read the port, close.
s = TCPServer.new("127.0.0.1", 0); closed_port = s.addr[1]; s.close
r = Redis.new(host: "127.0.0.1", port: closed_port, connect_timeout: 0.5, reconnect_attempts: 0)
p r.connected?, r._client.host, r._client.db, r._client.timeout, r.id.sub(closed_port.to_s, "PORT")
begin
  r.ping
rescue Redis::CannotConnectError => e
  p e.class, e.message.sub(closed_port.to_s, "PORT"), e.is_a?(Redis::BaseConnectionError)
end
u = Redis.new(url: "redis://:secret@127.0.0.1:6390/3")
p u._client.host, u._client.port, u._client.db, u._client.password, Redis::VERSION.is_a?(String)
p Redis::Connection.drivers.map(&:name)
p Object.new.extend(Redis::Connection::CommandHelper).build_command(["SET", "k", "vé", ["a", 1]])

# A tiny RESP server on 127.0.0.1 to see the commands the client sends.
srv = TCPServer.new("127.0.0.1", 0)
port = srv.addr[1]
log = []
store = {}
bulk = ->(v) { v.nil? ? "$-1\r\n" : "$#{v.bytesize}\r\n#{v}\r\n" }
Thread.new do
  c = srv.accept
  while (line = c.gets)
    n = line[1..].to_i
    args = Array.new(n) { len = c.gets[1..].to_i; d = c.read(len + 2); d[0, len] }
    log << args
    reply = case args[0].upcase
      when "PING" then "+PONG\r\n"
      when "SET" then store[args[1]] = args[2]; "+OK\r\n"
      when "GET" then bulk.(store[args[1]])
      when "INCR" then store[args[1]] = (store[args[1]].to_i + 1).to_s; ":#{store[args[1]]}\r\n"
      when "DEL" then ":#{args[1..].count { |k| store.delete(k) }}\r\n"
      when "MGET" then "*#{args.size - 1}\r\n" + args[1..].map { |k| bulk.(store[k]) }.join
      when "HGETALL" then "*4\r\n" + bulk.("f1") + bulk.("v1") + bulk.("f2") + bulk.("v2")
      when "SELECT" then "+OK\r\n"
      when "LPUSH" then ":#{args.size - 2}\r\n"
      when "BOOM" then "-ERR unknown command 'BOOM'\r\n"
      else "+OK\r\n"
    end
    c.write(reply)
  end
end
rc = Redis.new(host: "127.0.0.1", port: port, db: 2)
p rc.ping, rc.set("name", "kenny"), rc.get("name"), rc.get("missing"), rc.incr("n"), rc.incr("n")
p rc.mget("name", "n", "zz"), rc.hgetall("h"), rc.del("name", "nope"), rc.lpush("l", %w[a b c])
p rc.set("t", "1", ex: 10, nx: true)
p rc.pipelined { |pl| pl.set("a", "1"); pl.incr("a"); pl.get("a") }
begin
  rc.call("BOOM")
rescue Redis::CommandError => e
  p e.class, e.message
end
p rc.connected?
rc.close
p log
