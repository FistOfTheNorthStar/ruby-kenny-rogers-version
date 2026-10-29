# Loaded while coverage is suspended (coverage_api.rb).
module CovSecond
  def self.run(x)
    y = x * 2
    y + 1
  end
end
