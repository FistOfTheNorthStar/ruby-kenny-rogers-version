# frozen_string_literal: true
# Kenny: BigDecimal is implemented natively (crates/kenny-runtime/src/bigdecimal.rs).
require "bigdecimal.so"

class BigDecimal
  # Defined in Ruby so Hash keys use value equality (Kenny's Hash only calls
  # Ruby-level #hash / #eql? overrides).
  def hash
    __kn_hash
  end

  def eql?(other)
    __kn_eql(other)
  end
end
