# Array#flatten expands elements that respond to to_ary
# (ActiveRecord's CollectionProxy#<< flattens a Relation: DiscourseTagging.add_or_create_tags_by_name)
class Rel
  def initialize(*a) = @a = a
  def to_ary = @a
  def respond_to_missing?(n, p = false) = super
end
class Lazy
  def method_missing(n, *a) = n == :to_ary ? [7, 8] : super
  def respond_to_missing?(n, p = false) = n == :to_ary || super
end
class NilAry
  def to_ary = nil
end
p [1, Rel.new(2, [3, Rel.new(4)])].flatten
p [Rel.new].flatten
p [Rel.new(1, 2)].flatten(1)
p [Lazy.new].flatten
p [NilAry.new].flatten.size
a = [Rel.new(5)]
p a.flatten!, a
p [[1, [2]], 3].flatten!
p [1, 2].flatten!
class Bad
  def to_ary = 1
end
p(([Bad.new].flatten rescue [$!.class, $!.message]))
