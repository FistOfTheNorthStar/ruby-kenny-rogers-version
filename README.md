<img width="2752" height="1536" alt="Kenny: Ruby, compiled" src="https://github.com/user-attachments/assets/72be7f46-324f-41bf-98a5-ff60c31325e7" />

<h1 align="center">Kenny — Ruby, compiled</h1>

<p align="center">
  <b>Your Ruby program as one native executable, like Go.</b><br>
  No interpreter, no VM, no Puma: just a binary with its own multi-core HTTP server.
</p>

<p align="center">
  <img alt="Status: pre-alpha" src="https://img.shields.io/badge/status-pre--alpha-red">
  <img alt="Ruby 3.4 | 4.0" src="https://img.shields.io/badge/ruby-3.4%20%7C%204.0-CC342D?logo=ruby&logoColor=white">
  <img alt="Rails 8.1" src="https://img.shields.io/badge/rails-8.1-D30001?logo=rubyonrails&logoColor=white">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-000000?logo=rust&logoColor=white">
  <img alt="Codegen: Cranelift" src="https://img.shields.io/badge/codegen-Cranelift-6b4fbb">
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#benchmarks">Benchmarks</a> ·
  <a href="#run-a-rails-app">Rails</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#status">Status</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#roadmap">Roadmap</a>
</p>

> [!WARNING]
> DHH said go AI delirium, but you also like Ruby. Kenny is in **pre-alpha testing**.
> **Do not run it in production**: it needs serious work. That said, please continue.

## Why Kenny?

You keep Ruby's syntax and drop a few rarely used runtime features (see
[docs/LANGUAGE.md](docs/LANGUAGE.md)). In return you get:

