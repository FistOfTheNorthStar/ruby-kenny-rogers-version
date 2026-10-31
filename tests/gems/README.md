# Top 100 gems under Kenny

Smoke tests for the 100 most-downloaded gems on rubygems.org (all-time, October 2026), and 34
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

## Results (Ruby 3.4.12, 2026-10-30): 131 of 134 pass

40 passed when the suite was added; the fixes on the `gem-fixes-1` branch brought it to
76, RSA keys, the SSL/X509 names and an empty `RubyVM` (`openssl-pkey`) to 81, Nokogiri's
SAX parsers (`nokogiri-sax`) to 84, and `gem-fixes-2` (sockets, json 3.x, Psych's tree API,
Prism and Ripper, C extensions from the lock, chilled string literals, `Coverage`) to 97.
`gem-fixes-3` added 34 gems (C extensions: bcrypt, oj, pg, ox, redcarpet, google-protobuf;
and sequel, webmock, faker, factory_bot, graphql, sidekiq, sinatra, haml, slim, liquid,
jbuilder, kramdown, rouge, rack-cors, rack-attack, httparty, rest-client, money, pundit, pagy,
aasm, chronic, dry-struct, dry-validation, hashie, mustermann, prawn, sorbet-runtime), all
passing.

Built against Ruby 4.0.9 (`KENNY_RUBY=/path/to/ruby-4.0/bin/ruby tests/gems/run.sh`, with each gem
directory's `bundle install` run for that Ruby) the result is the same: 131 of 134, the same three failing.

Pass (the top 100): actioncable, actionmailer, actionpack, actionview, activejob, activemodel, activerecord, activestorage, activesupport, addressable, ast, aws-eventstream, aws-partitions, aws-sdk-core, aws-sdk-kms, aws-sdk-s3, aws-sigv4, base64, bigdecimal, builder, bundler, coderay, concurrent-ruby, connection_pool, crass, diff-lcs, docile, domain_name, dotenv, erubi, excon, faraday, faraday-net_http, globalid, googleauth, http-cookie, httpclient, i18n, jmespath, json, jwt, loofah, mail, method_source, mime-types, mime-types-data, mini_mime, mini_portile2, minitest, msgpack, multi_json, multi_xml, multipart-post, netrc, nio4r, nokogiri, parallel, parser, public_suffix, racc, rack, rack-protection, rack-test, rails, rails-dom-testing, rails-html-sanitizer, railties, rainbow, rake, rb-fsevent, redis, regexp_parser, rexml, rspec, rspec-core, rspec-expectations, rspec-mocks, rspec-support, rubocop, rubocop-ast, ruby-progressbar, rubygems-update, rubyzip, signet, simplecov, sprockets, sprockets-rails, thor, thread_safe, tilt, tzinfo, unf, unf_ext, unicode-display_width, websocket-driver, websocket-extensions, zeitwerk.

### Still failing

| Gems | Cause |
|---|---|
| puma | Left out by design: the binary is the server |
| ffi | Calling C functions through FFI is dropped (docs/LANGUAGE.md); also `FFI.type_size` |
| pry | Eval'd code is labelled `<eval>` in backtraces (CRuby: the calling frame's label) |

## Large applications

Run by hand (their setups need databases, Redis and local edits), before the fixes on
`gem-fixes-3` that they led to:

| Application | Under Kenny |
|---|---|
| Rails' own tests (activesupport, activemodel, actionview, activerecord on SQLite) | 9,242 tests pass, against CRuby's 9,896 |
| Jekyll | its Minitest suite runs, and its docs site builds about 30% faster than under CRuby |
| Mastodon | Rails and RSpec boot; the validator specs pass one file at a time (103 of 103); a run of several files crashes in the GC |
| Discourse | doesn't boot yet |

Still open from them: the GC crash in long multi-file runs (Mastodon); C extensions that
define the same C symbol (rinku and redcarpet both vendor Sundown's `buffer.c`); Gemfile
`if ENV[...]` branches and `optional: true` groups, which the build takes as always on; a
compile-time `require` cycle that bakes a `require` as `false`; `require` of a project file
through a `$LOAD_PATH` entry added at runtime; literal `require`s skip `Kernel#require`
overrides and aren't listed in `$LOADED_FEATURES`; `__callee__` through an alias of a
plain `def`; `st_foreach`/`st_table` (stackprof); `rb_fdset_t` (hiredis, eventmachine).

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
