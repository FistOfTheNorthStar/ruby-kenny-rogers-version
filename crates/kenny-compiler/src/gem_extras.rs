//! Whether the program uses the rest of RubyGems. Kenny's `Gem` (lib/kenny/kenny/gem.rb) is
//! compiled into every program; RubyGems' remaining classes (`Gem::ConfigFile`,
//! `Gem::Installer`, `Gem::SourceList` ...) and `Gem.load_yaml` live in kenny/gem_rubygems.rb
//! (`Options::gem_extras`), whose autoloads pull in about 150 of RubyGems' files and Psych. It
//! is compiled in only when code outside Kenny's own Gem files names one of those classes as
//! `Gem::<name>` or calls `Gem.load_yaml`. The compiler notes it where it compiles the constant
//! path or the call, so it costs nothing in the generated code. Only dynamic lookups
//! (`Gem.const_get(:Installer)`, eval'd strings) escape this: they raise NameError. What
//! doesn't count: Kenny's own Gem files and the standard library's `rubygems/` directory
//! (`Options::gem_extras_quiet`), whose files Kenny's Gem loads for `Gem::Version` and friends.

use crate::*;
use ruby_prism::*;

/// The classes kenny/gem_rubygems.rb autoloads (keep in sync with that file).
const NAMES: &[&[u8]] = &[
    b"ConfigFile", b"CIDetector", b"DependencyList", b"Installer", b"Licenses", b"NameTuple", b"PathSupport",
    b"RequestSet", b"Resolver", b"Source", b"SourceList", b"SpecFetcher", b"SpecificationPolicy", b"Util",
];

/// `Gem` or `::Gem`
pub(crate) fn is_gem(n: &Node) -> bool {
    if let Some(c) = n.as_constant_read_node() {
        return c.name().as_slice() == b"Gem";
    }
    if let Some(p) = n.as_constant_path_node() {
        return p.parent().is_none() && p.name().is_some_and(|c| c.as_slice() == b"Gem");
    }
    false
}

pub(crate) fn is_extra(name: &[u8]) -> bool {
    NAMES.contains(&name)
}

impl<M: Module> Compiler<M> {
    /// Code names a class (or method) of kenny/gem_rubygems.rb: see the module comment.
    pub(crate) fn note_gem_extras(&mut self) {
        if self.gem_extras_used || self.eval_unit || self.gem_extras.is_none() {
            return;
        }
        // (Kenny's own Gem files, and the standard library's RubyGems files that Kenny's Gem
        // autoloads (Gem::Version, Gem::Platform ...), refer to them lazily, from methods only
        // the rest of RubyGems calls)
        if self.file.ends_with("kenny/gem.rb") || self.file.ends_with("kenny/gem_specification.rb") {
            return;
        }
        if self.gem_extras_quiet.as_ref().is_some_and(|d| self.file.starts_with(d)) {
            return;
        }
        self.gem_extras_used = true;
    }
}
