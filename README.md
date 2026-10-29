<img width="2752" height="1536" alt="Gemini_Generated_Image_obri9hobri9hobri" src="https://github.com/user-attachments/assets/72be7f46-324f-41bf-98a5-ff60c31325e7" />

# Kenny — Ruby, compiled

DHH said go AI delerium, but you also like Ruby. This is in pre-alpha testing, DO NOT RUN THIS IN PRODUCTION, it needs serious work, that said please continue.

Kenny compiles Ruby to a single native executable, like Go. You keep Ruby's syntax and drop
a few rarely used runtime features (see [docs/LANGUAGE.md](docs/LANGUAGE.md)). The output
runs without an interpreter or VM, and includes its own HTTP server.

```
cargo build --release
./target/release/kenny run examples/hello.rb
./target/release/kenny build examples/hello_rack/config.ru -o server && ./server

# a Rails 8.1 app (Zeitwerk, Solid Cache/Queue/Cable on SQLite, Propshaft, importmap, Turbo),
# built with Ruby 3.3+ and the app's gems installed; its gems come from its Gemfile.lock, and
# `kenny build` embeds the app's files (views, config, public/) in the binary. Its assets are
# precompiled with Ruby
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=x bin/rails db:prepare assets:precompile)
./target/release/kenny build examples/blog8/config.ru -o blog8
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=$(openssl rand -hex 64) ../../blog8)
ruby examples/blog8/compare.rb blog8   # 45 requests against Puma and the binary: identical
```

