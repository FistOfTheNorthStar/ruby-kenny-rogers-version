# The Rack app tests/server/test.rb drives (built once, started with different settings).
require "socket"

class ServerTestApp
  def call(env)
    q = env["QUERY_STRING"]
    case env["PATH_INFO"]
    when "/hello"
      text("hello #{env["REQUEST_METHOD"]}")
    when "/echo"
      # the body back, and how it arrived
      input = env["rack.input"]
      $last_input = input
      body = +""
      if q == "pieces"
        buf = +""
        while input.read(65_536, buf)
          body << buf
        end
      else
        body = input.read
      end
      [200, {
        "content-type" => "application/octet-stream",
        "x-io" => input.is_a?(IO).to_s,
        "x-content-length" => env["CONTENT_LENGTH"].to_s,
        "x-transfer-encoding" => env["HTTP_TRANSFER_ENCODING"].to_s,
        "x-spool-files" => spool_files.size.to_s,
      }, [body]]
    when "/input-closed"
      text($last_input.closed?.to_s)
    when "/fds"
      # the process's open descriptors
      text(Dir.children("/dev/fd").size.to_s)
    when "/spool-files"
      text(spool_files.size.to_s)
    when "/headers"
      text(env.keys.count { |k| k.start_with?("HTTP_") }.to_s)
    when "/env"
      text(%w[REQUEST_METHOD PATH_INFO QUERY_STRING SERVER_PROTOCOL SERVER_NAME CONTENT_TYPE HTTP_X_ONE].map { |k| "#{k}=#{env[k]}" }.join("\n"))
    when "/sleep"
      sleep q.to_f
      text("slept")
    when "/wait"
      # blocks until the test writes a line to its control socket
      s = TCPSocket.new("127.0.0.1", q.to_i)
      line = s.gets.to_s.strip
      s.close
      text("waited #{line} #{Process.pid}")
    when "/pid"
      text(Process.pid.to_s)
    when "/big"
      [200, { "content-type" => "text/plain" }, ["x" * q.to_i]]
    when "/hijack"
      # full hijack: the app speaks HTTP itself, and keeps talking
      io = env["rack.hijack"].call
      io.write("HTTP/1.1 101 Switching Protocols\r\nupgrade: echo\r\nconnection: upgrade\r\n\r\n")
      io.flush
      line = io.gets
      io.write("echo: #{line}")
      io.close
      [200, {}, []]
    when "/hijack-thread"
      # full hijack, the socket served by a thread after the request returned (Action Cable)
      io = env["rack.hijack"].call
      Thread.new do
        io.write("HTTP/1.1 101 Switching Protocols\r\nconnection: upgrade\r\n\r\n")
        io.flush
        2.times { io.write("thread: #{io.gets}") }
        io.close
      end
      [-1, {}, []]
    when "/partial"
      # partial hijack: the server sends status and headers, the callback the rest
      hijack = lambda do |io|
        io.write("partial ")
        line = io.gets
        io.write("got: #{line}")
        io.close
      end
      [200, { "content-type" => "text/plain", "rack.hijack" => hijack }, []]
    when "/raise"
      raise "boom"
    else
      [404, { "content-type" => "text/plain" }, ["not found"]]
    end
  end

  def text(s)
    [200, { "content-type" => "text/plain" }, [s]]
  end

  # the server's temporary body files still in the temporary directory
  def spool_files
    dir = ENV["TMPDIR"].to_s
    return [] if dir.empty?
    Dir.children(dir).select { |f| f.start_with?("kenny-body-") }
  end
end

run ServerTestApp.new
