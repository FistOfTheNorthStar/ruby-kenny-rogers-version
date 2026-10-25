require "stringio"
io = StringIO.new(+"line1\nline2\nline3")
p io.gets, io.lineno, io.read(3), io.getc, io.ungetc("Z"), io.read, io.eof?, io.pos
io.rewind; p io.each_line.to_a, io.string.size
w = StringIO.new
w << "a"; w.print("b", 1); w.puts [2, 3]; w.printf("%03d", 7); w.write("x", "y"); p w.string, w.pos, w.tell
p StringIO.new("abc").readlines, StringIO.new("a,b\nc").gets(","), StringIO.new("abc").readchar, StringIO.new("héllo").each_char.to_a
p StringIO.new("x").internal_encoding, StringIO.new("x").external_encoding, StringIO.new("abc").set_encoding("BINARY").read.encoding
s = StringIO.new(+"hello"); s.seek(2); s.write("XY"); p s.string; s.truncate(3); p s.string
p StringIO.new("abc").respond_to?(:closed?), StringIO.new("abc").tap(&:close).closed?, StringIO.new("q\nw").each_line(chomp: true).to_a
r = StringIO.new("abc\r\ndef"); p r.gets(chomp: true), r.readline, (r.readline rescue $!.class)
p StringIO.new("abc").read(10), StringIO.new("").read(1), StringIO.new("").read, StringIO.new("ab").each_byte.to_a, StringIO.new("ab").size, StringIO.new("ab").length
p StringIO.new("abc", "r").tap { _1.read(1) }.pread(2, 0) rescue p $!.class
x = StringIO.new; x.binmode; p x.string.encoding, x.fileno, x.isatty, x.sync, x.fsync rescue p $!
require "csv"
p CSV.generate { |o| o << ["a", "b,c"]; o << [1, nil] }, CSV.parse("a,b\n1,2", headers: true).map(&:to_h), CSV.parse_line("x,\"y,z\""), ["q", 2].to_csv, "a;b".parse_csv(col_sep: ";")
