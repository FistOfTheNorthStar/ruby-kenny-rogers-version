# ARGF reads the files named in ARGV in turn
require "tmpdir"
dir = Dir.mktmpdir
a = File.join(dir, "a.txt"); File.write(a, "one\ntwo\n")
b = File.join(dir, "b.txt"); File.write(b, "three\nfour")
p ARGF.class.name, ARGF.to_s, ARGF.object_id == ARGF.object_id
ARGV.replace([a, b])
p ARGF.argv.size
p ARGF.gets, File.basename(ARGF.filename), ARGF.lineno
p ARGF.gets, ARGF.eof?
p ARGF.gets, File.basename(ARGF.filename), ARGF.lineno
p ARGF.read
p ARGF.gets, ARGF.argv
ARGV.replace([a, b])
p ARGF.each_line.map(&:chomp)
ARGV.replace([a, b])
p ARGF.read(6), ARGF.read
ARGV.replace([b])
p ARGF.readlines, (ARGF.readline rescue $!.class)
ARGV.replace([a])
p ARGF.getc, ARGF.readchar, ARGF.each_char.to_a.join
