# Kenny's Tempfile: the standard library API, as a subclass of File.
# (The stdlib version is a DelegateClass(File) cleaned up by an ObjectSpace finalizer;
# here an unlinked-at-close file is removed by #close!/#unlink, or Tempfile.create's block.)
require 'tmpdir'

class Tempfile < File
  def self.__tmpname(basename, tmpdir) # :nodoc:
    prefix, suffix = basename.is_a?(Array) ? basename : [basename.to_s, ""]
    dir = tmpdir || Dir.tmpdir
    stamp = Time.now.strftime("%Y%m%d")
    File.join(dir, "#{prefix}#{stamp}-#{$$}-#{rand(0x100000000).to_s(36)}#{suffix}")
  end

  def self.create(basename = "", tmpdir = nil, mode: 0, **options)
    path = __tmpname(basename, tmpdir)
    f = File.open(path, File::RDWR | File::CREAT | File::EXCL | mode, 0600, **options)
    return f unless block_given?
    begin
      yield f
    ensure
      f.close unless f.closed?
      File.unlink(path) if File.exist?(path)
    end
  end

  def self.open(*args, **kw)
    t = new(*args, **kw)
    return t unless block_given?
    begin
      yield t
    ensure
      t.close
    end
  end

  def initialize(basename = "", tmpdir = nil, mode: 0, **options)
    @unlinked = false
    @tmpname = Tempfile.__tmpname(basename, tmpdir)
    super(@tmpname, File::RDWR | File::CREAT | File::EXCL | mode, 0600, **options)
  end

  def path
    @unlinked ? nil : @tmpname
  end

  def open
    close unless closed?
    reopen(@tmpname, "r+")
    self
  end

  def close(unlink_now = false)
    super()
    unlink if unlink_now
    nil
  end

  def close!
    close(true)
  end

  def unlink
    return if @unlinked
    begin
      File.unlink(@tmpname)
    rescue Errno::ENOENT
    end
    @unlinked = true
  end
  alias delete unlink

  def size
    if closed?
      File.size(@tmpname)
    else
      flush
      super
    end
  end
  alias length size

  def inspect
    "#<#{self.class}:#{@tmpname}#{closed? ? ' (closed)' : ''}>"
  end
end
