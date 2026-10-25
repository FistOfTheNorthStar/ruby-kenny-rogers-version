# Array#hash / #eql? / #uniq on elements whose own hash or eql? runs Ruby code that
# collects or changes the Array (ActiveModel::Error#hash builds a temporary Array).
class Key
  attr_reader :n
  def initialize(n) = @n = n
  def hash
    Array.new(300) { |i| "junk#{i}" } # garbage: a collection may run here
    GC.start if @n % 7 == 0
    @n.hash
  end
  def eql?(o)
    GC.start if @n % 5 == 0
    o.is_a?(Key) && o.n == n
  end
  alias == eql?
end

def parts(n) = [Key.new(n), "s#{n}", [n, n.to_s]]

h1 = (0...200).map { |i| parts(i).hash }
h2 = (0...200).map { |i| parts(i).hash }
puts h1 == h2
puts (0...200).all? { |i| parts(i).eql?(parts(i)) }
puts (0...200).count { |i| parts(i).eql?(parts(i + 1)) }
errs = (0...300).map { |i| Key.new(i % 40) }
puts errs.uniq.size
puts errs.uniq!.size
puts({ parts(3) => :a }[parts(3)].inspect)

# set operations compare with eql?/hash that collect
xs = (0...60).map { |i| Key.new(i % 25) }
ys = (0...60).map { |i| Key.new(i % 10) }
puts (xs & ys).size, (xs | ys).size, (xs - ys).size, xs.intersection(ys).size, xs.union(ys).size, xs.difference(ys).size, xs.intersect?(ys)
puts [[Key.new(1), Key.new(14)], [Key.new(1), Key.new(14)]].uniq.size
