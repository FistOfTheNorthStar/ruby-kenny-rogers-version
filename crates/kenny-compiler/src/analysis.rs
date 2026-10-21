//! Closure-capture analysis: which locals of a scope are referenced from nested blocks
//! (those live in a heap environment; everything else stays in registers).

use ruby_prism::*;
use std::collections::{HashMap, HashSet};

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
    // `...` and anonymous `*` / `**` / `&` arguments read the method's hidden locals: a block
    // using them captures those (a name the analysed scope doesn't have is ignored)
    fn visit_forwarding_arguments_node(&mut self, _: &ForwardingArgumentsNode<'pr>) {
        if self.depth >= 1 {
            for n in [&b"*"[..], b"**", b"&"] {
                self.captured.insert(n.to_vec());
            }
        }
    }
    fn visit_splat_node(&mut self, node: &SplatNode<'pr>) {
        if node.expression().is_none() && self.depth >= 1 {
            self.captured.insert(b"*".to_vec());
        }
        visit_splat_node(self, node);
    }
    fn visit_assoc_splat_node(&mut self, node: &AssocSplatNode<'pr>) {
        if node.value().is_none() && self.depth >= 1 {
            self.captured.insert(b"**".to_vec());
        }
        visit_assoc_splat_node(self, node);
    }
    fn visit_block_argument_node(&mut self, node: &BlockArgumentNode<'pr>) {
        if node.expression().is_none() && self.depth >= 1 {
            self.captured.insert(b"&".to_vec());
        }
        visit_block_argument_node(self, node);
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

/// Locals of one scope (not counting nested blocks) assigned from arithmetic, except those
/// that can only hold Integers (`int_locals`): those don't need a Float shadow.
pub fn numeric_locals<'pr>(nodes: &[&Node<'pr>]) -> HashSet<Vec<u8>> {
    let mut v = Num { depth: 0, names: HashSet::new() };
    for n in nodes {
        v.visit(n);
    }
    let ints = int_locals(nodes);
    v.names.retain(|n| !ints.contains(n));
    v.names
}

/// Locals of one scope that only ever get Integer values: every assignment (here or in a
/// nested block) is an Integer literal or `+ - * / %` of such locals and literals, and they
/// aren't parameters or assigned any other way (multiple assignment, `||=`, `for`...).
/// Only a hint for unboxing (an Integer method redefined to return a Float still works,
/// just boxed).
pub fn int_locals<'pr>(nodes: &[&Node<'pr>]) -> HashSet<Vec<u8>> {
    let mut v = IntW { depth: 0, writes: HashMap::new() };
    for n in nodes {
        v.visit(n);
    }
    // assume every candidate is an Integer, then drop those depending on a non-Integer
    let mut ints: HashSet<Vec<u8>> = v.writes.iter().filter(|(_, ws)| ws.iter().all(|w| w.is_some())).map(|(n, _)| n.clone()).collect();
    loop {
        let drop: Vec<Vec<u8>> = ints
            .iter()
            .filter(|n| v.writes[*n].iter().any(|w| w.as_ref().is_some_and(|deps| deps.iter().any(|d| !ints.contains(d)))))
            .cloned()
            .collect();
        if drop.is_empty() {
            return ints;
        }
        for n in drop {
            ints.remove(&n);
        }
    }
}

/// Is `n` an Integer if the locals it reads (pushed to `deps`) are?
fn int_expr(n: &Node, depth: u32, deps: &mut Vec<Vec<u8>>) -> bool {
    if n.as_integer_node().is_some() {
        return true;
    }
    if let Some(r) = n.as_local_variable_read_node() {
        if r.depth() == depth {
            deps.push(r.name().as_slice().to_vec());
            return true;
        }
        return false;
    }
    if let Some(p) = n.as_parentheses_node() {
        return p.body().is_some_and(|b| match b.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                v.len() == 1 && int_expr(&v[0], depth, deps)
            }
            None => int_expr(&b, depth, deps),
        });
    }
    if let Some(c) = n.as_call_node() {
        if matches!(c.name().as_slice(), b"+" | b"-" | b"*" | b"/" | b"%") && c.block().is_none() && !c.is_safe_navigation() {
            if let (Some(r), Some(a)) = (c.receiver(), c.arguments()) {
                let args: Vec<Node> = a.arguments().iter().collect();
                return args.len() == 1 && int_expr(&r, depth, deps) && int_expr(&args[0], depth, deps);
            }
        }
    }
    false
}

/// Every assignment to the scope's locals: Some(locals the value is an Integer if they
/// are), None for anything else.
struct IntW {
    depth: u32,
    writes: HashMap<Vec<u8>, Vec<Option<Vec<Vec<u8>>>>>,
}
impl IntW {
    fn other(&mut self, name: &[u8]) {
        self.writes.entry(name.to_vec()).or_default().push(None);
    }
}
impl<'pr> Visit<'pr> for IntW {
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
        if node.depth() == self.depth {
            let mut deps = Vec::new();
            let w = int_expr(&node.value(), self.depth, &mut deps).then_some(deps);
            self.writes.entry(node.name().as_slice().to_vec()).or_default().push(w);
        }
        visit_local_variable_write_node(self, node);
    }
    fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
        if node.depth() == self.depth {
            let name = node.name().as_slice().to_vec();
            let mut deps = vec![name.clone()];
            let ok = matches!(node.binary_operator().as_slice(), b"+" | b"-" | b"*" | b"/" | b"%") && int_expr(&node.value(), self.depth, &mut deps);
            self.writes.entry(name).or_default().push(ok.then_some(deps));
        }
        visit_local_variable_operator_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        if node.depth() == self.depth {
            self.other(node.name().as_slice());
        }
        visit_local_variable_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        if node.depth() == self.depth {
            self.other(node.name().as_slice());
        }
        visit_local_variable_and_write_node(self, node);
    }
    fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
        if node.depth() == self.depth {
            self.other(node.name().as_slice());
        }
    }
    // the scope's own parameters (a nested block's are its own locals)
    fn visit_required_parameter_node(&mut self, node: &RequiredParameterNode<'pr>) {
        if self.depth == 0 {
            self.other(node.name().as_slice());
        }
    }
    fn visit_optional_parameter_node(&mut self, node: &OptionalParameterNode<'pr>) {
        if self.depth == 0 {
            self.other(node.name().as_slice());
        }
        visit_optional_parameter_node(self, node);
    }
    fn visit_required_keyword_parameter_node(&mut self, node: &RequiredKeywordParameterNode<'pr>) {
        if self.depth == 0 {
            self.other(node.name().as_slice());
        }
    }
    fn visit_optional_keyword_parameter_node(&mut self, node: &OptionalKeywordParameterNode<'pr>) {
        if self.depth == 0 {
            self.other(node.name().as_slice());
        }
        visit_optional_keyword_parameter_node(self, node);
    }
    fn visit_rest_parameter_node(&mut self, node: &RestParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.other(n.as_slice());
        }
    }
    fn visit_keyword_rest_parameter_node(&mut self, node: &KeywordRestParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.other(n.as_slice());
        }
    }
    fn visit_block_parameter_node(&mut self, node: &BlockParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.other(n.as_slice());
        }
    }
}
