#!/usr/bin/env python3
"""Regenerate the runtime symbol table in crates/kenny-runtime-jit/src/lib.rs from the
compiler's RUNTIME list (every kn_* function compiled code may call)."""
import glob, re, os
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
lib = open(f"{root}/crates/kenny-compiler/src/lib.rs").read()
names = [n for n in dict.fromkeys(re.findall(r'\("(kn_[a-z_0-9]+)"', lib)) if not n.startswith("kn_start")]
mods = {}
for f in glob.glob(f"{root}/crates/kenny-runtime/src/*.rs"):
    src = open(f).read()
    for n in names:
        if re.search(r'fn ' + n + r'\(', src):
            mods[n] = os.path.basename(f)[:-3]
missing = [n for n in names if n not in mods]
if missing:
    raise SystemExit(f"runtime functions not found: {missing}")
entries = "\n".join(f'        ("{n}", kenny_runtime::{mods[n]}::{n} as *const u8),' for n in names)
p = f"{root}/crates/kenny-runtime-jit/src/lib.rs"
s = open(p).read()
s = re.sub(r'(fn symbols\(\) -> Vec<\(&\'static str, \*const u8\)> \{\n    vec!\[\n)(.*?)(\n        \("KN_GC_PENDING")',
           lambda m: m.group(1) + entries + m.group(3), s, flags=re.S)
open(p, "w").write(s)
print(f"{len(names)} runtime symbols")
