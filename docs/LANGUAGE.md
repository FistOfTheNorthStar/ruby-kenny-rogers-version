# Kenny Ruby — language profile

Kenny compiles Ruby ahead of time to a single native binary, like Go.

## Rule 1: a strict subset of Ruby

- **No new syntax, keywords or core APIs.** Every valid Kenny program is a valid Ruby program
  and behaves the same under CRuby (except where noted below).
- Features we drop are **compile errors with a clear message** in your code, never silent
  differences. In dependencies (files found via `-I` load paths, gems, the stdlib) they are build
  warnings instead, and raise `NotImplementedError` only if that code path actually runs — so a
  gem whose debug page uses `binding` still compiles.
- Syntax is parsed by Prism, Ruby's official parser, so the grammar is exactly Ruby's: the
  newest one, so a build against Ruby 3.4 also accepts 4.0's syntax (a line starting with `&&`
  or `||` continuing the one before).
- Core behavior follows the Ruby the program is built against, 3.4 or 4.0 (see the README).
  In 4.0 builds `Binding#implicit_parameters` is always empty (a block's `it` / `_1` aren't in
  a Binding's locals). `ObjectSpace._id2ref` and `Ractor` are dropped in both (below).

## Rule 2: drop runtime powers that tax every program

| Dropped | Reason |
|---|---|
| Writing a caller's local variables from an `eval`'d string or through a `Binding` | Locals must be able to live in registers. `binding` and `eval` / `class_eval` / `instance_eval` of a string see a snapshot of the locals taken at the call, so `ERB#result(binding)`, `local_variable_get` and `class_eval "... #{x} ..."` reading `x` work; an assignment inside the string (or `local_variable_set`) changes the snapshot, not the variable. A block that calls `binding` or evals a string keeps the enclosing locals in its heap environment |
| Redefining Integer and Float operators (`Integer#+`, `Float#<`, `Integer#&`, ...: `+ - * / % < <= > >= == != & \| ^`) | Lets arithmetic and comparisons compile inline, on unboxed values, with no guards |
| `ObjectSpace.each_object` (except for `Class`/`Module`), `ObjectSpace._id2ref` | Heap doesn't need to be walkable; GC can move objects. Classes are tracked anyway, so `each_object(Class)` works |
| `TracePoint`, `set_trace_func` | No hooks on every call/line (a debug build flag replaces them) |
| String encodings other than UTF-8, ASCII-8BIT (binary), US-ASCII | Strings skip per-operation encoding checks. CRuby's other encodings exist as `Encoding` objects (every constant, `Encoding.find`, `list`, `aliases`, `dummy?`) so code that compares against them or looks them up runs. `String#encode` converts between UTF-8, UTF-16/32 (LE, BE, with a BOM), US-ASCII, binary, ISO-8859-1 and Windows-1252 (ASCII-only text to and from any ASCII-compatible encoding; other text raises `NotImplementedError`). A String in one of those other encodings (from `encode` or `force_encoding`) reports it from `encoding`, `encode` and `valid_encoding?`, but its other methods see bytes, as a binary String's do (`length` of a UTF-16 String is its byte count) |
| `require` / `autoload` of a library by name (searched on `$LOAD_PATH`) that wasn't compiled in | Whole program is compiled together. `load` of a file, and `require` of an absolute path, that is new or changed since the build are compiled at runtime by the embedded JIT (Rails' code reloading in development) |
| Calling C functions through FFI (`ffi` gem) | A Kenny binary doesn't load shared libraries (C extensions are compiled in: those of the gems in Gemfile.lock, and `--ext dir`). FFI declarations still work, so a gem that binds a C library loads; calling a bound function raises `NotImplementedError`. As with the ffi gem, `ffi_lib` raises `LoadError` when the library isn't installed. The `sassc` gem (libsass through FFI) is replaced by a native Sass compiler |
| `Thread#raise` / `Thread#kill` landing at arbitrary instructions | Delivered only at safe points (calls, loop back-edges, IO) |
| `RubyVM::*`, `Ractor`, `$;`, `$,`, `callcc`, flip-flop, `END` | CRuby internals, superseded, or deprecated (`BEGIN` works; use `at_exit` for `END`). `RubyVM` itself is an empty module, for gems that ask it about YJIT |

Other core methods (`Array#[]`, `Hash#[]=`, `String#==`, `Integer#times`, `Array#each`, ...) can
still be redefined, subclassed or refined: their specialised paths check through the call site's
inline cache that the builtin method is the one being called, and otherwise make a normal call.
Floats can't get singleton methods or instance variables (as in CRuby, they are immediate values
or frozen).

Refinements were on this list at first, but real gems (Rack among them) use them, so they are
supported in a way that costs nothing elsewhere: `using` is lexical, so the compiler knows exactly
which call sites come after it, and only those sites use a refinement-aware lookup.

## Rule 3: defaults that differ from CRuby

- **Frozen string literals by default** (Ruby is heading here already).
- **Metaprogramming is free during boot, slower after.** `define_method`, `class_eval`, `include`,
  `extend` work any time; after boot they invalidate caches and take a slow path.
- **`$~` / `$1`** are tracked only in methods that reference them.
- **Objects may move in memory** (the GC compacts sparse pages), but identity is stable:
  `object_id` and the default `hash` are derived from the address, and an object whose
  `object_id` or identity hash was ever taken is never moved.
- **Gems are resolved at build time.** With a `Gemfile.lock`, `kenny build` compiles the locked
  gems of the groups not in `BUNDLE_WITHOUT` (default `development:test`); in the binary,
  `require "bundler/setup"` and `Bundler.require(*groups)` use that build-time resolution.
  Application servers in the Gemfile (Puma, Unicorn, ...) are left out: the binary is the
  server. `require "bootsnap/setup"` does nothing (there is nothing to cache).
