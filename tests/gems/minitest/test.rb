require "bundler/setup"
require "minitest"
require "stringio"

class Calculator
  def add(a, b) = a + b
  def div(a, b) = a / b
end

class CalculatorTest < Minitest::Test
  def setup = @calc = Calculator.new

  def test_add = assert_equal(5, @calc.add(2, 3))
  def test_div_by_zero = assert_raises(ZeroDivisionError) { @calc.div(1, 0) }
  def test_failure = assert_equal([1, 2, 3], [1, 2, 4], "arrays differ")
  def test_error = @calc.frobnicate
  def test_skip = skip("later")
  def test_misc
    assert_in_delta 0.3, 0.1 + 0.2, 1e-9
    assert_match(/en+y/, "kenny")
    assert_includes %w[a b], "b"
    assert_predicate [], :empty?
    refute_nil 0
    assert_output("hi\n") { puts "hi" }
    assert_kind_of Numeric, 1r
  end
  def test_string_diff = assert_equal("one\ntwo\nthree\n", "one\n2\nthree\n")
end

require "minitest/spec"
describe "a spec" do
  before { @list = [3, 1, 2] }
  it("sorts") { _(@list.sort).must_equal [1, 2, 3] }
  it("fails") { _(@list.size).must_be :>, 5 }
  it("checks errors") { _ { raise ArgumentError }.must_raise ArgumentError }
end

class SortedReporter < Minitest::AbstractReporter
  def initialize = @results = []
  def record(result) = @results << result
  def report
    @results.sort_by { |r| [r.klass, r.name] }.each do |r|
      status = r.passed? ? "pass" : r.skipped? ? "skip" : r.error? ? "error" : "fail"
      $out.puts "#{r.klass}##{r.name}: #{status} (#{r.assertions} assertions)"
      r.failures.each do |f|
        $out.puts f.message.gsub(/^/, "    ")
        $out.puts "    at #{f.location}" unless r.skipped?
      end
    end
  end
end

module Minitest
  def self.plugin_sorted_init(_options) = reporter << SortedReporter.new
end
Minitest.extensions << "sorted"

$out = $stdout
captured = StringIO.new
$stdout = captured
ok = Minitest.run(["--seed", "4242"])
$stdout = $out
p ok
puts captured.string.lines.grep(/runs, .* assertions/)
