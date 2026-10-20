<img width="2752" height="1536" alt="Gemini_Generated_Image_obri9hobri9hobri" src="https://github.com/user-attachments/assets/72be7f46-324f-41bf-98a5-ff60c31325e7" />

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

# a whole generated Rails 5.2 app (`rails new` + a scaffold); its gems come from its Gemfile.lock.
# `kenny build` also precompiles its assets (Sass, CoffeeScript, Uglifier) into public/assets,
# then embeds the app's files (views, config, public/) in the binary
(cd examples/blog && RAILS_ENV=production SECRET_KEY_BASE=x bundle exec rails db:migrate)
./target/release/kenny build examples/blog/config.ru -o blog
(cd examples/blog && RAILS_ENV=production RAILS_SERVE_STATIC_FILES=1 SECRET_KEY_BASE=$(openssl rand -hex 64) ../../blog)
ruby examples/blog/compare.rb blog   # the same session against Puma and the binary: identical

# development mode: code reloading, live Sass/CoffeeScript, Rails' error pages, Action Cable
(cd examples/blog && bundle exec rails db:migrate)
BUNDLE_WITHOUT=test ./target/release/kenny build examples/blog/config.ru -o blog-dev
(cd examples/blog && RAILS_ENV=development ../../blog-dev)
```

When the program has a `Gemfile.lock` next to it (or in a parent directory), `kenny build`
resolves the locked gems itself from the installed gem directories (`GEM_HOME`/`GEM_PATH`, the
system Ruby's `Gem.path`, `~/.gem`, rvm, rbenv) and compiles them in; groups listed in
`BUNDLE_WITHOUT` (default `development:test`) are left out, and `require "bundler/setup"` /
`Bundler.require(*Rails.groups)` work in the binary. Without a lock file, `-I` adds load paths:
`tools/gem-paths.rb NAME VERSION` prints the `-I` flags for an installed gem and its
dependencies. Every file in those directories is compiled into the binary.

A Rack app's binary is self-contained: `kenny build` also embeds the app's directory (views,
`config/`, locales, `public/` with the precompiled assets) and every compiled gem's directory,
compressed (4 MB for `examples/blog`). Runtime data is left out: `log/`, `tmp/`,
`node_modules/`, `storage/`, SQLite databases, and `config/master.key` (give it as
`RAILS_MASTER_KEY` or a file on the server). To deploy, copy the binary and start it in the
directory that holds its data:

```
mkdir -p /srv/blog/db && cp blog /srv/blog/ && cp examples/blog/db/production.sqlite3 /srv/blog/db/
cd /srv/blog && RAILS_ENV=production RAILS_SERVE_STATIC_FILES=1 SECRET_KEY_BASE=... ./blog
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
- **C extensions:** a CRuby-compatible `ruby.h` and C API, built with `kenny build --ext dir`.
- **Libraries:** `require` / `require_relative` / `autoload` / `load` compile gems and the stdlib
  into the binary (`-I dir`; the stdlib of an installed Ruby is found automatically).
