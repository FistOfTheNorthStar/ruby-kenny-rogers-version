require "bundler/setup"
require "zip"
require "tmpdir"

Dir.mktmpdir do |dir|
  path = File.join(dir, "test.zip")
  Zip::File.open(path, create: true) do |zf|
    zf.get_output_stream("hello.txt") { |f| f.write("Hello, zip!\n" * 3) }
    zf.mkdir("sub")
    zf.get_output_stream("sub/data.bin") { |f| f.write((0..255).map(&:chr).join.b) }
    File.write(File.join(dir, "local.txt"), "from disk")
    zf.add("local.txt", File.join(dir, "local.txt"))
  end
  p File.binread(path, 4).bytes
  Zip::File.open(path) do |zf|
    p zf.entries.map(&:name).sort
    p zf.size
    e = zf.find_entry("hello.txt")
    p e.size, e.compressed_size < e.size, e.compression_method == Zip::Entry::DEFLATED, e.file?, e.crc
    p zf.read("hello.txt")
    p zf.read("sub/data.bin").bytesize, zf.read("sub/data.bin").encoding
    p zf.find_entry("sub/").directory?
    zf.get_input_stream("local.txt") { |io| p io.read }
    p zf.glob("**/*.txt").map(&:name).sort
  end
  Zip::File.open(path) do |zf|
    zf.rename("local.txt", "renamed.txt")
    zf.remove("sub/data.bin")
    zf.get_output_stream("new.txt") { |f| f.puts "new" }
  end
  Zip::File.open(path) { |zf| p zf.entries.map(&:name).sort }
  Zip::File.open(path) do |zf|
    zf.each do |entry|
      next unless entry.file?
      dest = File.join(dir, "out", entry.name)
      FileUtils.mkdir_p(File.dirname(dest))
      entry.extract(dest)
    end
  end
  p Dir.children(File.join(dir, "out")).sort, File.read(File.join(dir, "out", "renamed.txt"))
  buf = Zip::OutputStream.write_buffer do |zos|
    zos.put_next_entry("stream.txt")
    zos.write "streamed"
    zos.put_next_entry("stored.txt", nil, nil, Zip::Entry::STORED)
    zos.write "plain"
  end
  Zip::InputStream.open(StringIO.new(buf.string)) do |zis|
    while (entry = zis.get_next_entry)
      p [entry.name, zis.read]
    end
  end
  begin
    Zip::File.open(File.join(dir, "missing.zip"))
  rescue Zip::Error, Errno::ENOENT => e
    p e.class
  end
end
