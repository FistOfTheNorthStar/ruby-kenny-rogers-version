require "bundler/setup"
require "rspec/expectations"

class Ctx
  include RSpec::Matchers
end
CTX = Ctx.new

def show(&blk)
  CTX.instance_exec(&blk)
  puts "ok"
rescue RSpec::Expectations::ExpectationNotMetError => e
  puts "FAILED: #{e.message}"
end

Point = Struct.new(:x, :y)

show { expect(1 + 1).to eq(2) }
show { expect([1, 2, 3]).to include(2).and include(3) }
show { expect("hello world").to match(/wor/) }
show { expect(5).to be_between(1, 10).inclusive }
show { expect(3.14159).to be_within(0.01).of(3.14) }
show { expect(nil).to be_nil }
show { expect([]).to be_empty }
show { expect(Point.new(1, 2)).to have_attributes(x: 1, y: 2) }
show { expect({ a: { b: [1, 2] } }).to match(a: { b: [Integer, 2] }) }
show { expect([1, "a", :b]).to match_array([:b, 1, "a"]) }
show { expect { raise KeyError, "missing" }.to raise_error(KeyError) { |e| expect(e.message).to eq("missing") } }
show { expect { print "" }.not_to raise_error }
show { expect { $stdout.puts "hi" }.to output("hi\n").to_stdout }
show { expect { |b| [1, 2].each(&b) }.to yield_successive_args(1, 2) }
show { expect(1).to eq(2) }
show { expect("foo\nbar\nbaz").to eq("foo\nqux\nbaz") }
show { expect([1, 2, 3]).to contain_exactly(1, 2, 4) }
show { expect(Point.new(1, 2)).to have_attributes(x: 5) }
show { expect(7).to be > 10 }
show { expect(7).to be_even }
show { expect { 1 }.to raise_error(ArgumentError) }
show { expect([1, 2]).to all(be_odd) }
show { expect(:sym).to be_a(String).or eq("sym") }

RSpec::Matchers.define :be_multiple_of do |n|
  match { |actual| (actual % n).zero? }
  failure_message { |actual| "expected #{actual} to be divisible by #{n}" }
  description { "be a multiple of #{n}" }
end
show { expect(9).to be_multiple_of(3) }
show { expect(10).to be_multiple_of(3) }
p CTX.be_multiple_of(4).description

begin
  CTX.aggregate_failures("two things") do
    CTX.expect(1).to CTX.eq(1)
    CTX.expect(2).to CTX.eq(3)
    CTX.expect("a").to CTX.eq("b")
  end
rescue RSpec::Expectations::MultipleExpectationsNotMetError => e
  puts e.message.gsub(/^\s*# .*\n/, "")
end
