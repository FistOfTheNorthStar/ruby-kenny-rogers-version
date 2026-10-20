# Kenny — Ruby, compiled

Kenny compiles Ruby to a single native executable, like Go. You keep Ruby's syntax and drop
a few rarely used runtime features (see [docs/LANGUAGE.md](docs/LANGUAGE.md)). The output
runs without an interpreter or VM, and includes its own HTTP server.

```
cargo build --release
./target/release/kenny run examples/hello.rb
./target/release/kenny build examples/hello_rack/config.ru -o server && ./server

# a Rails (ActionPack 5.2) JSON API as one native binary with its own HTTP server
./target/release/kenny build examples/rails_api/config.ru -o rails_api \
    $(ruby tools/gem-paths.rb actionpack 5.2.8.1) && PORT=3000 ./rails_api

# the same with ActiveRecord on SQLite
./target/release/kenny build examples/rails_api_ar/config.ru -o rails_api_ar \
    $(ruby tools/gem-paths.rb actionpack 5.2.8.1) $(ruby tools/gem-paths.rb activerecord 5.2.8.1) \
    $(ruby tools/gem-paths.rb sqlite3 1.7.3) && ./rails_api_ar

# a whole generated Rails 5.2 app (`rails new` + a scaffold); its gems come from its Gemfile.lock
(cd examples/blog && RAILS_ENV=production SECRET_KEY_BASE=x bundle exec rails db:migrate assets:precompile)
./target/release/kenny build examples/blog/config.ru -o blog
(cd examples/blog && RAILS_ENV=production RAILS_SERVE_STATIC_FILES=1 SECRET_KEY_BASE=$(openssl rand -hex 64) ../../blog)
ruby examples/blog/compare.rb blog   # the same session against Puma and the binary: identical
```

