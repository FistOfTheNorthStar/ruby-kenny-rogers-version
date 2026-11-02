# kenny-run-flags: -I tests/ruby/require_cycle
# A `require` reached at build time while its file is still being compiled (cyc_a is compiled
# first, from the method below; cyc_b, reached from cyc_a, requires cyc_a back) runs the file
# when the cycle is entered elsewhere at runtime.
def self.unused = require("cyc_a")
require "cyc_b"
p CycA.optional_part
# a cycle at runtime too: the inner require is false, as in CRuby
p require("cyc_c")
