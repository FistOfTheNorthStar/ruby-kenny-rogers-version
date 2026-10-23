# `return` returns from the nearest lambda or define_method body, or else the method:
# also from blocks nested in them, and through `ensure`.

l = lambda { [1, 2].each { |v| return v * 7 }; 0 }
p l.call

def mk_l
  lambda { |a| [1, 2].each { |v| return v + a }; 0 }
end
p mk_l.call(10)

def in_m
  l = -> { [5].each { return :inner }; :no }
  [l.call, :after]
end
p in_m

# a proc made inside a lambda returns from the lambda
def proc_in_lambda
  r = -> { pr = proc { return 1 }; pr.call; 2 }.call
  [r, :method_continues]
end
p proc_in_lambda

# two levels of blocks inside a lambda
deep = -> { [1].each { [2].map { |x| return x * 100 } }; :no }
p deep.call

# through ensure
le = -> { begin; return 1; ensure; puts "ensure"; end; 2 }
p le.call
def m_ens
  -> { begin; return 1; ensure; puts "ensure2"; end }.call
  :m
end
p m_ens

# define_method bodies that stay blocks (they capture a local)
class K
  x = 5
  define_method(:foo) { return x + 1 }
  define_method(:bar) { |a| [1, 2].each { |v| return v * a }; 0 }
  define_method(:baz) { |a| begin; return a; ensure; puts "baz ensure"; end }
  y = 3
  define_singleton_method(:sing) { [y].each { |v| return v * 2 }; 0 }
end
p K.new.foo, K.new.bar(3), K.new.baz(:z), K.sing

# blocks in methods still return from the method
def m1
  [1, 2, 3].each { |v| return v * 10 if v == 2 }
  0
end
p m1
def m2
  pr = proc { return :early }
  pr.call
  :late
end
p m2
def m3
  yield
ensure
  puts "ensure ran"
end
def m4
  m3 { return :from_block }
  :no
end
p m4

# a lambda inside a method's block: its own return stays in the lambda
def m5
  [1].each do
    r = -> { return :lam }.call
    return [r, :from_block]
  end
  :no
end
p m5
