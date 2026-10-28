class Foo
  def bar( x,y )
    if x == nil then return y end
    z = "unused"
    puts "hi #{x}" ; return x+y
  end

  def baz
    [1,2,3].map { |i| i * 2 }.select { |i| i > 2 }.size == 0
  end
end
