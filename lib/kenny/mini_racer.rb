# frozen_string_literal: true

# The mini_racer gem's API on the JavaScript engine in the Kenny runtime (QuickJS: the one
# ExecJS uses, crates/kenny-runtime/src/assets.rs), as mini_racer's C extension embeds V8,
# which a Kenny binary can't link. Discourse requires it.
#
# Not like V8's: `attach` defines the JavaScript function, but calling it throws (the engine
# can't call back into Ruby yet); `timeout:` and `max_memory:` aren't enforced; results come
# back through JSON (a function is nil, a Date its ISO string); snapshots are only their source.
require "mini_racer/version"
require "json"

begin
  require "kenny_assets"
rescue LoadError
  # not linked in: Context.new raises
end

module MiniRacer
  class Binary
    attr_reader :data

    def initialize(data)
      raise TypeError, "wrong argument type #{data.class} (expected String)" unless data.is_a?(String)
      @data = data
    end
  end

  class Error < ::StandardError; end
  class ContextDisposedError < Error; end
  class PlatformAlreadyInitialized < Error; end
  class EvalError < Error; end
  class ParseError < EvalError; end
  class ScriptTerminatedError < EvalError; end
  class V8OutOfMemoryError < EvalError; end
  class InternalError < EvalError; end

  class RuntimeError < EvalError; end
  class ScriptError < EvalError; end
  class SnapshotError < Error; end
  # (their messages carry the JavaScript stack, which `backtrace` puts first)
  [RuntimeError, ScriptError, SnapshotError].each do |k|
    k.class_eval do
      def initialize(message)
        message, *@frames = message.to_s.split("\n")
        @frames.map! { "JavaScript #{_1.strip}" }
        super(message)
      end

      def backtrace
        frames = super
        @frames + frames unless frames.nil?
      end
    end
  end

  class JavaScriptFunction
    def to_s = "JavaScript Function"
  end

  class Platform
    def self.set_flags!(*_args, **_kwargs) = nil
  end

  class Snapshot
    attr_reader :source

    def initialize(source = "")
      @source = +source.to_s
    end

    def warmup!(src)
      Context.new(snapshot: self).eval(src)
      self
    end

    def dump = @source.dup
    def size = @source.bytesize
    def self.load(data) = new(data)
  end

  class Context
    def initialize(isolate: nil, snapshot: nil, max_memory: nil, timeout: nil, ensure_gc_after_idle: nil, marshal_stack_depth: nil)
      raise Error, "MiniRacer needs Kenny's JavaScript engine (kenny_assets), which isn't linked in" unless defined?(::Kenny::JS)
      @context = ::Kenny::JS.context
      @disposed = false
      @timeout = timeout
      eval(snapshot.source) if snapshot && !snapshot.source.empty?
    end

    def eval(str, options = nil)
      raise ArgumentError, "wrong type argument #{str.class} (should be a string)" unless str.is_a?(String)
      run(str.encode(Encoding::UTF_8))
    end
    alias eval_await eval

    def call(function_name, *arguments)
      run("#{function_name}.apply(this, #{::JSON.generate(arguments)})")
    end
    alias call_await call

    # (defines the function, and those of a dotted path's objects, but calling it throws)
    def attach(name, _callback)
      raise ContextDisposedError, "attempted to call function on a disposed context!" if @disposed
      parts = name.to_s.split(".")
      js = +"(function(){var o = globalThis;"
      parts[0...-1].each { |p| js << "o = (o[#{p.to_json}] = o[#{p.to_json}] || {});" }
      msg = "MiniRacer::Context#attach(#{name.to_s.to_json[1...-1]}): calling Ruby from JavaScript is not supported by Kenny (docs/LANGUAGE.md)"
      js << "o[#{parts.last.to_json}] = function(){ throw new Error(#{msg.to_json}); };})()"
      run(js)
      nil
    end

    def load(filename) = eval(File.read(filename))

    def dispose
      @disposed = true
      @context = nil
      nil
    end

    def stop = nil
    def low_memory_notification = nil
    def idle_notification(_ms = nil) = true
    def perform_microtask_checkpoint = nil
    def pump_message_loop = false
    def heap_stats = {}
    def heap_snapshot = "{}"

    def write_heap_snapshot(file_or_io)
      f = String === file_or_io ? File.open(file_or_io, "w") : file_or_io
      raise ArgumentError, "file_or_io" unless File === f
      f.write(heap_snapshot)
    ensure
      f.close if String === file_or_io && f
    end

    def isolate = self

    private

    def run(source)
      raise ContextDisposedError, "attempted to call function on a disposed context!" if @disposed
      status, kind_or_json, message = ::Kenny::JS.eval(@context, source)
      if status == "ok"
        kind_or_json && ::JSON.parse("[#{kind_or_json}]", create_additions: false)[0]
      elsif kind_or_json == "syntax"
        raise ParseError, message
      else
        raise RuntimeError, message
      end
    end
  end

  class Isolate
    def initialize(snapshot = nil)
      @snapshot = snapshot
    end

    def idle_notification(_ms = nil) = true
    def low_memory_notification = nil
  end
end
