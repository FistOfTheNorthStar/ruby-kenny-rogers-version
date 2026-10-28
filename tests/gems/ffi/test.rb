require "bundler/setup"
require "ffi"

module LibC
  extend FFI::Library
  ffi_lib FFI::Library::LIBC
  attach_function :strlen, [:string], :size_t
  attach_function :getpid, [], :int
  attach_function :abs, [:int], :int
end

class Pt < FFI::Struct
  layout :x, :int, :y, :double
end
p Pt.size, Pt.members, Pt.offset_of(:y)
p FFI::Type::INT32.size, FFI.type_size(:pointer), FFI::Platform::ADDRESS_SIZE
p LibC.respond_to?(:strlen), LibC.method(:strlen).arity rescue p $!.class
begin
  ffi_lib_missing = Module.new { extend FFI::Library; ffi_lib "libdoesnotexist_kenny" }
rescue LoadError => e
  puts "LoadError for missing lib"
end
p LibC.strlen("hello")
p LibC.abs(-5)
p LibC.getpid == Process.pid
mp = FFI::MemoryPointer.new(:int, 4)
mp.write_array_of_int([1, 2, 3, 4])
p mp.read_array_of_int(4)
s = Pt.new
s[:x] = 3
s[:y] = 1.5
p [s[:x], s[:y]]
