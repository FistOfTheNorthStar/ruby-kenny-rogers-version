# fixture for 32_dev_mode.rb: an autoloaded class whose body takes a while
class SlowClass
  sleep 0.2
  def self.ready = :ready
end
