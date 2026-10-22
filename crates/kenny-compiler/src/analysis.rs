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
        Some(c) if crate::fastpath::num_call_of(&c).is_some() => true,
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
    fn visit_multi_write_node(&mut self, node: &MultiWriteNode<'pr>) {
        // `a, b = x * y, z / w` (see `expr_void`)
        if let (0, Some(a)) = (self.depth, node.value().as_array_node()) {
            for (t, e) in node.lefts().iter().zip(a.elements().iter()) {
                if let Some(l) = t.as_local_variable_target_node() {
                    if l.depth() == 0 && is_numeric_expr(&e) {
                        self.names.insert(l.name().as_slice().to_vec());
                    }
                }
            }
        }
        visit_multi_write_node(self, node);
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

/// A local's type when every value it can hold is known (LANGUAGE.md rule 2: Integer and
/// Float operators are the builtin ones).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Float,
}

/// One assignment to a local: `x = v` or `x op= v`.
enum TWrite<'pr> {
    Set(Node<'pr>),
    Op(Vec<u8>, Node<'pr>),
}

struct TypedW<'pr> {
    depth: u32,
    writes: HashMap<Vec<u8>, Vec<TWrite<'pr>>>,
    /// every reference (read or write) to a local of the scope: name -> start offsets
    refs: HashMap<Vec<u8>, Vec<usize>>,
    /// `x = v` statements directly in a statement list: start offset -> the source range
    /// of the statements after it in that list (start, end)
    stmt_writes: HashMap<usize, (usize, usize)>,
    bad: HashSet<Vec<u8>>,
    /// the scope's parameters
    params: HashSet<Vec<u8>>,
    using: bool,
    /// reads (start offsets) that are an operand of an Integer/Float operator, or the value
    /// of an assignment to another local (`x = y`): they need no boxed value
    num_reads: HashSet<usize>,
    /// `x = y`: (x, y)
    copies: Vec<(Vec<u8>, Vec<u8>)>,
    /// every read: (name, start offset)
    reads: Vec<(Vec<u8>, usize)>,
}
impl<'pr> TypedW<'pr> {
    /// A read of a local of this scope at the top of `n` (inside parentheses), if any.
    fn operand_read(&self, n: &Node<'pr>) -> Option<(Vec<u8>, usize)> {
        if let Some(r) = n.as_local_variable_read_node() {
            return (r.depth() == self.depth).then(|| (r.name().as_slice().to_vec(), r.location().start_offset()));
        }
        let p = n.as_parentheses_node()?;
        let b = p.body()?;
        match b.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                if v.len() == 1 { self.operand_read(&v[0]) } else { None }
            }
            None => self.operand_read(&b),
        }
    }
    fn reference(&mut self, name: &[u8], depth: u32, at: usize) {
        if depth != self.depth {
            return;
        }
        if self.depth > 0 {
            // used in a block: lives in the block's environment
            self.bad.insert(name.to_vec());
        }
        self.refs.entry(name.to_vec()).or_default().push(at);
    }
    fn other(&mut self, name: &[u8], depth: u32) {
        if depth == self.depth {
            self.bad.insert(name.to_vec());
        }
    }
}
impl<'pr> Visit<'pr> for TypedW<'pr> {
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
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if node.receiver().is_none() && node.name().as_slice() == b"using" {
            self.using = true;
        }
        let op = node.name().as_slice();
        if matches!(op, b"+" | b"-" | b"*" | b"/" | b"%" | b"<" | b"<=" | b">" | b">=" | b"==" | b"!=") && node.block().is_none() && !node.is_safe_navigation() {
            if let (Some(r), Some(a)) = (node.receiver(), node.arguments()) {
                let args: Vec<Node> = a.arguments().iter().collect();
                if args.len() == 1 {
                    for n in [&r, &args[0]] {
                        if let Some((_, at)) = self.operand_read(n) {
                            self.num_reads.insert(at);
                        }
                    }
                }
            }
        }
        visit_call_node(self, node);
    }
    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        if self.depth == 0 {
            let body: Vec<Node> = node.body().iter().collect();
            let end = node.location().end_offset();
            for (i, s) in body.iter().enumerate() {
                if s.as_local_variable_write_node().is_some() {
                    let start = body.get(i + 1).map(|n| n.location().start_offset()).unwrap_or(end);
                    self.stmt_writes.insert(s.location().start_offset(), (start, end));
                }
            }
        }
        visit_statements_node(self, node);
    }
    fn visit_local_variable_read_node(&mut self, node: &LocalVariableReadNode<'pr>) {
        self.reference(node.name().as_slice(), node.depth(), node.location().start_offset());
        if node.depth() == self.depth {
            self.reads.push((node.name().as_slice().to_vec(), node.location().start_offset()));
        }
    }
    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        let name = node.name().as_slice();
        self.reference(name, node.depth(), node.location().start_offset());
        if node.depth() == self.depth {
            self.writes.entry(name.to_vec()).or_default().push(TWrite::Set(node.value()));
            if let Some(r) = node.value().as_local_variable_read_node() {
                if r.depth() == self.depth {
                    self.num_reads.insert(r.location().start_offset());
                    self.copies.push((name.to_vec(), r.name().as_slice().to_vec()));
                }
            }
        }
        visit_local_variable_write_node(self, node);
    }
    fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
        let name = node.name().as_slice();
        self.reference(name, node.depth(), node.location().start_offset());
        if node.depth() == self.depth {
            self.writes.entry(name.to_vec()).or_default().push(TWrite::Op(node.binary_operator().as_slice().to_vec(), node.value()));
            if let Some((_, at)) = self.operand_read(&node.value()) {
                self.num_reads.insert(at);
            }
        }
        visit_local_variable_operator_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        self.other(node.name().as_slice(), node.depth());
        visit_local_variable_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        self.other(node.name().as_slice(), node.depth());
        visit_local_variable_and_write_node(self, node);
    }
    fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
        self.other(node.name().as_slice(), node.depth());
    }
    fn visit_required_parameter_node(&mut self, node: &RequiredParameterNode<'pr>) {
        if self.depth == 0 {
            self.params.insert(node.name().as_slice().to_vec());
        }
    }
    fn visit_optional_parameter_node(&mut self, node: &OptionalParameterNode<'pr>) {
        if self.depth == 0 {
            self.params.insert(node.name().as_slice().to_vec());
        }
        visit_optional_parameter_node(self, node);
    }
    fn visit_required_keyword_parameter_node(&mut self, node: &RequiredKeywordParameterNode<'pr>) {
        if self.depth == 0 {
            self.params.insert(node.name().as_slice().to_vec());
        }
    }
    fn visit_optional_keyword_parameter_node(&mut self, node: &OptionalKeywordParameterNode<'pr>) {
        if self.depth == 0 {
            self.params.insert(node.name().as_slice().to_vec());
        }
        visit_optional_keyword_parameter_node(self, node);
    }
    fn visit_rest_parameter_node(&mut self, node: &RestParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.params.insert(n.as_slice().to_vec());
        }
    }
    fn visit_keyword_rest_parameter_node(&mut self, node: &KeywordRestParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.params.insert(n.as_slice().to_vec());
        }
    }
    fn visit_block_parameter_node(&mut self, node: &BlockParameterNode<'pr>) {
        if let (0, Some(n)) = (self.depth, node.name()) {
            self.params.insert(n.as_slice().to_vec());
        }
    }
}

