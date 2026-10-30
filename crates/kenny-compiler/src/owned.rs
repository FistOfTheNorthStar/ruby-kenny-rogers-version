//! Locals built by `s = s + x` / `s += x` (statements): appended to in place while the
//! local holds the only reference to its String.
//!
//! Each such local gets an i8 "owned" register, 1 while its value is a String no one else
//! can see: one just made by a non-frozen literal assigned to it, or by the builtin String#+
//! of an append (kenny-runtime string.rs `kn_str_append_own`). Any other read of the local
//! (the value may be kept somewhere) or other write clears it (scopes.rs `local_get` /
//! `local_set`); an append with it set extends the String instead of copying it.
//!
//! So that every read goes through those, the local must not be referenced from a block
//! (even an inlined one) and the appended value must not mention it.

use ruby_prism::*;
use std::collections::HashSet;

/// The scope's locals that may be appended to in place.
pub fn append_locals<'pr>(nodes: &[&Node<'pr>]) -> HashSet<Vec<u8>> {
    let mut w = scan(nodes);
    if w.off {
        return HashSet::new();
    }
    w.appends.retain(|n| !w.bad.contains(n) && w.strs.contains(n));
    w.appends
}

/// The scope's locals assigned a String literal somewhere (not Float locals: analysis.rs
/// `numeric_locals`).
pub fn string_locals<'pr>(nodes: &[&Node<'pr>]) -> HashSet<Vec<u8>> {
    scan(nodes).strs
}

fn scan<'pr>(nodes: &[&Node<'pr>]) -> W {
    let mut w = W { depth: 0, bad: HashSet::new(), appends: HashSet::new(), strs: HashSet::new(), off: false };
    for n in nodes {
        w.visit(n);
    }
    w
}

/// `name = name + x` / `name += x` with `x` not mentioning `name`: the appended value.
pub fn append_arg<'pr>(node: &Node<'pr>, name: &[u8]) -> Option<Node<'pr>> {
    let arg = if let Some(w) = node.as_local_variable_write_node() {
        if w.name().as_slice() != name || w.depth() != 0 {
            return None;
        }
        let c = w.value().as_call_node()?;
        if c.name().as_slice() != b"+" || c.block().is_some() || c.is_safe_navigation() {
            return None;
        }
        let r = c.receiver()?.as_local_variable_read_node()?;
        if r.name().as_slice() != name || r.depth() != 0 {
            return None;
        }
        let args: Vec<Node> = c.arguments()?.arguments().iter().collect();
        if args.len() != 1 || args[0].as_splat_node().is_some() || args[0].as_keyword_hash_node().is_some() || args[0].as_block_argument_node().is_some() {
            return None;
        }
        args.into_iter().next().unwrap()
    } else {
        let w = node.as_local_variable_operator_write_node()?;
        if w.name().as_slice() != name || w.depth() != 0 || w.binary_operator().as_slice() != b"+" {
            return None;
        }
        w.value()
    };
    if mentions(&arg, name) || is_number(&arg) {
        return None;
    }
    Some(arg)
}

/// The written name of an append statement (`append_arg`).
pub fn append_target(node: &Node) -> Option<Vec<u8>> {
    let name = match node.as_local_variable_write_node() {
        Some(w) => w.name().as_slice().to_vec(),
        None => node.as_local_variable_operator_write_node()?.name().as_slice().to_vec(),
    };
    append_arg(node, &name).map(|_| name)
}

fn is_number(n: &Node) -> bool {
    n.as_integer_node().is_some() || n.as_float_node().is_some() || n.as_rational_node().is_some() || n.as_imaginary_node().is_some()
}

/// A literal that makes a new String each time it runs (one with `frozen_string_literal:
/// false`, or a chilled one: an assigned literal is never compiled frozen, chilled.rs).
pub fn fresh_str(n: &Node) -> bool {
    n.as_string_node().is_some_and(|s| !s.is_frozen())
}

