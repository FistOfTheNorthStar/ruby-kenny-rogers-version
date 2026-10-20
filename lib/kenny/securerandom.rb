# frozen_string_literal: true

# Kenny's SecureRandom: bytes come from the OS (Random.urandom); hex, base64,
# uuid, alphanumeric, random_number, ... come from the native Random::Formatter.
require "random/formatter"

module SecureRandom
  VERSION = "0.4.1"

  def self.bytes(n)
    Random.urandom(n)
  end

  def self.gen_random(n)
    Random.urandom(n)
  end
end

SecureRandom.extend(Random::Formatter)
