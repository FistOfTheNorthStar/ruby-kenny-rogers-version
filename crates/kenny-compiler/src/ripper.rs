//! Whether the program can parse with Ripper. Kenny's Ripper parses with Prism, whose Ruby
//! library and libprism add megabytes to a binary, so that backend (`Options::ripper_backend`)
//! is compiled in only when code outside Ripper's own files evaluates the constant `Ripper` in
//! a way that can reach its parser: `Ripper.sexp(..)`, `Ripper.new(..)`, `class X < Ripper`,
//! `Ripper::Lexer`, `Ripper::SexpBuilder`, `Ripper` as a value ... The compiler notes it where it
//! compiles a constant read (expr.rs), so it costs nothing in the generated code. An AOT build
//! sees all the code: only dynamic lookups (`Object.const_get(:Ripper)`, eval'd strings) escape
//! this, and parsing reached that way raises NotImplementedError saying so
//! (lib/kenny/ripper_ext.rb). What doesn't count: Ripper's data (`Ripper::EVENTS`, the event
//! tables, `EXPR_*`, `Ripper::Version`, `Ripper.lex_state_name`, `Ripper.dedent_string`),
//! `defined?(Ripper)`, and reopening `class Ripper`.

use crate::*;
use ruby_prism::*;

/// `Ripper` or `::Ripper`
pub(crate) fn is_ripper(n: &Node) -> bool {
    if let Some(c) = n.as_constant_read_node() {
        return c.name().as_slice() == b"Ripper";
    }
    if let Some(p) = n.as_constant_path_node() {
        return p.parent().is_none() && p.name().is_some_and(|c| c.as_slice() == b"Ripper");
    }
    false
}

/// `Ripper::<name>` that is data, not the parser
pub(crate) fn is_data_const(name: &[u8]) -> bool {
    matches!(name, b"EVENTS" | b"PARSER_EVENTS" | b"SCANNER_EVENTS" | b"PARSER_EVENT_TABLE" | b"SCANNER_EVENT_TABLE" | b"Version") || name.starts_with(b"EXPR_")
}

/// `Ripper.<name>(..)` that doesn't parse
pub(crate) fn is_data_method(name: &[u8]) -> bool {
    matches!(name, b"lex_state_name" | b"dedent_string")
}

impl<M: Module> Compiler<M> {
    /// The constant `Ripper` is evaluated here (unless `ripper_quiet`): see the module comment.
    pub(crate) fn note_ripper(&mut self) {
        if self.ripper_used || self.ripper_quiet || self.eval_unit || self.ripper_backend.is_none() {
            return;
        }
        // Ripper's own files: the standard library's ripper.rb and ripper/, Kenny's ripper.so
        let own = self.load_paths.iter().any(|lp| {
            let lp = lp.canonicalize().unwrap_or(lp.clone());
            self.file.strip_prefix(&lp).is_ok_and(|rel| {
                rel == Path::new("ripper.rb") || rel == Path::new("ripper.so") || rel.starts_with("ripper") && rel.components().count() > 1
            })
        });
        if !own {
            self.ripper_used = true;
        }
    }

    /// `f` with the constant `Ripper` not counting as a use of its parser.
    pub(crate) fn ripper_quietly<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.ripper_quiet, true);
        let r = f(self);
        self.ripper_quiet = saved;
        r
    }
}
