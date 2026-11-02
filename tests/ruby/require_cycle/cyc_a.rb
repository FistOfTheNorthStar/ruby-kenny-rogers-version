# like i18n.rb: an optional part requires the library that requires this one
module CycA
  def self.optional_part = require("cyc_b")   # not called first
end
CYC_A = true
