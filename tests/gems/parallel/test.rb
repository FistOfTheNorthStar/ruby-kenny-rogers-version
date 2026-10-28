require "bundler/setup"
require "parallel"

p Parallel.map(1..10, in_threads: 4) { |i| i * i }
PARENT = Process.pid
p Parallel.map(%w[a b c d], in_processes: 2) { |s| [s.upcase, Process.pid != PARENT] }
p Parallel.map_with_index(%w[x y z], in_threads: 2) { |s, i| "#{i}#{s}" }
p Parallel.flat_map([1, 2, 3], in_threads: 2) { |i| [i, -i] }
p Parallel.filter_map(1..8, in_processes: 3) { |i| i * 10 if i.even? }
p Parallel.any?([1, 2, 3], in_threads: 2) { |i| i > 2 }
p Parallel.all?([1, 2, 3], in_processes: 2) { |i| i > 0 }

sum = 0
lock = Mutex.new
Parallel.each(1..100, in_threads: 8) { |i| lock.synchronize { sum += i } }
p sum

begin
  Parallel.map([1, 2, 3], in_processes: 2) { |i| raise ArgumentError, "bad #{i}" if i == 2; i }
rescue ArgumentError => e
  p [e.class, e.message]
end

p Parallel.map([1, 2, 3, 4], in_threads: 2) { |i| raise Parallel::Break if i == 3; i }

finished = []
Parallel.map([5, 6, 7], in_processes: 2, finish: ->(item, idx, result) { finished << [item, idx, result] }) { |i| i + 1 }
p finished.sort

p Parallel.map([[1, 2], [3, 4]], in_processes: 2) { |a, b| { sum: a + b, obj: Struct.new(:v).new(a).v } }
p Parallel.processor_count.positive?
p Parallel.worker_number.nil?
p Parallel.map([1, 2], in_threads: 2) { Parallel.worker_number.is_a?(Integer) }
p Parallel.map(1..3, in_processes: 0) { |i| i + 100 }
gen = [1, 2, 3].each
p Parallel.map(-> { gen.next rescue Parallel::Stop }, in_threads: 2) { |i| i * 3 }.sort