- **Native stdlib extensions** (Rust instead of C): date, json, yaml (Psych API), bigdecimal,
  strscan, digest, zlib, openssl (digests, HMAC, ciphers), Marshal (byte-compatible format 4.8),
  cgi/escape, plus `Gem::Version`/`Gem::Requirement`. Nokogiri is native too (html5ever and
  quick-xml underneath, its own XPath 1.0 engine, and nokogiri's Ruby layer on top), so loofah and
  the Rails HTML sanitizers work without libxml2. The `sqlite3` gem's C extension is native
  too (on the bundled SQLite library); the gem's own Ruby files load on top of it.
- **Asset pipeline without Ruby or Node:** the `sassc` gem's API runs on a native Sass
  compiler ([grass](https://github.com/connorskees/grass)), with Sass functions such as
  `asset-path` calling back into Ruby, and ExecJS runs on an embedded JavaScript engine
  ([QuickJS](https://github.com/quickjs-ng/quickjs)), so `uglifier` and `coffee-script` work
  in-process. `kenny build` of a Rails app with app/assets precompiles its assets (what
  `rails assets:precompile` does, run by the new binary itself) when public/assets has no
  manifest or the assets changed; `KENNY_PRECOMPILE=1` forces it, `=0` skips it. The blog's
  JavaScript comes out byte-identical to Ruby's; its CSS differs only in whitespace. Both are
  linked only into programs that use those gems.
- **Rails:** a whole generated Rails 5.2 application boots and serves production traffic:
  `examples/blog` is `rails new` plus a scaffold, unchanged (Railties boot, Bundler, every
  framework of `rails/all`, initializers, eager loading of `app/`, the scaffold's HTML forms with
  CSRF protection, its jbuilder JSON views, sessions, flash, the 404/422/500 pages and
  its assets, precompiled by Kenny and served by ActionDispatch::Static). `examples/blog/compare.rb` runs the same
  24-request session against Puma and the binary: the responses are identical. Smaller examples,
  with output identical to Ruby's:
  `examples/rails_api` (ActionController::API, routing, params, JSON),
  `examples/rails_html` (ActionController::Base with ERB layouts, partials and view helpers;
  templates are embedded in the binary, read at runtime and compiled once by the embedded JIT), and
  `examples/rails_api_ar` (below).
  ActiveRecord 5.2 works with the SQLite adapter (migrations/schema, associations, validations,
  scopes, transactions, eager loading): `examples/rails_api_ar` is a JSON API backed by it.
  Rack 2.2 apps run, and all 1,456 source files of Rails 5.2, ActiveSupport 7.1 and their
  dependencies compile.
- **Rails development mode and Action Cable:** a binary built with the development gems
  (`BUNDLE_WITHOUT=test`) and run with `RAILS_ENV=development` reloads code like `rails
  server`: `load` (which the classic autoloader, the routes reloader and initializers use)
  compiles a file with the embedded JIT when it differs from the copy compiled into the binary
  or is new, so edited controllers, models, routes and views, and newly added files, take
  effect on the next request. `listen` watches the files through FSEvents. Sprockets compiles
  Sass and CoffeeScript live. Exceptions and routing errors show Rails' development error
  pages. The server supports Rack hijacking (full and partial), so Action Cable's WebSockets
  work, on nio4r's pure-Ruby selector and websocket-driver's Ruby masking, in development and
  in production (async adapter). `examples/blog/dev_session.rb` (edits a running app's view,
  stylesheet, CoffeeScript, controller, model and routes, adds a file, raises) and
  `examples/blog/cable_session.rb` (two WebSocket clients subscribe, broadcast from a channel
  action and from a request, unsubscribe) pass against the binary and against Puma alike.
  Development mode runs Ruby threads on one core by default: Rails' classic autoloader counts
  on a GVL (two requests autoloading one module at once each define their own);
  `KENNY_THREADS` overrides it.
- **Reflection:** method visibility (`private`/`protected`, enforced on explicit receivers),
  `Method`/`UnboundMethod`, `Module#dup`, `Rational`/`Complex`.
- **Backtraces:** full `caller`, `caller_locations` and `Exception#backtrace`, at no cost to
  running code: compiled code keeps frame pointers and the binary carries a table of each
  call's source line, so a backtrace is a walk of the native stack, taken only when needed.
- **Bundler, socket, FFI:** Bundler's runtime API on gems resolved at build time;
  `TCPSocket`/`TCPServer`/`Socket` name resolution (blocking); FFI bindings load (their
  declarations work) but calling a C function through FFI raises NotImplementedError.
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

## Early numbers (Apple Silicon M-series, 12 cores, vs Ruby 2.7.8 interpreter, no YJIT)

| Benchmark | Speedup |
|---|---|
| Integer loop (`bench/loop.rb`) | 10.5x |
| Recursive `fib` | 2.2x |
| Blocks and collections (`blocks.rb`, `collections.rb`) | 3.7x, 3.6x |
| String building and hashes (`strings_hash.rb`) | 2.7x |
| Float math: `mandelbrot.rb`, `spectral_norm.rb`, `nbody.rb` | 4.9x, 4.7x, 2.0x |
| Method-call-heavy OO code (`methods.rb`) | 2.5x |
| Object allocation with GC (`objects.rb`) | 1.7x |
| 12 CPU-bound Ruby threads (`threads.rb`) | 24.5x (0.04 s vs 0.97 s: Ruby's GVL runs one at a time) |

Web (the machine was also running other work, so treat these as rough; `ab -c 10`/`-c 32`):

| App | One thread | All 12 cores, one process | Puma |
|---|---|---|---|
| Rails 5.2 API route + JSON (`examples/rails_api`) | 17.2k req/s, 39 MB | 26.1k req/s, 74 MB | 5.6k req/s, 39 MB (1 thread) |
| Rails 5.2 HTML page with layout and partials (`examples/rails_html`) | 10.4k req/s, 42 MB | 23.1k req/s, 83 MB | 4.5k req/s, 44 MB (1 thread) |
| Rails 5.2 + ActiveRecord/SQLite read (`examples/rails_api_ar`) | 5.7k req/s, 51 MB | 16.7k req/s, 100 MB | 3.0k req/s, 44 MB (1 thread) |
| Whole Rails 5.2 app (`examples/blog`), scaffold index | 2.7k req/s, 83 MB | 8.7k req/s, 144 MB (125 MB once idle) | 1.5k req/s, 108 MB (1 thread); 4.1k req/s, 1,058 MB (12 workers) |

Run them yourself: `bench/run.sh`. Tests: `tests/run.sh`. Compile-check a file without linking: `kenny check file.rb`.

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
sets how many OS threads run Ruby threads (default: one per core); `WEB_CONCURRENCY=n`
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
