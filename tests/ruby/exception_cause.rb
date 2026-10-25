# Exception#cause: set from $! on raise, `cause:`, and $! restored however a rescue body is left
def inner = raise(ArgumentError, "low")
def outer
  inner
rescue => e
  raise "high"
end
e = (outer rescue $!)
p e.cause, e.cause.cause
begin; raise "a"; rescue; end
p((raise "b" rescue $!).cause)
x = begin
  begin; raise "one"; rescue => o; raise TypeError, "two", cause: nil; end
rescue => t
  t
end
p x.cause
c = RuntimeError.new("c0")
y = (raise ArgumentError, "with", cause: c rescue $!)
p y.cause, y.cause.equal?(c)
z = begin
  begin; raise "first"; rescue => f; raise f; end
rescue => r
  r
end
p z.message, z.cause
begin
  raise cause: c
rescue ArgumentError => ae
  p ae.message
end
begin
  begin; raise "p"; rescue => pe; raise pe, cause: pe; end
rescue => q
  p q.class, q.message, q.cause
end
w = begin
  begin; nil.zork; rescue => n; Integer("zz"); end
rescue => ie
  ie
end
p w.class, w.cause.class
p (begin; [1].frist; rescue NoMethodError => nm; [nm.name, nm.receiver, nm.private_call?]; end)
p (begin; helo; rescue NameError => ne; [ne.name, ne.receiver.inspect]; end)
p (begin; Zork; rescue NameError => ce; [ce.name, ce.receiver]; end)
p (begin; Comparable::Zork; rescue NameError => ce; [ce.name, ce.receiver]; end)
# leaving a rescue body by return / next / break / retry restores $!
def ret_in_rescue
  begin; raise "r1"; rescue; return 1; end
end
ret_in_rescue
p $!, (raise "after-return" rescue $!).cause
[1].each { begin; raise "n1"; rescue; next; end }
p $!, (raise "after-next" rescue $!).cause
loop { begin; raise "b1"; rescue; break; end }
p $!, (raise "after-break" rescue $!).cause
tries = 0
begin
  tries += 1
  raise "t#{tries}" if tries < 3
  p $!, (raise "in-retry" rescue $!).cause
rescue
  retry
end
v = (Integer("x") rescue (begin; raise "inner"; rescue => ii; ii; end))
p v.cause.class, $!
def mod_ret = (Integer("x") rescue (return :ok))
p mod_ret, $!
x = [1, 2].map { |i| begin; raise "m#{i}"; rescue => me; next me.message; end }
p x, $!
p (begin; begin; raise "deep"; rescue; raise ArgumentError; end; rescue => de; [de.cause.message, $!.class]; end), $!
