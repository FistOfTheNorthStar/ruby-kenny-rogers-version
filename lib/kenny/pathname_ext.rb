# Kenny's replacement for CRuby's pathname.so (ext/pathname/pathname.c): the core of
# Pathname written in Ruby on top of File / Dir / IO. The stdlib pathname.rb is compiled
# on top of this (it does `require 'pathname.so'`).
class Pathname
  def initialize(arg)
    arg = arg.to_path if !arg.is_a?(String) && arg.respond_to?(:to_path)
    raise TypeError, "no implicit conversion of #{arg.nil? ? 'nil' : arg.class} into String" unless arg.is_a?(String)
    raise ArgumentError, "pathname contains null byte" if arg.include?("\0")
    @path = arg.dup
  end

  def to_s
    @path.dup
  end
  alias to_path to_s

  def inspect
    "#<#{self.class}:#{@path}>"
  end

  def freeze
    @path.freeze
    super
  end

  def ==(other)
    return false unless other.is_a?(Pathname)
    @path == other.to_s
  end
  alias === ==
  alias eql? ==

  def <=>(other)
    return nil unless other.is_a?(Pathname)
    @path.tr("/", "\0") <=> other.to_s.tr("/", "\0")
  end

  def hash
    @path.hash
  end

  def sub(*args, &block)
    self.class.new(block ? @path.sub(*args, &block) : @path.sub(*args))
  end

  def sub_ext(repl)
    ext = File.extname(@path)
    self.class.new(@path.chomp(ext) + repl)
  end

  def realpath(*args) self.class.new(File.realpath(@path, *args)) end
  def realdirpath(*args) self.class.new(File.realdirpath(@path, *args)) end

  def each_line(*args, &block) File.foreach(@path, *args, &block) end
  def read(*args, **kw) File.read(@path, *args, **kw) end
  def binread(*args) File.binread(@path, *args) end
  def readlines(*args, **kw) File.readlines(@path, *args, **kw) end
  def write(*args, **kw) File.write(@path, *args, **kw) end
  def binwrite(*args, **kw) File.binwrite(@path, *args, **kw) end
  def sysopen(*args) IO.sysopen(@path, *args) end

  def atime() File.atime(@path) end
  def birthtime() File.birthtime(@path) end
  def ctime() File.ctime(@path) end
  def mtime() File.mtime(@path) end
  def chmod(mode) File.chmod(mode, @path) end
  def lchmod(mode) File.lchmod(mode, @path) end
  def chown(owner, group) File.chown(owner, group, @path) end
  def lchown(owner, group) File.lchown(owner, group, @path) end
  def fnmatch(pattern, *args) File.fnmatch(pattern, @path, *args) end
  def fnmatch?(pattern, *args) File.fnmatch?(pattern, @path, *args) end
  def ftype() File.ftype(@path) end
  def make_link(old) File.link(old, @path) end
  def open(*args, **kw, &block) File.open(@path, *args, **kw, &block) end
  def readlink() self.class.new(File.readlink(@path)) end
  def rename(to) File.rename(@path, to) end
  def stat() File.stat(@path) end
  def lstat() File.lstat(@path) end
  def make_symlink(old) File.symlink(old, @path) end
  def truncate(length) File.truncate(@path, length) end
  def utime(atime, mtime) File.utime(atime, mtime, @path) end
  def basename(*args) self.class.new(File.basename(@path, *args)) end
  def dirname() self.class.new(File.dirname(@path)) end
  def extname() File.extname(@path) end
  def expand_path(*args) self.class.new(File.expand_path(@path, *args)) end
  def split() File.split(@path).map { |f| self.class.new(f) } end

  def blockdev?() File.blockdev?(@path) end
  def chardev?() File.chardev?(@path) end
  def executable?() File.executable?(@path) end
  def executable_real?() File.executable_real?(@path) end
  def exist?() File.exist?(@path) end
  def grpowned?() File.grpowned?(@path) end
  def directory?() File.directory?(@path) end
  def file?() File.file?(@path) end
  def pipe?() File.pipe?(@path) end
  def socket?() File.socket?(@path) end
  def owned?() File.owned?(@path) end
  def readable?() File.readable?(@path) end
  def world_readable?() File.world_readable?(@path) end
  def readable_real?() File.readable_real?(@path) end
  def setuid?() File.setuid?(@path) end
  def setgid?() File.setgid?(@path) end
  def size() File.size(@path) end
  def size?() File.size?(@path) end
  def sticky?() File.sticky?(@path) end
  def symlink?() File.symlink?(@path) end
  def writable?() File.writable?(@path) end
  def world_writable?() File.world_writable?(@path) end
  def writable_real?() File.writable_real?(@path) end
  def zero?() File.zero?(@path) end

  def empty?
    File.directory?(@path) ? Dir.empty?(@path) : File.empty?(@path)
  end

  def self.glob(*args, **kw)
    if block_given?
      Dir.glob(*args, **kw) { |f| yield new(f) }
      nil
    else
      Dir.glob(*args, **kw).map { |f| new(f) }
    end
  end

  def self.getwd() new(Dir.getwd) end
  def self.pwd() new(Dir.getwd) end

  def glob(*args)
    flags = args[1] || 0
    found = Dir.glob(args[0], flags, base: @path).map { |f| self + f }
    if block_given?
      found.each { |f| yield f }
      nil
    else
      found
    end
  end

  def entries() Dir.entries(@path).map { |f| self.class.new(f) } end
  def mkdir(*args) Dir.mkdir(@path, *args) end
  def rmdir() Dir.rmdir(@path) end
  def opendir(&block) Dir.open(@path, &block) end

  def each_entry
    Dir.foreach(@path) { |f| yield self.class.new(f) }
  end

  def unlink
    if File.directory?(@path)
      Dir.unlink(@path)
    else
      File.unlink(@path)
    end
  end
  alias delete unlink
end

module Kernel
  private

  def Pathname(arg)
    arg.is_a?(Pathname) ? arg : Pathname.new(arg)
  end
end
