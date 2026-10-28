# Top 100 gems under Kenny

Smoke tests for the 100 most-downloaded gems on rubygems.org (all-time, October 2026). Each
`<gem>/` has a `Gemfile` naming that gem, its `Gemfile.lock`, and a `test.rb` that uses the
gem's main API offline. `run.sh` runs each test under CRuby and as a Kenny binary
(`kenny build`, then the binary); a gem passes when the two outputs are identical.

```
tests/gems/run.sh                 # all gems (KENNY_GEMS_TIMEOUT=900 for the Rails ones)
tests/gems/run.sh rack json       # some
```

Outputs land in `target/gems/<gem>.{ruby,build,kenny}`. The gems must be installed for the
Ruby the tests run against (`bundle install` in a gem's directory).

## Results (Ruby 3.4.12, 2026-10-28): 40 of 100 pass

Pass: activejob activemodel activerecord actionmailer actionpack actionview ast aws-eventstream
aws-partitions aws-sigv4 builder connection_pool diff-lcs docile http-cookie i18n jmespath
loofah mime-types mini_mime mini_portile2 multipart-post netrc parallel public_suffix rack
rack-protection rack-test rails-dom-testing rainbow rb-fsevent ruby-progressbar rubyzip
sprockets sprockets-rails thread_safe tilt tzinfo unicode-display_width websocket-extensions

Fail, by design (a feature LANGUAGE.md drops, or the binary replaces): puma (left out: the
binary is the server), erubi (frozen literals; `eval` reading locals), ffi (calling C
functions), simplecov (no `Coverage`; not yet listed in LANGUAGE.md).

Fail, Kenny bugs: the other 56. Most fail on the first bug in the list below; several have
more behind it.

## Bugs, by the number of gems they block

| # | Bug | Gems | Where |
|---|---|---|---|
| 1 | `require_relative` with a non-literal argument returns false and loads nothing | rspec, rspec-core, rspec-expectations, rspec-mocks, rspec-support, regexp_parser, rubocop, multi_json | `kenny-compiler/src/calls.rs:273`, `kenny-runtime/src/kernel.rs:584` |
| 2 | OpenSSL: no `PKey`, `X509`, `SSL::SSLErrorWaitReadable`/`OP_ALL`; `OpenSSL::Digest` isn't a `Digest::Class` (no `.file`) | aws-sdk-core, aws-sdk-s3, aws-sdk-kms, httpclient, jwt, googleauth, signet, activestorage | `kenny-runtime/src/openssl.rs` |
| 3 | `ENV` has only `[] []= fetch key? to_h inspect` (no `include?`, `has_key?`, `update`, `delete`, `each`, ...); `OptionParser#parse!` always fails | rake, dotenv, minitest, rspec-core, rspec, excon, googleauth | `kenny-runtime/src/misc.rs:1452` |
| 4 | `Method`/`Proc#source_location`, `const_source_location` always nil | redis, method_source, pry, rspec-core, rspec | `methodobj.rs:343`, `extra.rs:1780,1813` |
| 5 | Regexps on binary strings: `StringScanner#scan(/../n)` panics the process; a non-`/n` regexp matches UTF-8 characters in a binary string | rexml, multi_xml, aws-sdk-core, aws-sdk-s3, addressable | `strscan.rs:20-83`, `regex.rs:165` |
| 6 | Gems' C extensions aren't compiled from the lock file, silently; `--ext` misses C API functions (msgpack) and doesn't take C++ (unf_ext) | msgpack, unf_ext, rubocop-ast + rubocop (prism), websocket-driver (falls back to Ruby code that mutates frozen literals) | `kenny/src/main.rs` `build_exts`, `include/ruby.h` |
| 7 | `Struct.new(:a, :b)` on a Struct subclass makes an instance, not a new class | faraday, faraday-net_http, signet, googleauth | `misc.rs:1127` |
| 8 | `warn` prints when `$VERBOSE` is nil (`silence_warnings` leaks) | activesupport, globalid, rails, activestorage | `io.rs:4419`, `kernel.rs:546` |
| 9 | `Gem::Deprecate`, `Gem::Platform` missing (CRuby always loads rubygems) | addressable, signet, googleauth, aws-sdk-* | `lib/kenny/kenny/gem.rb` |
| 10 | Autoload: `defined?`/`const_defined?` true while the autoload is pending; `module X` over a pending autoload skips a Ruby `require` override (Zeitwerk) and defines an empty module; `const_get("A::B::C")` stops at an autoloaded segment | unf, coderay (`autoload :YAML`), actioncable, activestorage | `abi.rs:1481`, `kernel.rs:1303`, `lib/kenny/yaml.rb:5` |
| 11 | `format`: no `%<name>s`, `%{name}`, `%1$s`, `#` flag, integer precision (`%.5d`), `%x` of a Bignum | parser, pry, domain_name | `fmt.rs:305-446` |
| 12 | `require_relative "x.tab"` doesn't try `x.tab.rb` | racc, rubocop-ast, rubocop | `calls.rs:627` |
| 13 | `super` in a block inside a `define_method` body finds the enclosing `def` | rspec-core, rspec-expectations (custom matchers silently pass), rspec | `calls.rs:797` |
| 14 | A new singleton class can't call its object's class methods (`class << obj; self; end.mac`) | rspec-core, rspec-expectations, rspec | `core.rs:1421` |
| 15 | `String#lines`/`each_line` ignore the separator and `chomp:` (so `URI.decode_www_form` is wrong) | mime-types-data, signet | `string.rs:1157-1180` |
| 16 | `Float::MAX_10_EXP`, `MIN_10_EXP`, `MAX_EXP`, `MIN_EXP`, `MANT_DIG`, `RADIX` missing | crass, rails-html-sanitizer | `numeric.rs:718` |
| 17 | Errno table incomplete (`EHOSTDOWN`, `ESHUTDOWN`, `ECANCELED`, ...) | redis, googleauth | `io.rs:80-142` |
| 18 | Sockets: no `UNIXSocket.pair`, `Socket.new`, `Addrinfo`, `sockaddr_in`, `connect_nonblock` | nio4r, redis, excon | `lib/kenny/socket.rb`, `socket.rs` |
| 19 | `Thread#group` is nil, so `Timeout.timeout` (timeout gem 0.4) fails on first use | httpclient | `extra.rs:2256` |
| 20 | `exit` ends the process instead of raising `SystemExit` (skips `ensure`; `SystemExit#status` missing; uncaught `SystemExit` exits 0) | rubygems-update | `kernel.rs:819`, `boot.rs:113`, `abi.rs:145` |
| 21 | Native json/bigdecimal behave as json 2.9.1/bigdecimal 3.1.8 whatever the lock says; `BigDecimal#to_r` calls `Rational.new` | json, bigdecimal | `json.rs:950`, `bigdecimal.rs:1106,1431` |
| 22 | `method_added` not called for runtime `define_method`, `attr_*`, `alias_method`; `public_methods(false)` ignores its argument | thor | `kernel.rs:1162`, `visibility.rs:218` |
| 23 | Refinements picked in definition order, not by the receiver's ancestors; not active inside `refine` blocks | rexml (XPath predicates) | `abi.rs:2105`, `calls.rs:278` |
| 24 | `Array#fill` with a Range panics; negative or nil start wrong | concurrent-ruby | `array.rs:1420` |
| 25 | `Location#absolute_path` returns a relative path (Rails engines can't find their root) | railties | `compat.rs:130` |
| 26 | `gsub` with a block and interpolation drop a non-UTF-8 encoding (ISO-8859-1 mail subjects garbled) | mail | `string.rs:370`, `regex.rs:476`, `abi.rs:465` |
| 27 | Native Nokogiri has no SAX parser; `require "nokogiri"` works without it in the lock | nokogiri, aws-sdk-core | `lib/kenny/nokogiri/xml/sax/parser.rb` |
| 28 | `pack("m")` returns BINARY, not US-ASCII | base64 | `pack.rs` |
| 29 | Forwardable accessor `:'Mod::Name'` / `:"self.class"` unsupported | faraday | `lib/kenny/forwardable.rb:7` |
| 30 | Bundler runtime: no `Bundler.definition`, `locked_gems`, `LockfileParser`; `specs` is a plain Array | bundler | `lib/kenny/bundler.rb` |
| 31 | `NIO::ByteBuffer#size`, `FFI.type_size` missing | nio4r, ffi | `lib/kenny/nio.rb`, `lib/kenny/ffi.rb` |
| 32 | `require` of a runtime-compiled file again after deleting it from `$LOADED_FEATURES` returns false and doesn't run it (Zeitwerk's `reload`); `autoload` of an absolute path that wasn't compiled in raises LoadError instead of compiling it like `require` does | zeitwerk | `kernel.rs` require / autoload |

Smaller: `Module.nesting` is always `[]`; backtrace labels of code in a file required from
inside a class get that class prepended (`Outer::Lib2.boom`); `RubyVM` is absent though
`RUBY_ENGINE == "ruby"` (aws-sdk-core checks `RubyVM.const_defined?(:YJIT)`, so an empty
module would do); rspec's failure report uses Ripper (by design, but every failing example
raises while being reported).
