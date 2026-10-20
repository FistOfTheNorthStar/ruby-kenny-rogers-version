#!/bin/sh
# Compile every tests/ruby/*.rb with Kenny and compare against the expected .out file.
# A test may name extra `kenny run` flags in a `# kenny-run-flags: ...` line (shell-expanded,
# e.g. `$(ruby tools/gem-paths.rb sqlite3 1.7.3)` for a test that needs an installed gem).
cd "$(dirname "$0")/.."
pass=0; fail=0
for f in tests/ruby/*.rb; do
  exp="${f%.rb}.out"
  flags=$(sed -n 's/^# kenny-run-flags: //p' "$f" | head -1)
  if [ -n "$flags" ]; then eval "set -- $flags"; else set --; fi
  got=$(./target/release/kenny run "$f" "$@" 2>&1)
  if [ "$got" = "$(cat "$exp")" ]; then pass=$((pass+1)); else fail=$((fail+1)); echo "FAIL $f"; printf '%s\n' "$got" | diff "$exp" - | head -20; fi
done
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
