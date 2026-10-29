# The Coverage API: states and their errors, suspend/resume, peek_result, result's stop:
# and clear:, compatible mode, oneshot_lines, supported?, line_stub.
require "coverage"

def show(r)
  r.keys.select { |k| k.include?("/coverage/") }.sort.each do |k|
    puts "#{File.basename(k)}: #{r[k].inspect}"
  end
end

def err
  yield
rescue => e
  puts "#{e.class}: #{e.message}"
end

p Coverage.state, Coverage.running?
err { Coverage.result }
err { Coverage.peek_result }
err { Coverage.resume }
err { Coverage.suspend }
p %i[lines oneshot_lines branches methods eval all].map { |m| Coverage.supported?(m) }
err { Coverage.supported?("lines") }
err { Coverage.setup(lines: true, oneshot_lines: true) }
err { Coverage.setup(3) }
p Coverage.state

Coverage.setup(lines: true, branches: true)
p Coverage.state
err { Coverage.setup }
err { Coverage.suspend }
require_relative "coverage/counter"
show Coverage.peek_result
Coverage.resume
err { Coverage.resume }
p Coverage.state
CovCounter.tick(2)
CovCounter.loop_to(3)
show Coverage.peek_result
Coverage.suspend
CovCounter.tick(3)
require_relative "coverage/second"
CovSecond.run(1)
show Coverage.peek_result
Coverage.resume
CovSecond.run(2)
r = Coverage.result(stop: false, clear: true)
show r
p r.frozen?, Coverage.state
CovCounter.tick(4)
show Coverage.peek_result
show Coverage.result
p Coverage.state
CovCounter.tick(5)

# compatible mode: the lines arrays themselves
Coverage.start
require_relative "coverage/third"
CovThird.go(2)
r = Coverage.result
show r
p r.values.all?(&:frozen?)

# oneshot_lines: the lines run once, in the order they first ran
Coverage.start(oneshot_lines: true)
require_relative "coverage/fourth"
CovFourth.go(5)
CovFourth.go(0)
show Coverage.peek_result
Coverage.result(stop: false, clear: true)
CovFourth.go(5)
show Coverage.result

Coverage.start(:all)
p Coverage.state
show Coverage.result
Coverage.start(lines: true, eval: true)
show Coverage.result

p Coverage.line_stub(File.join(__dir__, "coverage/counter.rb"))
p Coverage.line_stub(File.join(__dir__, "coverage/fourth.rb"))
