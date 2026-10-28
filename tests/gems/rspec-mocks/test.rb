require "bundler/setup"
require "rspec/mocks"

class Mailer
  def self.deliver(to, body) = raise("real delivery to #{to}")
  def greet(name) = "hello #{name}"
end

class Ctx
  include RSpec::Mocks::ExampleMethods
end

def scenario(title, &blk)
  RSpec::Mocks.with_temporary_scope do
    Ctx.new.instance_exec(&blk)
    RSpec::Mocks.verify
  end
  puts "#{title}: ok"
rescue RSpec::Mocks::MockExpectationError => e
  puts "#{title}: #{e.class.name.split("::").last}: #{e.message.gsub(/0x\h+/, "0x")}"
end

scenario("double") do
  db = double("db", fetch: 42)
  allow(db).to receive(:query).with("select 1").and_return([1])
  allow(db).to receive(:query).with(/drop/).and_raise(ArgumentError, "no")
  p db.fetch, db.query("select 1")
  begin
    db.query("drop table")
  rescue ArgumentError => e
    p e.message
  end
end

scenario("partial double on class") do
  allow(Mailer).to receive(:deliver).and_return(:sent)
  p Mailer.deliver("a@b", "hi")
  expect(Mailer).to have_received(:deliver).with("a@b", "hi")
end

scenario("instance_double") do
  m = instance_double(Mailer, greet: "stubbed")
  p m.greet("x")
end

scenario("instance_double rejects unknown") do
  m = instance_double(Mailer)
  allow(m).to receive(:nope)
end

scenario("expected once, got twice") do
  d = double("counter")
  expect(d).to receive(:tick).once
  d.tick
  d.tick
end

scenario("never called") do
  d = double("svc")
  expect(d).to receive(:call).with(1, kw: 2)
end

scenario("wrong args") do
  d = double("svc")
  expect(d).to receive(:call).with(1)
  d.call(2)
end

scenario("ordered and sequences") do
  d = double("seq")
  allow(d).to receive(:next).and_return(1, 2, 3)
  p [d.next, d.next, d.next, d.next]
  expect(d).to receive(:a).ordered
  expect(d).to receive(:b).ordered
  d.a
  d.b
end

scenario("block implementation and yields") do
  d = double("calc")
  allow(d).to receive(:add) { |a, b| a + b }
  allow(d).to receive(:each).and_yield(:x).and_yield(:y)
  p d.add(2, 3)
  got = []
  d.each { |v| got << v }
  p got
end

scenario("any_instance") do
  allow_any_instance_of(Mailer).to receive(:greet).and_return("intercepted")
  p Mailer.new.greet("bob")
end

scenario("spy") do
  s = spy("spy")
  s.foo(1)
  s.bar
  expect(s).to have_received(:foo).with(1)
  expect(s).to have_received(:baz)
end

p Mailer.new.greet("bob")
