# Kenny's StringIO: the stringio extension's API in Ruby, on the runtime's minimal StringIO
# (kenny-runtime server.rs, which makes the Rack input). Positions are byte offsets into
# the String it wraps, which it shares: writes change that String.
class StringIO
  include Enumerable

  VERSION = "3.1.2"
  MAX_LENGTH = (1 << 62) - 1

  def self.open(string = nil, mode = nil, **opts)
    io = new(string, mode, **opts)
    return io unless block_given?
    begin
      yield io
    ensure
      io.close unless io.closed?
    end
  end

  def initialize(string = nil, mode = nil, **opts)
    mode = opts[:mode] if mode.nil? && opts.key?(:mode)
    if string.nil?
      string = +""
      mode ||= "r+"
    elsif !string.is_a?(String)
      string = String.try_convert(string) or raise TypeError, "no implicit conversion of #{string.class} into String"
    end
    __open(string, mode)
    enc = opts[:encoding] || opts[:external_encoding]
    set_encoding(enc) if enc
    self
  end

  def initialize_copy(other)
    super
    @__buf = other.string
    self
  end

  def reopen(string = nil, mode = nil)
    if string.is_a?(StringIO)
      initialize_copy(string)
      return self
    end
    __open(string || +"", mode || (string ? nil : "r+"))
    self
  end

  def string = @__buf

  def string=(s)
    __open(s.to_str, nil)
    s
  end

  # ---- reading ----

  def read(length = nil, outbuf = nil)
    __readable!
    buf = @__buf
    pos = @__pos
    size = buf.bytesize
    if length.nil?
      s = pos >= size ? +"" : buf.byteslice(pos, size - pos)
      s.force_encoding(@enc || buf.encoding)
      @__pos = size if pos < size
    else
      length = Integer(length)
      raise ArgumentError, "negative length #{length} given" if length < 0
      if length == 0
        s = +"".b
      elsif pos >= size
        outbuf&.replace("")
        return nil
      else
        s = buf.byteslice(pos, length).b
        @__pos = pos + s.bytesize
      end
    end
    return s unless outbuf
    outbuf.replace(s)
    outbuf
  end

  def readpartial(length, outbuf = nil)
    r = read(length, outbuf)
    raise EOFError, "end of file reached" if r.nil? && length.to_i > 0
    r || +""
  end
  alias sysread readpartial

  def read_nonblock(length, outbuf = nil, exception: true)
    r = read(length, outbuf)
    if r.nil? && length.to_i > 0
      raise EOFError, "end of file reached" if exception
      return nil
    end
    r || +""
  end

  def pread(length, offset, outbuf = nil)
    __readable!
    length = Integer(length)
    raise ArgumentError, "negative string size (or size too big): #{length}" if length < 0
    raise Errno::EINVAL, "pread: Invalid offset argument: #{offset}" if offset < 0
    if length == 0
      s = +"".b
    else
      raise EOFError, "end of file reached" if offset >= @__buf.bytesize
      s = @__buf.byteslice(offset, length).b
    end
    return s unless outbuf
    outbuf.replace(s)
  end

  def getc
    __readable!
    buf = @__buf
    pos = @__pos
    return nil if pos >= buf.bytesize
    n = __charlen(buf, pos)
    @__pos = pos + n
    c = buf.byteslice(pos, n)
    @enc ? c.force_encoding(@enc) : c
  end

  def getbyte
    __readable!
    return nil if @__pos >= @__buf.bytesize
    b = @__buf.getbyte(@__pos)
    @__pos += 1
    b
  end

  def readchar
    c = getc
    raise EOFError, "end of file reached" if c.nil?
    c
  end

  def readbyte
    b = getbyte
    raise EOFError, "end of file reached" if b.nil?
    b
  end

  def ungetc(c)
    __readable!
    return nil if c.nil?
    c = c.is_a?(Integer) ? c.chr(@__buf.encoding) : c.to_str
    __unget(c.b)
  end

  def ungetbyte(b)
    __readable!
    return nil if b.nil?
    __unget(b.is_a?(Integer) ? (b & 0xff).chr.b : b.to_str.b)
  end

  def gets(*args, chomp: false)
    __readable!
    sep, limit = __line_args(args)
    line = __line(sep, limit, chomp)
    @lineno = lineno + 1 if line
    line
  end

  def readline(*args, chomp: false)
    line = gets(*args, chomp: chomp)
    raise EOFError, "end of file reached" if line.nil?
    line
  end

  def each_line(*args, chomp: false)
    return to_enum(:each_line, *args, chomp: chomp) unless block_given?
    __readable!
    sep, limit = __line_args(args)
    raise ArgumentError, "invalid limit: 0 for each_line" if limit == 0
    while (line = __line(sep, limit, chomp))
      @lineno = lineno + 1
      yield line
    end
    self
  end
  alias each each_line

  def readlines(*args, chomp: false)
    __readable!
    sep, limit = __line_args(args)
    raise ArgumentError, "invalid limit: 0 for readlines" if limit == 0
    out = []
    while (line = __line(sep, limit, chomp))
      @lineno = lineno + 1
      out << line
    end
    out
  end

  def each_char
    return to_enum(:each_char) unless block_given?
    while (c = getc)
      yield c
    end
    self
  end

  def each_byte
    return to_enum(:each_byte) unless block_given?
    while (b = getbyte)
      yield b
    end
    self
  end

  def each_codepoint
    return to_enum(:each_codepoint) unless block_given?
    while (c = getc)
      yield c.ord
    end
    self
  end

  def eof?
    __readable!
    @__pos >= @__buf.bytesize
  end
  alias eof eof?

  # ---- writing ----

  def write(*objs)
    __writable!
    n = 0
    objs.each do |o|
      s = o.to_s
      next if s.empty?
      buf = @__buf
      size = buf.bytesize
      @__pos = size if @append
      pos = @__pos
      if pos > size
        buf << ("\0" * (pos - size))
        size = pos
      end
      if pos == size
        buf << s
      else
        buf.bytesplice(pos, [s.bytesize, size - pos].min, s)
      end
      @__pos = pos + s.bytesize
      n += s.bytesize
    end
    n
  end
  alias syswrite write

  def write_nonblock(s, exception: true) = write(s)

  def <<(obj)
    write(obj)
    self
  end

  def print(*objs)
    objs = [$_] if objs.empty?
    objs.each_with_index do |o, i|
      write($,) if $, && i > 0
      write(o.to_s)
    end
    write($\) if $\
    nil
  end

  def puts(*objs)
    if objs.empty?
      write("\n")
      return nil
    end
    objs.each { |o| __puts_one(o, {}) }
    nil
  end

  def printf(fmt, *args)
    write(format(fmt, *args))
    nil
  end

  def putc(c)
    write(c.is_a?(Integer) ? (c & 0xff).chr : c.to_s[0])
    c
  end

  def truncate(length)
    __writable!
    length = Integer(length)
    raise Errno::EINVAL, "negative length" if length < 0
    size = @__buf.bytesize
    if length < size
      @__buf.bytesplice(length, size - length, "")
    else
      @__buf << ("\0" * (length - size))
    end
    0
  end

  # ---- position ----

  def pos = @__pos
  alias tell pos

  def pos=(n)
    n = Integer(n)
    raise Errno::EINVAL, n.to_s if n < 0
    @__pos = n
  end

  def seek(offset, whence = 0)
    raise IOError, "closed stream" if closed?
    offset = Integer(offset)
    base = case whence
           when 0, :SET, nil then 0
           when 1, :CUR then @__pos
           when 2, :END then @__buf.bytesize
           else raise Errno::EINVAL, "invalid whence"
           end
    raise Errno::EINVAL, offset.to_s if base + offset < 0
    @__pos = base + offset
    0
  end

  def rewind
    @__pos = 0
    @lineno = 0
    0
  end

  def lineno = @lineno || 0

  def lineno=(n)
    @lineno = Integer(n)
  end

  def size = @__buf.bytesize
  alias length size

  # ---- state ----

  def close
    @closed_read = @closed_write = true
    nil
  end

  def close_read
    raise IOError, "closing non-duplex IO for reading" unless @readable
    @closed_read = true
    nil
  end

  def close_write
    raise IOError, "closing non-duplex IO for writing" unless @writable
    @closed_write = true
    nil
  end

  # (a StringIO the runtime made, the Rack input, has none of these set: read-write)
  def closed? = closed_read? && closed_write?
  def closed_read? = @closed_read || @readable == false
  def closed_write? = @closed_write || @writable == false

  def sync = true
  def sync=(v)
    v
  end
  def fsync = 0
  def flush = self
  def fileno = nil
  def isatty = false
  alias tty? isatty
  def pid = nil
  def fcntl(*) = raise(NotImplementedError, "fcntl() function is unimplemented on this machine")

  def binmode
    @enc = Encoding::BINARY
    @__buf.force_encoding(Encoding::BINARY) unless @__buf.frozen?
    self
  end

  def external_encoding = @enc || @__buf.encoding
  def internal_encoding = nil

  def set_encoding(ext, _int = nil, **_opts)
    ext = ext.split(":").first if ext.is_a?(String) && ext.include?(":")
    enc = ext.nil? ? Encoding.default_external : Encoding.find(ext.to_s)
    @enc = enc
    @__buf.force_encoding(enc) unless @__buf.frozen?
    self
  end

  def set_encoding_by_bom
    return nil unless @__buf.byteslice(0, 3) == "\xEF\xBB\xBF".b
    @__pos = 3 if @__pos < 3
    set_encoding(Encoding::UTF_8)
    Encoding::UTF_8
  end

  private

  def __open(string, mode)
    readable = writable = true
    truncate = false
    @append = false
    case mode
    when nil
      writable = !string.frozen?
    when Integer
      acc = mode & 3
      readable = acc != 1
      writable = acc != 0
      @append = defined?(File::APPEND) && mode & File::APPEND != 0
      truncate = defined?(File::TRUNC) && mode & File::TRUNC != 0
    else
      m = mode.to_s.split(":").first.to_s.delete("bt")
      readable = m.start_with?("r") || m.include?("+")
      writable = m.start_with?("w", "a") || m.include?("+")
      @append = m.start_with?("a")
      truncate = m.start_with?("w")
      set_enc = mode.to_s.split(":")[1]
    end
    raise Errno::EACCES, "Permission denied" if writable && string.frozen?
    string.replace("") if truncate
    @__buf = string
    @__pos = 0
    @lineno = 0
    @readable = readable
    @writable = writable
    @closed_read = @closed_write = false
    @enc = nil
    set_encoding(set_enc) if set_enc
  end

  def __readable!
    raise IOError, "not opened for reading" if @closed_read || @readable == false
  end

  def __writable!
    raise IOError, "not opened for writing" if @closed_write || @writable == false
  end

  # bytes of the character at byte `pos`
  def __charlen(buf, pos)
    return 1 unless buf.encoding == Encoding::UTF_8
    b = buf.getbyte(pos)
    n = if b < 0x80 then 1
        elsif b >= 0xf0 then 4
        elsif b >= 0xe0 then 3
        elsif b >= 0xc2 then 2
        else 1
        end
    n = 1 if pos + n > buf.bytesize
    n
  end

  def __unget(bytes)
    n = bytes.bytesize
    return nil if n == 0
    pos = @__pos
    size = @__buf.bytesize
    if pos > size
      @__buf << ("\0" * (pos - size))
    end
    if n <= pos
      @__buf.bytesplice(pos - n, n, bytes)
      @__pos = pos - n
    else
      @__buf.bytesplice(0, pos, bytes)
      @__pos = 0
    end
    nil
  end

  def __line_args(args)
    sep = $/
    limit = nil
    case args.size
    when 0
    when 1
      a = args[0]
      if a.nil? || a.is_a?(String)
        sep = a
      elsif a.respond_to?(:to_str)
        sep = a.to_str
      else
        limit = Integer(a)
      end
    else
      sep = args[0].nil? ? nil : args[0].to_str
      limit = args[1].nil? ? nil : Integer(args[1])
    end
    limit = nil if limit && limit < 0
    [sep, limit]
  end

  # the next line (nil at the end)
  def __line(sep, limit, chomp)
    buf = @__buf
    pos = @__pos
    size = buf.bytesize
    return nil if pos >= size
    return +"" if limit == 0
    paragraph = sep == ""
    if paragraph
      pos += 1 while pos < size && buf.getbyte(pos) == 10
      return nil if pos >= size
      sep = "\n\n"
    end
    stop = if sep.nil?
             size
           else
             i = buf.byteindex(sep, pos)
             i ? i + sep.bytesize : size
           end
    if limit && stop - pos > limit
      stop = pos + limit
      # not inside a character
      stop += 1 while stop < size && buf.encoding == Encoding::UTF_8 && (buf.getbyte(stop) & 0xc0) == 0x80
    end
    line = buf.byteslice(pos, stop - pos)
    line.force_encoding(@enc) if @enc
    if paragraph
      stop += 1 while stop < size && buf.getbyte(stop) == 10
    end
    @__pos = stop
    if chomp && sep
      if paragraph
        line = line.sub(/\n+\z/, "")
      elsif sep == "\n"
        line = line.chomp
      elsif line.end_with?(sep)
        line = line.byteslice(0, line.bytesize - sep.bytesize)
      end
    end
    line
  end

  def __puts_one(o, seen)
    if o.is_a?(Array) || (!o.is_a?(String) && o.respond_to?(:to_ary) && (o = o.to_ary).is_a?(Array))
      if seen[o.object_id]
        write("[...]\n")
        return
      end
      seen[o.object_id] = true
      o.each { |x| __puts_one(x, seen) }
      seen.delete(o.object_id)
      return
    end
    s = o.nil? ? "" : o.to_s
    s = o.inspect unless s.is_a?(String)
    write(s)
    write("\n") unless s.end_with?("\n")
  end
end
