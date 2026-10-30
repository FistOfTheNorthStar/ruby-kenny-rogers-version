# frozen_string_literal: true
# Kenny: Digest is implemented natively by the runtime (crates/kenny-runtime/src/digest.rs);
# this replaces the stdlib digest.rb, which wraps the digest C extension.
require "digest.so"

module Digest
  # Digest::SHA2 (the stdlib's digest/sha2.rb, which CRuby's Digest.const_missing loads on
  # first use; the other digests are native)
  def self.const_missing(name) # :nodoc:
    return __kenny_sha2 if name == :SHA2
    lib = File.join("digest", name.to_s.downcase)
    raise LoadError, "library not found for class Digest::#{name} -- #{lib}", caller(1)
  end

  def self.__kenny_sha2 # :nodoc:
    return const_get(:SHA2, false) if const_defined?(:SHA2, false)
    k = ::Class.new(Digest::Class) do
      def initialize(bitlen = 256)
        case bitlen
        when 256 then @sha2 = Digest::SHA256.new
        when 384 then @sha2 = Digest::SHA384.new
        when 512 then @sha2 = Digest::SHA512.new
        else raise ArgumentError, "unsupported bit length: %s" % bitlen.inspect
        end
        @bitlen = bitlen
      end

      def reset
        @sha2.reset
        self
      end

      def update(str)
        @sha2.update(str)
        self
      end
      alias_method :<<, :update

      def finish = @sha2.digest!
      private :finish

      def block_length = @sha2.block_length
      def digest_length = @sha2.digest_length

      def initialize_copy(other)
        @sha2 = other.instance_eval { @sha2.clone }
      end

      def inspect = "#<%s:%d %s>" % [self.class.name, @bitlen, hexdigest]
    end
    const_set(:SHA2, k)
  end
  private_class_method :__kenny_sha2
end
