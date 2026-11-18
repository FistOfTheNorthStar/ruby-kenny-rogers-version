require "bundler/setup"
require "eventmachine"

class Echo < EM::Connection
  def receive_data(d)
    send_data(d.upcase)
    close_connection_after_writing
  end
end

class Client < EM::Connection
  def post_init = send_data("hello")
  def receive_data(d) = puts("client got #{d}")
  def unbind = EM.stop
end

EM.run { EM.add_timer(0.01) { puts "timer"; EM.stop } }
EM.run do
  port = EM.start_server("127.0.0.1", 0, Echo)
  port = Socket.unpack_sockaddr_in(EM.get_sockname(port)).first
  EM.connect("127.0.0.1", port, Client)
end
p EM.reactor_running?
ticks = 0
EM.run do
  t = EM.add_periodic_timer(0.001) { (ticks += 1) == 3 and (t.cancel; EM.stop) }
end
p ticks
EM.run do
  EM.next_tick do
    puts "tick"
    EM.defer(-> { 21 * 2 }, ->(r) { p r; EM.stop })
  end
end
