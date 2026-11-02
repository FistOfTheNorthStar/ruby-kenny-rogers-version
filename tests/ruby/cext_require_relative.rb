# kenny-run-flags: --ext tests/ruby/cext_require_relative/relext
# `require_relative` of a C extension (landlock's lib/landlock/native.rb requires the
# landlock.bundle next to it): the linked-in extension's Init runs. (CRuby: build
# cext_require_relative/relext with `ruby extconf.rb && make` and copy relext.bundle there.)
p require_relative("cext_require_relative/relext/relext")
p relext_hi
p require_relative("cext_require_relative/relext/relext")
begin
  require_relative "cext_require_relative/missing"
rescue LoadError => e
  puts "LoadError"
end
