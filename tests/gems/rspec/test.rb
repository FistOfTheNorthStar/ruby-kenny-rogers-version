require "bundler/setup"
require "rspec"
require "stringio"

class Stack
  def initialize; @items = []; end
  def push(x); @items.push(x); self; end
  def pop; raise IndexError, "empty stack" if @items.empty?; @items.pop; end
  def size; @items.size; end
  def empty?; @items.empty?; end
end

RSpec.describe Stack do
  subject(:stack) { described_class.new }
  let(:filled) { stack.push(1).push(2) }

  it { is_expected.to be_empty }

  it "pushes and pops" do
    expect(filled.size).to eq(2)
    expect(filled.pop).to eq(2)
    expect(filled).to have_attributes(size: 1)
  end

  it "raises on empty pop" do
    expect { stack.pop }.to raise_error(IndexError, /empty/)
  end

  context "with a double" do
    it "talks to a logger" do
      logger = double("logger")
      expect(logger).to receive(:info).with("pushed 3").once
      allow(logger).to receive(:debug).and_return(:ignored)
      stack.push(3)
      logger.info("pushed #{stack.size + 2}")
      expect(logger.debug).to eq(:ignored)
    end
  end

  it "fails with a helpful message" do
    expect(filled.size).to eq(3)
  end

  it "compares hashes", :focus_me do
    expect({ a: 1, b: [1, 2] }).to eq({ a: 1, b: [1, 3] })
  end

  pending "is not done yet" do
    expect(1).to eq(2)
  end

  it "matches collections" do
    expect([3, 1, 2]).to contain_exactly(1, 2, 3)
    expect([1, 2, 3]).to all(be_a(Integer))
    expect("kenny rogers").to start_with("kenny").and end_with("rogers")
    expect { stack.push(9) }.to change(stack, :size).by(1)
  end
end

out = StringIO.new
err = StringIO.new
status = RSpec::Core::Runner.run(["--format", "documentation", "--no-color"], err, out)
puts out.string.gsub(/Finished in [\d.]+ seconds \(files took [\d.]+ seconds? to load\)/, "Finished in X seconds")
puts err.string
p status
