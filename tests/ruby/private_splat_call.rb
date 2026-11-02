# `obj.foo(*args)` / `obj.foo(**opts)` with an explicit receiver don't reach a private method
# (method_missing, or NoMethodError), as without the splat; `self.foo(*args)` and `foo(*args)`
# do. (Discourse's EvalHelper: `redis.eval @script, *args` went to the private Kernel#eval.)
class A
  def method_missing(m, *a, **kw) = [:mm, m, a, kw]
  def respond_to_missing?(*) = true

  def inside(args)
    [pv(*args), self.pv(*args), pv(**{ k: 1 })]
  end

  private def pv(*a, **kw) = [:pv, a, kw]
end
x = A.new
args = [1, 2]
h = { k: 1 }
p x.pv(1)
p x.pv(*args)
p x.pv(**h)
p x.pv(*args, **h)
p x.pv(*args, &nil)
p x.inside(args)
p x.send(:pv, *args)
p x.eval("1 + 1")
p x.eval("s", *[["k"], ["a"]])

class B
  def prot_ok(o, args) = o.pr(*args)
  protected def pr(*a) = [:pr, a]
end
p B.new.prot_ok(B.new, [3])
begin
  B.new.pr(*[3])
rescue NoMethodError => e
  puts e.message
end
class Plain; end
begin
  Plain.new.eval(*["1"])
rescue NoMethodError => e
  puts e.message
end
p eval(*["1 + 2"])
