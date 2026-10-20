# frozen_string_literal: true

# ExecJS in a Kenny binary: the execjs gem, plus a JavaScript runtime that runs in-process
# (QuickJS, in the runtime: crates/kenny-runtime/src/assets.rs), chosen ahead of the gem's
# runtimes, so uglifier, coffee-script and other ExecJS users need no Node. EXECJS_RUNTIME still
# picks another runtime (e.g. EXECJS_RUNTIME=Node).
require "execjs/module"
require "execjs/runtimes"
require "json"

begin
  require "kenny_assets"
rescue LoadError
  # not linked in: ExecJS::Runtimes::Kenny is unavailable
end

module ExecJS
  class KennyRuntime < Runtime
    class Context < Runtime::Context
      def initialize(runtime, source = "", options = {})
        @context = ::Kenny::JS.context
        run(source.encode(Encoding::UTF_8)) if /\S/ =~ source
      end

      def exec(source, options = {})
        source = source.encode(Encoding::UTF_8)
        eval "(function(){#{source}})()" if /\S/ =~ source
      end

      def eval(source, options = {})
        source = source.encode(Encoding::UTF_8)
        run("(#{source})") if /\S/ =~ source
      end

      def call(identifier, *args)
        eval "#{identifier}.apply(this, #{::JSON.generate(args)})"
      end

      private

      def run(source)
        status, kind_or_json, message = ::Kenny::JS.eval(@context, source)
        if status == "ok"
          kind_or_json && ::JSON.parse("[#{kind_or_json}]", create_additions: false)[0]
        elsif kind_or_json == "syntax"
          raise RuntimeError, message
        else
          raise ProgramError, message
        end
      end
    end

    def name
      "Kenny (QuickJS)"
    end

    def available?
      defined?(::Kenny::JS) ? true : false
    end
  end

  module Runtimes
    Kenny = KennyRuntime.new
    runtimes.unshift(Kenny)
  end

  def self.runtime
    @runtime ||= Runtimes.autodetect
  end
end
