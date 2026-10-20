# File / IO / Dir / Process / subprocess support
require "fileutils"
require "tmpdir"
require "tempfile"
require "pathname"

BASE = File.join(Dir.tmpdir, "kenny_io_test_#{$$}")
def clean(s)
  s.to_s.gsub(BASE, "BASE")
end
FileUtils.rm_rf(BASE)
Dir.mkdir(BASE)
puts Dir.exist?(BASE)

# --- writing and reading ---
path = File.join(BASE, "a.txt")
p File.write(path, "hello\nworld\n")
p File.read(path)
File.open(path, "a") { |f| f.puts "more"; f.print "x", "y"; f << "z" << "\n"; f.printf("%03d\n", 7) }
p File.read(path)
p File.readlines(path)
p File.readlines(path, chomp: true)
File.foreach(path) { |l| print "> ", l }
p File.foreach(path).map(&:chomp).last
p IO.readlines(path, chomp: true).size
File.open(path) do |f|
  p f.gets, f.lineno
  p f.read(3)
  p f.readline
  p f.eof?
  p f.read
  p f.eof?, f.read(1), f.gets, f.read
  f.rewind
  p f.each_line.to_a.size
  f.pos = 2
  p f.getc, f.readchar, f.getbyte, f.pos
  f.ungetc("Q")
  p f.read(3)
  f.seek(-4, IO::SEEK_END)
  p f.read
end
p File.open(path, "r") { |f| f.readlines(chomp: true).first(2) }
p File.open(path, &:readlines).size
p File.read(path, 5), File.read(path, 5, 6)
f = File.new(path)
p f.closed?, clean(f.path), clean(f.inspect)
f.close
p f.closed?
begin
  f.read
rescue IOError => e
  p e.message
end

# buffering, sync and r+ mode
f = File.open(path, "w")
f.sync = true
f.write "abc"
p File.read(path)
f.sync = false
f.write "def"
p File.read(path).size <= 6
f.flush
p File.read(path)
f.close
File.open(path, "r+") do |g|
  p g.read(2)
  g.write("X")
  g.seek(0)
  p g.read
  g.truncate(4)
end
p File.read(path)
File.write(path, "12", 1)
p File.read(path)
File.write(path, "héllo\nwörld")
File.open(path) do |g|
  p g.each_char.to_a.size
  g.rewind
  p g.each_byte.first(3)
  g.rewind
  p g.gets("l"), g.gets(nil)
end
File.write(path, "para1\nline\n\n\n\npara2\n")
p File.open(path) { |g| g.readlines("") }
p File.open(path) { |g| g.gets(4) }

# --- stat and predicates ---
p File.size(path), File.exist?(path), File.file?(path), File.directory?(BASE), File.zero?(path)
st = File.stat(path)
p st.size, st.file?, st.directory?, st.mtime.class, st.ftype
t = Time.at(1_000_000_000)
File.utime(t, t, path)
p File.mtime(path).to_i, File.stat(path).mtime == t
File.chmod(0600, path)
p File.stat(path).mode.to_s(8), File.executable?(path), File.readable?(path), File.world_readable?(path)
File.symlink(path, File.join(BASE, "link"))
p File.symlink?(File.join(BASE, "link")), clean(File.readlink(File.join(BASE, "link")))
p File.identical?(path, File.join(BASE, "link")), File.lstat(File.join(BASE, "link")).symlink?

# --- paths ---
p File.basename("/a/b/c.rb"), File.basename("/a/b/c.rb", ".rb"), File.basename("a/b.tar.gz", ".*")
p File.dirname("/a/b/c.rb"), File.dirname("c.rb"), File.dirname("/a/b/c.rb", 2)
p File.extname("x/y.rb"), File.extname(".bashrc"), File.extname("foo.")
p File.split("/a/b/c.rb"), File.join("a", "b", ["c", "d"]), File.join("a/", "/b")
p File.expand_path("a/../b", "/x/y"), File.absolute_path("x", "/r"), File.absolute_path?("/x")
p File.expand_path("~") == Dir.home
p File.fnmatch("*.rb", "a.rb"), File.fnmatch("*", ".x"), File.fnmatch("*", ".x", File::FNM_DOTMATCH)
p File.fnmatch("**/*.rb", "a/b/c.rb", File::FNM_PATHNAME), File.fnmatch("*.rb", "a/b.rb", File::FNM_PATHNAME)
p File.fnmatch("{a,b}.rb", "b.rb", File::FNM_EXTGLOB), File.fnmatch("[a-c]?", "bz"), File.fnmatch("[!a]", "a")

# --- errors ---
def show
  yield
rescue SystemCallError => e
  puts "#{e.class}: #{clean(e.message)} (errno=#{e.errno})"
end
show { Dir.mkdir(BASE) }
show { Dir.rmdir(BASE) }
show { File.read(BASE) }
show { File.open(File.join(path, "x"), "w") }
show { File.open(File.join(BASE, "nope")) }
show { File.stat(File.join(BASE, "nope")) }
show { File.rename(File.join(BASE, "nope"), File.join(BASE, "x")) }
show { File.unlink(File.join(BASE, "nope")) }
show { File.open(path, File::WRONLY | File::CREAT | File::EXCL) }
show { raise Errno::EACCES, "custom" }
p Errno::ENOENT.new("x").message, Errno::ENOENT.ancestors.include?(SystemCallError), SystemCallError.superclass

