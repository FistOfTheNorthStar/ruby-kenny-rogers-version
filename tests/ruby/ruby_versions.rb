# What differs between Ruby 3.4 and 4.0 builds (the expected output for a 4.0 build is
# ruby_versions.4.0.out), and fixes made while adding 4.0 that apply to both.

def t(label)
  r = yield
  puts "#{label}: #{r.inspect}"
rescue Exception => e
  puts "#{label}: #{e.class}: #{e.message}"
end

# --- the Ruby module, Set and Pathname without require
t(:ruby_module) { defined?(Ruby::VERSION) && [Ruby::VERSION == RUBY_VERSION, Ruby::ENGINE] }
t(:set_literal) { Set[3, 1, 2] }
t(:set_class) { [Set[1].class, Set.new([1, 2]).size] }
t(:pathname) { defined?(Pathname) ? Pathname("/usr/lib").basename.to_s : :not_core }

# --- reopening Set before its first use adds to the real Set
class Set
  def kenny_extra = :reopened
end
t(:set_reopen) { Set[1].kenny_extra }

# --- Set subclasses
class MySet < Set; end
t(:set_subclass) { MySet.new([1]).inspect }

# --- Warning and warn
t(:warning_categories) { %i[deprecated experimental performance].map { Warning[_1] } }
t(:warn_bad_uplevel) { warn("x", uplevel: -1) }
t(:warn_bad_category) { warn("x", category: :nope) }

# --- FrozenError#receiver
t(:frozen_receiver) do
  s = "abc".freeze
  begin
    s << "d"
  rescue FrozenError => e
    e.receiver.equal?(s)
  end
end
t(:frozen_no_receiver) { FrozenError.new("x").receiver }

# --- arity and parameters
t(:arity) { [->(a, k: 1) {}.arity, proc { |a, b = 1| }.arity, method(:t).arity] }
t(:proc_params) { proc { |(a, b)| }.parameters }

# --- Ruby 4.0 additions
t(:rfind) { [1, 2, 3, 4].rfind(&:odd?) }
t(:find_ifnone) { [1].find(-> { :none }) { false } }
t(:strip_selectors) { ["xxhixx".strip("x"), "aabba".lstrip("a"), "--x--".rstrip("-")] }
t(:log1p) { Math.log1p(0.0) }
t(:produce_size) { [Enumerator.produce(1) { _1 + 1 }.size, Enumerator.produce(1, size: 3) { _1 + 1 }.size] }
t(:endless_to_set) { RUBY_VERSION >= "4" ? (1..).to_set : :loops_before_4 }
t(:status_and) { `true`; $?.respond_to?(:&) }
t(:pipe_open) { open("|echo hi", &:read) }
class Secretive
  def initialize = (@host = "h"; @password = "p")
  private def instance_variables_to_inspect = [:@host]
end
t(:ivars_to_inspect) { Secretive.new.inspect.sub(/0x\h+/, "X") }
t(:beginless_max) { (...5).max }

# --- both versions
t(:range_overlap) { [(1..3).overlap?(3..5), (1...3).overlap?(3..5), (..1).overlap?(1..)] }
t(:beginless_max_incl) { (..5).max }
t(:endless_max) { (1..).max }
t(:thread_raise_cause) do
  th = Thread.new { Thread.current.report_on_exception = false; sleep }
  sleep 0.05 until th.status == "sleep"
  th.raise(RuntimeError, "boom", cause: ArgumentError.new("why"))
  begin
    th.join
  rescue => e
    [e.message, e.cause&.message]
  end
end
t(:fiber_raise) do
  f = Fiber.new { Fiber.yield 1 rescue [:rescued, $!.message] }
  f.resume
  f.raise(RuntimeError, "in fiber")
end
t(:fiber_raise_unborn) { Fiber.new {}.raise("x") }
t(:sleep_inf) { sleep(Float::INFINITY) }
t(:sleep_neg) { sleep(-1) }
t(:join_inf) { Thread.new { 1 }.join(Float::INFINITY).value }
t(:select_inf) { r, w = IO.pipe; w << "x"; IO.select([r], nil, nil, Float::INFINITY)&.first&.size }
t(:shift_jis_name) { "\u00e9".encode("Shift_JIS") }
