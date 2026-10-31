require "bundler/setup"
require "sidekiq"
require "sidekiq/api"
require "sidekiq/testing"

Sidekiq.configure_client { |c| c.redis = { url: "redis://localhost:6379/9" } }
Sidekiq.logger.level = Logger::WARN

class HardJob
  include Sidekiq::Job
  sidekiq_options queue: "critical", retry: 3
  RESULTS = []
  def perform(name, count, opts = {})
    RESULTS << [name, count * 2, opts["tag"]]
  end
end

class PlainJob
  include Sidekiq::Job
  def perform(x) = HardJob::RESULTS << [:plain, x]
end

# fake mode: jobs pile up in arrays
Sidekiq::Testing.fake! do
  HardJob.perform_async("bob", 5)
  HardJob.perform_async("ann", 1, { "tag" => "x" })
  HardJob.set(queue: "low").perform_async("low", 2)
  PlainJob.perform_in(60, 7)
  p HardJob.jobs.size, HardJob.jobs.map { |j| [j["args"], j["queue"], j["retry"]] }
  p PlainJob.jobs.first.key?("at"), PlainJob.jobs.first["queue"]
  HardJob.drain
  PlainJob.drain
  p HardJob::RESULTS, HardJob.jobs.size
end

# inline mode: performed on push
HardJob::RESULTS.clear
Sidekiq::Testing.inline! do
  HardJob.perform_async("inline", 21)
  p HardJob::RESULTS
  begin
    HardJob.perform_async("bad", nil)
  rescue NoMethodError => e
    p e.class
  end
end

# real Redis: Client.push, the Queue API, scheduled jobs, Stats
Sidekiq::Testing.disable! do
  Sidekiq.redis(&:flushdb)
  jid = HardJob.perform_async("real", 3)
  p jid.size
  Sidekiq::Client.push("class" => "PlainJob", "args" => [1], "queue" => "default")
  Sidekiq::Client.push_bulk("class" => PlainJob, "args" => [[2], [3], [4]])
  PlainJob.perform_in(3600, 5)
  q = Sidekiq::Queue.new("critical")
  p q.size, q.first.klass, q.first.args, q.first.jid == jid
  dq = Sidekiq::Queue.new("default")
  p dq.size, dq.map(&:args).sort
  p Sidekiq::Queue.all.map(&:name).sort
  ss = Sidekiq::ScheduledSet.new
  p ss.size, ss.first.klass, ss.first.args, (ss.first.at - Time.now).round(-2)
  stats = Sidekiq::Stats.new
  p stats.enqueued, stats.scheduled_size, stats.queues
  dq.find_job(dq.first.jid)&.delete
  p dq.size
  q.clear
  p Sidekiq::Queue.new("critical").size
  Sidekiq.redis(&:flushdb)
end
p Sidekiq::VERSION.split(".").size
