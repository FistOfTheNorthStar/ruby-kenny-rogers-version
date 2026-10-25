# A `hash` that falls back to `super` (ActiveRecord::Core#hash does for unsaved records)
class Rec
  attr_reader :id
  def initialize(id = nil) = @id = id
  def hash = id ? [self.class, id].hash : super
  def ==(o) = o.is_a?(Rec) && (id ? id == o.id : equal?(o))
  alias eql? ==
end
a = Rec.new
b = Rec.new
p a.hash == a.hash, a.hash == b.hash
p Rec.new(1).hash == Rec.new(1).hash
h = { a => 1, Rec.new(1) => 2 }
p h[a], h[b], h[Rec.new(1)]
p [a, a, b, Rec.new(2), Rec.new(2)].uniq.size

class Str2 < String
  def hash = length > 3 ? super : 0
end
p Str2.new("abcd").hash == "abcd".hash, Str2.new("ab").hash
class Ary2 < Array
  def hash = empty? ? 7 : super
end
p Ary2.new([1, 2]).hash == [1, 2].hash, Ary2.new.hash
S = Struct.new(:x) do
  def hash = x.nil? ? 1 : super
end
p S.new(1).hash == S.new(1).hash, S.new(nil).hash