- **`YAML.load` follows the psych of the Ruby the program is built against** (or of a psych
  gem in the lock file): with psych 3 (Ruby ≤ 3.0) it is a full load that accepts aliases and
  any class, as older gems expect; with psych 4+ it is a safe load.
- **Prism at runtime is the compiler's libprism, Ripper runs on it.** `require "prism"` loads the
  prism gem's Ruby library on libprism 1.9.0 (linked into the binary only when Prism or Ripper is
  loaded), so it needs that library at 1.9.0: the Gemfile.lock's prism, else an installed prism
  1.9.0 gem replaces the standard library's (Ruby 3.4 ships 1.5.x, so `Prism::VERSION` is newer
  than under CRuby); otherwise `kenny build` warns and `require "prism"` raises LoadError.
  Ripper is `Prism::Translation::Ripper` sending Ripper's parser events (and `Prism.lex_compat`
  giving `Ripper.lex` its tokens): `Ripper.sexp`, `Ripper.lex` and subclasses work, but the lexer
  state of some tokens (spaces, heredoc ends) differs from CRuby's, and `Ripper#state` and
  `#token` are nil. `require "ripper"` alone (as Rails does) costs nothing.
- **Trailing-Hash keyword arguments in dependencies.** Methods and blocks defined in files found
  via load paths (gems, the stdlib) and in `eval`'d strings take a trailing positional Hash with
  Symbol keys as their keywords — older gems still pass options that way. The app's own code
  keeps today's keyword semantics (except that a Hash which would otherwise be one positional
  argument too many is taken as keywords instead of raising ArgumentError).
- **`DidYouMean` and `ARGF` load on first use** (CRuby sets them up at startup): when code
  refers to them (Rails refers to `DidYouMean`, csv to `ARGF`), or, for `DidYouMean`, to report
  an uncaught `NameError`. `ErrorHighlight.spot` returns nil (it needs
  `RubyVM::AbstractSyntaxTree`), so error messages have no `^^^^` line under the code.
- **Backtraces show only Ruby code.** Frames carry Ruby 3.4's and 4.0's labels (`Foo#bar`, `Foo.baz`,
  `block in Foo#bar`, inlined methods and blocks included), but methods built into Kenny
  (`Array#map`, `Kernel#require` ...) have no frame of their own, where CRuby lists them.
- **Every `*args` method forwards keywords as if marked `ruby2_keywords`.** Keywords collected
  into a rest parameter stay flagged (`Hash.ruby2_keywords_hash?` is true), so `m(*args)` passes
  them on as keywords; in CRuby that needs `ruby2_keywords(:name)`, without it they become a
  positional Hash.

## Rule 4: concurrency is fixed with the APIs Ruby already has

No GVL. No new concurrency API. The existing ones become lightweight:

- **`Thread.new` is a lightweight task**, like a goroutine: its own small stack, cheap to create,
  switched by a scheduler when it blocks and every 10ms for fairness.
- **Status:** threads of one process run on all cores at once (M:N): one scheduler per OS
  thread (`KENNY_THREADS`, default one per core; one under `RAILS_ENV`/`RACK_ENV=development`,
  since Rails' autoloading in development counts on a GVL), each with its own run queue, allocator and
  caches. A scheduler with nothing to run takes a waiting thread from a busy one (work
  stealing), so threads move between cores as the load changes.
- **IO doesn't stop other threads.** A thread waiting on a socket, pipe or `IO.select` yields
  its core to the other threads; a file read or write, `fsync`, or SQLite query runs as a
  blocking call, during which the core's waiting threads are handed to an idle core (or a spare
  one when all are busy) and collections go ahead without waiting for it. Ruby's
  `Fiber::Scheduler` behaviour, built in, no gem. A thread waiting on a socket, pipe or
  terminal (`read`, `gets`, `IO.select`, `accept`, `connect`) is parked, and woken by an event
  loop (kqueue on macOS and the BSDs, epoll on Linux) when the fd is ready; inside a `Fiber`
  it polls instead. `KENNY_NO_POLLER=1` turns the event loop off.
- **`Mutex`, `Queue`, `SizedQueue`, `ConditionVariable`, `Monitor`** work as documented, backed by
  the scheduler (parking, not OS blocking).
- **`Fiber`** keeps its Ruby semantics (`Enumerator#next` etc.).
- **Memory safety under races:** a data race in Ruby code can give wrong answers but never crash
  the process. Readers of Arrays, Hashes, Strings, instance variables and class tables never
  lock; writers take a per-object spin lock, only once more than one OS thread runs Ruby code.
  A buffer that grows is replaced, never freed under a reader: the old one is freed by the GC
  once no stack points into it. Racy unsynchronized writes (two threads doing `@n += 1` or
  `h[k] = v` on the same object) can lose an update, as on JRuby; use a `Mutex` as there.
- **Compatibility bar:** Ruby code that is correct on JRuby / TruffleRuby (no GVL) is correct here.
  Rails is thread-safe and already tested that way.

## Rule 5: the server is part of the binary

Like Go's `net/http`, the runtime contains an HTTP/1.1 + HTTP/2 server written in Rust.
No Puma, no Unicorn, no new API: a standard Rack `config.ru` (`run MyApp`) is the entry point,
each request runs on its own lightweight `Thread`, and `./app` starts serving.

```
kenny build config.ru -o app
./app            # listens on $PORT (default 3000)
```

## Compatibility target

Measured, not claimed: pass rate on `ruby/spec` for the kept features, then the test suites of
Rack, Sinatra, and Rails' default gem set.
