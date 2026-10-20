# Kenny's stand-ins for libraries a compiled binary handles itself: Bundler (gems resolved
# at build time), socket, FFI declarations, YAML.load following the stdlib's psych.
require "bundler/setup"
p defined?(Bundler::Kenny), Bundler.respond_to?(:require)
Bundler.require(:default)

require "socket"
require "ipaddr"
p IPAddr.new("10.1.2.3").private?, IPAddr.new("::1").ipv6?
p Socket::AF_INET.is_a?(Integer), TCPServer.ancestors.include?(IO)
srv = TCPServer.new("127.0.0.1", 0)
p srv.addr[0], srv.addr[1] > 0
srv.close

require "ffi"
module Lib
  extend FFI::Library
  ffi_lib "nosuchlib"
  typedef :pointer, :handle
  Color = enum(:red, :green, 5, :blue)
  attach_function :do_it, [:handle, :int], :int
  class Pair < FFI::Struct
    layout :a, :int, :b, :double
  end
end
p Lib::Color[:blue], Lib::Color[1], Lib::Pair.size, Lib::Pair.offset_of(:b)
begin
  Lib.do_it(nil, 1)
rescue NotImplementedError => e
  p e.message[/\Ado_it/]
end

require "yaml"
db = "default: &default\n  adapter: sqlite3\n  pool: 5\nproduction:\n  <<: *default\n  database: db/p.sqlite3\n"
# psych 3 (Ruby <= 3.0's stdlib): YAML.load accepts aliases; psych 4+: it doesn't
loaded = Psych::VERSION.to_i < 4 ? YAML.load(db) : (YAML.load(db) rescue :aliases_refused)
p loaded == :aliases_refused || loaded["production"]["pool"] == 5
p YAML.safe_load(db, aliases: true)["production"]
