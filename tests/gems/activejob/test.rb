require "bundler/setup"
require "active_job"

ActiveJob::Base.logger = Logger.new(nil)
$log = []
class ApplicationJob < ActiveJob::Base; end
class GreetJob < ApplicationJob
  queue_as :mailers
  before_perform { |job| $log << [:before, job.arguments] }
  after_perform { $log << :after }
  def perform(name, times: 1, opts: {})
    $log << "hello #{name}" * times
    $log << opts
    name.upcase
  end
end
class FlakyJob < ApplicationJob
  retry_on ArgumentError, attempts: 3, wait: 0
  discard_on KeyError
  def perform(kind)
    $log << [:flaky, executions]
    raise ArgumentError, "again" if kind == :retry && executions < 3
    raise KeyError, "gone" if kind == :discard
  end
end

ActiveJob::Base.queue_adapter = :inline
p GreetJob.perform_now("ann"), GreetJob.perform_later("bob", times: 2, opts: { "k" => [1, :sym] }).class
ActiveJob::Base.queue_adapter = :test
adapter = ActiveJob::Base.queue_adapter
adapter.perform_enqueued_jobs = adapter.perform_enqueued_at_jobs = true
FlakyJob.perform_later(:retry)
FlakyJob.perform_later(:discard)
$log.each { |x| p x }
p adapter.performed_jobs.map { _1[:job] }
adapter.perform_enqueued_jobs = adapter.perform_enqueued_at_jobs = false
adapter.enqueued_jobs.clear
GreetJob.perform_later("cat")
GreetJob.set(queue: :low, wait_until: Time.utc(2030, 1, 1)).perform_later("dog")
GreetJob.set(priority: 5).perform_later("eel", times: 3)
p adapter.enqueued_jobs.size
adapter.enqueued_jobs.each { |j| p j.slice(:job, :queue, :at, :priority).merge(args: j[:args]) }
job = GreetJob.new("fox", times: 1)
s = job.serialize
p s.slice("job_class", "queue_name", "arguments", "priority", "executions")
j2 = ActiveJob::Base.deserialize(s); p j2.class, j2.queue_name
p ActiveJob::Arguments.serialize([1, "a", :b, { x: Time.utc(2020, 1, 1) }, 1.5, nil, [true]])
p ActiveJob::Arguments.deserialize(ActiveJob::Arguments.serialize([{ d: Date.new(2020, 1, 2), s: :sym, bd: BigDecimal("1.5") }]))
begin
  ActiveJob::Arguments.serialize([Object.new])
rescue ActiveJob::SerializationError => e
  p e.class
end
p GreetJob.queue_name, FlakyJob.new.queue_name
