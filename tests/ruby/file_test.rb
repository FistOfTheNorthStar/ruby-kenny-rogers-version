# FileTest module functions (Logger's log rotation calls FileTest.identical?)
f = __FILE__
p FileTest.exist?(f), FileTest.file?(f), FileTest.directory?(f), FileTest.directory?(".")
p FileTest.identical?(f, f), FileTest.size?(f).is_a?(Integer), FileTest.zero?(f)
p FileTest.exist?("/no/such/file"), FileTest.size?("/no/such/file")
p FileTest.singleton_methods.sort == File.singleton_methods.select { |m| FileTest.respond_to?(m) }.sort
p FileTest.singleton_methods.size
class UsesFileTest
  include FileTest
  def check(path) = file?(path)
end
p UsesFileTest.new.check(f)
p UsesFileTest.new.respond_to?(:file?)

require "logger"
require "tmpdir"
Dir.mktmpdir do |dir|
  l = Logger.new(File.join(dir, "app.log"), 3, 200)
  40.times { |i| l.info("request #{i} done") }
  l.close
  p Dir.children(dir).sort
end
