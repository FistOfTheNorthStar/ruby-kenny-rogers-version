r = require "cyc_c"   # a real cycle: cyc_c is still loading
puts "cyc_d: require cyc_c -> #{r.inspect}"
