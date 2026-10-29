# The coverage library for Kenny binaries: Coverage is native (kenny-runtime/src/coverage.rs),
# and a program that requires this file is compiled with coverage counters
# (kenny-compiler/src/coverage.rs). Coverage.line_stub is native too (no RubyVM).
require "kenny/coverage_native"
