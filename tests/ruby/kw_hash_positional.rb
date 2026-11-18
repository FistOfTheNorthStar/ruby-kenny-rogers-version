# A Hash passed as a positional argument stays positional for a method with `*args, **kw`,
# also in code compiled at run time (eval, load) and in gems: Minitest::Mock's
# `method_missing(sym, *args, **kwargs)` saw `mock.call({name: "e"})` as keywords.
src = <<~R
  class KwPos
    def method_missing(sym, *args, **kwargs, &block) = [sym, args, kwargs]
    def respond_to_missing?(*) = true
    def rest(*args, **kw) = [args, kw]
    def opt(a = 1, **kw) = [a, kw]
    define_method(:dm) { |*args, **kw| [args, kw] }
  end
R
eval(src)
h = { name: "e" }
o = KwPos.new
p o.call(h), o.call(1, h), o.call(h, k: 1), o.call(**h)
p o.rest(h), o.rest(**h), o.opt(h), o.opt(**h), o.dm(h), o.dm(**h)
p eval("o.rest(h)"), eval("o.rest(name: 1)")

# a keyword hash collected into *args still passes on as keywords (delegation)
eval <<~R
  class KwPos
    def fwd(*args, &b) = rest(*args, &b)
    ruby2_keywords def fwd2(*args) = rest(*args)
  end
R
p o.fwd(h), o.fwd2(k: 2), o.fwd2(h)
