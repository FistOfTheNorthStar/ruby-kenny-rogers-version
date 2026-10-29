# Top 100 gems under Kenny

Smoke tests for the 100 most-downloaded gems on rubygems.org (all-time, October 2026). Each
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

## Results (Ruby 3.4.12, 2026-10-29): 84 of 100 pass

40 passed when the suite was added; the fixes on the `gem-fixes-1` branch brought it to
76, RSA keys, the SSL/X509 names and an empty `RubyVM` (`openssl-pkey`) to 81, and Nokogiri's
SAX parsers (`nokogiri-sax`) to 84.

Pass: actioncable, actionmailer, actionpack, actionview, activejob, activemodel, activerecord, activestorage, activesupport, addressable, ast, aws-eventstream, aws-partitions, aws-sdk-core, aws-sdk-kms, aws-sdk-s3, aws-sigv4, base64, bigdecimal, builder, bundler, coderay, concurrent-ruby, connection_pool, crass, diff-lcs, docile, domain_name, dotenv, faraday, faraday-net_http, globalid, googleauth, http-cookie, httpclient, i18n, jmespath, jwt, loofah, mail, method_source, mime-types, mime-types-data, mini_mime, mini_portile2, minitest, multi_json, multi_xml, multipart-post, netrc, nokogiri, parallel, parser, public_suffix, racc, rack, rack-protection, rack-test, rails, rails-dom-testing, rails-html-sanitizer, railties, rainbow, rake, rb-fsevent, regexp_parser, rexml, rspec-core, rspec-expectations, rspec-mocks, rspec-support, ruby-progressbar, rubyzip, signet, sprockets, sprockets-rails, thor, thread_safe, tilt, tzinfo, unf, unicode-display_width, websocket-extensions, zeitwerk.

### Still failing

| Gems | Cause |
|---|---|
| puma | Left out by design: the binary is the server |
| ffi | Calling C functions through FFI is dropped (docs/LANGUAGE.md); also `FFI.type_size` |
| simplecov | No `Coverage` |
| erubi, websocket-driver | String literals are frozen by design (docs/LANGUAGE.md). erubi sees `eval("''").frozen?` and leaves `.freeze` out of its generated code; `WebSocket::Mask.mask` writes into the caller's literal, which CRuby 3.4 still allows |
| rspec | Its failure report uses Ripper (not supported) |
| excon, redis, nio4r | Sockets: no `Socket.sockaddr_in`, raw `Socket.new`, `UNIXSocket.pair` |
| msgpack, unf_ext, rubocop-ast, rubocop | Gems' C extensions aren't built from the lock (rubocop and rubocop-ast need prism's) |
| json | The native JSON follows json 2.9 whatever the lock says: no `JSON::Coder`, `allow_trailing_comma`, `allow_comments`, `load_file` options |
| rubygems-update | Building a `.gem` writes the spec with Psych's tree API (`Psych::Visitors::YAMLTree`), which the native Psych lacks |
| pry | Eval'd code is labelled `<eval>` in backtraces (CRuby: the calling frame's label) |

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

Not yet: OpenSSL TLS/X509 certificates/EC keys/ASN1, sockets, C extensions from the lock, json
3.x, Psych's tree API, `Module.nesting` (always `[]`).