/// The type of `n` given the types of the scope's locals (None: unknown).
pub fn ty_expr(n: &Node, env: &HashMap<Vec<u8>, Ty>) -> Option<Ty> {
    if n.as_integer_node().is_some() {
        return Some(Ty::Int);
    }
    if n.as_float_node().is_some() {
        return Some(Ty::Float);
    }
    if let Some(r) = n.as_local_variable_read_node() {
        return if r.depth() == 0 { env.get(r.name().as_slice()).copied() } else { None };
    }
    if let Some(p) = n.as_parentheses_node() {
        let b = p.body()?;
        return match b.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                if v.len() == 1 { ty_expr(&v[0], env) } else { None }
            }
            None => ty_expr(&b, env),
        };
    }
    let c = n.as_call_node()?;
    if c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let args: Vec<Node> = c.arguments()?.arguments().iter().collect();
    if args.len() != 1 {
        return None;
    }
    let a = ty_expr(&c.receiver()?, env)?;
    let b = ty_expr(&args[0], env)?;
    ty_op(c.name().as_slice(), a, b)
}

/// The type of `a op b` for an Integer/Float operator (rule 2), None for others.
pub fn ty_op(op: &[u8], a: Ty, b: Ty) -> Option<Ty> {
    match (op, a, b) {
        (b"+" | b"-" | b"*" | b"/" | b"%" | b"&" | b"|" | b"^", Ty::Int, Ty::Int) => Some(Ty::Int),
        (b"+" | b"-" | b"*" | b"/" | b"%", _, _) => Some(Ty::Float),
        _ => None,
    }
}

