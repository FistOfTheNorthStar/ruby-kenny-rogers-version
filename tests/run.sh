#!/bin/sh
# Compile every tests/ruby/*.rb with Kenny and compare against the expected output.
#
#   tests/run.sh          build against the Ruby .ruby-version names (3.4)
#   tests/run.sh 4.0      build against an installed Ruby 4.0 (or 4.0.7, 3.4.11 ...)
#   KENNY_RUBY=/path/to/ruby tests/run.sh
#
# The expected output is the test's .out file, or its .X.Y.out (e.g. .4.0.out) when that Ruby
# version prints something else. A test may name extra `kenny run` flags in a
# `# kenny-run-flags: ...` line (shell-expanded, e.g. `$(ruby -e '...')` for a test that needs
# an installed gem: that `ruby` is the one the tests build against).
cd "$(dirname "$0")/.."

# An installed Ruby `$1` (exact, or the newest $1.*), as kenny finds it.
find_ruby() {
  for root in "$HOME/.rvm/rubies/ruby-" "$HOME/.rbenv/versions/" "$HOME/.asdf/installs/ruby/" "$HOME/.rubies/ruby-" "$HOME/.local/share/mise/installs/ruby/" "/opt/rubies/ruby-"; do
    found=$(for d in "$root$1" "$root$1".*; do [ -x "$d/bin/ruby" ] && echo "${d##*[-/]} $d"; done | sort -t. -k1,1n -k2,2n -k3,3n | tail -1 | cut -d' ' -f2-)
    if [ -n "$found" ]; then echo "$found/bin/ruby"; return 0; fi
  done
  return 1
}

if [ -n "$1" ]; then
  KENNY_RUBY=$(find_ruby "$1") || { echo "no Ruby $1 installed (rvm, rbenv, asdf, chruby, mise)" >&2; exit 2; }
elif [ -z "$KENNY_RUBY" ]; then
  v=$(sed -n '1{s/^ruby-//;s/@.*//;p;}' .ruby-version 2>/dev/null)
  [ -n "$v" ] && KENNY_RUBY=$(find_ruby "$v")
fi
if [ -n "$KENNY_RUBY" ]; then
  export KENNY_RUBY
  PATH="$(dirname "$KENNY_RUBY"):$PATH"
  # (the shell's gem directories belong to its own Ruby; rvm keeps a Ruby's gems apart)
  unset GEM_HOME GEM_PATH
  name=$(basename "$(dirname "$(dirname "$KENNY_RUBY")")")
  if [ -d "$HOME/.rvm/gems/$name" ] && case "$KENNY_RUBY" in "$HOME/.rvm/"*) true;; *) false;; esac; then
    export GEM_HOME="$HOME/.rvm/gems/$name" GEM_PATH="$HOME/.rvm/gems/$name:$HOME/.rvm/gems/$name@global"
  fi
fi
ver=$(ruby -e 'print RUBY_VERSION[/\A\d+\.\d+/]')
echo "Ruby $(ruby -e 'print RUBY_VERSION') ($(command -v ruby))"

pass=0; fail=0
for f in tests/ruby/*.rb; do
  exp="${f%.rb}.$ver.out"
  [ -f "$exp" ] || exp="${f%.rb}.out"
  flags=$(sed -n 's/^# kenny-run-flags: //p' "$f" | head -1)
  if [ -n "$flags" ]; then eval "set -- $flags"; else set --; fi
  got=$(./target/release/kenny run "$f" "$@" 2>&1)
  if [ "$got" = "$(cat "$exp")" ]; then pass=$((pass+1)); else fail=$((fail+1)); echo "FAIL $f"; printf '%s\n' "$got" | diff "$exp" - | head -20; fi
done
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
