# A small file for coverage_api.rb (required in several states of Coverage).
module CovCounter
  def self.tick(n)
    if n.even?
      :even
    else
      :odd
    end
  end

  def self.loop_to(n)
    s = 0
    n.times { |i| s += i }
    s
  end
end
CovCounter.tick(1)