When the program has a `Gemfile.lock` next to it (or in a parent directory), `kenny build`
resolves the locked gems itself from the installed gem directories (`GEM_HOME`/`GEM_PATH`, the
system Ruby's `Gem.path`, `~/.gem`, rvm, rbenv) and compiles them in; groups listed in
`BUNDLE_WITHOUT` (default `development:test`) are left out, and `require "bundler/setup"` /
`Bundler.require(*Rails.groups)` work in the binary. Without a lock file, `-I` adds load paths:
`tools/gem-paths.rb NAME VERSION` prints the `-I` flags for an installed gem and its
dependencies. Every file in those directories is compiled into the binary.

## How it works

| Layer | Technology |
|---|---|
| Parsing | [Prism](https://github.com/ruby/prism), Ruby's official parser, so the syntax is exactly Ruby's |
| Code generation | [Cranelift](https://cranelift.dev): each method, block and class body becomes a native function |
| Runtime | Rust (`crates/kenny-runtime`): object model, method dispatch, core classes |
| HTTP | Built into the runtime: a Rack `config.ru` is the entry point, no Puma needed |

Notable choices:
- **Tagged values** use the same layout as CRuby (Fixnum, Symbol, nil/true/false), so a C-extension API shim stays possible later.
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
- **Language:** everything in Ruby 3.4 syntax except the few runtime features dropped in
  [docs/LANGUAGE.md](docs/LANGUAGE.md). That includes pattern matching, argument forwarding,
  refinements, `redo`/`retry`, `Rational`, and keyword arguments on methods and blocks.
- **Garbage collection:** generational mark-and-sweep that doesn't move objects. Most
  collections are minor: they mark only objects allocated since the last one, plus what write
  barriers recorded (stores into old Arrays, Hashes, objects, closure environments and class
  tables). Each OS thread allocates from its own 64 KB pages of fixed-size slots; a collection
  stops all threads, sweeps in parallel, and leaves most freeing (young pages, dead buffers)
  until after the pause. A minor pause in the blog example under load is about 0.7 ms. The
  stack is scanned conservatively and the heap precisely.
- **Regular expressions** via Oniguruma, the engine family Ruby's own comes from: `$~`/`$1`,
  named captures, and the regex-aware `String` methods.
- **Threads and Fibers:** lightweight threads that run on all cores at once (M:N over one OS
  thread per core), with `Queue`, `SizedQueue`, `Mutex`, `ConditionVariable`, `Monitor`, a
  10 ms fairness timer, `Thread#raise`/`kill`, and deadlock detection. No GVL: a data race can
  lose an update but never crashes the process ([docs/LANGUAGE.md](docs/LANGUAGE.md), rule 4).
  External enumerators (`next`) run on Fibers.
- **`eval` / `class_eval` / `instance_eval` with strings:** compiled at runtime by an embedded
  Cranelift JIT, linked in only when a program needs it. `binding` captures `self` (not locals),
  so `ERB#result(binding)` works.
- **C extensions:** a CRuby-compatible `ruby.h` and C API, built with `kenny build --ext dir`.
- **Libraries:** `require` / `require_relative` / `autoload` / `load` compile gems and the stdlib
  into the binary (`-I dir`; the stdlib of an installed Ruby is found automatically).
- **Native stdlib extensions** (Rust instead of C): date, json, yaml (Psych API), bigdecimal,
  strscan, digest, zlib, openssl (digests, HMAC, ciphers), Marshal (byte-compatible format 4.8),
  cgi/escape, plus `Gem::Version`/`Gem::Requirement`. Nokogiri is native too (html5ever and
  quick-xml underneath, its own XPath 1.0 engine, and nokogiri's Ruby layer on top), so loofah and
  the Rails HTML sanitizers work without libxml2. The `sqlite3` gem's C extension is native
  too (on the bundled SQLite library); the gem's own Ruby files load on top of it.
- **Rails:** a whole generated Rails 5.2 application boots and serves production traffic:
  `examples/blog` is `rails new` plus a scaffold, unchanged (Railties boot, Bundler, every
  framework of `rails/all`, initializers, eager loading of `app/`, the scaffold's HTML forms with
  CSRF protection, its jbuilder JSON views, sessions, flash, the 404/422/500 pages and
  precompiled assets served by ActionDispatch::Static). `examples/blog/compare.rb` runs the same
  24-request session against Puma and the binary: the responses are identical. Smaller examples,
  with output identical to Ruby's:
  `examples/rails_api` (ActionController::API, routing, params, JSON),
  `examples/rails_html` (ActionController::Base with ERB layouts, partials and view helpers;
  templates are read at runtime and compiled once by the embedded JIT), and
  `examples/rails_api_ar` (below).
  ActiveRecord 5.2 works with the SQLite adapter (migrations/schema, associations, validations,
  scopes, transactions, eager loading): `examples/rails_api_ar` is a JSON API backed by it.
  Rack 2.2 apps run, and all 1,456 source files of Rails 5.2, ActiveSupport 7.1 and their
  dependencies compile.
- **Reflection:** method visibility (`private`/`protected`, enforced on explicit receivers),
  `Method`/`UnboundMethod`, `Module#dup`, `Rational`/`Complex`.
- **Backtraces:** full `caller`, `caller_locations` and `Exception#backtrace`, at no cost to
  running code: compiled code keeps frame pointers and the binary carries a table of each
  call's source line, so a backtrace is a walk of the native stack, taken only when needed.
- **Bundler, socket, FFI:** Bundler's runtime API on gems resolved at build time;
  `TCPSocket`/`TCPServer`/`Socket` name resolution (blocking); FFI bindings load (their
  declarations work) but calling a C function through FFI raises NotImplementedError.
- **Built-in HTTP server** for Rack apps: one process serves requests on every core
  (`rack.multithread` is true); `WEB_CONCURRENCY=n` forks n processes instead.

Not yet:
- Compiling assets in the binary (Sprockets with sass/coffee/uglifier needs libsass through FFI
  and a JavaScript runtime): precompile them with Ruby (`rails assets:precompile`); serving
  them works. Rails' development mode (code reloading, `listen`) and Action Cable's
  WebSocket server are untested; production mode is the target.
- A self-contained deployment: the binary contains all Ruby code, but reads the app's
  non-Ruby files (views, `config/*.yml`, locales, `public/`) and the gems' locale files from
  where they were at build time.
- Freeing JIT-compiled code and singleton classes: code that `eval`s a *new* string or
  `extend`s objects on every request grows memory (compiled `eval` units are cached by source
  and packed into shared code pages, so repeated or boot-time evals stay cheap; `instance_exec`
  and `instance_eval` with a block only make a singleton class if the block defines a method).
- Moving threads between cores after they start (a core's threads stay on it), and
  non-blocking file and SQLite IO (they block their core's scheduler).
- A compacting GC: objects never move, because the stack and C extensions hold raw pointers
  that the collector can't update.
- Memory: serving on 12 cores takes about 2x the memory of one (per-core heaps and caches),
  still far less than 12 Puma workers.
- `ObjectSpace`, `TracePoint` (dropped by design).

## Early numbers (Apple Silicon M-series, 12 cores, vs Ruby 2.7.8 interpreter, no YJIT)

| Benchmark | Speedup |
|---|---|
| Integer loop (`bench/loop.rb`) | 10.5x |
| Recursive `fib` | 2.1x |
| Blocks and collections (`blocks.rb`, `collections.rb`) | 4.1x, 3.6x |
| String building and hashes (`strings_hash.rb`) | 2.6x |
| Float math: `mandelbrot.rb`, `spectral_norm.rb`, `nbody.rb` | 4.9x, 4.6x, 2.0x |
| Method-call-heavy OO code (`methods.rb`) | 2.4x |
| Object allocation with GC (`objects.rb`) | 1.8x |
| 12 CPU-bound Ruby threads | 0.02 s vs 0.27 s (Ruby's GVL runs one at a time) |

Web (the machine was also running other work, so treat these as rough; `ab -c 10`/`-c 32`):

| App | One thread | All 12 cores, one process | Puma |
|---|---|---|---|
| Rails 5.2 API route + JSON (`examples/rails_api`) | 18.1k req/s, 41 MB | 21.1k req/s, 99 MB | 5.6k req/s, 39 MB (1 thread) |
| Rails 5.2 HTML page with layout and partials (`examples/rails_html`) | 10.9k req/s, 45 MB | 20.4k req/s, 115 MB | 4.5k req/s, 44 MB (1 thread) |
| Rails 5.2 + ActiveRecord/SQLite read (`examples/rails_api_ar`) | 5.8k req/s, 54 MB | 15.9k req/s, 138 MB | 3.0k req/s, 44 MB (1 thread) |
| Whole Rails 5.2 app (`examples/blog`), scaffold index | 2.8k req/s, 91 MB | 8.3k req/s, 166 MB | 1.5k req/s, 108 MB (1 thread); 4.1k req/s, 1,058 MB (12 workers) |

Run them yourself: `bench/run.sh`. Tests: `tests/run.sh`. Compile-check a file without linking: `kenny check file.rb`.

GC tuning and diagnostics (environment variables read by compiled programs):
`KENNY_GC_STATS=1` prints collections, GC time and heap size at exit (`=2` after every
collection); `KENNY_GC_GROWTH` (default 2.0) sets how much a program may allocate between
collections, as a multiple of the live data; `KENNY_GC_MALLOC_LIMIT` (MB, default 32) starts a
collection once that much memory was malloc'd for object contents since the last one;
`KENNY_GC_MIN=500` makes a GC stress test; `KENNY_GC_GENERATIONAL=0` turns minor collections
off; `KENNY_GC_VERIFY=1` checks after every minor collection that nothing reachable was left
unmarked (a write-barrier test, slow); `KENNY_GC_SCAN_REGIONS=1` also scans compiled code's data
sections for roots (normally literals are pinned instead). `KENNY_THREADS` sets how many OS threads run Ruby
threads (default: one per core); `WEB_CONCURRENCY=n` additionally forks n server processes
(default 1). `KENNY_NO_INLINE=1 kenny build ...` compiles without block inlining (to compare, or
to rule it out when debugging). Debugging: `KENNY_DEBUG_RAISE=1` logs every exception raised
(rescued or not) with its location; `KENNY_DEBUG_EVAL=1` logs every string compiled at runtime
by `eval`/`class_eval`; an uncaught exception and a failed request print the full backtrace.

## Roadmap

1. Moving threads between cores (work stealing) and non-blocking file/SQLite IO
2. Less memory per core (shared caches, returning idle pages sooner); assets compiled in the binary
3. Direct calls between compiled methods (guarded by the inline cache) and inlining of small methods
4. Precise stack maps (Cranelift) for compiled frames, the first step towards a compacting GC
