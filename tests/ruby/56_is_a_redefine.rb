# is_a? at a site that sees many classes, then a class defines its own is_a?
class A; end; class B; end; class C; end; class D; end; class E; end; class F; end
module M; end
class G; include M; end
def kinds(xs) = xs.map { |x| [x.is_a?(A), x.kind_of?(M), x.is_a?(Object), x.is_a?(Comparable)] }
xs = [A.new, B.new, C.new, D.new, E.new, F.new, G.new, "s", 1, nil, :sym, [1]]
p kinds(xs)
begin
  A.new.is_a?(3)
rescue TypeError => e
  puts "TypeError #{e.message}"
end
class C
  def is_a?(k) = :mine
end
p kinds(xs)
o = Object.new
def o.kind_of?(k) = :single
p o.kind_of?(String), o.is_a?(String)
