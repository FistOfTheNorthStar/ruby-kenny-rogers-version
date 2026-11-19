# Frames of eval'd code carry the label of the frame it runs as part of, and its file is
# named after the call (Ruby 3.3).
def first_lines(e) = e.backtrace.first(1).map { |l| l.sub(/\A.*\/(?=eval_frames)/, "").sub("(eval at ", "(eval at ").sub(/at .*eval_frames/, "at eval_frames") }
def in_method
  eval("raise 'x'")
rescue => e
  p first_lines(e)
end
in_method
begin
  eval("raise 'y'")
rescue => e
  p first_lines(e)
end
begin
  eval("[1].each { raise 'z' }")
rescue => e
  p first_lines(e)
end
class K
  def m = instance_eval("raise 'i'")
  def self.c = class_eval("raise 'c'")
end
[-> { K.new.m }, -> { K.c }].each do |l|
  l.call
rescue => e
  p first_lines(e)
end
# through a Binding: the Binding's frame, not the caller's
def bound = binding
def evaluates(b) = b.eval("raise 'b'", "(given)", 7)
begin
  evaluates(bound)
rescue => e
  p e.backtrace.first
end
begin
  evaluates(TOPLEVEL_BINDING)
rescue => e
  p e.backtrace.first
end
begin
  eval("raise 'f'", nil, "(file)", 3)
rescue => e
  p e.backtrace.first
end
p eval("__FILE__").sub(/at .*eval_frames/, "at eval_frames"), eval("__LINE__"), eval("__FILE__", nil, "named")
def label_of = eval("caller_locations(0, 1).first.label")
p label_of
