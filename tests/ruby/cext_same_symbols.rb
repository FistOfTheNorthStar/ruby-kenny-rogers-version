# kenny-run-flags: --ext tests/ruby/cext_same_symbols/exta --ext tests/ruby/cext_same_symbols/extb
# Two C extensions that define the same C symbols (rinku and redcarpet both vendor Sundown's
# buffer.c) link into one binary, each using its own. (CRuby: build each directory with
# `ruby extconf.rb && make`, then `ruby -I exta -I extb`.)
require "exta"
require "extb"
p ext_a(1), ext_b(1)
