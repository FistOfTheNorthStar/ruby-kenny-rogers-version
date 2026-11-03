# `next` and `break` that leave a begin/ensure run the ensure clause on the way (message_bus's
# DistributedCache#process_message decrements its loop index in an ensure)
i = 3
seen = []
while i >= 0
  begin
    next if i.odd?
    seen << i
  ensure
    i -= 1
  end
end
p seen

log = []
n = 0
while true
  begin
    begin
      n += 1
      break if n == 3
      next
    ensure
      log << "inner #{n}"
    end
  ensure
    log << "outer #{n}"
  end
end
p log

# in a block: next returns from it, break leaves the method it was passed to
log = []
r = [1, 2, 3].map do |x|
  begin
    next x * 10 if x.odd?
    x
  ensure
    log << "ensure #{x}"
  end
end
p r, log

log = []
r = [1, 2, 3].each do |x|
  begin
    break :broke if x == 2
  ensure
    log << "ensure #{x}"
  end
end
p r, log

# a loop inside the begin: its next stays inside, the ensure runs once
log = []
begin
  [1, 2].each { |x| next if x == 1; log << x }
  3.times { |k| begin; next if k == 0; log << k; ensure; log << "e#{k}"; end }
ensure
  log << "outer"
end
p log

# with rescue, and an exception raised by the ensure clause itself
log = []
4.times do |k|
  begin
    raise "boom #{k}" if k == 1
    next if k == 2
    log << k
  rescue => e
    log << e.message
    next
  ensure
    log << "ensure #{k} #{$!.inspect}"
  end
end
p log

def ensure_raises
  [1].each do
    begin
      next
    ensure
      raise ArgumentError, "from ensure"
    end
  end
rescue ArgumentError => e
  "rescued #{e.message}"
end
p ensure_raises

# lambda: next and break return from it, through the ensure
log = []
l = ->(x) { begin; next :n if x; break :b; ensure; log << x; end }
p [l.(true), l.(false), log]

# until / loop with break value
k = 0
v = until k > 5
  begin
    break k * 100 if k == 2
  ensure
    k += 1
  end
end
p [v, k]
