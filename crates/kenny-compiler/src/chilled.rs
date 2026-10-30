//! String literals of a file without a `frozen_string_literal` comment are *chilled*, as in
//! CRuby 3.4 and 4.0: every evaluation is a new String, not frozen, that shares the literal's
//! bytes until it is changed (kenny-runtime abi.rs `kn_str_chilled`). That costs an object
//! each time, where a frozen literal costs nothing (one object for the run).
//!
//! When the program can't tell the two apart, the literal is compiled frozen: it is only read
//! by the call it is passed to, which neither keeps it, returns it, changes it nor asks whether
//! it is frozen — an operand of `==`, `h["key"]`, `s.start_with?("x")`, `"%d" % n`,
//! `puts "..."`, `"abc".size`, ... (`shared_lits`). The methods are recognised by name, so a
//! user method of one of these names that keeps or changes its String argument would see it
//! frozen; methods that commonly keep their argument (`<<`, `push`, `new`, `write`, `raise`,
//! ...) are not on the lists.

use ruby_prism::*;
use std::collections::HashSet;

/// Methods whose String receiver can be the frozen literal (the call has no block): they
/// don't change it or return it, and answer the same for a frozen String.
const RECV: &[&[u8]] = &[
    b"size", b"length", b"bytesize", b"empty?", b"==", b"!=", b"===", b"=~", b"!~", b"eql?", b"<=>", b"<", b">", b"<=",
    b">=", b"+", b"*", b"%", b"hash", b"to_sym", b"intern", b"to_i", b"to_f", b"to_r", b"to_c", b"ord", b"start_with?",
    b"end_with?", b"include?", b"index", b"rindex", b"byteindex", b"byterindex", b"match?", b"count", b"unpack",
    b"unpack1", b"split", b"chars", b"bytes", b"lines", b"codepoints", b"upcase", b"downcase", b"capitalize",
    b"swapcase", b"strip", b"lstrip", b"rstrip", b"chomp", b"chop", b"squeeze", b"delete", b"tr", b"tr_s", b"sub",
    b"gsub", b"center", b"ljust", b"rjust", b"reverse", b"succ", b"next", b"encoding", b"ascii_only?",
    b"valid_encoding?", b"b", b"scrub", b"unicode_normalize", b"inspect", b"dump", b"freeze", b"-@", b"dedup", b"+@",
    b"dup", b"casecmp", b"casecmp?", b"partition", b"rpartition", b"delete_prefix", b"delete_suffix", b"byteslice",
    b"slice", b"[]", b"hex", b"oct", b"sum", b"encode", b"to_json",
];

/// Methods whose String arguments (all positional ones) can be frozen literals: they only
/// read them (compare, search, use as a key, a separator, a pattern, a format or a name).
const ARGS: &[&[u8]] = &[
    b"==", b"!=", b"===", b"=~", b"!~", b"eql?", b"<=>", b"<", b">", b"<=", b">=", b"+", b"start_with?", b"end_with?",
    b"include?", b"index", b"rindex", b"byteindex", b"match", b"match?", b"scan", b"split", b"sub", b"gsub", b"sub!",
    b"gsub!", b"tr", b"tr!", b"tr_s", b"tr_s!", b"delete", b"delete!", b"count", b"squeeze", b"squeeze!", b"join",
    b"casecmp", b"casecmp?", b"partition", b"rpartition", b"delete_prefix", b"delete_suffix", b"delete_prefix!",
    b"delete_suffix!", b"chomp", b"chomp!", b"center", b"ljust", b"rjust", b"each_line", b"lines", b"unpack",
    b"unpack1", b"pack", b"encode", b"force_encoding", b"strftime", b"puts", b"print", b"warn", b"[]", b"key?",
    b"has_key?", b"member?", b"dig", b"format", b"sprintf", b"printf", b"respond_to?", b"const_get",
    b"const_defined?", b"instance_variable_get", b"instance_variable_defined?", b"method", b"public_method",
    b"instance_method", b"alias_method", b"method_defined?", b"public_method_defined?", b"private_method_defined?",
    b"protected_method_defined?", b"remove_method", b"undef_method", b"require", b"Integer", b"Float", b"replace",
];

/// Methods whose first argument only can be a frozen literal (a key or a name; the others
/// may be kept: `h.fetch("k", "default")`, `h["k"] = "v"`).
const FIRST_ARG: &[&[u8]] = &[b"fetch", b"[]=", b"send", b"public_send", b"__send__", b"define_method", b"instance_variable_set"];

/// The chilled String literals under `root` that may be compiled frozen, by address (`key`).
pub fn shared_lits(root: &Node) -> HashSet<usize> {
    let mut v = V { found: HashSet::new() };
    v.visit(root);
    v.found
}

/// A literal's identity for `shared_lits`: where its source text is.
pub fn key(s: &StringNode) -> usize {
    s.location().as_slice().as_ptr() as usize
}

/// A literal that is chilled (no magic comment either way).
fn chilled<'pr>(n: &Node<'pr>) -> Option<StringNode<'pr>> {
    n.as_string_node().filter(|s| !s.is_frozen() && !s.is_mutable())
}

struct V {
    found: HashSet<usize>,
}

impl<'pr> Visit<'pr> for V {
    fn visit_call_node(&mut self, c: &CallNode<'pr>) {
        let name = c.name().as_slice();
        let block = c.block().is_some();
        if !block && RECV.contains(&name) {
            if let Some(s) = c.receiver().as_ref().and_then(chilled) {
                self.found.insert(key(&s));
            }
        }
        let all = ARGS.contains(&name);
        if all || FIRST_ARG.contains(&name) {
            if let Some(args) = c.arguments() {
                for (i, a) in args.arguments().iter().enumerate() {
                    if i > 0 && !all {
                        break;
                    }
                    if let Some(s) = chilled(&a) {
                        self.found.insert(key(&s));
                    }
                }
            }
        }
        visit_call_node(self, c);
    }
}
