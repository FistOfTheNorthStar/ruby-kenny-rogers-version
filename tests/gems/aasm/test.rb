require "bundler/setup"
require "aasm"

class Job
  include AASM
  attr_reader :log
  def initialize = @log = []

  aasm do
    state :sleeping, initial: true
    state :running, before_enter: -> { log << "before_enter running" }
    state :cleaning
    state :done, final: true

    after_all_transitions :log_status

    event :run, after: :notify do
      transitions from: :sleeping, to: :running, guard: :ready?
    end
    event :clean do
      transitions from: :running, to: :cleaning
    end
    event :sleep do
      transitions from: [:running, :cleaning], to: :sleeping
    end
    event :finish do
      transitions from: [:running, :cleaning], to: :done, after: ->(*args) { log << "finished #{args.inspect}" }
    end
  end

  attr_accessor :ready
  def ready? = ready
  def notify = log << "notified"
  def log_status = log << "#{aasm.from_state} -> #{aasm.to_state} (#{aasm.current_event})"
end

j = Job.new
p j.aasm.current_state, j.sleeping?, j.may_run?
begin
  j.run
rescue AASM::InvalidTransition => e
  puts "InvalidTransition: #{e.message}"
  p e.event_name, e.originating_state, e.failures
end
j.ready = true
p j.may_run?, j.run, j.running?, j.aasm.current_state
p j.aasm.events.map(&:name), j.aasm.events(permitted: true).map(&:name)
p j.aasm.states(permitted: true).map(&:name)
p j.clean!, j.cleaning?
p j.finish(:fast, 1), j.done?, j.aasm.current_state
p j.log
p Job.aasm.states.map(&:name), Job.aasm.initial_state, Job.aasm.events.map(&:name)
p Job::STATE_SLEEPING, Job.aasm.state_machine.config.whiny_transitions
p Job.aasm.states.map { |s| [s.name, s.display_name] }
j2 = Job.new
p j2.run, j2.aasm.current_state rescue puts "#{$!.class}"

class Light
  include AASM
  aasm column: :color, whiny_transitions: false do
    state :red, initial: true
    state :green, :yellow
    event(:go) { transitions from: :red, to: :green }
    event(:slow) { transitions from: :green, to: :yellow }
    event(:stop) { transitions from: :yellow, to: :red }
  end
end
l = Light.new
p l.slow, l.aasm.current_state, l.go, l.slow, l.stop, l.red?
l.aasm.current_state = :yellow
p l.yellow?, l.may_stop?

class Doc
  include AASM
  aasm(:status) do
    state :draft, initial: true
    state :published
    event(:publish) { transitions to: :published }
  end
  aasm(:review, namespace: :review) do
    state :pending, initial: true
    state :approved
    event(:approve) { transitions from: :pending, to: :approved }
  end
end
d = Doc.new
p d.aasm(:status).current_state, d.aasm(:review).current_state
d.publish; d.approve_review
p d.published?, d.review_approved?, Doc.aasm(:review).states.map(&:name)
