//! Closure-capture analysis: which locals of a scope are referenced from nested blocks
//! (those live in a heap environment; everything else stays in registers).

use ruby_prism::*;
use std::collections::HashSet;

pub struct Captures {
    pub captured: HashSet<Vec<u8>>,
    pub has_block: bool,
    pub zsuper_in_block: bool,
}

struct V {
    depth: u32,
    captured: HashSet<Vec<u8>>,
    has_block: bool,
    zsuper_in_block: bool,
}

impl V {
    fn local(&mut self, name: &[u8], depth: u32) {
        if self.depth >= 1 && depth == self.depth {
            self.captured.insert(name.to_vec());
        }
    }
}

impl<'pr> Visit<'pr> for V {
    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        if self.depth == 0 {
            self.has_block = true;
        }
        self.depth += 1;
        visit_block_node(self, node);
        self.depth -= 1;
    }
    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        if self.depth == 0 {
            self.has_block = true;
        }
        self.depth += 1;
        visit_lambda_node(self, node);
        self.depth -= 1;
    }
    // New scopes: only the parts evaluated in the current scope are visited.
    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        if let Some(r) = node.receiver() {
            self.visit(&r);
        }
    }
    fn visit_class_node(&mut self, node: &ClassNode<'pr>) {
        self.visit(&node.constant_path());
        if let Some(s) = node.superclass() {
            self.visit(&s);
        }
    }
    fn visit_module_node(&mut self, node: &ModuleNode<'pr>) {
        self.visit(&node.constant_path());
    }
    fn visit_singleton_class_node(&mut self, node: &SingletonClassNode<'pr>) {
        self.visit(&node.expression());
    }
    fn visit_forwarding_super_node(&mut self, node: &ForwardingSuperNode<'pr>) {
        if self.depth >= 1 {
            self.zsuper_in_block = true;
        }
        visit_forwarding_super_node(self, node);
    }
    fn visit_local_variable_read_node(&mut self, node: &LocalVariableReadNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
    }
    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
        visit_local_variable_write_node(self, node);
    }
    fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
    }
    fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
        visit_local_variable_operator_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
        visit_local_variable_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        self.local(node.name().as_slice(), node.depth());
        visit_local_variable_and_write_node(self, node);
    }
}

/// Analyse the nodes making up one scope's body (and parameter defaults).
pub fn captures<'pr>(nodes: &[&Node<'pr>]) -> Captures {
    let mut v = V { depth: 0, captured: HashSet::new(), has_block: false, zsuper_in_block: false };
    for n in nodes {
        v.visit(n);
    }
    Captures { captured: v.captured, has_block: v.has_block, zsuper_in_block: v.zsuper_in_block }
}

/// Is `n` an arithmetic expression whose result may be a Float (`a * b`, `x + 1.5`,
/// a Float literal, ...)? Locals assigned such values get an unboxed shadow (fastpath.rs).
pub fn is_numeric_expr(n: &Node) -> bool {
    if n.as_float_node().is_some() {
        return true;
    }
    if let Some(p) = n.as_parentheses_node() {
        return p.body().is_some_and(|b| match b.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                v.len() == 1 && is_numeric_expr(&v[0])
            }
            None => is_numeric_expr(&b),
        });
    }
    match n.as_call_node() {
        Some(c) => {
            matches!(c.name().as_slice(), b"+" | b"-" | b"*" | b"/")
                && c.receiver().is_some()
                && c.block().is_none()
                && c.arguments().is_some_and(|a| a.arguments().iter().count() == 1)
        }
        None => false,
    }
}

struct Num {
    depth: u32,
    names: HashSet<Vec<u8>>,
}
impl<'pr> Visit<'pr> for Num {
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
    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        if self.depth == 0 && node.depth() == 0 && is_numeric_expr(&node.value()) {
            self.names.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_write_node(self, node);
    }
    fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
        if self.depth == 0 && node.depth() == 0 && matches!(node.binary_operator().as_slice(), b"+" | b"-" | b"*" | b"/") {
            self.names.insert(node.name().as_slice().to_vec());
        }
        visit_local_variable_operator_write_node(self, node);
    }
}

/// Locals of one scope (not counting nested blocks) assigned from arithmetic.
pub fn numeric_locals<'pr>(nodes: &[&Node<'pr>]) -> HashSet<Vec<u8>> {
    let mut v = Num { depth: 0, names: HashSet::new() };
    for n in nodes {
        v.visit(n);
    }
    v.names
}
