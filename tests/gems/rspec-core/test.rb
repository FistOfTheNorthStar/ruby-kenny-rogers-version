require "bundler/setup"
require "rspec/core"
require "stringio"

class EventFormatter
  RSpec::Core::Formatters.register self, :example_group_started, :example_passed,
                                   :example_failed, :example_pending, :dump_summary
  def initialize(output) = @out = output
  def example_group_started(n) = @out.puts("group: #{n.group.description}")
  def example_passed(n) = @out.puts("  pass: #{n.example.full_description}")
  def example_pending(n) = @out.puts("  pending: #{n.example.description} (#{n.example.execution_result.pending_message})")
  def example_failed(n)
    @out.puts("  fail: #{n.example.description} -> #{n.example.exception.class}: #{n.example.exception.message.strip.lines.first.strip}")
  end
  def dump_summary(s) = @out.puts("#{s.example_count} examples, #{s.failure_count} failures, #{s.pending_count} pending")
end

LOG = []
module Check
  def check(a, b) = (a == b) || raise("expected #{b.inspect}, got #{a.inspect}")
end

RSpec.configure do |c|
  c.include Check
  c.before(:suite) { LOG << :before_suite }
  c.around(:each) { |ex| LOG << :around_in; ex.run; LOG << :around_out }
  c.filter_run_excluding :skip_me
end

RSpec.shared_examples "a sized thing" do |expected|
  it("has size #{expected}") { check(subject.size, expected) }
end

RSpec.shared_context "with numbers" do
  let(:numbers) { [1, 2, 3] }
end

RSpec.describe "Hooks and lets", order: :defined do
  include_context "with numbers"
  before(:context) { LOG << :before_all }
  before { LOG << :before_each; @n = 10 }
  after { LOG << :after_each }

  it("sees ivars from before") { check(@n, 10) }
  it("memoizes let") { check(numbers.object_id, numbers.object_id) }
  it("is excluded", :skip_me) { raise "never" }
  it("is skipped") { skip "not now" }
  it("raises") { raise ArgumentError, "boom" }
  it("fails a check") { check(numbers.sum, 7) }

  describe Array do
    subject { [7, 8] }
    it_behaves_like "a sized thing", 2
    it { check(subject.first, 7) }
  end

  describe "metadata", type: :model do
    it("reads metadata") { |ex| check(ex.metadata[:type], :model) }
    it("knows description") { |ex| check(ex.description, "knows description") }
  end
end

out = StringIO.new
status = RSpec::Core::Runner.run(["--format", "EventFormatter"], $stderr, out)
puts out.string
p status
p LOG.uniq
p LOG.count(:around_in)
p RSpec.world.example_count(RSpec.world.ordered_example_groups)
