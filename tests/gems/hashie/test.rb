require "bundler/setup"
require "hashie"

m = Hashie::Mash.new(name: "Kenny", info: { age: 87, tags: ["country", { genre: "pop" }] })
p m.name, m.info.age, m.info.tags[1].genre, m.name?, m.missing, m.missing?
m.nested!.deep = 1
m.under_.x = 2
p m.to_hash, m.keys
m[:sym] = "s"
p m["sym"], m.fetch("sym"), m.key?(:sym)
m2 = m.deep_merge(info: { age: 88, extra: true })
p m2.info.to_h, m.info.age
p Hashie::Mash.new(a: 1).merge(b: 2).class
p m.info.tags.class, Hashie::Mash.new("a_b" => 3).a_b, m.under_.to_h

class Person < Hashie::Dash
  property :name, required: true
  property :email
  property :role, default: "guest"
end
pr = Person.new(name: "Dolly")
p pr.name, pr.role, pr.email, pr.to_h
begin
  Person.new(email: "x")
rescue ArgumentError => e
  puts "ArgumentError: #{e.message}"
end
begin
  Person.new(name: "a", nope: 1)
rescue NoMethodError => e
  puts "NoMethodError: #{e.message}"
end

class Trashy < Hashie::Trash
  property :first_name, from: :firstName
  property :id, transform_with: ->(v) { v.to_i }
end
t = Trashy.new(firstName: "Willie", id: "42")
p t.first_name, t.id

class Coerced < Hashie::Dash
  include Hashie::Extensions::Dash::Coercion
  property :count, coerce: Integer
  property :price, coerce: Float
  property :tags, coerce: Array[String]
end
c = Coerced.new(count: "7", price: "1.5", tags: [:a, 1])
p c.count, c.price, c.tags

h = { "a" => { "b" => { "c" => 1 } }, "list" => [{ "x" => 1 }] }
h.extend(Hashie::Extensions::DeepFind)
p h.deep_find("c"), h.deep_find_all("x")
h2 = { "a" => 1, "b" => { "c" => 2 } }.extend(Hashie::Extensions::SymbolizeKeys)
p Hashie.symbolize_keys(h2), Hashie.stringify_keys(a: { b: 1 })
ia = Hashie::Mash.new.extend(Hashie::Extensions::DeepFetch)
ia.merge!(a: { b: [{ c: 5 }] })
p ia.deep_fetch(:a, :b, 0, :c) rescue p $!.class
class IAHash < Hash
  include Hashie::Extensions::MergeInitializer
  include Hashie::Extensions::IndifferentAccess
end
iah = IAHash.new(foo: 1)
p iah[:foo], iah["foo"]
class MyHash < Hash
  include Hashie::Extensions::MethodAccess
end
mh = MyHash.new; mh["abc"] = 1; mh.xyz = 2
p mh.abc, mh["xyz"]
p Hashie::Clash.new.where(a: 1).order(:b).limit(3)
r = Hashie::Rash.new(/hel/ => 1, 1..5 => :range, "s" => 3)
p r["hello"], r[3], r["s"]
