#!/bin/sh
# Compare Kenny-compiled binaries against CRuby on the same scripts.
#   RUBY=/path/to/ruby       the Ruby to compare against (default: `ruby` on PATH)
#   RUBY_OPTS=--yjit         options for it. Add --yjit-call-threshold=1 for loop.rb,
#                            mandelbrot.rb and threads.rb: YJIT skips their top-level loops
#                            otherwise (it slows the other scripts)
cd "$(dirname "$0")"
K=../target/release/kenny
RUBY=${RUBY:-ruby}
OUT=${TMPDIR:-/tmp}/kenny-bench
mkdir -p "$OUT"
now() { python3 -c 'import time; print(time.time())'; }
echo "vs $("$RUBY" $RUBY_OPTS -e 'print RUBY_DESCRIPTION')"
for f in ${@:-*.rb}; do
  b=$(basename "$f" .rb)
  $K build "$f" -o "$OUT/$b" || continue
  "$OUT/$b" >/dev/null 2>&1   # first run of a new binary is slowed by the OS security scan
  t0=$(now); "$OUT/$b" > "$OUT/$b.kenny.out"; t1=$(now)
  "$RUBY" $RUBY_OPTS "$f" > "$OUT/$b.ruby.out"; t2=$(now)
  same=$(cmp -s "$OUT/$b.kenny.out" "$OUT/$b.ruby.out" && echo "same output" || echo "OUTPUT DIFFERS")
  python3 -c "k=$t1-$t0; r=$t2-$t1; print(f'{\"$b\":14s} kenny {k:6.2f}s   ruby {r:6.2f}s   {r/k:5.1f}x faster   $same')"
done
