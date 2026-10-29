# frozen_string_literal: true
# Kenny: the tree API's Psych::ClassLoader, Psych::ScalarScanner and Psych::Visitors::ToRuby
# (Node#to_ruby) are the standard library's; these are the two methods psych's C extension
# adds to them.
require "psych/class_loader"
require "psych/visitors/to_ruby"

module Psych
  class ClassLoader
    private

    def path2class(path)
      Object.const_get(path)
    rescue NameError
      raise ArgumentError, "undefined class/module #{path}"
    end
  end

  module Visitors
    class ToRuby
      private

      def build_exception(klass, message)
        klass.new(message)
      end
    end
  end
end
