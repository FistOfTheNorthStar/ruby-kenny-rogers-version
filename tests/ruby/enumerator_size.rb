p Enumerator.new { |y| y << 1 }.size, Enumerator.new(3) { |y| y << 1 }.size, Enumerator.new(-> { 7 }) { }.size
p [1, 2].each.size, [1, 2, 3].map.with_index.size, [1, 2, 3].each_slice(2).size, [1, 2, 3].each_cons(2).size, [1, 2].cycle.size, [].cycle.size, [1, 2].cycle(3).size
p loop.size, 3.times.size, -1.times.size, 1.upto(4).size, 5.downto(1).size, "héllo".each_char.size, "héllo".each_byte.size, "a\nb".each_line.size
p({ a: 1, b: 2 }.each.size, { a: 1 }.map.size, (1..10).each.size, (1..10).each_slice(3).size, ("a".."e").each.size, (1..).each.size)
o = Object.new
def o.foo(n) = yield n
p o.to_enum(:foo, 3) { |n| n * 2 }.size, o.enum_for(:foo, 1).size
p [3, 1, 2].sort_by.size, [1, 2].each_with_object([]).size, [1, 2, 3].select.size
e = [1, 2, 3].each
p e.next, e.next, e.size
p [1, 2].zip(3..4), [1, 2].zip([3].each), [1, 2, 3].zip(1.step), [1, 2].zip({ a: 1 })
begin; [1].zip(5); rescue => x; p x.class, x.message; end
p 1.step(by: 2, to: 7).to_a, 1.step(10, 4).to_a, 1.0.step(2.0, 0.5).to_a, 1.step.first(3), 10.step(1, -3).to_a, 1.step(nil, 2).first(2)
