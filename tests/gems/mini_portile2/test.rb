require "bundler/setup"
require "mini_portile2"

r = MiniPortile.new("libfoo", "1.2.3")
r.files = [{ url: "http://example.invalid/libfoo-1.2.3.tar.gz", sha256: "0" * 64 }]
r.configure_options += ["--enable-static"]
r.patch_files = []
r.target = "ports"
r.host = "x86_64-linux-gnu"
p r.name, r.version, r.files.size, r.target, r.host
p r.configure_options.include?("--enable-static")
p r.path.end_with?("ports/x86_64-linux-gnu/libfoo/1.2.3")
p r.respond_to?(:cook), r.respond_to?(:activate)
r2 = MiniPortile.new("libz", "1.3", make_command: "gmake", gcc_command: "clang")
p r2.make_cmd, r2.gcc_cmd, r2.source_directory
r2.source_directory = "/src/libz"
p r2.source_directory, r2.files
c = MiniPortileCMake.new("libbar", "0.1", cmake_build_type: "Release")
p c.name, c.version, c.configure_options.grep(/BUILD_TYPE/)
p MiniPortile::VERSION.is_a?(String)
