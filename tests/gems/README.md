# Top 100 gems under Kenny

Smoke tests for the 100 most-downloaded gems on rubygems.org (all-time, October 2026), and 37
more that large Ruby applications depend on. Each
`<gem>/` has a `Gemfile` naming that gem, its `Gemfile.lock`, and a `test.rb` that uses the
gem's main API offline. `run.sh` runs each test under CRuby and as a Kenny binary
(`kenny build`, then the binary); a gem passes when the two outputs (stdout, and stderr
compared apart) are identical.

```
tests/gems/run.sh                 # all gems (KENNY_GEMS_TIMEOUT=900 for the Rails ones)
tests/gems/run.sh rack json       # some
```

Outputs land in `target/gems/<gem>.{ruby,build,kenny}` (stderr in `.ruby.err`, `.kenny.err`).
The gems must be installed for the Ruby the tests run against (`bundle install` in a gem's directory).
`pg` and `sidekiq` need a server: PostgreSQL on its default socket, Redis on `localhost:6379`.

## Results (Ruby 3.4.12, 2026-11-01): 135 of 137 pass

40 passed when the suite was added; the fixes on the `gem-fixes-1` branch brought it to
76, RSA keys, the SSL/X509 names and an empty `RubyVM` (`openssl-pkey`) to 81, Nokogiri's
SAX parsers (`nokogiri-sax`) to 84, and `gem-fixes-2` (sockets, json 3.x, Psych's tree API,
Prism and Ripper, C extensions from the lock, chilled string literals, `Coverage`) to 97.
`gem-fixes-3` added 34 gems (C extensions: bcrypt, oj, pg, ox, redcarpet, google-protobuf;
and sequel, webmock, faker, factory_bot, graphql, sidekiq, sinatra, haml, slim, liquid,
jbuilder, kramdown, rouge, rack-cors, rack-attack, httparty, rest-client, money, pundit, pagy,
aasm, chronic, dry-struct, dry-validation, hashie, mustermann, prawn, sorbet-runtime), all
passing. `gem-fixes-4` added eventmachine, hiredis and rinku (with redcarpet in one binary), and
pry passes.

Built against Ruby 4.0.9 (`KENNY_RUBY=/path/to/ruby-4.0/bin/ruby tests/gems/run.sh`, with each gem
directory's `bundle install` run for that Ruby), before `gem-fixes-4`, the result was the same: 131 of 134,
the same three failing.

All 137 also run with no crash and the same output under `KENNY_GC_MIN=300`, `KENNY_GC_MIN=1500`,
`KENNY_GC_VERIFY=1` and `KENNY_GC_COMPACT=always KENNY_GC_COMPACT_VERIFY=1` (macOS, Ruby 3.4.12).

Pass (the top 100): actioncable, actionmailer, actionpack, actionview, activejob, activemodel, activerecord, activestorage, activesupport, addressable, ast, aws-eventstream, aws-partitions, aws-sdk-core, aws-sdk-kms, aws-sdk-s3, aws-sigv4, base64, bigdecimal, builder, bundler, coderay, concurrent-ruby, connection_pool, crass, diff-lcs, docile, domain_name, dotenv, erubi, excon, faraday, faraday-net_http, globalid, googleauth, http-cookie, httpclient, i18n, jmespath, json, jwt, loofah, mail, method_source, mime-types, mime-types-data, mini_mime, mini_portile2, minitest, msgpack, multi_json, multi_xml, multipart-post, netrc, nio4r, nokogiri, parallel, parser, public_suffix, racc, rack, rack-protection, rack-test, rails, rails-dom-testing, rails-html-sanitizer, railties, rainbow, rake, rb-fsevent, redis, regexp_parser, rexml, rspec, rspec-core, rspec-expectations, rspec-mocks, rspec-support, rubocop, rubocop-ast, ruby-progressbar, rubygems-update, rubyzip, signet, simplecov, sprockets, sprockets-rails, thor, thread_safe, tilt, tzinfo, unf, unf_ext, unicode-display_width, websocket-driver, websocket-extensions, zeitwerk.

### Still failing

| Gems | Cause |
|---|---|
| puma | Left out by design: the binary is the server |
| ffi | Left out by design: calling C functions through FFI is dropped (docs/LANGUAGE.md) |

## Large applications

Run by hand (their setups need databases, Redis and local edits), on Linux (Docker, Ruby 4.0),
at the end of `gem-fixes-4`:

| Application | Under Kenny | Under CRuby |
|---|---|---|
| Mastodon: spec/models, services, lib, helpers, policies, presenters, serializers and workers in one run | 5,025 examples, 74 failures | 33 failures |
| Rails 8.1.4's activesupport tests, one binary | 5,052 runs, 42 failures, 43 errors | 5,134 runs, 0 failures, 2 errors |
| Discourse (test environment, PostgreSQL) | boots; creates users and categories | — |

Each of the 45 Mastodon examples that fail only under Kenny needs something left out by design:
libvips through `ruby-vips` (35), llhttp through FFI (6), and text in Shift_JIS, KOI8-R and
Windows-125x (4).

Of activesupport's, 24 are its evented file watcher (rb-inotify, through FFI) and 4 GB18030
or Shift_JIS text; `string_ext_test.rb` doesn't load (EUC-JP at class level, so those 82 runs
are missing), and the redis cache store tests were left out. The rest is a tail of small
differences: `eval` of a string against a block's binding, `ActiveSupport::Multibyte::Chars`,
`Logger` levels, process allocation counts in notifications, and others.

Discourse: `Bundler.require`, the database, initializers and models work; it was run with
`config/initializers/003-vips.rb` skipped (it needs libvips 8.18) and without plugins. Rendering
a post (`PostCreator`) needs Discourse's JavaScript bundle (`pnpm install`), which wasn't built
here, so its Markdown through `mini_racer` is untried. CRuby couldn't run it in that container
(mini_racer's V8 needs a newer glibc).

From before `gem-fixes-3` (not run again): Rails' activemodel, actionview and activerecord
tests on SQLite, and Jekyll, whose Minitest suite runs and whose docs site builds about 30% faster
than under CRuby.

Merged in from main after those runs (each with a test in tests/ruby or tests/bundle):
- The GC crash in long multi-file runs (Mastodon): a minor collection ran C mark functions on
  freed data; Method objects didn't keep a `define_method` body's block alive; `clone` lost
  the singleton class.
- C extensions that define the same C symbol (rinku and redcarpet both vendor Sundown's
  `buffer.c`) link together: each extension's objects are combined with `ld -r`, and only
  its `Init_` functions stay global.
- The Gemfile is evaluated by Bundler's DSL at build time, so `if ENV[...]` branches and
  other conditions count as Bundler counts them. `optional: true` groups are left out
  unless `BUNDLE_WITH` names them.
- A compile-time `require` cycle runs the file when it is entered at runtime; before, the
  `require` was baked as `false`.
- `require` finds project files through `$LOAD_PATH` entries added at runtime, and `-I`
  directories are on `$LOAD_PATH`.
- Literal `require`s go through a `Kernel#require` override once one is defined, and are
  listed in `$LOADED_FEATURES` by absolute path.
- `__callee__` through an alias of a plain `def` returns the alias's name.
- From Discourse: the app's own `-I` directories count as the app; a `def` in a block at a file's top level
  (`ActiveSupport.on_load`) defines on the receiver of `instance_eval`/`class_exec`;
  `next`/`break` run the `ensure` clauses they leave; JIT code and data stay within ADRP
  range of each other; `require` skips a compiled-in file the program listed in
  `$LOADED_FEATURES` itself; `require_relative` of a C extension; `require "rubygems"` doesn't
  compile in the stdlib's RubyGems; `obj.foo(*args)` doesn't reach a private method;
  YAML flow collections indented as far as their key; `String#scrub` with a replacement;
  `Array#flatten` of objects with `to_ary`; case mapping with `:turkic`/`:ascii`/`:fold`;
  `grapheme_clusters` by Oniguruma's `\X`; ASCII-only `\w \d \s` in regexps; Nokogiri SAX
  parsers given an `Encoding`; `map` in a config.ru.
- A trailing Hash is never taken as keywords unless it was passed as them (as in CRuby;
  this replaces main's rule for a trailing Hash in dependency code).

Still open: stackprof, which has `st_table` now but needs the profiling API too
(`rb_profile_frames`, postponed jobs, and `rb_tracepoint_*`, which are dropped with TracePoint),
so Discourse's production boot stops there.

## Fixed

The bugs found when the suite was added, all fixed on `gem-fixes-1` except where noted:

- `require_relative` with a computed path; `require` of a file again after deleting it from
  `$LOADED_FEATURES` (Zeitwerk's reload); `autoload` of a file that isn't compiled in.
- `ENV`'s Hash methods (`include?`, `update`, `delete`, `each`, ...), which also made
  `OptionParser#parse!` fail.
- `Method`/`Proc#source_location`.
- Regexps on binary strings: `StringScanner#scan(/../n)` panicked; a non-`/n` regexp matched
  UTF-8 characters in a binary string.
- `Struct.new` on a Struct subclass; `warn` under `$VERBOSE = nil`; `Float::MAX_10_EXP` and
  friends; missing `Errno` codes.
- `Gem::Deprecate`, `Gem::Platform`, `Gem::Version`/`Requirement`/`Dependency`/`Specification`
  are RubyGems' own; RbConfig's install-layout keys; `gem_original_require`.
- Autoload: `defined?` during the autoload's own file; `class X` over a pending autoload with a
  `require` override (Zeitwerk); `const_get("A::B::C")` through autoloads.
- `format`'s full syntax (`%<name>s`, `%{name}`, `%1$s`, flags, precision, Bignums).
- `require_relative "x.tab"` finds `x.tab.rb`.
- `super` in a block inside a `define_method` body; a new singleton class sees its object's
  class methods.
- `String#lines`/`each_line` separators and `chomp:`; `Array#fill` with a Range.
- `Thread#group` and `ThreadGroup` (the timeout gem).
- `exit` raises `SystemExit`.
- bigdecimal 4.x arithmetic when the lock has it; `BigDecimal#to_r`. (json 3.x: not yet.)
- `method_added` for `define_method`, `attr_*`, `alias_method`; `public_methods(false)`.
- Refinements: lookup by the receiver's ancestors; active inside `refine` blocks.
- `Location#absolute_path`; backtrace labels of files required from inside a class.
- `gsub`/`sub` keep a non-UTF-8 encoding; `pack("m")` is US-ASCII.
- Forwardable accessors like `:"self.class"` and constant paths.
- Bundler's `LockfileParser`, `current_dependencies`, requirements; `Gem::Specification.find_by_name`.
- `eval`/`class_eval`/`binding` see the caller's local variables (a snapshot) and take
  `file`/`line`; a top-level `BEGIN` runs first.
- `OpenSSL::Digest.file`/`#file`/`#new`.
- OpenSSL: `PKey::RSA` (generate, PEM/DER in PKCS#1/PKCS#8/SPKI, PKCS#1 v1.5 and PSS
  signatures, `params`), `PKey.read`, `BN`, and the `SSL`/`X509` constants and classes gems
  name when loading (TLS, certificates and EC keys: not yet).
- Nokogiri's SAX parsers (`XML::SAX::Parser`, `PushParser`, `HTML4::SAX::Parser`), which the
  AWS SDK parses its XML responses with; HTML4 documents that start with text get libxml2's
  implied `<p>`.

On `gem-fixes-2`:

- Sockets: `Socket.new` and the `sockaddr_*` helpers, connect/bind/listen/accept and their
  `_nonblock` forms, `Addrinfo`, `Socket::Option`, `UNIXSocket`/`UNIXServer`, `Socket.pair`,
  `IO::EAGAINWaitReadable` and friends, `IO#wait_readable`/`wait_writable` (excon, redis,
  nio4r).
- JSON follows the json version the program gets (the lock's, else the Ruby's): json 3.x's
  parser and generator, `JSON::Coder`, `JSON::Fragment`, the new options and error messages.
- Psych's tree API (`Psych.parse`, `Psych::Nodes`, `YAMLTree`, `Emitter`) is psych's own Ruby
  code on native parse events and libyaml's emitter (rubygems-update builds a `.gem`);
  `pack` of nil, `String#sum`'s bits.
- Prism at runtime on the compiler's libprism (rubocop, rubocop-ast); Ripper on
  `Prism::Translation::Ripper` (rspec's failure snippets), linked in only when the code uses
  Ripper's parser.
- C extensions of locked gems (C and C++) are compiled into the binary (msgpack, unf_ext,
  websocket-driver's mask), with a much larger C API.
- String literals are chilled as in Ruby 3.4 in files without a `frozen_string_literal`
  comment (erubi, websocket-driver); `Hash#[]=` copies only new String keys; `clone(freeze:)`.
- `Coverage` (lines, branches, methods, oneshot_lines), compiled in only for programs that
  require it (simplecov); `Range#minmax` on endless ranges, `Array#sum` with NaN/Infinity.

On `openssl-tls`:
- OpenSSL TLS (`SSLContext`, `SSLSocket`, `SSLServer`, `Session`) on rustls, with CRuby's own
  `openssl/ssl.rb` and `openssl/buffering.rb` on top: verification with OpenSSL's errors and
  alerts, `verify_callback`, hostname checks, client certificates, ALPN, session resumption
  (Net::HTTP's), non-blocking IO. TLS 1.2/1.3 with rustls' AES-GCM/ChaCha20 suites.
- `OpenSSL::ASN1`, `OpenSSL::X509` (certificates, names, extensions, CSRs, CRLs, a `Store`
  that verifies as OpenSSL 3.6), `PKey::EC` public keys; CRuby's `openssl/x509.rb` on top.
- Fixes found on the way: `Zlib::Inflate#inflate`/`Deflate#deflate` with a block (Net::HTTP's
  gzip bodies), `String#index`/`rindex` offsets in binary Strings (chunked responses),
  `IO.select` on objects with `#to_io`, `defined?(obj.private_method)`.

On `ec-keys`:
- `OpenSSL::PKey::EC` on P-256, P-384, P-521 and secp256k1: `generate`, SEC1/PKCS#8/SPKI keys,
  ECDSA (`sign`/`verify`, `dsa_sign_asn1`), ECDH (`derive`, `dh_compute_key`), `Group` and
  `Point` arithmetic; Ed25519 keys (`PKey.generate_key("ED25519")`, raw keys); certificates,
  CSRs and TLS servers with EC and Ed25519 keys. jwt's ES256/ES384/ES512/ES256K and JWKs.
- Fixes found on the way: `String#ljust`/`rjust`/`center` on binary Strings (jwt pads ECDSA
  signatures with them), `OpenSSL::Digest` is a `Digest::Class` and `require "openssl"`
  loads `digest` (jwt's JWK thumbprints use `::Digest`).

On `gem-fixes-3`, from the new gems and from running large applications (Rails' own test
suites, Jekyll, Mastodon, Discourse) under Kenny:

- C API: `rb_call_super`, `rb_integer_pack`/`unpack`, `rb_io_wait`, `rb_ivar_foreach`,
  `rb_range_beg_len`, `rb_reg_new_str`, `rb_obj_encoding`, `rb_struct_alloc_noinit`,
  `rb_define_class_id`, `rb_memhash` and the `st_hash` family, `rb_str_to_inum`, `rb_isspace`
  and friends, `ruby/internal/symbol.h`; `rb_respond_to` ignores private methods; strings C
  code fills through `RSTRING_PTR` keep their bytes; `__FILE__` is the source's own name.
- Build: files that C code `rb_require`s are compiled in; path-source gems in subdirectories
  (rails/rails' layout); platform gem directories (pg-1.6.3-arm64-darwin), built from the C
  sources they ship; per-language `-std=` flags; a runtime `require` that two load paths
  could satisfy takes the earlier one, the same in every build.
- `super` from a module that appears twice in the ancestors; protected methods refused from
  outside; `Module.nesting`; constant autoloads resolved by the code's lexical scope
  (`module A::B` doesn't see A's); `remove_const` of a pending autoload.
- `$~` is per method; a bare `rescue` is `::StandardError`; `__method__`/`__callee__` in
  `define_method` bodies, also when the block reaches `define_method` through `&blk`
  (sorbet-runtime); frozen modules (`Module#freeze`).
- Strings: Windows-1252 and binary strings keep their encoding through the String methods
  and interpolation; `String#upto`; `Regexp.union`'s edge cases and `/` escaping in `to_s`;
  `Regexp.timeout` and `Regexp::TimeoutError`; a binary `StringIO` takes text as bytes.
- Ranges, Comparable and Numeric: String ranges' `include?`, Range/Array subclasses' `new`,
  `Range#cover?(range)`, `clamp`, comparison error messages, `min`/`max` order, Numeric
  subclasses, `Time` with zones and Rationals.
- Runaway recursion raises SystemStackError in under a second (the main thread's stack had
  1 GB usable, so it took minutes).
- Mastodon's multi-file RSpec runs: a minor collection ran C mark functions on freed data
  (pg's type maps), Method objects let a removed `define_method` method's block be freed
  (rspec-mocks keeps them), and `clone` dropped the singleton class (rspec's composed DSL
  matchers).
- Also: `Hash#merge` keeps the subclass, an overridden `Hash#default`, `Integer#[]`'s forms,
  `const_source_location`, `Binding#source_location`, `Proc#binding` locals,
  `Module#set_temporary_name`, `IO#wait`, `fcntl`, `ruby2_keywords`, `$LOADED_FEATURES` order
  and hand-added entries, `Hash/Array/String#initialize` return self,
  `each_grapheme_cluster.size`, `Forwardable.debug`, `Socket.tcp_server_sockets`, BOM-aware
  reads, `Dir.glob` with `FNM_DOTMATCH`, `Thread.abort_on_exception`.

Not yet: OpenSSL DSA/DH and X25519/Ed448 keys, encrypted private keys, TLS session
serialization (`Session#to_der`),
UDP `sendto`/`recvfrom`,
`JSON::ResumableParser`, C extensions that use CRuby's internal headers (rbs).

On `gem-fixes-4`:
- A GC bug: a `Method`/`UnboundMethod` whose method left its class (`define_method`, then
  `remove_method` or a new definition: Sinatra's routes) didn't keep its block alive or see it
  move, so the object could call whatever took the block's place, or freed memory.
- `require`: a cycle the build met from one side runs in the order the program reaches it (it was
  compiled as `false`); literal `require`s go through an overridden `Kernel#require`
  with the name as written (`require_relative` doesn't, as in CRuby); `$LOADED_FEATURES` lists
  all of them by absolute path, once they have run; a file in a `$LOAD_PATH` directory added at
  runtime is found (compiled in, or by the runtime compiler if it is on disk).
- `__callee__` through an alias of a plain `def`.
- Frames of `eval`'d code carry the label of the frame they run in, or of a Binding's, and
  without a `file` its name is `(eval at file:line)`, as since Ruby 3.3 (pry).
- Gemfile: `if`/`unless` on `ENV` (`ENV["X"]`, `ENV.fetch`, `key?`, `==`, `!`, `&&`, `||`) follow
  the build's environment; `group ..., optional: true` only with `BUNDLE_WITH`. Other
  conditions still take both branches, and the lock decides.
- C extensions: each is linked with only its `Init_` functions exported, so two may define the
  same C symbol; static libraries an extconf names in `LDFLAGS` are linked (hiredis). C API:
  `st_table`, `rb_fdset_t` and `rb_thread_fd_select`, `rb_wait_for_single_fd`, `RB_ALLOCV`,
  `rb_thread_alone`, `rb_ary_aref`, `rb_rescue` with `ANYARGS` functions.
- `SystemCallError.new(msg, errno, func)` makes the Errno class of that number; `Socket.tcp` and
  `Addrinfo#connect` with a timeout raise a refused connection (they returned the socket).
- `FFI.type_size`.

Found by running Mastodon, Discourse and Rails' activesupport tests again (also `gem-fixes-4`):
- A crash (webmock, in Mastodon's specs): freeing the code of an `eval` unpinned frozen
  literals it shared with compiled code, which then read whatever took their place. Interned
  literals now live as long as the program.
- Keywords: a Hash passed as a positional argument to a gem's (or `eval`'d) method with
  `*args, **kw` became the keywords (a Ruby 2 rule kept for old gems); it stays positional, as
  in CRuby (`Minitest::Mock#expect` with a Hash argument).
- `fork` with threads running (Puma's and ActiveSupport's fork tests, Discourse's
  `Process._fork` hook): the GC stops the other threads first, and the child resets the
  runtime's locks, its threads (only the forking one survives) and the GC's freeing thread
  (on macOS the child crashed on the parent's channel). `Process._fork` is called by `fork`
  and can be overridden.
- `SIGPIPE` is ignored, so writing to a closed pipe raises `Errno::EPIPE`.
- `public`/`private` of an inherited method with the visibility it already has doesn't copy it
  into the class (rspec-mocks restored the copy, not the stub's target).
- `Thread::SizedQueue#close` wakes a `push` waiting for room (ClosedQueueError).
- `IO::TimeoutError`.
- `eval` of a string runs in the class body its call site is in, also from a proc run with
  `instance_exec`: `class X` in it defined X at the top level, and `Module.nesting` was empty.
- `DateTime#to_time` keeps the sub-second fraction exactly; `Date#to_time` of a Julian date is
  that day; `DateTime == Time` goes through `<=>` (ActiveSupport defines it); `Date.new` takes
  at most four arguments; Date arithmetic takes a Rational with Bignum parts
  (`Rational(0.1, 86400)`).
- `Time.local` of an ambiguous local time (clocks going back) is the later instant, unless the
  ten-argument form's `isdst` picks the other; a skipped one moves forward. A Time in a timezone
  object (`Time.new(..., zone)`) keeps it through `+`, `-` and `succ`, and `%Z` is its `abbr`.
  A field out of range raises CRuby's message ("argument out of range" when it doesn't fit).
- Strings: `prepend`, `insert` and `replace` take the encoding CRuby does; `gsub!` without a
  block or replacement returns an Enumerator.
- `Array#flatten` converts elements with `to_ary` (ActiveSupport's
  `Inflections::Uncountables`); `Hash[[[key]]]` is `key => nil`; `Hash#default(key)` runs the
  default proc; Enumerable methods (`count`, `entries`, ...) of a Hash or Array subclass that
  redefines `each` go through it (ActiveSupport's InheritableOptions); `transform_keys(nil)` is a
  TypeError; `Range#cover?` of a range by an endless range.
- `pp` prints a Hash as Ruby 3.4 inspects it (`{one: 1}`).
- YAML: `!omap` loads as a `Psych::Omap` (ActiveSupport::OrderedHash); a quoted scalar's
  continuation lines may be indented as little as its key, as libyaml allows (Discourse's
  locale files).
- Bundler: `BUNDLE_PATH` (in the environment or `.bundle/config`) is where the gems are found;
  `Gem.path` lists only installed gems' directories, not the parents of path gems (a Rails
  checkout's), which made ActiveSupport's FileUpdateChecker skip the app's files.
- Overlays that stand in for gems Kenny can't link: `mini_suffix` (on public_suffix) and
  `mini_racer` (on the JavaScript engine ExecJS uses; see docs/LANGUAGE.md). An overlay file
  that shadows a gem's file is compiled in when the gem is.
