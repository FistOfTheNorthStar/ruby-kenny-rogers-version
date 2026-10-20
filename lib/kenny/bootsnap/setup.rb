# Bootsnap caches compiled code and load-path scans to speed up boot. A Kenny binary has
# no load path to scan and no code to compile at boot (it is compiled ahead of time), so
# `require "bootsnap/setup"` has nothing to do.
