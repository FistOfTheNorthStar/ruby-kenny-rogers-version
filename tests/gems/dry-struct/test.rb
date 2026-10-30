require "bundler/setup"
require "dry-struct"

module Types
  include Dry.Types()
end

class Address < Dry::Struct
  transform_keys(&:to_sym)
  attribute :city, Types::String
  attribute? :zip, Types::String.optional
end

class User < Dry::Struct
  transform_keys(&:to_sym)
  attribute :name, Types::Strict::String
  attribute :age, Types::Coercible::Integer
  attribute :role, Types::String.default("member".freeze).enum("member", "admin")
  attribute :email, Types::String.constrained(format: /@/).optional
  attribute :address, Address
  attribute :tags, Types::Array.of(Types::Symbol).default([].freeze)
  attribute :prefs do
    transform_keys(&:to_sym)
    attribute :theme, Types::String.default("dark".freeze)
    attribute :size, Types::Integer
  end
end

u = User.new("name" => "Kenny", "age" => "87", "email" => "k@r.com", "address" => { "city" => "Houston" }, "prefs" => { "size" => 3 })
p u.name, u.age, u.role, u.email, u.address.city, u.address.zip, u.tags, u.prefs.theme, u.prefs.size
p u.to_h
p u.address.class, u.prefs.class.superclass
u2 = u.new(age: 88, tags: [:a])
p u2.age, u2.tags, u.age, u2 == u, u == User.new(u.to_h)
p User.attribute_names, User.schema.keys.map(&:name)
p u.inspect

def try
  yield
rescue Dry::Struct::Error => e
  puts "#{e.class}: #{e.message}"
end
try { User.new(name: 1, age: 1, email: nil, address: { city: "x" }, prefs: { size: 1 }) }
try { User.new(name: "a", age: "x", email: nil, address: { city: "x" }, prefs: { size: 1 }) }
try { User.new(name: "a", age: 1, role: "boss", email: nil, address: { city: "x" }, prefs: { size: 1 }) }
try { User.new(name: "a", age: 1, email: "nope", address: { city: "x" }, prefs: { size: 1 }) }
try { User.new(name: "a") }

class Point < Dry::Struct
  attribute :x, Types::Integer
  attribute :y, Types::Integer
  def +(o) = Point.new(x: x + o.x, y: y + o.y)
end
p (Point.new(x: 1, y: 2) + Point.new(x: 3, y: 4)).to_h
p Point.new(x: 1, y: 2).frozen?, Point.new(x: 1, y: 2).hash == Point.new(x: 1, y: 2).hash

class Base < Dry::Struct
  attribute :id, Types::Integer
end
class Child < Base
  attribute :extra, Types::String
end
p Child.new(id: 1, extra: "e").to_h, Child.attribute_names

class Shape < Dry::Struct
  abstract
end
class Circle < Shape
  attribute :r, Types::Float
end
class Rect < Shape
  attribute :w, Types::Float
  attribute :h, Types::Float
end
ShapeT = Circle | Rect
p ShapeT[{ r: 1.0 }].class, ShapeT[{ w: 1.0, h: 2.0 }].class

class Prod < Dry::Struct::Value
  attribute :sku, Types::String
end
p Prod.new(sku: "A").frozen?, Prod.new(sku: "A").eql?(Prod.new(sku: "A"))
p Types::Coercible::Float["1.5"], Types::Params::Bool["true"], Types::Params::Date["2026-01-02"].to_s
p Types::Strict::Integer.optional[nil], Types::Integer.default(5).call
try { Prod.new({}) }
