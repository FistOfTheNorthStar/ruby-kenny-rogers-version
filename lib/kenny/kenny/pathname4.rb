# Ruby 4.0's Pathname, a core class there: a Kenny build against Ruby 4.0 loads this on first
# use of `Pathname` (as 4.0 has it without a require). Ruby 4.0 writes Pathname in Ruby
# (pathname_builtin.rb, kept here as kenny/pathname_builtin.rb) except for `<=>` and `sub`
# (pathname.c), which are these. `require "pathname"` then adds the standard library's
# pathname.rb (Pathname#find, #rmtree ...) on top.
class Pathname
  def <=>(other)
    return nil unless other.is_a?(Pathname)
    @path.tr("/", "\0") <=> other.to_s.tr("/", "\0")
  end

  def sub(*args, &block)
    self.class.new(block ? @path.sub(*args, &block) : @path.sub(*args))
  end
end

require "kenny/pathname_builtin"