Kenny supports Ruby 3.4 and Ruby 4.0 (the repository's `.ruby-version` is 3.4.12) and builds
against an installed Ruby: the one `KENNY_RUBY` names, else the version in the program's
`.ruby-version` (or `.tool-versions`, next to it or above) found among rvm, rbenv, asdf, chruby
and mise installs, else `ruby` on PATH. That Ruby's standard library and gems are what goes into
the binary, and the program behaves as that version: built against 4.0 it gets `Set` and
`Pathname` as core classes, the `Ruby` module, `Array#rfind`, `String#strip` selectors and the
rest of 4.0's changes (`open("|cmd")` is gone, conversion error messages changed, the openssl
gem's new errors). Building against any other version prints a warning: the program gets
that Ruby's libraries but behaves as 3.4 (older) or 4.0 (newer).

When the program has a `Gemfile.lock` next to it (or in a parent directory), `kenny build`
resolves the locked gems itself from the installed gem directories (that Ruby's first, then
`GEM_HOME`/`GEM_PATH`, `~/.gem`, rvm, rbenv) and compiles them in; groups listed in
`BUNDLE_WITHOUT` (default `development:test`) are left out, and `require "bundler/setup"` /
`Bundler.require(*Rails.groups)` work in the binary. Without a lock file, `-I` adds load paths:
`tools/gem-paths.rb NAME VERSION` prints the `-I` flags for an installed gem and its
dependencies. Every file in those directories is compiled into the binary.

A Rack app's binary is self-contained: `kenny build` also embeds the app's directory (views,
`config/`, locales, `public/` with the precompiled assets) and every compiled gem's directory,
compressed (6.5 MB for `examples/blog8`). Runtime data is left out: `log/`, `tmp/`,
`node_modules/`, `storage/`, SQLite databases, and `config/master.key` (give it as
`RAILS_MASTER_KEY` or a file on the server). To deploy, copy the binary and start it in the
directory that holds its data:

```
mkdir -p /srv/blog8/storage && cp blog8 /srv/blog8/ && cp examples/blog8/storage/production*.sqlite3 /srv/blog8/storage/
cd /srv/blog8 && RAILS_ENV=production SECRET_KEY_BASE=... ./blog8
```

Embedded files are always read from the binary, as they were at build time. Everything else
under the app's directory (the database, `log/`, `tmp/`, uploads) lives in the directory the
binary was started in, or in `$KENNY_APP_ROOT`. Started inside its build directory (or above
it), the binary uses the build directory itself. Writing to an embedded file first copies it
there. `--embed <dir>` embeds a directory for any program; `--no-embed` turns embedding off.
In development (`RAILS_ENV`/`RACK_ENV=development`, started in its build directory) the app's
own files are read from disk instead, so edits show up; `KENNY_LIVE_FILES=1` turns that on
anywhere, `=0` off. The gems' files always come from the binary.

## How it works

| Layer | Technology |
|---|---|
| Parsing | [Prism](https://github.com/ruby/prism), Ruby's official parser, so the syntax is exactly Ruby's |
| Code generation | [Cranelift](https://cranelift.dev): each method, block and class body becomes a native function |
| Runtime | Rust (`crates/kenny-runtime`): object model, method dispatch, core classes |
| HTTP | Built into the runtime: a Rack `config.ru` is the entry point, no Puma needed |

Notable choices:
- **Tagged values** use the same layout as CRuby (Fixnum, Symbol, nil/true/false), so C extensions run on a thin API shim.
- **Floats are immediates ("flonums")** when their exponent is in range, with CRuby's encoding (low bits `10`, the
  double rotated by 3); +0.0 has its own flonum, and -0.0, NaN, Infinity and very large or small values stay heap
  objects. `ruby.h` mirrors the encoding (`FLONUM_P`, `RFLOAT_VALUE`, `rb_float_new`).
- **Integer and Float operators** (`+ - * / % < <= > >= == != & | ^`) are compiled inline: Fixnum math with an
  overflow check, Float math on unboxed doubles for any mix of Floats and Fixnums, falling back to a method call
  (Bignums, heap Floats, other types). Results of arithmetic stay unboxed through nested expressions and in
  locals assigned from arithmetic, and are boxed only when something needs the object. This needs no guards
  because redefining Integer/Float operators is not allowed (rule 2 in LANGUAGE.md).
- **Method calls** go through per-call-site inline caches, backed by a direct-mapped global method cache for
  polymorphic sites. Calls with up to two arguments and no block skip the argument buffer, and hot core methods
  (`Array#[]`/`[]=`/`<<`/`size`, `Hash#[]`/`[]=`/`key?`, `String#==`/`<<`, `Symbol#==`, `nil?`, ...) run
  without a method invocation when the cached method is the builtin one; a monkey patch, subclass override,
  singleton method or `method_missing` resolves to a different method and takes the normal path.
  Instance-variable reads and writes use per-site shape caches.
- **Blocks of `times`, `upto`, `downto`, `each`, `each_with_index`, `map`, `select`, `reject`, `any?`, `all?`** (on
  Integers, Arrays and Integer Ranges) are compiled as loops in the caller when the block is small and doesn't need
  its own frame (`return`, nested blocks, ...). A guard checks at runtime that the method is the builtin one;
  otherwise the call is made normally with the block as a closure.
- **Locals live in CPU registers.** Only variables that a block captures move to a heap environment, based on a closure-capture analysis.
- **Exceptions, `break` and non-local `return`** use one return-value check per call, with no stack unwinding tables.

## Status

Working:
- **Language:** everything in Ruby 3.4 and 4.0 syntax except the few runtime features dropped in
  [docs/LANGUAGE.md](docs/LANGUAGE.md). That includes pattern matching, argument forwarding,
  refinements, `redo`/`retry`, `Rational`, and keyword arguments on methods and blocks.
- **Garbage collection:** generational, and compacting when it pays. Most collections are
  minor: they mark only objects allocated since the last one, plus what write barriers recorded
  (stores into old Arrays, Hashes, objects, closure environments and class tables). Each OS
  thread allocates from its own 64 KB pages of fixed-size slots; a collection stops all
  threads, scans their stacks with their help, sweeps in parallel, and leaves most freeing
  (young pages, dead buffers) to a background thread after the pause. A minor pause in the
  blog example under load is under 1 ms. The stack is scanned conservatively and the heap
  precisely. A major collection can **move** plain objects, strings, arrays, hashes, ranges,
  floats and bignums out of sparsely used pages into gaps in fuller ones, rewrite the fields
  that refer to them, and return the emptied pages to the OS. What it can't update stays put:
  objects reached from a stack (also through a pointer into them), from a root or a C
  extension, objects with singleton classes, and objects whose `object_id` or identity hash
  was taken. Since native code may hold object references in Rust collections, objects move
  only while no thread is inside a native method (the scheduler's blocking calls excepted):
  `GC.compact` at top level, and an idle server's tidy-up a second after its last request,
  which also gives memory back (`KENNY_GC_COMPACT=0` turns it off). Singleton classes of dead
  objects are freed (`extend` per request doesn't grow memory).
- **Regular expressions** via Oniguruma, the engine family Ruby's own comes from: `$~`/`$1`,
  named captures, and the regex-aware `String` methods.
- **Threads and Fibers:** lightweight threads that run on all cores at once (M:N over one OS
  thread per core), with `Queue`, `SizedQueue`, `Mutex`, `ConditionVariable`, `Monitor`, a
  10 ms fairness timer, `Thread#raise`/`kill`, and deadlock detection. A core with nothing to
  run takes waiting threads from busy ones (work stealing), so threads move between cores.
  Waiting on a socket, pipe or `IO.select` lets the core's other threads run; file reads and
  writes and SQLite queries run as blocking calls, during which the core's waiting threads go
  to an idle (or spare) core and collections don't wait for them. No GVL: a data race can lose
  an update but never crashes the process ([docs/LANGUAGE.md](docs/LANGUAGE.md), rule 4).
  As in CRuby, a thread that reaches a constant another thread is autoloading waits until that
  file has run, and one IO object can be used by several threads at once.
  External enumerators (`next`) run on Fibers.
- **`eval` / `class_eval` / `instance_eval` with strings:** compiled at runtime by an embedded
  Cranelift JIT, linked in only when a program needs it, and cached by source. The cache keeps
  the 2,000 most recently used units (`KENNY_EVAL_CACHE`); an evicted unit's code and data are
  freed once nothing can run them (no method, block, frame or Method object refers to them), so
  code that evals a new string per request doesn't grow memory. `binding` captures `self` (not
  locals), so `ERB#result(binding)` works.
- **C extensions:** a CRuby-compatible `ruby.h` and C API. The C (and C++) extensions of the
  gems in Gemfile.lock are compiled into the binary, with the flags and `HAVE_*` defines of the
  Makefile their `extconf.rb` made (msgpack and unf_ext, for example), so `require` of the
  extension runs its `Init_` function; `kenny build --ext dir` adds one of your own. Gems the
  runtime has a native replacement for (nokogiri, json, date, sqlite3, ...) keep using it. An
  extension that doesn't compile or link against Kenny's API is left out with a warning
  (`require` of it then raises `LoadError`); `KENNY_NO_GEM_EXTS=1` leaves them all out.
- **Libraries:** `require` / `require_relative` / `autoload` / `load` compile gems and the stdlib
  into the binary (`-I dir`; the stdlib of the Ruby built against is found automatically, and
  without a Gemfile so are the gems that Ruby bundles in place of former stdlib parts, such as
  `csv` and `bigdecimal` in 3.4).
- **Native stdlib extensions** (Rust instead of C): date, json (2.x or 3.x, as the lock file
  says), yaml (Psych API; its tree API, `Psych.parse`, `Psych::Nodes`, `YAMLTree` ..., is psych's
  Ruby code on native parse events and a port of libyaml's emitter), bigdecimal,
  strscan, digest, zlib, openssl (digests, HMAC, ciphers, RSA keys and signatures; no TLS, X509
  certificates or EC keys), Marshal (byte-compatible format 4.8),
  cgi/escape, erb/escape (Rails' `html_escape`), plus `Gem::Version`/`Gem::Requirement`.
  Nokogiri is native too (html5ever and quick-xml underneath, its own XPath 1.0 engine, a SAX
  scanner that reports what libxml2's does, and nokogiri's Ruby layer on top), so loofah, the
  Rails HTML sanitizers and the AWS SDK's XML parsing work without libxml2.
  The `sqlite3` gem's C extension is native too (on the bundled SQLite library, for both the
  1.x and 2.x gem APIs); the gem's own Ruby files load on top of it.
- **Asset pipeline without Ruby or Node:** the `sassc` gem's API runs on a native Sass
  compiler ([grass](https://github.com/connorskees/grass)), with Sass functions such as
  `asset-path` calling back into Ruby, and ExecJS runs on an embedded JavaScript engine
  ([QuickJS](https://github.com/quickjs-ng/quickjs)), so ExecJS users such as `uglifier` and
  `terser` work in-process. `kenny build` of a Rails app with app/assets precompiles its assets (what
  `rails assets:precompile` does, run by the new binary itself) when public/assets has no
  manifest or the assets changed; `KENNY_PRECOMPILE=1` forces it, `=0` skips it. Both are
  linked only into programs that use those gems.
- **Rails:** a whole generated Rails 8.1 application boots and serves production traffic:
  `examples/blog8` is `rails new` (8.1.4) plus two scaffolds (posts, and
  comments that belong to them), with Zeitwerk eager loading, Solid Cache, Solid Queue and
  Solid Cable on SQLite, Propshaft, importmap, Turbo and Stimulus, Active Storage (libvips
  isn't installed, so it falls back as under Ruby) and jbuilder. `examples/blog8/compare.rb`
  runs a 45-request session (forms with CSRF tokens, validations, JSON views and a JSON API
  create, deletes, 404/422 pages, the import map's assets) against Puma and the binary: the
  responses are identical. Its assets are precompiled with Ruby (`rails assets:precompile`)
  before `kenny build`, since the build-time precompile above runs only for apps that use the
  sassc or ExecJS gems. Rack 2.2 and Rack 3 apps run, and the server supports Rack
  hijacking (full and partial), so WebSockets work.
- **Reflection:** method visibility (`private`/`protected`, enforced on explicit receivers),
  `Method`/`UnboundMethod`, `Module#dup`, `Rational`/`Complex`.
- **Backtraces:** full `caller`, `caller_locations` and `Exception#backtrace`, at no cost to
  running code: compiled code keeps frame pointers and the binary carries a table of each
  call's source line, so a backtrace is a walk of the native stack, taken only when needed.
- **Bundler, socket, FFI:** Bundler's runtime API on gems resolved at build time;
  the socket library: `TCPSocket`/`TCPServer`, `UNIXSocket`/`UNIXServer` (and `.pair`),
  `Socket` on sockaddr strings (`connect`/`connect_nonblock`/`bind`/`listen`/`accept`),
  `Addrinfo`, `Socket::Option` (UDP's `sendto`/`recvfrom` and `send_io`: not yet;
  name resolution blocks); FFI bindings load (their
  declarations work) but calling a C function through FFI raises NotImplementedError. As with
  the ffi gem, `ffi_lib` raises LoadError when the library isn't installed, so optional native
  dependencies (Active Storage's libvips) are skipped.
- **Prism and Ripper at runtime:** the prism gem's C extension is replaced by the libprism
  Kenny's compiler parses with (linked only into programs that load Prism or Ripper), so
  `Prism.parse`/`lex`/`dump`, rubocop-ast and RuboCop work. Its parse is serialized for the
  gem's Ruby library, whose version must be libprism's (1.9.0): `kenny build` uses the
  Gemfile.lock's prism, else an installed prism 1.9.0 gem in place of the standard library's
  (Ruby 3.4 ships 1.5), and otherwise `require "prism"` raises LoadError (with a build
  warning). Ripper runs on Prism (`Prism::Translation::Ripper` sends Ripper's parser events,
  `Prism.lex_compat` gives `Ripper.lex` its tokens), so `Ripper.sexp`, `Ripper.lex` and
  Ripper subclasses work; lexer states of some tokens (spaces, heredoc ends) can differ from
  CRuby's, and `Ripper#state`/`#token` are nil.
- **Built-in HTTP server** for Rack apps: one process serves requests on every core
  (`rack.multithread` is true); `WEB_CONCURRENCY=n` forks n processes instead. `rack.hijack`
  (full and partial) hands the connection's socket to the app.

Not yet:
- Sass source maps, and sassc-rails' ERB (`.scss.erb`) and glob (`@import "dir/*"`) imports:
  imports are resolved by the Sass compiler on the load paths.
- In development: debuggers (`byebug` loads, but calling `byebug` raises LoadError: its core
  is a C extension), and Action Cable's redis adapter (untested: the redis gem isn't installed
  here).
- Compacting while requests run: objects move only when no thread is inside a native method,
  which in a busy server is rare, so compaction mostly happens when the server goes idle (or
  on `GC.compact`). Precise stack maps would lift this (roadmap).
- Sockets and pipes are polled while other threads wait for them (2 ms steps), not parked on
  an event loop; the HTTP server's own connections use kqueue/epoll.
- Memory: serving on 12 cores takes about 1.7x the memory of one (each core has its own
  young-generation space, `KENNY_GC_YOUNG_SCALE=1` trades some throughput for ~1.3x); still
  far less than 12 Puma workers. The allocator choice was measured on macOS only (see
  `KENNY_MALLOC`).
- `ObjectSpace`, `TracePoint` (dropped by design).

## Early numbers (Apple M4 Pro, 12 cores, vs Ruby 3.4.12 and 4.0.7)

Speedup of the Kenny binary over CRuby on the same script, best of 3, wall time including
process start (about 27 ms for `ruby`, which flatters Kenny on the shortest runs). The YJIT
column is `--yjit`, except for `loop.rb`, `mandelbrot.rb` and `threads.rb`: they run their hot
loops at the top level, which YJIT compiles only with `--yjit-call-threshold=1` (that setting
slows the others: `methods.rb` runs 3x slower). Kenny is as fast built against 3.4 as
against 4.0.

| Benchmark | vs Ruby 3.4 | vs Ruby 4.0 | vs Ruby 4.0 + YJIT |
|---|---|---|---|
| Integer loop (`bench/loop.rb`) | 13x | 13x | 1.3x |
| Recursive `fib` (`fib.rb`) | 14x | 13x | 4.4x |
| Blocks (`blocks.rb`) | 5.8x | 5.2x | 4.8x |
| Collections (`collections.rb`) | 5.2x | 5.0x | 1.8x |
| String building and hashes (`strings_hash.rb`) | 4.9x | 4.4x | 3.1x |
| Float math: `mandelbrot.rb` | 43x | 41x | 28x |
| Float math: `spectral_norm.rb` | 8.7x | 7.0x | 1.7x |
| Float math: `nbody.rb` | 5.0x | 4.5x | 1.9x |
| Method-call-heavy OO code (`methods.rb`) | 6.6x | 6.5x | 0.86x (YJIT is faster) |
| Object allocation with GC (`objects.rb`) | 5.9x | 4.4x | 2.5x |
| 12 CPU-bound Ruby threads (`threads.rb`) | 59x | 62x | 26x (Ruby's GVL runs one at a time) |

Web (the machine was also running other work, so treat these as rough; `ab -c 10`/`-c 32`):

| App | One thread | All 12 cores, one process | Puma |
|---|---|---|---|
| Whole Rails 8.1 app (`examples/blog8`, SQLite, Solid Cache/Queue), scaffold index | 2.2k req/s, 119 MB | 6.7k req/s, 195 MB | 1.3k req/s, 71 MB (1 thread, Ruby 3.3); 4.8k req/s, 1,040 MB (12 workers) |

Run them yourself: `bench/run.sh` (`RUBY=/path/to/ruby`, `RUBY_OPTS=--yjit`). Tests: `tests/run.sh` (against Ruby 3.4), `tests/run.sh 4.0`
(against an installed Ruby 4.0; a test's `.4.0.out` holds what 4.0 prints differently). Compile-check a file without linking: `kenny check file.rb`.

GC tuning and diagnostics (environment variables read by compiled programs):
`KENNY_GC_STATS=1` prints collections, GC time and heap size at exit (`=2` after every
collection); `KENNY_GC_GROWTH` (default 2.0) sets how much a program may allocate between
collections, as a multiple of the live data; `KENNY_GC_MALLOC_LIMIT` (MB, default 32) starts a
collection once that much memory was malloc'd for object contents since the last one;
`KENNY_GC_MIN=500` makes a GC stress test; `KENNY_GC_GENERATIONAL=0` turns minor collections
off; `KENNY_GC_VERIFY=1` checks after every minor collection that nothing reachable was left
unmarked (a write-barrier test, slow); `KENNY_GC_SCAN_REGIONS=1` also scans compiled code's data
sections for roots (normally literals are pinned instead). `KENNY_GC_YOUNG_SCALE` (default 4)
caps how much the young generation grows with the number of cores (less memory, more pauses).
`KENNY_GC_COMPACT=0` never moves objects, `=always` tries at every major collection;
`KENNY_GC_COMPACT_STATS=1` reports each compaction (and why one couldn't run);
`KENNY_GC_COMPACT_VERIFY=1` checks every rewritten reference after compacting.
`KENNY_MALLOC=auto|system|mimalloc` picks the allocator behind Rust allocations: on macOS the
default `auto` uses mimalloc while one core runs Ruby code and the system allocator once
several do (it keeps far less memory resident when threads free each other's objects); elsewhere
mimalloc. `KENNY_EVAL_CACHE` (default 2000) sizes the cache of `eval`'d code. `KENNY_THREADS`
sets how many OS threads run Ruby threads (default: one per core, or one with
`RAILS_ENV=development`, whose code reloading counts on a GVL); `WEB_CONCURRENCY=n`
additionally forks n server processes (default 1). `KENNY_SCHED_STATS=1` prints how often
threads moved between cores (`KENNY_NO_STEAL=1` keeps them where they started, to compare); `KENNY_CRASH_TRACE=1` prints a native backtrace on a crash. `KENNY_NO_INLINE=1 kenny build ...` compiles without block inlining (to compare, or
to rule it out when debugging). Debugging: `KENNY_DEBUG_RAISE=1` logs every exception raised
(rescued or not) with its location; `KENNY_DEBUG_EVAL=1` logs every string compiled at runtime
by `eval`/`class_eval`; an uncaught exception and a failed request print the full backtrace.

## Roadmap

1. Direct calls between compiled methods (guarded by the inline cache) and inlining of small methods
2. An event-driven poller for Ruby-level sockets and pipes (park the thread, not poll)
3. Precise stack maps (Cranelift) for compiled frames and rooted native code, so compaction can
   run under load, not only when idle
4. Measure and tune memory and allocator choice on Linux
