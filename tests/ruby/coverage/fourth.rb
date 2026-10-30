# Measured with oneshot_lines (coverage_api.rb).
module CovFourth
  def self.go(n)
    if n > 1
      :many
    else
      :few
    end
  end
end
CovFourth.go(0)
