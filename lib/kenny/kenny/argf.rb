# ARGF: the files named in ARGV read one after another, or $stdin when ARGV is empty.
# Kenny loads this on first use of the constant (as with Set and DidYouMean).

ARGF = Class.new do
  include Enumerable

  def self.name = "ARGF.class"
  def self.to_s = name
  def self.inspect = name

  def initialize(argv = ARGV)
    @argv = argv
    @current = nil
    @filename = nil
    @started = false
    @lineno = 0
    @binmode = false
  end

  def argv = @argv
  def to_s = "ARGF"
  alias_method :inspect, :to_s

  def filename
    next_file
    @filename
  end
  alias_method :path, :filename

  def file = next_file || $stdin
  def to_io = file
  def fileno = file.fileno
  def lineno = @lineno
  def lineno=(n)
    @lineno = n
  end

  def binmode
    @binmode = true
    @current&.binmode
    self
  end
  def binmode? = @binmode
  def external_encoding = Encoding.default_external
  def internal_encoding = Encoding.default_internal

  def gets(*args)
    while (f = next_file)
      line = f.gets(*args)
      if line
        @lineno += 1
        return line
      end
      break unless finish_file
    end
    nil
  end

  def readline(*args)
    gets(*args) or raise EOFError, "end of file reached"
  end

  def readlines(*args)
    lines = []
    while (l = gets(*args))
      lines << l
    end
    lines
  end
  alias_method :to_a, :readlines

  def each_line(*args)
    return enum_for(:each_line, *args) unless block_given?
    while (l = gets(*args))
      yield l
    end
    self
  end
  alias_method :each, :each_line

  def read(length = nil, outbuf = nil)
    out = +""
    while (f = next_file)
      chunk = length ? f.read(length - out.bytesize) : f.read
      out << chunk if chunk
      break if length && out.bytesize >= length
      break unless finish_file
    end
    res = length && length > 0 && out.empty? ? nil : out
    return res unless outbuf
    outbuf.replace(res || "")
    res && outbuf
  end

  def readpartial(maxlen, outbuf = nil)
    res = read_nonempty { |f| f.readpartial(maxlen) }
    raise EOFError, "end of file reached" unless res
    outbuf ? outbuf.replace(res) : res
  end

  def getc = read_nonempty(&:getc)
  def getbyte = read_nonempty(&:getbyte)
  def readchar = getc || raise(EOFError, "end of file reached")
  def readbyte = getbyte || raise(EOFError, "end of file reached")

  def each_char
    return enum_for(:each_char) unless block_given?
    while (c = getc)
      yield c
    end
    self
  end

  def each_byte
    return enum_for(:each_byte) unless block_given?
    while (b = getbyte)
      yield b
    end
    self
  end

  def eof?
    f = next_file
    f.nil? || f.eof?
  end
  alias_method :eof, :eof?

  def skip
    finish_file if @current
    self
  end

  def close
    finish_file if @current
    self
  end

  def closed?
    next_file.nil?
  end

  private

  # The file being read: the next one from ARGV once the last is done ($stdin if ARGV was
  # empty from the start); nil when all are read.
  def next_file
    return @current if @current
    if !@argv.empty?
      @filename = @argv.shift
      @current = @filename == "-" ? $stdin : File.open(@filename, @binmode ? "rb" : "r")
    elsif !@started
      @filename = "-"
      @current = $stdin
    else
      return nil
    end
    @started = true
    @current
  end

  # Done with the current file; true when another one follows.
  def finish_file
    @current.close unless @current.equal?($stdin) || @current.closed?
    @current = nil
    !@argv.empty?
  end

  # The first non-nil result of the block on the files in turn.
  def read_nonempty
    while (f = next_file)
      r = begin
        yield f
      rescue EOFError
        nil
      end
      return r if r
      break unless finish_file
    end
    nil
  end
end.new
