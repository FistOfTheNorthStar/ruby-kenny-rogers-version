# Files embedded in the binary (`--embed`, kenny-runtime/src/vfs.rs) behave like real files
# for File, Dir, IO and the libraries on top of them.
# kenny-run-flags: --embed tests/ruby/31_embedded_files
require "yaml"

base = File.join(__dir__, "31_embedded_files")
rel = ->(path) { path.delete_prefix("#{base}/") }
show = "#{base}/views/pages/show.html.erb"

p [File.exist?(base), File.directory?(base), File.file?("#{base}/config/app.yml"), File.exist?("#{base}/nope")]
p File.read("#{base}/config/app.yml")
p YAML.load_file("#{base}/config/app.yml").to_a
p [File.size(show), File.zero?(show), File.readable?(show), File.executable?(show)]
p File.readlines(show, chomp: true)
p File.foreach(show).map(&:size)
p Dir.glob("#{base}/**/*").map(&rel).sort
p Dir["#{base}/views/**/*.erb"].map(&rel).sort
p Dir.glob("*.erb", base: "#{base}/views/pages").sort
p Dir.entries("#{base}/views/pages").sort, Dir.children("#{base}/config")
p [Dir.exist?("#{base}/views"), Dir.empty?("#{base}/views"), File.ftype(base), File.stat(show).file?]
p File.realpath(show) == show, File.expand_path("../config", "#{base}/views") == "#{base}/config"

File.open(show) do |f|
  p f.gets
  p f.pos
  f.seek(4)
  p f.read(5)
  f.seek(-6, IO::SEEK_END)
  p f.read
  f.rewind
  p f.each_line.count
  p [f.stat.size, f.size, f.eof?]
end
File.open(show) do |f|
  # code that needs a real descriptor (Logger does this) still reads the file
  p IO.new(f.fileno, autoclose: false).read
end

# Dir.chdir into an embedded directory, then relative paths
Dir.chdir("#{base}/views") do
  p Dir.pwd == "#{base}/views"
  p Dir.glob("**/*").sort
  p File.read("pages/_item.html.erb"), File.exist?("pages/show.html.erb")
end