fn mentions(n: &Node, name: &[u8]) -> bool {
    struct M<'a> {
        name: &'a [u8],
        found: bool,
    }
    impl<'pr> Visit<'pr> for M<'_> {
        fn visit_local_variable_read_node(&mut self, n: &LocalVariableReadNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
        }
        fn visit_local_variable_write_node(&mut self, n: &LocalVariableWriteNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
            visit_local_variable_write_node(self, n);
        }
        fn visit_local_variable_operator_write_node(&mut self, n: &LocalVariableOperatorWriteNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
            visit_local_variable_operator_write_node(self, n);
        }
        fn visit_local_variable_or_write_node(&mut self, n: &LocalVariableOrWriteNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
            visit_local_variable_or_write_node(self, n);
        }
        fn visit_local_variable_and_write_node(&mut self, n: &LocalVariableAndWriteNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
            visit_local_variable_and_write_node(self, n);
        }
        fn visit_local_variable_target_node(&mut self, n: &LocalVariableTargetNode<'pr>) {
            self.found |= n.name().as_slice() == self.name;
        }
        fn visit_call_node(&mut self, n: &CallNode<'pr>) {
            // (`binding`, `eval`, `local_variables` see every local)
            self.found |= n.receiver().is_none() && matches!(n.name().as_slice(), b"binding" | b"eval" | b"local_variables");
            visit_call_node(self, n);
        }
    }
    let mut m = M { name, found: false };
    m.visit(n);
    m.found
}

struct W {
    depth: u32,
    bad: HashSet<Vec<u8>>,
    appends: HashSet<Vec<u8>>,
    strs: HashSet<Vec<u8>>,
    /// `binding` / `eval` / `local_variables`: none
    off: bool,
}

impl W {
    /// A reference to `name` that isn't through `local_get` / `local_set` of this scope.
    fn other(&mut self, name: &[u8], depth: u32) {
        if depth == self.depth {
            self.bad.insert(name.to_vec());
        }
    }
    /// A plain read or write: only a block's disqualifies.
    fn plain(&mut self, name: &[u8], depth: u32) {
        if self.depth > 0 {
            self.other(name, depth);
        }
    }
}

impl<'pr> Visit<'pr> for W {
    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        self.depth += 1;
        visit_block_node(self, node);
        self.depth -= 1;
    }
    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        self.depth += 1;
        visit_lambda_node(self, node);
        self.depth -= 1;
    }
    fn visit_def_node(&mut self, _: &DefNode<'pr>) {}
    fn visit_class_node(&mut self, _: &ClassNode<'pr>) {}
    fn visit_module_node(&mut self, _: &ModuleNode<'pr>) {}
    fn visit_singleton_class_node(&mut self, _: &SingletonClassNode<'pr>) {}
    fn visit_call_node(&mut self, n: &CallNode<'pr>) {
        if n.receiver().is_none() && matches!(n.name().as_slice(), b"binding" | b"eval" | b"local_variables") {
            self.off = true;
        }
        visit_call_node(self, n);
    }
    fn visit_forwarding_super_node(&mut self, n: &ForwardingSuperNode<'pr>) {
        // (passes the method's parameters without reading them)
        self.off = true;
        visit_forwarding_super_node(self, n);
    }
    fn visit_local_variable_read_node(&mut self, n: &LocalVariableReadNode<'pr>) {
        self.plain(n.name().as_slice(), n.depth());
    }
    fn visit_local_variable_write_node(&mut self, n: &LocalVariableWriteNode<'pr>) {
        let name = n.name().as_slice();
        self.plain(name, n.depth());
        if self.depth == 0 && n.depth() == 0 {
            // (a String local, not a number: frozen literals too, the first append copies)
            let v = n.value();
            if v.as_string_node().is_some() || v.as_interpolated_string_node().is_some() {
                self.strs.insert(name.to_vec());
            }
            if append_arg(&n.as_node(), name).is_some() {
                self.appends.insert(name.to_vec());
            }
        }
        visit_local_variable_write_node(self, n);
    }
    fn visit_local_variable_operator_write_node(&mut self, n: &LocalVariableOperatorWriteNode<'pr>) {
        let name = n.name().as_slice();
        self.plain(name, n.depth());
        if self.depth == 0 && n.depth() == 0 && append_arg(&n.as_node(), name).is_some() {
            self.appends.insert(name.to_vec());
        }
        visit_local_variable_operator_write_node(self, n);
    }
    fn visit_local_variable_or_write_node(&mut self, n: &LocalVariableOrWriteNode<'pr>) {
        self.other(n.name().as_slice(), n.depth());
        visit_local_variable_or_write_node(self, n);
    }
    fn visit_local_variable_and_write_node(&mut self, n: &LocalVariableAndWriteNode<'pr>) {
        self.other(n.name().as_slice(), n.depth());
        visit_local_variable_and_write_node(self, n);
    }
    fn visit_local_variable_target_node(&mut self, n: &LocalVariableTargetNode<'pr>) {
        self.other(n.name().as_slice(), n.depth());
    }
}
