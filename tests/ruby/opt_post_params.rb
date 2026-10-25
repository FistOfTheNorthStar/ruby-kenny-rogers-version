def ts(name, content = nil, options, escape: true, &block) = [name, content, options, escape]
p ts(:a, {x: 1}), ts(:a, 2, {x: 1}), ts(:a, {x: 1}, escape: false), ts(:a, 2, {x: 1}, escape: false)
def mm(called, *args, escape: true, **options, &block) = ts(called, *args, options, escape: escape)
p mm(:b, x: 1), mm(:b, "c", x: 1), mm(:b), mm(:b, escape: false, y: 2)
def f(a, b = 1, c = 2, d, e) = [a, b, c, d, e]
p f(1, 2, 3), f(1, 2, 3, 4), f(1, 2, 3, 4, 5)
def g(a = 1, b) = [a, b]
p g(5), g(5, 6), g({k: 1}), g(1, k: 2)
def h(a, b = 0, *r, c, k: 1) = [a, b, r, c, k]
p h(1, 2), h(1, 2, 3), h(1, 2, 3, 4, k: 5), h(1, {z: 1})
pr = proc { |a, b = 5, c| [a, b, c] }
p pr.call(1, 2), pr.call(1, 2, 3), pr.call([1, 2])
pr2 = proc { |a, b = 5, *r, c| [a, b, r, c] }
p pr2.call, pr2.call(1), pr2.call(1, 2), pr2.call(1, 2, 3), pr2.call(1, 2, 3, 4, 5), pr2.call([1, 2, 3])
pr3 = proc { |a, b = 1, c = 2, d| [a, b, c, d] }
p pr3.call(1), pr3.call(1, 2), pr3.call(1, 2, 3), pr3.call(1, 2, 3, 4), pr3.call(1, 2, 3, 4, 5)
l = ->(a, b = 7, c) { [a, b, c] }
p l.(1, 2), l.(1, 2, 3), (l.(1) rescue $!.class)
def y = yield(1, 2)
p y { |a, b = 9, c| [a, b, c] }
p [[1, 2], [3, 4, 5]].map { |a, b = :d, c| [a, b, c] }
