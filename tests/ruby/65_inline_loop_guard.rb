# inline block loops: guards and Arrays changed while iterated

def all_iters(a)
  r = []
  a.each { |x| r << x }
  a.each_with_index { |x, i| r << [x, i] }
  a.each_index { |i| r << i }
  r << a.map { |x| x * 2 }
  r << a.collect { |x| x + 1 }
  r << a.select { |x| x.odd? }
  r << a.filter { |x| x.even? }
  r << a.reject { |x| x > 1 }
  r << a.any? { |x| x > 2 }
  r << a.all? { |x| x > 0 }
  r << a.count { |x| x > 1 }
  r << a.sum { |x| x * 10 }
  r
end
p all_iters([1, 2, 3])
p all_iters([])
p all_iters([1, 2, 3].freeze)

class MyArr < Array; end
p all_iters(MyArr[3, 4])

# growing and shrinking while iterated
a = [1, 2, 3]
seen = []
a.each { |x| seen << x; a << x + 10 if x < 3 }
p seen, a
b = [1, 2, 3, 4, 5]
seen = []
b.each { |x| seen << x; b.pop }
p seen, b
c = [1, 2, 3]
p c.map { |x| c.clear if x == 1; x }

# Integer loops
def int_loops(n)
  r = []
  n.times { |i| r << i }
  1.upto(n) { |i| r << i }
  n.downto(1) { |i| r << i }
  r
end
p int_loops(3), int_loops(0)
big = 2**64
k = 0
big.times { |i| k += 1; break if i == 2 }
p k

# a class with its own each
class Each3
  def each = [7, 8, 9].each { |x| yield x }
end
r = []
Each3.new.each { |x| r << x }
p r
r = []
(1..3).each { |x| r << x }
p r

# redefinitions
class Array
  alias_method :orig_each, :each
  def each
    orig_each { |x| yield x * 100 }
  end
end
p all_iters([1, 2])
class Array
  alias_method :each, :orig_each
end
p all_iters([1, 2])
class Integer
  alias_method :orig_times, :times
  def times = orig_times { |i| yield(-i) }
end
p int_loops(2)
class Integer
  alias_method :times, :orig_times
end
p int_loops(2)
class Array
  private :map
end
begin
  [1].map { |x| x }
rescue NoMethodError => e
  p e.message
end
class Array
  public :map
end
p [1].map { |x| x + 1 }
