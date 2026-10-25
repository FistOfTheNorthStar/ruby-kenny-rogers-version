# Process.clock_gettime: the platform's clocks and units
p Process.constants.grep(/\ACLOCK_(REALTIME|MONOTONIC|PROCESS_CPUTIME_ID|THREAD_CPUTIME_ID)\z/).sort
a = Process.clock_gettime(Process::CLOCK_MONOTONIC)
b = Process.clock_gettime(Process::CLOCK_MONOTONIC)
p a.is_a?(Float), b >= a
cpu = Process.clock_gettime(Process::CLOCK_THREAD_CPUTIME_ID, :float_millisecond)
x = 0; 200_000.times { |i| x += i }
p Process.clock_gettime(Process::CLOCK_THREAD_CPUTIME_ID, :float_millisecond) >= cpu
p Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID).is_a?(Float)
p (Process.clock_gettime(Process::CLOCK_REALTIME) - Time.now.to_f).abs < 1
p Process.clock_gettime(Process::CLOCK_MONOTONIC, :nanosecond).is_a?(Integer)
p Process.clock_gettime(Process::CLOCK_MONOTONIC, :millisecond).is_a?(Integer)
p (Process.clock_gettime(Process::CLOCK_MONOTONIC, :parsec) rescue $!.class)
p (Process.clock_gettime(12345) rescue $!.class)
