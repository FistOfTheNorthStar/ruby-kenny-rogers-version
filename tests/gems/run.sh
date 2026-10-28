#!/bin/sh
# Smoke-test popular gems under Kenny: each tests/gems/<gem>/test.rb runs under CRuby and
# under `kenny run`, and the two outputs must match.
#
#   tests/gems/run.sh              every gem
#   tests/gems/run.sh rack json    only these
#   KENNY_RUBY=/path/to/ruby tests/gems/run.sh
#
# Each gem has its own Gemfile and Gemfile.lock; run `bundle install` in a gem's directory
# (with the same Ruby) if its gems are missing.
cd "$(dirname "$0")/../.."
root=$(pwd)

if [ -z "$KENNY_RUBY" ]; then
  v=$(sed -n '1{s/^ruby-//;s/@.*//;p;}' .ruby-version)
  for d in "$HOME/.rvm/rubies/ruby-$v" "$HOME/.rbenv/versions/$v" "$HOME/.asdf/installs/ruby/$v" "$HOME/.rubies/ruby-$v" "$HOME/.local/share/mise/installs/ruby/$v"; do
    [ -x "$d/bin/ruby" ] && { KENNY_RUBY="$d/bin/ruby"; break; }
  done
fi
export KENNY_RUBY
PATH="$(dirname "$KENNY_RUBY"):$PATH"
unset GEM_HOME GEM_PATH BUNDLE_GEMFILE
name=$(basename "$(dirname "$(dirname "$KENNY_RUBY")")")
if [ -d "$HOME/.rvm/gems/$name" ]; then
  export GEM_HOME="$HOME/.rvm/gems/$name" GEM_PATH="$HOME/.rvm/gems/$name:$HOME/.rvm/gems/$name@global"
fi
echo "Ruby $(ruby -e 'print RUBY_VERSION') ($KENNY_RUBY)"

out=${KENNY_GEMS_OUT:-$root/target/gems}
mkdir -p "$out"
if [ $# -gt 0 ]; then gems="$*"; else gems=$(cd tests/gems && ls -d */ | tr -d /); fi
# (a hung test is killed after $KENNY_GEMS_TIMEOUT seconds)
limit() { perl -e '$t = shift; $p = fork // die; if (!$p) { exec @ARGV or exit 127 } $SIG{ALRM} = sub { kill "KILL", $p; print STDERR "timed out after ${t}s\n"; exit 124 }; alarm $t; waitpid $p, 0; exit($? & 127 ? 128 + ($? & 127) : $? >> 8)' "${KENNY_GEMS_TIMEOUT:-300}" "$@"; }

pass=0; fail=0; failed=""
for g in $gems; do
  dir="$root/tests/gems/$g"
  [ -f "$dir/test.rb" ] || { echo "no test for $g"; continue; }
  (cd "$dir" && limit ruby test.rb) > "$out/$g.ruby" 2>&1
  if ! (cd "$dir" && limit "$root/target/release/kenny" build test.rb -o "$out/$g.bin") > "$out/$g.build" 2>&1; then
    fail=$((fail+1)); failed="$failed $g"; echo "FAIL $g (build)"
    grep -v '^warning: ' "$out/$g.build" | head -${KENNY_GEMS_DIFF:-15}
    continue
  fi
  (cd "$dir" && limit "$out/$g.bin") > "$out/$g.kenny" 2>&1
  if cmp -s "$out/$g.ruby" "$out/$g.kenny"; then
    pass=$((pass+1)); echo "ok   $g"
  else
    fail=$((fail+1)); failed="$failed $g"; echo "FAIL $g"
    diff "$out/$g.ruby" "$out/$g.kenny" | head -${KENNY_GEMS_DIFF:-15}
  fi
done
echo "$pass passed, $fail failed${failed:+:$failed}"
[ "$fail" -eq 0 ]
