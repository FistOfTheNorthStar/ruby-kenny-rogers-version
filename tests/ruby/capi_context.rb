# kenny-run-flags: --ext tests/ruby/capi_context
# Where an exception raised in C code goes, and which C method is running, belong to the Ruby
# thread or fiber and not the OS thread. A thread waiting in C code can continue on another OS
# thread, and others run C methods meanwhile: it crashed, or jumped onto the wrong stack.
require "capi_context"

# a fiber left inside a C method: the resumer's C method must not jump into it
fiber = Fiber.new do
  CapiContext.yield_then_raise(1) { Fiber.yield :in_fiber }
rescue ArgumentError => e
  e.message
end
begin
  CapiContext.yield_then_raise(2) { p fiber.resume }
rescue ArgumentError => e
  puts e.message
end
p fiber.resume

# which C method is running, with a fiber left in another
fiber = Fiber.new { CapiContext.plain; CapiContext.yield_then_raise(3) { Fiber.yield } rescue :done }
p CapiContext.yield_then_name { fiber.resume }
p fiber.resume

# rb_protect around a fiber switch
fiber = Fiber.new { CapiContext.protected_yield { Fiber.yield; raise "inner" } }
p CapiContext.protected_yield { fiber.resume; raise "outer" }
p fiber.resume

# threads waiting inside C methods, all at once
wrong = Queue.new
threads = 24.times.map do |i|
  Thread.new do
    500.times do |n|
      begin
        CapiContext.yield_then_raise(i) { n % 10 == 0 ? sleep(0.0005) : Thread.pass }
        wrong << [i, :no_raise]
      rescue ArgumentError => err
        wrong << [i, err.message] unless err.message == "raised for #{i}"
      end
      wrong << [i, :name] unless CapiContext.yield_then_name { Thread.pass } == :yield_then_name
    end
  end
end
threads.each(&:join)
p wrong.size
