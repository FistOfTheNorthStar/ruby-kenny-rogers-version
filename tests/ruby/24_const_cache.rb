# constant lookups are cached per call site; these must still see changes
X = 1
def read_x; X; end
p read_x
Object.send(:remove_const, :X)
X = 2
p read_x

module Shapes
  def self.kind; Circle; end
end
class Circle; end
p Shapes.kind
module Shapes
  class Circle; end # a nearer definition now shadows ::Circle
end
p Shapes.kind

module Lib; end
module Holder
  include Lib
  def self.get; Thing; end
end
Thing = :top
p Holder.get
module Lib
  Thing = :from_included
end
p Holder.get
3.times { |i| p(i.zero? ? Comparable : Kernel) }
