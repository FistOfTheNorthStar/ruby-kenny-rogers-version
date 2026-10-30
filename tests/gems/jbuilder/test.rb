require "bundler/setup"
require "jbuilder"
require "active_support/core_ext/string/inflections"

Person = Struct.new(:name, :age, :email, :tags)
people = [Person.new("Ann", 40, "ann@x.io", %w[a b]), Person.new("Bob", 31, nil, [])]

json = Jbuilder.encode do |j|
  j.title "Report"
  j.count people.size
  j.author do
    j.name "Kenny"
    j.url "http://example.com"
  end
  j.people people do |person|
    j.name person.name
    j.age person.age
    j.email person.email if person.email
  end
  j.extract! people.first, :name, :age
  j.set! "dynamic-key", 42
  j.nested { j.deep { j.deeper true } }
  j.list [1, 2, 3]
  j.tags people.first.tags
  j.nothing nil
end
puts json

puts Jbuilder.encode { |j| j.array!(people, :name, :age) }
puts Jbuilder.encode { |j| j.array!([1, 2]) { |x| j.v x * 10 } }
puts Jbuilder.encode { |j| j.key_format! camelize: :lower; j.first_name "A"; j.last_name_here "B" }
puts Jbuilder.encode { |j| j.ignore_nil!; j.a nil; j.b 1 }
puts Jbuilder.encode { |j| j.merge!({ "x" => 1, "y" => [2] }); j.z 3 }
puts Jbuilder.encode { |j| j.child! { j.a 1 }; j.child! { j.b 2 } }
puts Jbuilder.encode { |j| j.comments [], :id }
puts Jbuilder.encode { |j| j.person people.last, :name, :email }
puts Jbuilder.encode { |j| j.deep_format_keys!; j.key_format! :upcase; j.inner({ "k" => 1 }); j.other 2 }

b = Jbuilder.new
b.a 1
b.b { b.c 2 }
p b.attributes!
p b.target!

b2 = Jbuilder.new(key_formatter: Jbuilder::KeyFormatter.new(camelize: :lower))
b2.some_key "v"
p b2.attributes!

puts Jbuilder.encode { |j| j.time_like "2020-01-01"; j.float 1.5; j.big 2**70; j.sym :hello }

begin
  Jbuilder.encode { |j| j.a 1; j.array! [1] }
rescue => e
  p e.class
end
begin
  Jbuilder.encode { |j| j.array! [1]; j.a 1 }
rescue => e
  p e.class
end
