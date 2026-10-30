# Fixes from running Jekyll's test suite on Kenny.
require "forwardable"
require "digest"
require "socket"
require "tmpdir"
require "uri"

# Forwardable.debug (an accessor the stdlib has)
Forwardable.debug = false
p Forwardable.debug

# Digest::SHA2 is autoloaded by `require "digest"`
d = Digest::SHA2.new(384)
d << "abc"
p d.hexdigest[0, 16], Digest::SHA2.new.digest_length
begin
  Digest::SHA2.new(100)
rescue ArgumentError => e
  p e.message
end

# Socket.tcp_server_sockets
socks = Socket.tcp_server_sockets("127.0.0.1", 0)
p socks.map(&:class).uniq, socks.first.local_address.ip_address
socks.each(&:close)

# Thread.report_on_exception = false applies to the threads made after it
Thread.report_on_exception = false
t = Thread.new { raise "quiet" }
p t.report_on_exception
begin
  t.join
rescue => e
  p e.message
end
Thread.report_on_exception = true

# abort_on_exception: the exception reaches a main thread waiting on a ConditionVariable
m = Mutex.new
cv = ConditionVariable.new
begin
  m.synchronize do
    t = Thread.new { m.synchronize {}; raise IOError, "aborted" }
    t.report_on_exception = false
    t.abort_on_exception = true
    cv.wait(m)
  end
rescue IOError => e
  p [:main, e.message]
end

# regex literal sources keep their escapes
p %r{a\/b}.source, /a\/b/.source, %r!a\!b!.source, %r{a\}b}.source, /\d\s\./.source
p %r{x\-y#{1}\/}.source

# matching invalid UTF-8 raises
bad = "abc\xff".dup.force_encoding("UTF-8")
[-> { bad =~ /a/ }, -> { /a/.match?(bad) }, -> { bad.gsub(/a/, "b") }, -> { bad.scan(/a/) }].each do |f|
  f.call
rescue ArgumentError => e
  p e.message
end
p "abc\xff".b =~ /c/

Dir.mktmpdir do |dir|
  # bom|utf-8 reads
  path = File.join(dir, "bom.md")
  File.binwrite(path, "\xEF\xBB\xBF---\nx: 1\n---\n")
  p File.read(path, encoding: "bom|utf-8")
  p File.open(path, "r:bom|utf-8", &:gets)
  p File.read(path)[0, 4].bytes.size
  File.binwrite(path, "\xFF\xFE".b + "--\r\n".encode("UTF-16LE").b)
  begin
    File.read(path, encoding: "bom|utf-8")
  rescue ArgumentError => e
    p e.message
  end
  p File.open(path, "rb:bom|utf-8") { |f| s = f.read; [f.external_encoding, s.encoding, s.bytesize] }

  # Dir.glob with FNM_DOTMATCH: "." in the directories the pattern names
  Dir.mkdir(File.join(dir, "s"))
  File.write(File.join(dir, "s", ".h"), "")
  Dir.chdir(dir) do
    p Dir.glob("s/*", File::FNM_DOTMATCH).sort
    p Dir.glob("**/*", File::FNM_DOTMATCH).sort
    p Dir.glob("s/**/*", File::FNM_DOTMATCH).sort
    p Dir.glob("s/*/", File::FNM_DOTMATCH).sort
  end
end

# constants of a module included into Object
module JekyllFixesTop
  JFX_CONST = :found
end
class Object
  include JekyllFixesTop
end
module JekyllFixesDeep
  class Inner
    def self.get = JFX_CONST
  end
end
p JekyllFixesDeep::Inner.get, defined?(JFX_CONST), Object.const_get(:JFX_CONST)

# no extra public methods on Object; Kernel's conversion functions are private
p Object.public_method_defined?(:=~), Object.new.respond_to?(:__kenny_require_relative)
p %i[URI Digest Integer].map { |n| Kernel.private_method_defined?(n) }
p Kernel.respond_to?(:URI), URI("http://x").host
module JfxMf
  def helper = 1
  module_function :helper
end
p JfxMf.helper, JfxMf.private_instance_methods(false)
