#!/bin/sh
# Compile every tests/ruby/*.rb with Kenny and compare against the expected .out file.
# A test may name extra `kenny run` flags in a `# kenny-run-flags: ...` line (shell-expanded,
# e.g. `$(ruby tools/gem-paths.rb sqlite3 1.7.3)` for a test that needs an installed gem).
cd "$(dirname "$0")/.."
# the gem lookups in kenny-run-flags use the Ruby that .ruby-version names, as kenny does
# (unless KENNY_RUBY picks one)
v=$(sed -n '1{s/^ruby-//;s/@.*//;p;}' .ruby-version 2>/dev/null)
if [ -z "$KENNY_RUBY" ] && [ -n "$v" ]; then
  for d in "$HOME/.rvm/rubies/ruby-$v" "$HOME/.rbenv/versions/$v" "$HOME/.asdf/installs/ruby/$v" "$HOME/.rubies/ruby-$v" "$HOME/.local/share/mise/installs/ruby/$v"; do
    [ -x "$d/bin/ruby" ] || continue
    PATH="$d/bin:$PATH"
    if [ -d "$HOME/.rvm/gems/ruby-$v" ]; then
      export GEM_HOME="$HOME/.rvm/gems/ruby-$v" GEM_PATH="$HOME/.rvm/gems/ruby-$v:$HOME/.rvm/gems/ruby-$v@global"
    fi
    break
  done
elif [ -n "$KENNY_RUBY" ]; then
  PATH="$(dirname "$KENNY_RUBY"):$PATH"
fi
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
