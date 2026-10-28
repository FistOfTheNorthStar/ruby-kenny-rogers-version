# frozen_string_literal: true
# Kenny: BigDecimal is implemented natively (crates/kenny-runtime/src/bigdecimal.rs).
require "bigdecimal.so"

# The bundled bigdecimal's version decides the arithmetic: 4.x changed `/`'s precision,
# divmod's quotient (an Integer) and sqrt's rounding
if defined?(Gem) && (spec = Gem.loaded_specs["bigdecimal"])
  BigDecimal.send(:remove_const, :VERSION)
  BigDecimal::VERSION = spec.version.to_s.freeze
  BigDecimal.__kenny_v4! if spec.version.to_s.to_i >= 4
end

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