- 🚀 **Speed:** 5x to 59x faster than CRuby 3.4 on the [benchmarks](#benchmarks), and faster than YJIT on every one of them.
- 📦 **One file to deploy:** the app, its gems, views and precompiled assets in a single binary (6.5 MB for a whole Rails app).
- 🧵 **Every core, one process:** lightweight threads with no GVL, and a built-in Rack server that uses all cores.
- 🛤️ **Real Rails:** a generated Rails 8.1 app gives the same responses as under Puma, request for request.
- 🧩 **Your gems:** gems from your `Gemfile.lock` are compiled in, C extensions included.

## Quick start

You need Rust (`cargo`) and an installed Ruby 3.4 or 4.0 (Kenny compiles in that Ruby's standard library and gems).

```sh
cargo build --release

# Compile and run a script
./target/release/kenny run examples/hello.rb

# Build a Rack app into a server binary, then start it (port 3000, or $PORT)
./target/release/kenny build examples/hello_rack/config.ru -o server && ./server
curl localhost:3000/json
```

| Command | What it does |
|---|---|
| `kenny run file.rb [-- args...]` | Compile and run a program |
| `kenny build file.rb -o out` | Build a standalone executable (`config.ru` for a web app) |
| `kenny check file.rb` | Compile-check a file without linking |
| `-I dir` | Add a load path (not needed when there is a `Gemfile.lock`) |
| `--ext dir` | Compile one of your own C extensions into the binary |
| `--embed dir` / `--no-embed` | Choose which directory gets embedded in the binary, or turn embedding off |

## Benchmarks

Apple M4 Pro, 12 cores, against Ruby 3.4.12 and 4.0.7. Speedup of the Kenny binary over CRuby on
the same script, best of 3, wall time including process start (about 27 ms for `ruby`, which
flatters Kenny on the shortest runs). Kenny is as fast built against 3.4 as against 4.0.

| Benchmark | vs Ruby 3.4 | vs Ruby 4.0 | vs Ruby 4.0 + YJIT |
|---|---:|---:|---:|
| Integer loop (`bench/loop.rb`) | 13x | 13x | 1.3x |
| Recursive `fib` (`fib.rb`) | 14x | 13x | 4.4x |
| Blocks (`blocks.rb`) | 5.8x | 5.2x | 4.8x |
| Collections (`collections.rb`) | 5.2x | 5.0x | 1.8x |
| String building and hashes (`strings_hash.rb`) | 4.9x | 4.4x | 3.1x |
| Float math: `mandelbrot.rb` | **43x** | **41x** | **28x** |
| Float math: `spectral_norm.rb` | 8.7x | 7.0x | 1.7x |
| Float math: `nbody.rb` | 5.0x | 4.5x | 1.9x |
| Method-call-heavy OO code (`methods.rb`) | 13x | 13x | 1.7x |
| Object allocation with GC (`objects.rb`) | 5.9x | 4.4x | 2.5x |
| 12 CPU-bound Ruby threads (`threads.rb`) | **59x** | **62x** | **26x** (Ruby's GVL runs one at a time) |

The YJIT column is `--yjit`, except for `loop.rb`, `mandelbrot.rb` and `threads.rb`: they run
their hot loops at the top level, which YJIT compiles only with `--yjit-call-threshold=1` (that
setting slows the others: `methods.rb` runs 3x slower).

**Web.** The machine was also running other work, so treat these as rough (`ab -c 10` / `-c 32`).
The app is the whole Rails 8.1 app in `examples/blog8` (SQLite, Solid Cache/Queue), scaffold index:

| Server | Throughput | Memory |
|---|---:|---:|
| Kenny, one thread | 2.2k req/s | 119 MB |
| Kenny, all 12 cores, one process | **6.7k req/s** | 195 MB |
| Puma, 1 thread (Ruby 3.3) | 1.3k req/s | 71 MB |
| Puma, 12 workers | 4.8k req/s | 1,040 MB |

Run them yourself: `bench/run.sh` (`RUBY=/path/to/ruby`, `RUBY_OPTS=--yjit`).

## Run a Rails app

`examples/blog8` is a Rails 8.1 app (Zeitwerk, Solid Cache/Queue/Cable on SQLite, Propshaft,
importmap, Turbo). Build it with Ruby 3.3+ and the app's gems installed: its gems come from its
`Gemfile.lock`, and `kenny build` embeds the app's files (views, config, `public/`) in the binary.
Its assets are precompiled with Ruby.

```sh
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=x bin/rails db:prepare assets:precompile)
./target/release/kenny build examples/blog8/config.ru -o blog8
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=$(openssl rand -hex 64) ../../blog8)
ruby examples/blog8/compare.rb blog8   # 45 requests against Puma and the binary: identical
```

### Deploying

A Rack app's binary is self-contained: `kenny build` also embeds the app's directory (views,
`config/`, locales, `public/` with the precompiled assets) and every compiled gem's directory,
compressed (6.5 MB for `examples/blog8`). Runtime data is left out: `log/`, `tmp/`,
`node_modules/`, `storage/`, SQLite databases, and `config/master.key` (give it as
`RAILS_MASTER_KEY` or a file on the server). To deploy, copy the binary and start it in the
directory that holds its data:

```sh
mkdir -p /srv/blog8/storage && cp blog8 /srv/blog8/ && cp examples/blog8/storage/production*.sqlite3 /srv/blog8/storage/
cd /srv/blog8 && RAILS_ENV=production SECRET_KEY_BASE=... ./blog8
```

<details>
<summary><b>Where files are read and written at runtime</b></summary>

Embedded files are always read from the binary, as they were at build time. Everything else
under the app's directory (the database, `log/`, `tmp/`, uploads) lives in the directory the
binary was started in, or in `$KENNY_APP_ROOT`. Started inside its build directory (or above
it), the binary uses the build directory itself. Writing to an embedded file first copies it
there. `--embed <dir>` embeds a directory for any program; `--no-embed` turns embedding off.

In development (`RAILS_ENV`/`RACK_ENV=development`, started in its build directory) the app's
own files are read from disk instead, so edits show up; `KENNY_LIVE_FILES=1` turns that on
anywhere, `=0` off. The gems' files always come from the binary.
</details>

## Ruby versions and gems

Kenny supports **Ruby 3.4 and Ruby 4.0** (the repository's `.ruby-version` is 3.4.12) and builds
against an installed Ruby. It picks, in order:

1. the one `KENNY_RUBY` names,
2. the version in the program's `.ruby-version` (or `.tool-versions`, next to it or above), found among rvm, rbenv, asdf, chruby and mise installs,
3. `ruby` on `PATH`.

That Ruby's standard library and gems are what goes into the binary, and the program behaves as
that version: built against 4.0 it gets `Set` and `Pathname` as core classes, the `Ruby` module,
`Array#rfind`, `String#strip` selectors and the rest of 4.0's changes (`open("|cmd")` is gone,
conversion error messages changed, the openssl gem's new errors). Building against any other
version prints a warning: the program gets that Ruby's libraries but behaves as 3.4 (older) or
4.0 (newer).

**Gems.** When the program has a `Gemfile.lock` next to it (or in a parent directory),
`kenny build` resolves the locked gems itself from the installed gem directories (that Ruby's
first, then `GEM_HOME`/`GEM_PATH`, `~/.gem`, rvm, rbenv) and compiles them in. Groups listed in
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

<details>
<summary><b>Notable design choices</b> (tagged values, inline caches, inlining, registers…)</summary>

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
- **Calls between compiled methods are direct.** When a site's inline cache maps the receiver's class
  to a compiled method, compiled code calls that method's native function itself, with its arguments
  in registers; attr_readers and attr_writers become a load or a store, and `is_a?` in a loop a
  lookup in the receiver's superclass list. Each of the cache's four classes is checked with one
  compare, so a site whose receivers are of a few classes (a superclass's method called on its
  subclasses' objects) stays on the fast path. The stack check before such a call works for every
  thread at once: a table with a byte per MiB of address space says whether that MiB of a stack
  is clear of its limit, so threads on several cores keep their direct calls.
- **Small methods are inlined** (attr-style getters, predicates, arithmetic, `def x = @x`, small
  `initialize`s through `C.new`, methods taking a literal block): the body is compiled into the caller
  behind a guard slot holding up to four receiver classes for which the runtime has checked that the
  call reaches that very `def` under the current method serial. In a loop, a call to a method the
  program defines in two or three classes inlines each body behind its own guard. Redefining a
  method, a singleton method, `include`/`prepend`, a visibility change or `method_missing` changes
  the serial or misses the guard, and the call is made normally. Slots fill while several threads
  run Ruby code too: classes go into free ways, and a slot a method definition made stale is
  cleared, then filled again once every thread has stopped (at the next collection). Backtraces
  still show the inlined method's frame (its calls record an inline location).
- **Blocks of `times`, `upto`, `downto`, `each`, `each_with_index`, `map`, `select`, `reject`, `any?`, `all?`** (on
  Integers, Arrays and Integer Ranges) are compiled as loops in the caller when the block is small and doesn't need
  its own frame (`return`, nested blocks, ...). A guard checks at runtime that the method is the builtin one;
  otherwise the call is made normally with the block as a closure.
- **Locals live in CPU registers.** Only variables that a block captures move to a heap environment, based on a closure-capture analysis.
- **Exceptions, `break` and non-local `return`** use one return-value check per call, with no stack unwinding tables.
</details>

## Status

### ✅ Working

| Area | In short |
|---|---|
| Language | All of Ruby 3.4 and 4.0 syntax, minus the few runtime features in [LANGUAGE.md](docs/LANGUAGE.md) |
| Garbage collection | Generational, parallel, compacting; sub-millisecond minor pauses under load |
| Threads and Fibers | M:N threads on all cores, no GVL, kqueue/epoll IO, work stealing |
| Rails | A whole generated Rails 8.1 app serves production traffic, identical to Puma |
| HTTP server | Built in, multi-core, Puma-style limits, Rack hijacking and WebSockets |
| C extensions | CRuby-compatible `ruby.h`; your Gemfile's extensions are compiled in |
| Native stdlib | json, yaml, openssl (with TLS), nokogiri, sqlite3, Marshal and more, in Rust |
| Assets | Sass and ExecJS without Ruby or Node at runtime |
| `eval` | Strings compiled at runtime by an embedded JIT |
| Tooling | Full backtraces, Prism, Ripper, RuboCop, Bundler runtime API |

<details>
<summary><b>Language</b></summary>

Everything in Ruby 3.4 and 4.0 syntax except the few runtime features dropped in
[docs/LANGUAGE.md](docs/LANGUAGE.md). That includes pattern matching, argument forwarding,
refinements, `redo`/`retry`, `Rational`, and keyword arguments on methods and blocks.
</details>

<details>
<summary><b>Garbage collection</b></summary>

Generational, and compacting when it pays. Most collections are
minor: they mark only objects allocated since the last one, plus what write barriers recorded
(stores into old Arrays, Hashes, objects, closure environments and class tables). Each OS
thread allocates from its own 64 KB pages of fixed-size slots; a collection stops all
threads, scans their stacks with their help, sweeps in parallel, and leaves most freeing
(young pages, dead buffers) to a background thread after the pause. A minor pause in the
blog example under load is under 1 ms.

The heap is scanned precisely, and so are compiled
frames: the compiler emits a stack map for every call (Cranelift's, in the binary and in
eval'd code) listing the slots that hold Ruby values across it, and a compacting collection
finds those frames by walking frame pointers. The rest of a stack (native frames,
registers, values the maps don't list) is scanned conservatively.

A major collection can
**move** plain objects, strings, arrays, hashes, ranges, floats and bignums out of sparsely
used pages into gaps in fuller ones, rewrite the fields and compiled-frame slots that refer
to them, and return the emptied pages to the OS. What it can't update stays put: objects
reached from a conservatively scanned word (also through a pointer into them), from a root
or a C extension, objects with singleton classes, and objects whose `object_id` or identity
hash was taken. Native code may hold object references in Rust collections, so objects move
only while no thread is inside a native method that hasn't been audited not to (the
scheduler's blocking calls, `each`/`map`/`send`/`instance_exec`/`Proc#call`/`Class#new`
and other hot ones are): with precise frames that is most collections of a busy server,
not only `GC.compact` and an idle server's tidy-up a second after its last request, which
also gives memory back (`KENNY_GC_COMPACT=0` turns it off). Singleton classes of dead
objects are freed (`extend` per request doesn't grow memory).
</details>

<details>
<summary><b>Threads and Fibers</b></summary>

Lightweight threads that run on all cores at once (M:N over one OS
thread per core), with `Queue`, `SizedQueue`, `Mutex`, `ConditionVariable`, `Monitor`, a
10 ms fairness timer, `Thread#raise`/`kill`, and deadlock detection. A core with nothing to
run takes waiting threads from busy ones (work stealing), so threads move between cores.

A thread waiting on a socket, pipe or child process (`read`, `write`, `accept`, `connect`,
`IO.select`, `wait_readable`, `Process.wait`, also inside a Fiber) is parked and woken by
kqueue/epoll; a core with nothing to run waits in the event loop itself, so a wake-up costs
one OS wake. Timeouts, `Thread#raise`/`kill`, `fork`, and closing the IO from another thread
(`IOError`, as in CRuby) all end such waits; a thread waiting on IO doesn't count as
deadlocked.

Measured on an M4 Pro: a thread ping-pong over a socket pair takes 10 µs a round
trip (CRuby 9 µs), with 1,000 other threads parked on sockets 16 µs (CRuby 10 µs, the 2 ms
polling Kenny had 8.7 ms); those 1,000 idle threads use no CPU (polling used 6.6 cores), and
writing to all 1,000 wakes them in 6 ms (CRuby 14 ms).

File reads and writes and SQLite
queries run as blocking calls, during which the core's waiting threads go to an idle (or
spare) core and collections don't wait for them. A thread waiting for another connection's
write lock (`busy_timeout`, or Rails 8's `busy_handler_timeout`) lets other threads run and
resumes on its own OS thread, which SQLite's connection mutex belongs to. No GVL: a data race can lose
an update but never crashes the process ([docs/LANGUAGE.md](docs/LANGUAGE.md), rule 4).
As in CRuby, a thread that reaches a constant another thread is autoloading waits until that
file has run, and one IO object can be used by several threads at once.
External enumerators (`next`) run on Fibers.
</details>

<details>
<summary><b>Rails</b></summary>

A whole generated Rails 8.1 application boots and serves production traffic:
`examples/blog8` is `rails new` (8.1.4) plus two scaffolds (posts, and
comments that belong to them), with Zeitwerk eager loading, Solid Cache, Solid Queue and
Solid Cable on SQLite, Propshaft, importmap, Turbo and Stimulus, Active Storage (libvips
isn't installed, so it falls back as under Ruby) and jbuilder. `examples/blog8/compare.rb`
runs a 45-request session (forms with CSRF tokens, validations, JSON views and a JSON API
create, deletes, 404/422 pages, the import map's assets) against Puma and the binary: the
responses are identical. Its assets are precompiled with Ruby (`rails assets:precompile`)
before `kenny build`, since the build-time precompile (see Assets) runs only for apps that use the
sassc or ExecJS gems. Rack 2.2 and Rack 3 apps run, and the server supports Rack
hijacking (full and partial), so WebSockets work.
</details>

<details>
<summary><b>Built-in HTTP server</b></summary>

For Rack apps: one process serves requests on every core
(`rack.multithread` is true); `WEB_CONCURRENCY=n` forks n processes instead. An event loop
per core reads and writes the connections, and the requests run on a pool of lightweight
threads (`KENNY_SERVER_THREADS`, default 4 per core; `0` runs them on the event loops, one
at a time per core), so a request waiting on the database or a socket doesn't hold up others.

Request bodies may be chunked; one over 1 MB goes to a temporary file (`rack.input` is then
an IO on it), and `Expect: 100-continue` is answered. Limits as in Puma: 112 KB of request
line and headers (431 above it), no body limit unless `KENNY_MAX_BODY` sets one (413);
headers must arrive within `KENNY_READ_TIMEOUT` seconds (default 30; 408 otherwise), and an
upload or a response download that stalls that long is cut off; an idle keep-alive connection
is closed after `KENNY_KEEPALIVE_TIMEOUT` seconds (default 20). A running request is not
timed. `0` turns a timeout off. `rack.hijack`
(full and partial) hands the connection's socket to the app.
</details>

<details>
<summary><b>C extensions</b></summary>

A CRuby-compatible `ruby.h` and C API. The C (and C++) extensions of the
gems in Gemfile.lock are compiled into the binary, with the flags and `HAVE_*` defines of the
Makefile their `extconf.rb` made (msgpack and unf_ext, for example), so `require` of the
extension runs its `Init_` function; `kenny build --ext dir` adds one of your own. Gems the
runtime has a native replacement for (nokogiri, json, date, sqlite3, ...) keep using it. An
extension that doesn't compile or link against Kenny's API is left out with a warning
(`require` of it then raises `LoadError`); `KENNY_NO_GEM_EXTS=1` leaves them all out.
</details>

<details>
<summary><b>Libraries and native stdlib extensions</b></summary>

`require` / `require_relative` / `autoload` / `load` compile gems and the stdlib
into the binary (`-I dir`; the stdlib of the Ruby built against is found automatically, and
without a Gemfile so are the gems that Ruby bundles in place of former stdlib parts, such as
`csv` and `bigdecimal` in 3.4).

**Native stdlib extensions** (Rust instead of C):

- date, bigdecimal, strscan, digest, zlib
- json (2.x or 3.x, as the lock file says)
- yaml (Psych API; its tree API, `Psych.parse`, `Psych::Nodes`, `YAMLTree` ..., is psych's
  Ruby code on native parse events and a port of libyaml's emitter)
- openssl: digests, HMAC, ciphers, RSA, EC (P-256/384/521, secp256k1: ECDSA, ECDH) and
  Ed25519 keys, ASN1, X509 certificates verified as OpenSSL does, and TLS 1.2/1.3 on rustls
  under the gem's own `openssl/ssl.rb`, so Net::HTTP does https; no DSA, DH or X25519
- Marshal (byte-compatible format 4.8)
- cgi/escape, erb/escape (Rails' `html_escape`), plus `Gem::Version`/`Gem::Requirement`
- **Nokogiri** (html5ever and quick-xml underneath, its own XPath 1.0 engine, a SAX
  scanner that reports what libxml2's does, and nokogiri's Ruby layer on top), so loofah, the
  Rails HTML sanitizers and the AWS SDK's XML parsing work without libxml2
- **sqlite3**: the gem's C extension, on the bundled SQLite library, for both the
  1.x and 2.x gem APIs; the gem's own Ruby files load on top of it
</details>

<details>
<summary><b>Assets without Ruby or Node</b></summary>

The `sassc` gem's API runs on a native Sass
compiler ([grass](https://github.com/connorskees/grass)), with Sass functions such as
`asset-path` calling back into Ruby, and ExecJS runs on an embedded JavaScript engine
([QuickJS](https://github.com/quickjs-ng/quickjs)), so ExecJS users such as `uglifier` and
`terser` work in-process. `kenny build` of a Rails app with app/assets precompiles its assets (what
`rails assets:precompile` does, run by the new binary itself) when public/assets has no
manifest or the assets changed; `KENNY_PRECOMPILE=1` forces it, `=0` skips it. Both are
linked only into programs that use those gems.
</details>

<details>
<summary><b><code>eval</code> with strings</b></summary>

`eval` / `class_eval` / `instance_eval` with strings are compiled at runtime by an embedded
Cranelift JIT, linked in only when a program needs it, and cached by source. The cache keeps
the 2,000 most recently used units (`KENNY_EVAL_CACHE`); an evicted unit's code and data are
freed once nothing can run them (no method, block, frame or Method object refers to them), so
code that evals a new string per request doesn't grow memory. `binding` captures `self` (not
locals), so `ERB#result(binding)` works.
</details>

<details>
<summary><b>Regular expressions, reflection, backtraces</b></summary>

- **Regular expressions** via Oniguruma, the engine family Ruby's own comes from: `$~`/`$1`,
  named captures, and the regex-aware `String` methods.
- **Reflection:** method visibility (`private`/`protected`, enforced on explicit receivers),
  `Method`/`UnboundMethod`, `Module#dup`, `Rational`/`Complex`.
- **Backtraces:** full `caller`, `caller_locations` and `Exception#backtrace`, at no cost to
  running code: compiled code keeps frame pointers and the binary carries a table of each
  call's source line, so a backtrace is a walk of the native stack, taken only when needed.
</details>

<details>
<summary><b>Bundler, socket, FFI</b></summary>

Bundler's runtime API on gems resolved at build time;
the socket library: `TCPSocket`/`TCPServer`, `UNIXSocket`/`UNIXServer` (and `.pair`),
`Socket` on sockaddr strings (`connect`/`connect_nonblock`/`bind`/`listen`/`accept`),
`Addrinfo`, `Socket::Option` (UDP's `sendto`/`recvfrom` and `send_io`: not yet;
name resolution blocks); FFI bindings load (their
declarations work) but calling a C function through FFI raises NotImplementedError. As with
the ffi gem, `ffi_lib` raises LoadError when the library isn't installed, so optional native
dependencies (Active Storage's libvips) are skipped.
</details>

<details>
<summary><b>Prism and Ripper at runtime</b></summary>

The prism gem's C extension is replaced by the libprism
Kenny's compiler parses with (linked only into programs that load Prism or name Ripper's
parser: `require "ripper"` alone costs nothing), so `Prism.parse`/`lex`/`dump`, rubocop-ast
and RuboCop work. Its parse is serialized for the
gem's Ruby library, whose version must be libprism's (1.9.0): `kenny build` uses the
Gemfile.lock's prism, else an installed prism 1.9.0 gem in place of the standard library's
(Ruby 3.4 ships 1.5), and otherwise `require "prism"` raises LoadError (with a build
warning). Ripper runs on Prism (`Prism::Translation::Ripper` sends Ripper's parser events,
`Prism.lex_compat` gives `Ripper.lex` its tokens), so `Ripper.sexp`, `Ripper.lex` and
Ripper subclasses work; lexer states of some tokens (spaces, heredoc ends) can differ from
CRuby's, and `Ripper#state`/`#token` are nil.
</details>

### 🚧 Not yet

- Sass source maps, and sassc-rails' ERB (`.scss.erb`) and glob (`@import "dir/*"`) imports:
  imports are resolved by the Sass compiler on the load paths.
- In development: debuggers (`byebug` loads, but calling `byebug` raises LoadError: its core
  is a C extension), and Action Cable's redis adapter (untested: the redis gem isn't installed
  here).
- Compacting while requests run is partial: a major collection moves objects only when no
  thread is inside a native method that isn't audited (`KENNY_GC_COMPACT_STATS=2` names
  them). In the blog example under load on macOS that is 105 of 105 majors on its pages and
  642 of 642 while it also creates and updates posts and comments, and 97 to 100 in 100 on
  Linux (aarch64; none before precise stack maps); the rest skip compaction (a thread
  inside a native method not audited yet). Precise frames need frame pointers on every
  frame: Apple platforms keep them, and
  on Linux the runtime and the C it builds are compiled with them (`.cargo/config.toml`).
  A runtime built without them scans stacks word by word for native frames instead, which
  rarely lets a busy server compact (`KENNY_GC_PRECISE=1` forces precise frames).
- Memory: serving on 12 cores takes about 1.7x the memory of one (each core has its own
  young-generation space, `KENNY_GC_YOUNG_SCALE=1` trades some throughput for ~1.3x); still
  far less than 12 Puma workers. On Linux (aarch64, 12 cores) the blog peaks at ~350 MB with
  mimalloc, the default, and ~255 MB with glibc's malloc (`KENNY_MALLOC=system`), which serves
  about 30% fewer requests; one core takes ~190 MB.
- `ObjectSpace`, `TracePoint` (dropped by design).

## Testing

| Command | What it runs |
|---|---|
| `tests/run.sh` | The test suite against Ruby 3.4 |
| `tests/run.sh 4.0` | Against an installed Ruby 4.0 (a test's `.4.0.out` holds what 4.0 prints differently) |
| | On Linux a test's `.linux.out` (or `.4.0.linux.out`) holds what CRuby prints there: other errno numbers, `Fcntl` constants and libm digits ([tools/linux/Dockerfile](tools/linux/Dockerfile) runs the suite in a container) |
| CI ([.github/workflows/ci.yml](.github/workflows/ci.yml)) | `tests/run.sh` on Linux x86-64 and arm64 and on macOS, against Ruby 3.4 and 4.0 |
| `tests/gems/run.sh [gem ...]` | Gems, under CRuby and Kenny |
| `tests/server/run.sh [pattern]` | The HTTP server: a test app's binary started with different settings, driven over raw sockets by a CRuby script (~20 s) |
| `bench/run.sh` | The benchmarks (`RUBY=/path/to/ruby`, `RUBY_OPTS=--yjit`) |

## Configuration

Environment variables read by `kenny build` or by compiled programs.

<details>
<summary><b>Build and app</b></summary>

| Variable | Effect |
|---|---|
| `KENNY_RUBY` | The Ruby to build against (see [Ruby versions](#ruby-versions-and-gems)) |
| `BUNDLE_WITHOUT` | Gem groups to leave out (default `development:test`) |
| `KENNY_PRECOMPILE` | `1` forces the build-time asset precompile, `0` skips it |
| `KENNY_NO_GEM_EXTS=1` | Leave out all gems' C extensions |
| `KENNY_STACK_MAPS` | At build time: `0` leaves the stack maps out of the binary; `stats` reports their size (about a fifth of the blog example's binary; keeping values in their slots across calls costs 0–4% in `bench/`, about 3% in `mandelbrot.rb`) |
| `KENNY_APP_ROOT` | Where the app's non-embedded files (database, `log/`, `tmp/`) live |
| `KENNY_LIVE_FILES` | `1` reads the app's own files from disk anywhere, `0` never |
| `RAILS_MASTER_KEY` | Rails' master key (`config/master.key` isn't embedded) |
</details>

<details>
<summary><b>Server and threads</b></summary>

| Variable | Effect |
|---|---|
| `PORT` | Port the built-in server listens on (default 3000) |
| `KENNY_THREADS` | OS threads that run Ruby threads (default one per core, or one with `RAILS_ENV=development`, whose code reloading counts on a GVL) |
| `WEB_CONCURRENCY=n` | Additionally fork n server processes (default 1) |
| `KENNY_SERVER_THREADS` | Threads that run requests in each process (default 4 per `KENNY_THREADS`; `0` runs them on the event loops) |
| `KENNY_MAX_BODY` | Request body limit in bytes, or with a K/M/G suffix (default none) |
| `KENNY_READ_TIMEOUT` | Seconds for headers to arrive, and for a stalled upload/download (default 30; `0` off) |
| `KENNY_KEEPALIVE_TIMEOUT` | Seconds before an idle keep-alive connection is closed (default 20; `0` off) |
| `KENNY_SCHED_STATS=1` | Print how often threads moved between cores |
| `KENNY_NO_STEAL=1` | Keep threads on the core they started on (to compare) |
</details>

<details>
<summary><b>Garbage collector and memory</b></summary>

| Variable | Effect |
|---|---|
| `KENNY_GC_STATS` | `1` prints collections, GC time and heap size at exit; `2` after every collection |
| `KENNY_GC_GROWTH` | How much a program may allocate between collections, as a multiple of the live data (default 2.0) |
| `KENNY_GC_MALLOC_LIMIT` | MB malloc'd for object contents since the last collection that starts a new one (default 32) |
| `KENNY_GC_YOUNG_SCALE` | Caps how much the young generation grows with the number of cores (default 4; less memory, more pauses) |
| `KENNY_GC_GENERATIONAL=0` | Turn minor collections off |
| `KENNY_GC_COMPACT` | `0` never moves objects, `always` tries at every major collection |
| `KENNY_GC_PRECISE` | `0` scans compiled frames conservatively (pinning what they hold), `1` precisely where that is not the default |
| `KENNY_MALLOC` | `auto` / `system` / `mimalloc`: the allocator behind Rust allocations. On macOS `auto` (the default) uses mimalloc while one core runs Ruby code and the system allocator once several do (it keeps far less memory resident when threads free each other's objects); elsewhere mimalloc, which once several cores run Ruby code purges freed pages at once and has each core tidy its heap after every major collection (`KENNY_MI_TRIM=0` doesn't: ~470 MB instead of ~350 for the blog on Linux) |
| `KENNY_EVAL_CACHE` | Size of the cache of `eval`'d code (default 2000) |

GC testing and diagnostics:

| Variable | Effect |
|---|---|
| `KENNY_GC_MIN=500` | A GC stress test |
| `KENNY_GC_VERIFY=1` | After every minor collection, check nothing reachable was left unmarked (a write-barrier test, slow) |
| `KENNY_GC_SCAN_REGIONS=1` | Also scan compiled code's data sections for roots (normally literals are pinned instead) |
| `KENNY_GC_COMPACT_STATS` | `1` reports each compaction (and why one couldn't run); `2` also names the native methods on each stack that kept it from running |
| `KENNY_GC_COMPACT_VERIFY=1` | Check every rewritten reference after compacting |
| `KENNY_GC_COMPACT_POISON=1` | Evacuate only pages that empty completely, then make them inaccessible for good, so a reference the collector failed to update faults at once |
</details>

<details>
<summary><b>Compiler and debugging</b></summary>

| Variable | Effect |
|---|---|
| `KENNY_NO_INLINE=1` | `kenny build` without block inlining (to compare, or to rule it out when debugging) |
| `KENNY_NO_POLY_INLINE=1` | Don't inline calls to a method with several definitions |
| `KENNY_DEBUG_INLINE=1` | List each method that gets inlined |
| `KENNY_DEBUG_RAISE=1` | Log every exception raised (rescued or not) with its location |
| `KENNY_DEBUG_EVAL=1` | Log every string compiled at runtime by `eval`/`class_eval` |
| `KENNY_CRASH_TRACE=1` | Print a native backtrace on a crash |

An uncaught exception and a failed request always print the full backtrace.
</details>

## Roadmap

1. Compaction under load: audit the natives still seen now and then (`KENNY_GC_COMPACT_STATS=2`)
