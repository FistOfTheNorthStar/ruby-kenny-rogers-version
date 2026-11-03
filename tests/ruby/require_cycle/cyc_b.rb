# like active_support/i18n.rb: `require "i18n"`
r = require "cyc_a"
puts "cyc_b: require cyc_a -> #{r.inspect}, CYC_A #{defined?(CYC_A) ? "defined" : "missing"}"
