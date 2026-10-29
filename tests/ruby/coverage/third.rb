# Measured in compatible mode and with oneshot_lines (coverage_api.rb).
module CovThird
  def self.go(n)
    r = []
    n.times do |i|
      r << i
    end
    r.size
  end
end
CovThird.go(1)
