#!/bin/sh
# Test the built-in HTTP server: builds tests/server/app/config.ru with Kenny, then
# tests/server/test.rb (run by CRuby) starts the binary with different settings on free ports
# and drives it over raw TCP sockets.
#
#   tests/server/run.sh              every test
#   tests/server/run.sh chunked      only the tests whose "group: name" matches (a regexp)
#   KENNY_RUBY=/path/to/ruby tests/server/run.sh
cd "$(dirname "$0")/../.."
root=$(pwd)

if [ -z "$KENNY_RUBY" ]; then
  v=$(sed -n '1{s/^ruby-//;s/@.*//;p;}' .ruby-version)
  for d in "$HOME/.rvm/rubies/ruby-$v" "$HOME/.rbenv/versions/$v" "$HOME/.asdf/installs/ruby/$v" "$HOME/.rubies/ruby-$v" "$HOME/.local/share/mise/installs/ruby/$v"; do
    [ -x "$d/bin/ruby" ] && { KENNY_RUBY="$d/bin/ruby"; break; }
  done
fi
[ -n "$KENNY_RUBY" ] || KENNY_RUBY=$(command -v ruby)
export KENNY_RUBY
unset GEM_HOME GEM_PATH BUNDLE_GEMFILE
echo "Ruby $("$KENNY_RUBY" -e 'print RUBY_VERSION') ($KENNY_RUBY)"

out=${KENNY_SERVER_OUT:-$root/target/server}
mkdir -p "$out"
if ! (cd tests/server/app && "$root/target/release/kenny" build config.ru -o "$out/app") > "$out/build.log" 2>&1; then
  echo "FAIL build"; grep -v '^warning: ' "$out/build.log" | head -20; exit 1
fi
exec "$KENNY_RUBY" tests/server/test.rb "$out/app" "$@"