/// Locals of one scope whose type is fixed: every assignment is a literal or an Integer /
/// Float operator on such locals and literals, every read comes after the first assignment
/// (statements following it in the same list, so the local is never nil), and the local
/// isn't a parameter, used in a block, or assigned any other way. Float locals then live
/// unboxed in an f64 register; Integer ones may still be Bignums. (No locals at all in a
/// file with `using`: refinements may change the operators.)
pub struct Typed {
    pub types: HashMap<Vec<u8>, Ty>,
    /// every reference to a local of the scope (start offsets), for speculation (spec.rs)
    pub refs: HashMap<Vec<u8>, Vec<usize>>,
    /// locals used in a block or assigned other than by `x = v` / `x op= v`
    pub bad: HashSet<Vec<u8>>,
    /// the type of the values a local is assigned (anywhere in the scope) that have a
    /// known type, when they agree: a hint for speculation (spec.rs)
    pub seed: HashMap<Vec<u8>, Ty>,
    /// Float locals whose value may be needed boxed (read other than as an operand of an
    /// operator, or copied to such a local): they keep their box, for a heap Float's
    /// identity. The others are only ever used unboxed.
    pub boxed: HashSet<Vec<u8>>,
}

pub fn typed_locals<'pr>(nodes: &[&Node<'pr>]) -> Typed {
    let mut v = TypedW {
        depth: 0,
        writes: HashMap::new(),
        refs: HashMap::new(),
        stmt_writes: HashMap::new(),
        bad: HashSet::new(),
        params: HashSet::new(),
        using: false,
        num_reads: HashSet::new(),
        copies: Vec::new(),
        reads: Vec::new(),
    };
    for n in nodes {
        v.visit(n);
    }
    if v.using {
        return Typed { types: HashMap::new(), boxed: HashSet::new(), refs: HashMap::new(), bad: HashSet::new(), seed: HashMap::new() };
    }
    let types = typed_env(&v);
    let mut boxed: HashSet<Vec<u8>> = v.reads.iter().filter(|(_, at)| !v.num_reads.contains(at)).map(|(n, _)| n.clone()).collect();
    // a copy into a Float local that is boxed, or into any other local, needs the box
    loop {
        let mut changed = false;
        for (x, y) in &v.copies {
            let into_boxed = boxed.contains(x) || types.get(x) != Some(&Ty::Float);
            if into_boxed && !boxed.contains(y) {
                boxed.insert(y.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    boxed.retain(|n| types.get(n) == Some(&Ty::Float));
    let mut seed = HashMap::new();
    for (name, ws) in &v.writes {
        let mut ts = ws.iter().filter_map(|w| match w {
            TWrite::Set(n) => ty_expr(n, &types),
            TWrite::Op(..) => None,
        });
        if let Some(t) = ts.next() {
            if ts.all(|x| x == t) {
                seed.insert(name.clone(), t);
            }
        }
    }
    Typed { types, boxed, refs: v.refs, bad: v.bad, seed }
}

fn typed_env(v: &TypedW) -> HashMap<Vec<u8>, Ty> {
    let mut env: HashMap<Vec<u8>, Ty> = HashMap::new();
    // definitely assigned before every read
    let mut cands: Vec<Vec<u8>> = Vec::new();
    for (name, refs) in &v.refs {
        if v.bad.contains(name) || v.params.contains(name) || !v.writes.contains_key(name) {
            continue;
        }
        let first = *refs.iter().min().unwrap();
        let Some(&(s, e)) = v.stmt_writes.get(&first) else { continue };
        if refs.iter().all(|&r| r == first || (r >= s && r < e)) {
            cands.push(name.clone());
        }
    }
    // types from the writes that don't depend on other candidates, then propagate
    loop {
        let mut changed = false;
        for name in &cands {
            if env.contains_key(name) {
                continue;
            }
            for w in &v.writes[name] {
                if let TWrite::Set(n) = w {
                    if let Some(t) = ty_expr(n, &env) {
                        env.insert(name.clone(), t);
                        changed = true;
                        break;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // then drop those with a write of another (or unknown) type, until stable
    loop {
        let drop: Vec<Vec<u8>> = env
            .iter()
            .filter(|(name, t)| {
                v.writes[*name].iter().any(|w| {
                    let wt = match w {
                        TWrite::Set(n) => ty_expr(n, &env),
                        TWrite::Op(op, n) => ty_expr(n, &env).and_then(|b| ty_op(op, **t, b)),
                    };
                    wt != Some(**t)
                })
            })
            .map(|(n, _)| n.clone())
            .collect();
        if drop.is_empty() {
            return env;
        }
        for n in drop {
            env.remove(&n);
        }
    }
}