# --- directories and glob ---
FileUtils.mkdir_p(File.join(BASE, "sub/deep"))
File.write(File.join(BASE, "sub", "x.rb"), "1")
File.write(File.join(BASE, "sub", "deep", "y.rb"), "2")
File.write(File.join(BASE, "z.rb"), "3")
File.write(File.join(BASE, ".hidden.rb"), "4")
Dir.chdir(BASE) do
  p Dir.pwd == File.realpath(BASE)
  p Dir.glob("**/*.rb")
  p Dir["*"]
  p Dir.glob("**/*")
  p Dir.glob("sub/*")
  p Dir.glob("{sub,nope}/**/*.rb")
  p Dir.glob("*.{rb,txt}")
  p Dir.glob("*/")
  p Dir.glob("**/")
  p Dir.glob("*", File::FNM_DOTMATCH)
  p Dir.glob("s?b/[xy].rb")
  p Dir.glob(["*.rb", "sub/*.rb"])
end
p Dir.glob("**/*.rb", base: BASE)
p Dir.children(BASE).sort, Dir.entries(BASE).sort.first(2)
p Dir.each_child(BASE).to_a.size, Dir.empty?(BASE), Dir.exist?(path)
Dir.open(BASE) { |d| p d.children.sort.first, clean(d.path) }

# --- FileUtils / Pathname / Tempfile / mktmpdir ---
FileUtils.cp(File.join(BASE, "z.rb"), File.join(BASE, "copy.rb"))
FileUtils.mv(File.join(BASE, "copy.rb"), File.join(BASE, "moved.rb"))
FileUtils.touch(File.join(BASE, "t.txt"))
FileUtils.cp_r(File.join(BASE, "sub"), File.join(BASE, "sub2"))
p File.read(File.join(BASE, "moved.rb")), File.zero?(File.join(BASE, "t.txt"))
p Dir.glob("sub2/**/*", base: BASE)
FileUtils.rm_rf(File.join(BASE, "sub2"))
p File.exist?(File.join(BASE, "sub2"))
pn = Pathname.new(BASE) + "sub" + "x.rb"
p pn.read, pn.basename.to_s, pn.extname, pn.file?, pn.dirname.basename.to_s
p Pathname("a/b/../c").cleanpath.to_s, Pathname("/a/b").relative_path_from(Pathname("/a/c")).to_s
p Pathname(BASE).glob("**/*.rb").map { |x| x.relative_path_from(Pathname(BASE)).to_s }
tf = Tempfile.new("kn")
tf.write("temp data")
tf.flush
p File.read(tf.path)
tf.rewind
p tf.read
tpath = tf.path
tf.close!
p File.exist?(tpath)
Tempfile.create("kn2") { |g| g.puts "x"; g.rewind; p g.gets; $tp = g.path }
p File.exist?($tp)
Dir.mktmpdir("kn") { |d| p File.directory?(d), File.basename(d).start_with?("kn"); $td = d }
p File.exist?($td)

# --- subprocesses ---
puts "before system"
system("echo from system")
p $?.success?, $?.exitstatus, $?.class
p system("exit 3"), $?.exitstatus, $?.success?
p system("nonexistent_command_xyz_kenny")
p system("echo", "array", "form")
p `echo hi there`
p `printf 'a\\nb\\n'`.lines
p %x(echo #{1 + 2}).to_i
begin
  `nonexistent_command_xyz_kenny`
rescue Errno::ENOENT => e
  p e.message
end
begin
  system("exit 2", exception: true)
rescue => e
  p e.class, e.message
end
IO.popen("echo popen") { |io| p io.read }
IO.popen(["echo", "arr", "args"]) { |io| p io.gets }
p IO.popen("cat", "r+") { |io| io.write "abc"; io.close_write; io.read }
p IO.popen(["sh", "-c", "echo out; echo err 1>&2"], err: [:child, :out], &:read)
p IO.popen("exit 5", &:read), $?.exitstatus
p open("|echo kernel open").read
open(File.join(BASE, "k"), "w") { |g| g << "kernel open file" }
p open(File.join(BASE, "k"), &:read)
r, w = IO.pipe
w.puts "through pipe"
w.close
p r.read
r.close
IO.pipe do |r2, w2|
  pid = spawn("echo child_out", out: w2)
  w2.close
  Process.wait(pid)
  p r2.read, $?.exitstatus
end
pid = fork { puts "in child"; $stdout.flush; exit!(7) }
Process.wait(pid)
p $?.exitstatus, $?.pid == pid
pid = fork { exit 3 }
_, status = Process.wait2(pid)
p status.exitstatus, status.inspect.sub(/pid \d+/, "pid N")
pid = spawn("sh", "-c", "exit 4")
Process.waitpid(pid)
p $?.exitstatus
pid = spawn("sleep 0.05")
p Process.detach(pid).value.success?
system("kill -TERM $$")
p $?.signaled?, $?.termsig, $?.exitstatus
p Process.pid.is_a?(Integer), Process.ppid.is_a?(Integer), $$ == Process.pid

# threads keep running while a pipe read blocks
r, w = IO.pipe
t = Thread.new { r.read }
sleep 0.02
w.write "from main"
w.close
p t.value
p IO.select([r], nil, nil, 0).nil?

# $stdout redirection
require "stringio"
old = $stdout
$stdout = StringIO.new
puts "captured"
print "x"
p 1
cap = $stdout.string
$stdout = old
p cap
$stdout.puts "via $stdout"
STDOUT.write("via STDOUT", "\n")
STDOUT.flush

FileUtils.rm_rf(BASE)
p Dir.exist?(BASE)
