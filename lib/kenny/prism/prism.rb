# frozen_string_literal: true
#
# The prism gem's native part (`prism/prism`, a C extension in CRuby), as far as Kenny provides
# it: libprism 1.9.0, linked into the runtime (kenny-runtime/src/prism.rs, `Kenny::Prism`),
# serializes each parse and the gem's own Prism::Serialize loads it, as the gem's FFI backend
# (prism/ffi.rb, whose glue this follows) does. The serialized format is tied to the prism
# release, so `kenny build` pairs this file only with a prism Ruby library of the same version
# (crates/kenny/src/main.rs); `dump_options` below is prism 1.9.0's.
require "kenny_prism"

module Prism
  # (the C extension defines it from libprism's version)
  VERSION = ::Kenny::Prism::LIBPRISM_VERSION

  class << self
    # Mirror the Prism.dump API by using the serialization API.
    def dump(source, **options)
      dump_common(source, options)
    end

    # Mirror the Prism.dump_file API by using the serialization API.
    def dump_file(filepath, **options)
      options[:filepath] = filepath
      dump_common(kenny_read_file(filepath), options)
    end

    # Mirror the Prism.lex API by using the serialization API.
    def lex(code, **options)
      lex_common(code, code, options)
    end

    # Mirror the Prism.lex_file API by using the serialization API.
    def lex_file(filepath, **options)
      options[:filepath] = filepath
      code = kenny_read_file(filepath)
      lex_common(code, code, options)
    end

    # Mirror the Prism.parse API by using the serialization API.
    def parse(code, **options)
      parse_common(code, code, options)
    end

    # Mirror the Prism.parse_file API by using the serialization API.
    def parse_file(filepath, **options)
      options[:filepath] = filepath
      code = kenny_read_file(filepath)
      parse_common(code, code, options)
    end

    # Mirror the Prism.parse_stream API: the stream is read line by line (`gets`) to its end.
    def parse_stream(stream, **options)
      source = +""
      while (line = stream.gets)
        source << line
      end
      parse_common(source, source, options)
    end

    # Mirror the Prism.parse_comments API by using the serialization API.
    def parse_comments(code, **options)
      parse_comments_common(code, code, options)
    end

    # Mirror the Prism.parse_file_comments API by using the serialization API.
    def parse_file_comments(filepath, **options)
      options[:filepath] = filepath
      code = kenny_read_file(filepath)
      parse_comments_common(code, code, options)
    end

    # Mirror the Prism.parse_lex API by using the serialization API.
    def parse_lex(code, **options)
      parse_lex_common(code, code, options)
    end

    # Mirror the Prism.parse_lex_file API by using the serialization API.
    def parse_lex_file(filepath, **options)
      options[:filepath] = filepath
      code = kenny_read_file(filepath)
      parse_lex_common(code, code, options)
    end

    # Mirror the Prism.parse_success? API by using the serialization API.
    def parse_success?(code, **options)
      ::Kenny::Prism.success?(kenny_source(code), dump_options(options))
    end

    # Mirror the Prism.parse_failure? API by using the serialization API.
    def parse_failure?(code, **options)
      !parse_success?(code, **options)
    end

    # Mirror the Prism.parse_file_success? API by using the serialization API.
    def parse_file_success?(filepath, **options)
      options[:filepath] = filepath
      ::Kenny::Prism.success?(kenny_read_file(filepath), dump_options(options))
    end

    # Mirror the Prism.parse_file_failure? API by using the serialization API.
    def parse_file_failure?(filepath, **options)
      !parse_file_success?(filepath, **options)
    end

    # Mirror the Prism.profile API by using the serialization API.
    def profile(source, **options)
      ::Kenny::Prism.serialize(0, kenny_source(source), dump_options(options))
      nil
    end

    # Mirror the Prism.profile_file API by using the serialization API.
    def profile_file(filepath, **options)
      options[:filepath] = filepath
      ::Kenny::Prism.serialize(0, kenny_read_file(filepath), dump_options(options))
      nil
    end

    private

    # The source as a String (as the C extension, raising TypeError for anything else).
    def kenny_source(source)
      raise TypeError, "wrong argument type #{source.class} (expected String)" unless source.is_a?(String)
      source
    end

    # A file's contents, binary (as libprism reads it).
    def kenny_read_file(filepath)
      raise TypeError, "no implicit conversion of #{filepath.class} into String" unless filepath.is_a?(String)
      File.binread(filepath)
    end

    def dump_common(source, options) # :nodoc:
      dumped = ::Kenny::Prism.serialize(0, kenny_source(source), dump_options(options))
      dumped.freeze if options.fetch(:freeze, false)
      dumped
    end

    def lex_common(source, code, options) # :nodoc:
      serialized = ::Kenny::Prism.serialize(1, kenny_source(source), dump_options(options))
      Serialize.load_lex(code, serialized, options.fetch(:freeze, false))
    end

    def parse_common(source, code, options) # :nodoc:
      serialized = dump_common(source, options)
      Serialize.load_parse(code, serialized, options.fetch(:freeze, false))
    end

    def parse_comments_common(source, code, options) # :nodoc:
      serialized = ::Kenny::Prism.serialize(2, kenny_source(source), dump_options(options))
      Serialize.load_parse_comments(code, serialized, options.fetch(:freeze, false))
    end

    def parse_lex_common(source, code, options) # :nodoc:
      serialized = ::Kenny::Prism.serialize(3, kenny_source(source), dump_options(options))
      Serialize.load_parse_lex(code, serialized, options.fetch(:freeze, false))
    end

    # Return the value that should be dumped for the command_line option.
    def dump_options_command_line(options)
      command_line = options.fetch(:command_line, "")
      raise ArgumentError, "command_line must be a string" unless command_line.is_a?(String)

      command_line.each_char.inject(0) do |value, char|
        case char
        when "a" then value | 0b000001
        when "e" then value | 0b000010
        when "l" then value | 0b000100
        when "n" then value | 0b001000
        when "p" then value | 0b010000
        when "x" then value | 0b100000
        else raise ArgumentError, "invalid command_line option: #{char}"
        end
      end
    end

    # Return the value that should be dumped for the version option.
    def dump_options_version(version)
      current = version == "current"

      case current ? RUBY_VERSION : version
      when nil, "latest"
        0 # Handled in pm_parser_init
      when /\A3\.3(\.\d+)?\z/
        1
      when /\A3\.4(\.\d+)?\z/
        2
      when /\A3\.5(\.\d+)?\z/, /\A4\.0(\.\d+)?\z/
        3
      when /\A4\.1(\.\d+)?\z/
        4
      else
        if current
          raise CurrentVersionError, RUBY_VERSION
        else
          raise ArgumentError, "invalid version: #{version}"
        end
      end
    end

    # Convert the given options into a serialized options string.
    def dump_options(options)
      template = +""
      values = []

      template << "L"
      if (filepath = options[:filepath])
        values.push(filepath.bytesize, filepath.b)
        template << "A*"
      else
        values << 0
      end

      template << "l"
      values << options.fetch(:line, 1)

      template << "L"
      if (encoding = options[:encoding])
        name = encoding.is_a?(Encoding) ? encoding.name : encoding
        values.push(name.bytesize, name.b)
        template << "A*"
      else
        values << 0
      end

      template << "C"
      values << (options.fetch(:frozen_string_literal, false) ? 1 : 0)

      template << "C"
      values << dump_options_command_line(options)

      template << "C"
      values << dump_options_version(options[:version])

      template << "C"
      values << (options[:encoding] == false ? 1 : 0)

      template << "C"
      values << (options.fetch(:main_script, false) ? 1 : 0)

      template << "C"
      values << (options.fetch(:partial_script, false) ? 1 : 0)

      template << "C"
      values << (options.fetch(:freeze, false) ? 1 : 0)

      template << "L"
      if (scopes = options[:scopes])
        values << scopes.length

        scopes.each do |scope|
          locals = nil
          forwarding = 0

          case scope
          when Array
            locals = scope
          when Scope
            locals = scope.locals

            scope.forwarding.each do |forward|
              case forward
              when :*     then forwarding |= 0x1
              when :**    then forwarding |= 0x2
              when :&     then forwarding |= 0x4
              when :"..." then forwarding |= 0x8
              else raise ArgumentError, "invalid forwarding value: #{forward}"
              end
            end
          else
            raise TypeError, "wrong argument type #{scope.class.inspect} (expected Array or Prism::Scope)"
          end

          template << "L"
          values << locals.length

          template << "C"
          values << forwarding

          locals.each do |local|
            name = local.name
            template << "L"
            values << name.bytesize

            template << "A*"
            values << name.b
          end
        end
      else
        values << 0
      end

      values.pack(template)
    end
  end

  # The C extension's StringQuery class methods.
  class StringQuery
    class << self
      # Mirrors the C extension's StringQuery::local? method.
      def local?(string)
        kenny_query(0, string)
      end

      # Mirrors the C extension's StringQuery::constant? method.
      def constant?(string)
        kenny_query(1, string)
      end

      # Mirrors the C extension's StringQuery::method_name? method.
      def method_name?(string)
        kenny_query(2, string)
      end

      private

      def kenny_query(kind, string)
        raise TypeError, "wrong argument type #{string.class} (expected String)" unless string.is_a?(String)
        case ::Kenny::Prism.string_query(kind, string, string.encoding.name)
        when -1 then raise ArgumentError, "Invalid or non ascii-compatible encoding"
        when 0 then false
        else true
        end
      end
    end
  end
end
