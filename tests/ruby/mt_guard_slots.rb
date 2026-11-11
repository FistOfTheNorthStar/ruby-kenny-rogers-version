# Guard slots of inlined calls, inlined `C.new` and virtual objects filled while several
# threads run Ruby code: the fast paths must still see later method changes.
class Pt
  def initialize(x)
    @x = x
  end
  def x = @x
  def twice = @x * 2
end
class Pt3 < Pt
end

def round(objs, n)
  s = 0
  i = 0
  while i < n
    objs.each { |o| s += o.twice }
    s += Pt.new(i).x
    "garbage #{i}" if i % 7 == 0
    i += 1
  end
  s
end

go = 3.times.map { Queue.new }
back = Queue.new
ts = 4.times.map do
  Thread.new do
    objs = [Pt.new(1), Pt3.new(2)]
    r = []
    3.times do |phase|
      go[phase].pop
      r << round(objs, 20_000)
      back << phase
    end
    r
  end
end

# first reached under load
4.times { go[0] << 1 }
4.times { back.pop }
# an unrelated method: every slot is now of an older serial
Pt.define_method(:other) { 0 }
4.times { go[1] << 1 }
4.times { back.pop }
# the inlined methods themselves change
Pt.define_method(:twice) { @x * 3 }
Pt.define_method(:x) { -@x }
4.times { go[2] << 1 }
4.times { back.pop }
p ts.map(&:value).uniq
p round([Pt.new(1)], 10)
