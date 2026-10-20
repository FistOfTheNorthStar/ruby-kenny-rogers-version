# nio4r for Kenny binaries: the gem's own pure-Ruby selector (NIO::Selector on IO.select),
# since its C extension (libev) isn't built into the binary. This is what nio4r does itself
# with NIO4R_PURE=true. Action Cable's stream event loop runs on it.
require "socket"
require "nio/version"

module NIO
  def self.engine
    ENGINE
  end

  def self.pure?(_env = ENV)
    true
  end
end

require "nio/monitor"
require "nio/selector"
require "nio/bytebuffer"
NIO::ENGINE = "ruby"
