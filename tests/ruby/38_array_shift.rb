# Array#shift is O(1) (the buffer's start moves forward); the array keeps working after:
# empty, refilled, grown, inserted into, sorted, collected, used from threads

a = (1..10).to_a
p a.shift, a.shift, a, a.size
a.push(11, 12)
p a, a.first, a.last, a[0], a[-1]
a.unshift(0)
p a
p a.shift(3), a
8.times { a.shift }
p a, a.shift, a.empty?
a << :x
p a, a.shift, a

# a queue: push at the end, shift at the front, many rounds (buffer growth after shifts)
q = []
total = 0
20_000.times do |i|
  q.push(i, i + 1)
  total += q.shift
end
p q.size, total, q.first(3), q.last

# objects stay alive in a shifted buffer across collections
objs = Array.new(1000) { |i| "s#{i}" }
500.times { objs.shift }
GC.start
3.times { 1000.times { Object.new }; GC.start }
p objs.size, objs.first, objs.last, objs.all? { |s| s.start_with?("s") }

# sort / reverse / insert / slice on a shifted array
b = (1..20).map { |i| (i * 7) % 20 + 1 }
5.times { b.shift }
p b.sort, b.reverse.size, b.insert(2, :ins).size, b[1, 3].size
b.sort_by! { |x| x.to_s }
p b.size

# freeing shifted arrays (many short-lived queues)
1000.times { c = (1..50).to_a; 49.times { c.shift }; c.push(1) }
GC.start
p :freed

# threads: shift from several OS threads at once (falls back to copying)
shared = (1..40_000).to_a
m = Mutex.new
ts = 4.times.map { Thread.new { n = 0; loop { x = m.synchronize { shared.shift }; break unless x; n += x }; n } }
p ts.map(&:value).sum, shared
local = (1..1000).to_a
999.times { local.shift }
p local

# String#getbyte / setbyte / bytesize, Integer#succ (fast paths and edge cases)
s = +"hello"
p s.getbyte(0), s.getbyte(-1), s.getbyte(5), s.getbyte(-6), s.getbyte(1.9), s.bytesize
p s.setbyte(0, 72), s.setbyte(-1, 79), s, s.setbyte(1, 256 + 69), s
begin
  s.setbyte(9, 1)
rescue IndexError => e
  p e.message
end
begin
  "frozen".freeze.setbyte(0, 1)
rescue => e
  p e.class
end
begin
  s.getbyte("x")
rescue TypeError => e
  p e.message
end
p "héllo".bytesize, "héllo".getbyte(1)
p 1.succ, (-1).succ, (2**61).succ, (2**62 - 1).succ, (2**64).succ
x = 0
x = x.succ while x < 100_000
p x

# count / sum with a block (inlined for Arrays)
a = [1, 2, 3, 4]
p a.count { |x| x.even? }, a.sum { |x| x * 10 }, [].sum { |x| x }, [].count { true }
p [0.1, 0.2, 0.3].sum { |x| x }, [0.1] * 10 == [0.1] * 10, ([0.1] * 10).sum { _1 }, [1, 2.5].sum { |x| x }
p a.count { |x| break :early if x == 3; true }, a.sum { |x| break false if x == 2; x }, a.count { break false }
p a.count { |x| next false if x == 1; true }, a.sum { |x| next 100 if x == 1; x }
p ["a", "b"].sum { |s| s.size }, [[1, 2], [3, 4]].sum { |x, y| x * y }, [[1, 2], [3, 4]].count { |x, y| y > 2 }
p (1..4).sum { |x| x }, (1..4).count { |x| x > 1 }, { a: 1, b: 2 }.count { |k, v| v > 1 }, { a: 1 }.sum([]) { |k, v| [k] }
class MyAry < Array
  def count = :mine
end
p MyAry.new([1, 2]).count { true }, MyAry.new([1, 2]).sum { |x| x }
b = [1, 2, 3]
p b.count { |x| b << 9 if b.size < 5; x > 1 }, b
p [1, 2].sum { |x| x.to_s } rescue p $!.class
OFFS = [-1, 0, 1]
def nb(x) = OFFS.sum { |iy| OFFS.count { |ix| (x + ix + iy).odd? } }
p (0..5).map { |x| nb(x) }
