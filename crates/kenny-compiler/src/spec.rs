//! Speculative loops: a `while`/`until` loop whose locals are most likely Floats (or
//! Fixnums), though the compiler can't prove it (they are assigned from attribute readers,
//! method calls, `to_f`, `Math`...), is compiled twice:
//!
//! - a fast copy, where those locals live unboxed in f64 registers (Fixnum ones need no tag
//!   checks), entered when their values at the loop's start have the expected types;
//! - the normal (generic) copy.
//!
//! In the fast copy, an assignment to a speculated local whose value doesn't have the
//! expected type boxes the speculated locals back into their normal variables and continues
//! in the generic copy right after the same assignment (each such assignment is a statement,
//! so the generic copy has the same place, with nothing half-evaluated). Leaving the fast
//! copy any other way (the loop ends, `break`, an exception) boxes them too.

use crate::analysis::Ty;
use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::F64, types::I64, types::I8, InstBuilder};
use ruby_prism::*;
use std::collections::{HashMap, HashSet};

/// One speculated local.
#[derive(Clone, Debug)]
pub struct SpecLocal {
    pub name: Vec<u8>,
    pub ty: Ty,
    /// its value is used before the loop assigns it (or after the loop): checked on entry
    /// and boxed again on exit
    pub live: bool,
    /// read other than as an operator's operand: keeps a box (a heap Float's identity)
    pub boxed: bool,
    /// the loop assigns it
    pub assigned: bool,
}

/// While compiling the fast copy of a loop.
pub struct SpecCtx {
    pub locals: Vec<SpecLocal>,
    /// Float locals: (f64 register, box or 0)
    pub fvars: HashMap<Vec<u8>, (Variable, Option<Variable>)>,
    /// the speculated locals' unboxed shadows of the generic copy (analysis::numeric_locals),
    /// reset to "boxed only" when the fast copy hands over
    pub shadows: HashMap<Vec<u8>, (Variable, Variable)>,
}

// ---------- analysis ----------

/// What an expression gives, as far as speculation goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum G {
    Int,
    Float,
    /// a Float while the builtin method is used (`to_f`, `Math.sqrt`, `**`, `-@`)
    FloatG,
    IntG,
    /// can't tell
    Unknown,
    /// never a number (a String, Array, nil...)
    Other,
}

struct Walk<'pr> {
    /// assignments in the loop: name -> (value class source, statement position)
    writes: HashMap<Vec<u8>, Vec<(WriteK<'pr>, bool)>>,
    /// operator nodes in the loop: (operator, receiver, argument)
    ops: Vec<(Vec<u8>, Node<'pr>, Node<'pr>)>,
    /// reads in the loop: name -> (offset, as an operator's operand)
    reads: HashMap<Vec<u8>, Vec<(usize, bool)>>,
    /// `x = v` statements in the loop: offset -> (range of the statements after it)
    stmt_writes: HashMap<usize, (usize, usize)>,
    /// a `def` / `class` / `module` / `BEGIN` inside: compiled once only
    nested_scope: bool,
    /// `x = y` in the loop: (x, y)
    copies: Vec<(Vec<u8>, Vec<u8>)>,
    /// locals in the arithmetic of `Math.f(...)` arguments
    math_args: Vec<Vec<u8>>,
}

enum WriteK<'pr> {
    Set(Node<'pr>),
    Op(Vec<u8>, Node<'pr>),
}

fn is_op(name: &[u8]) -> bool {
    matches!(name, b"+" | b"-" | b"*" | b"/" | b"%" | b"<" | b"<=" | b">" | b">=" | b"==" | b"!=")
}

/// A read of a depth-0 local at the top of `n` (through parentheses).
fn bare_read<'pr>(n: &Node<'pr>) -> Option<(Vec<u8>, usize)> {
    if let Some(r) = n.as_local_variable_read_node() {
        return (r.depth() == 0).then(|| (r.name().as_slice().to_vec(), r.location().start_offset()));
    }
    let p = n.as_parentheses_node()?;
    let b = p.body()?;
    match b.as_statements_node() {
        Some(s) => {
            let v: Vec<Node> = s.body().iter().collect();
            if v.len() == 1 { bare_read(&v[0]) } else { None }
        }
        None => bare_read(&b),
    }
}

impl<'pr> Walk<'pr> {
    /// `stmt`: `n` is in statement position (its value isn't an operand of anything that
    /// was evaluated before it, so the generic copy can be entered right after it).
    fn walk(&mut self, n: &Node<'pr>, stmt: bool) {
        match n {
            Node::StatementsNode { .. } => {
                let s = n.as_statements_node().unwrap();
                let body: Vec<Node> = s.body().iter().collect();
                let end = s.location().end_offset();
                for (i, c) in body.iter().enumerate() {
                    if stmt && c.as_local_variable_write_node().is_some() {
                        let start = body.get(i + 1).map(|x| x.location().start_offset()).unwrap_or(end);
                        self.stmt_writes.insert(c.location().start_offset(), (start, end));
                    }
                    self.walk(c, stmt);
                }
            }
            Node::IfNode { .. } => {
                let x = n.as_if_node().unwrap();
                self.walk(&x.predicate(), false);
                if let Some(s) = x.statements() {
                    self.walk(&s.as_node(), stmt);
                }
                if let Some(s) = x.subsequent() {
                    self.walk(&s, stmt);
                }
            }
            Node::UnlessNode { .. } => {
                let x = n.as_unless_node().unwrap();
                self.walk(&x.predicate(), false);
                if let Some(s) = x.statements() {
                    self.walk(&s.as_node(), stmt);
                }
                if let Some(e) = x.else_clause() {
                    self.walk(&e.as_node(), stmt);
                }
            }
            Node::ElseNode { .. } => {
                if let Some(s) = n.as_else_node().unwrap().statements() {
                    self.walk(&s.as_node(), stmt);
                }
            }
            Node::WhileNode { .. } => {
                let x = n.as_while_node().unwrap();
                self.walk(&x.predicate(), false);
                if let Some(s) = x.statements() {
                    self.walk(&s.as_node(), stmt && !x.is_begin_modifier());
                }
            }
            Node::UntilNode { .. } => {
                let x = n.as_until_node().unwrap();
                self.walk(&x.predicate(), false);
                if let Some(s) = x.statements() {
                    self.walk(&s.as_node(), stmt && !x.is_begin_modifier());
                }
            }
            Node::LocalVariableWriteNode { .. } => {
                let w = n.as_local_variable_write_node().unwrap();
                if w.depth() == 0 {
                    self.writes.entry(w.name().as_slice().to_vec()).or_default().push((WriteK::Set(w.value()), stmt));
                    if let Some(r) = w.value().as_local_variable_read_node() {
                        if r.depth() == 0 {
                            let y = r.name().as_slice().to_vec();
                            self.reads.entry(y.clone()).or_default().push((r.location().start_offset(), true));
                            self.copies.push((w.name().as_slice().to_vec(), y));
                            return;
                        }
                    }
                }
                self.walk(&w.value(), false);
            }
            Node::LocalVariableOperatorWriteNode { .. } => {
                let w = n.as_local_variable_operator_write_node().unwrap();
                let v = w.value();
                if w.depth() == 0 {
                    let name = w.name().as_slice().to_vec();
                    self.writes.entry(name.clone()).or_default().push((WriteK::Op(w.binary_operator().as_slice().to_vec(), w.value()), stmt));
                    // (reads the local as an operand)
                    self.reads.entry(name).or_default().push((w.location().start_offset(), true));
                    if let Some((r, at)) = bare_read(&v) {
                        self.reads.entry(r).or_default().push((at, true));
                        return;
                    }
                }
                self.walk(&v, false);
            }
            Node::LocalVariableReadNode { .. } => {
                let r = n.as_local_variable_read_node().unwrap();
                if r.depth() == 0 {
                    self.reads.entry(r.name().as_slice().to_vec()).or_default().push((r.location().start_offset(), false));
                }
            }
            Node::CallNode { .. } => {
                let c = n.as_call_node().unwrap();
                if let Some(crate::fastpath::NumCall::Math(_, arg)) = crate::fastpath::num_call_of(&c) {
                    arith_leaves(&arg, &mut self.math_args);
                }
                if is_op(c.name().as_slice()) && c.block().is_none() && !c.is_safe_navigation() {
                    if let (Some(r), Some(a)) = (c.receiver(), c.arguments()) {
                        let args: Vec<Node> = a.arguments().iter().collect();
                        if args.len() == 1 {
                            self.ops.push((c.name().as_slice().to_vec(), c.receiver().unwrap(), a.arguments().iter().next().unwrap()));
                            for x in [&r, &args[0]] {
                                match bare_read(x) {
                                    Some((name, at)) => self.reads.entry(name).or_default().push((at, true)),
                                    None => self.walk(x, false),
                                }
                            }
                            return;
                        }
                    }
                }
                self.walk_children(n);
            }
            Node::DefNode { .. } | Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. } | Node::PreExecutionNode { .. } | Node::PostExecutionNode { .. } => {
                self.nested_scope = true;
            }
            _ => self.walk_children(n),
        }
    }
    fn walk_children(&mut self, n: &Node<'pr>) {
        // children of `n`: visit with a visitor that hands each direct child to `walk`
        struct Direct<'a, 'pr> {
            w: &'a mut Walk<'pr>,
            depth: u32,
        }
        impl<'a, 'pr> Visit<'pr> for Direct<'a, 'pr> {
            fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
                if self.depth == 1 {
                    self.w.walk(&node, false);
                }
                self.depth += 1;
            }
            fn visit_branch_node_leave(&mut self) {
                self.depth -= 1;
            }
            fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
                if self.depth == 1 {
                    self.w.walk(&node, false);
                }
            }
        }
        let mut d = Direct { w: self, depth: 0 };
        d.visit(n);
    }
}

/// The class of `n` given the known/guessed types of locals.
fn class(n: &Node, env: &HashMap<Vec<u8>, Ty>) -> G {
    if n.as_integer_node().is_some() {
        return G::Int;
    }
    if n.as_float_node().is_some() {
        return G::Float;
    }
    if n.as_string_node().is_some()
        || n.as_interpolated_string_node().is_some()
        || n.as_array_node().is_some()
        || n.as_hash_node().is_some()
        || n.as_nil_node().is_some()
        || n.as_true_node().is_some()
        || n.as_false_node().is_some()
        || n.as_symbol_node().is_some()
    {
        return G::Other;
    }
    if let Some(r) = n.as_local_variable_read_node() {
        return match (r.depth(), env.get(r.name().as_slice())) {
            (0, Some(Ty::Int)) => G::Int,
            (0, Some(Ty::Float)) => G::Float,
            _ => G::Unknown,
        };
    }
    if let Some(p) = n.as_parentheses_node() {
        let Some(b) = p.body() else { return G::Other };
        return match b.as_statements_node() {
            Some(s) => {
                let v: Vec<Node> = s.body().iter().collect();
                if v.len() == 1 { class(&v[0], env) } else { G::Unknown }
            }
            None => class(&b, env),
        };
    }
    let Some(c) = n.as_call_node() else { return G::Unknown };
    if c.block().is_some() || c.is_safe_navigation() {
        return G::Unknown;
    }
    if let Some(nc) = crate::fastpath::num_call_of(&c) {
        return match nc {
            crate::fastpath::NumCall::Math(..) | crate::fastpath::NumCall::ToF(_) => G::FloatG,
            crate::fastpath::NumCall::Pow(r, _) => match class(&r, env) {
                G::Float | G::FloatG => G::FloatG,
                _ => G::Unknown,
            },
            crate::fastpath::NumCall::Neg(r) => match class(&r, env) {
                G::Float | G::FloatG => G::FloatG,
                G::Int | G::IntG => G::IntG,
                _ => G::Unknown,
            },
            crate::fastpath::NumCall::Shift(r, _, _) => match class(&r, env) {
                G::Int | G::IntG => G::IntG,
                _ => G::Unknown,
            },
        };
    }
    let name = c.name().as_slice();
    if matches!(name, b"+" | b"-" | b"*" | b"/" | b"%" | b"&" | b"|" | b"^") {
        if let (Some(r), Some(a)) = (c.receiver(), c.arguments()) {
            let args: Vec<Node> = a.arguments().iter().collect();
            if args.len() == 1 {
                let (x, y) = (class(&r, env), class(&args[0], env));
                let sure = |g: G| match g {
                    G::Int | G::IntG => Some(Ty::Int),
                    G::Float | G::FloatG => Some(Ty::Float),
                    _ => None,
                };
                let guarded = matches!(x, G::FloatG | G::IntG) || matches!(y, G::FloatG | G::IntG);
                if let (Some(a), Some(b)) = (sure(x), sure(y)) {
                    return match (crate::analysis::ty_op(name, a, b), guarded) {
                        (Some(Ty::Int), false) => G::Int,
                        (Some(Ty::Float), false) => G::Float,
                        (Some(Ty::Int), true) => G::IntG,
                        (Some(Ty::Float), true) => G::FloatG,
                        (None, _) => G::Unknown,
                    };
                }
            }
        }
    }
    if matches!(name, b"to_i" | b"floor" | b"ceil" | b"round" | b"truncate" | b"length" | b"size" | b"ord") && c.arguments().is_none() {
        return G::IntG;
    }
    G::Unknown
}

/// The locals operated on at the top of `n`'s arithmetic (`a * b + c` -> a, b, c).
fn arith_leaves(n: &Node, out: &mut Vec<Vec<u8>>) {
    if let Some((name, _)) = bare_read(n) {
        out.push(name);
        return;
    }
    if let Some(p) = n.as_parentheses_node() {
        if let Some(b) = p.body() {
            if let Some(s) = b.as_statements_node() {
                let v: Vec<Node> = s.body().iter().collect();
                if v.len() == 1 {
                    arith_leaves(&v[0], out);
                }
            }
        }
        return;
    }
    if let Some(c) = n.as_call_node() {
        if matches!(c.name().as_slice(), b"+" | b"-" | b"*" | b"/") && c.block().is_none() {
            if let (Some(r), Some(a)) = (c.receiver(), c.arguments()) {
                let args: Vec<Node> = a.arguments().iter().collect();
                if args.len() == 1 {
                    arith_leaves(&r, out);
                    arith_leaves(&args[0], out);
                }
            }
        }
    }
}

/// The locals to speculate on in loop `lp` (a WhileNode / UntilNode), if any.
/// `typed`: the scope's analysis; `register`: is a local in a register (not captured).
pub fn plan(lp: &Node, typed: &crate::analysis::Typed, register: &dyn Fn(&[u8]) -> bool) -> Option<Vec<SpecLocal>> {
    let (ls, le) = (lp.location().start_offset(), lp.location().end_offset());
    let mut w = Walk { writes: HashMap::new(), ops: Vec::new(), reads: HashMap::new(), stmt_writes: HashMap::new(), nested_scope: false, copies: Vec::new(), math_args: Vec::new() };
    let (pred, body) = match lp.as_while_node() {
        Some(x) => (x.predicate(), x.statements()),
        None => {
            let x = lp.as_until_node()?;
            (x.predicate(), x.statements())
        }
    };
    w.walk(&pred, false);
    if let Some(b) = &body {
        w.walk(&b.as_node(), true);
    }
    if w.nested_scope {
        return None;
    }
    // candidates: register locals the loop uses, of no fixed type, assigned (in the loop)
    // only by statements
    let mut cands: Vec<Vec<u8>> = Vec::new();
    for name in w.reads.keys().chain(w.writes.keys()) {
        if cands.contains(name) || typed.types.contains_key(name) || typed.bad.contains(name) || !register(name) {
            continue;
        }
        if w.writes.get(name).is_some_and(|ws| ws.iter().any(|(_, stmt)| !stmt)) {
            continue;
        }
        cands.push(name.clone());
    }
    // (no candidates: always-Integer locals may still be, below)
    // guess each candidate's type from its assignments and what it is used with
    let mut env: HashMap<Vec<u8>, Ty> = typed.types.clone();
    let mut guess: HashMap<Vec<u8>, Ty> = HashMap::new();
    let mut dropped: HashSet<Vec<u8>> = HashSet::new();
    for _round in 0..8 {
        // evidence by strength: 2 = assigned such a value, 1 = compared with one,
        // 0 = in arithmetic with one (`2 * x` says little about x)
        let mut votes: HashMap<Vec<u8>, [(bool, bool); 3]> = HashMap::new();
        let vote = |n: &[u8], t: Ty, level: usize, votes: &mut HashMap<Vec<u8>, [(bool, bool); 3]>| {
            let e = &mut votes.entry(n.to_vec()).or_default()[level];
            if t == Ty::Float { e.0 = true } else { e.1 = true }
        };
        for name in &cands {
            for (k, _) in w.writes.get(name).into_iter().flatten() {
                let g = match k {
                    WriteK::Set(v) => class(v, &env),
                    WriteK::Op(op, v) => match (env.get(name), class(v, &env)) {
                        (Some(&t), g @ (G::Int | G::IntG | G::Float | G::FloatG)) => {
                            let b = if matches!(g, G::Int | G::IntG) { Ty::Int } else { Ty::Float };
                            match crate::analysis::ty_op(op, t, b) {
                                Some(Ty::Int) => G::Int,
                                Some(Ty::Float) => G::Float,
                                None => G::Unknown,
                            }
                        }
                        (None, G::Float | G::FloatG) if op != b"&" && op != b"|" && op != b"^" => G::Float,
                        _ => G::Unknown,
                    },
                };
                match g {
                    G::Float | G::FloatG => vote(name, Ty::Float, 2, &mut votes),
                    G::Int | G::IntG => vote(name, Ty::Int, 2, &mut votes),
                    _ => {}
                }
            }
        }
        // what it is assigned elsewhere in the method (`s = 0.0` before the loop)
        for name in &cands {
            if let Some(&t) = typed.seed.get(name) {
                vote(name, t, 2, &mut votes);
            }
        }
        // `Math.sqrt(x)`: a Float argument, most likely
        for name in &w.math_args {
            if cands.contains(name) {
                vote(name, Ty::Float, 0, &mut votes);
            }
        }
        // operands of an operator whose other side is a number
        for (op, a, b) in &w.ops {
            let level = if matches!(op.as_slice(), b"<" | b"<=" | b">" | b">=" | b"==" | b"!=") { 1 } else { 0 };
            for (x, y) in [(a, b), (b, a)] {
                if let Some((name, _)) = bare_read(x) {
                    if cands.contains(&name) {
                        match class(y, &env) {
                            G::Float | G::FloatG => vote(&name, Ty::Float, level, &mut votes),
                            G::Int | G::IntG => vote(&name, Ty::Int, level, &mut votes),
                            _ => {}
                        }
                    }
                }
            }
        }
        // the locals in a Float local's arithmetic are Floats too (`d = dx * dx + dy * dy`)
        for (name, ws) in &w.writes {
            if env.get(name) != Some(&Ty::Float) {
                continue;
            }
            for (k, _) in ws {
                let v = match k {
                    WriteK::Set(v) | WriteK::Op(_, v) => v,
                };
                let mut leaves = Vec::new();
                arith_leaves(v, &mut leaves);
                for l in leaves {
                    if cands.contains(&l) {
                        vote(&l, Ty::Float, 0, &mut votes);
                    }
                }
            }
        }
        let mut changed = false;
        for (name, levels) in votes {
            if dropped.contains(&name) {
                continue;
            }
            let (f, i) = levels.iter().rev().find(|(f, i)| *f || *i).copied().unwrap_or_default();
            let t = match (f, i) {
                (true, false) => Ty::Float,
                (false, true) => Ty::Int,
                _ => {
                    // both: no single type
                    guess.remove(&name);
                    env.remove(&name);
                    dropped.insert(name);
                    changed = true;
                    continue;
                }
            };
            if guess.get(&name) != Some(&t) {
                guess.insert(name.clone(), t);
                env.insert(name, t);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // a guess contradicted by an assignment (of another type, or never a number) is dropped
    loop {
        let bad: Vec<Vec<u8>> = guess
            .iter()
            .filter(|(name, t)| {
                w.writes.get(*name).into_iter().flatten().any(|(k, _)| {
                    let g = match k {
                        WriteK::Set(v) => class(v, &env),
                        WriteK::Op(_, v) => match class(v, &env) {
                            G::Other => G::Other,
                            _ => G::Unknown,
                        },
                    };
                    match (g, t) {
                        (G::Other, _) => true,
                        (G::Int | G::IntG, Ty::Float) => true,
                        (G::Float | G::FloatG, Ty::Int) => true,
                        _ => false,
                    }
                })
            })
            .map(|(n, _)| n.clone())
            .collect();
        if bad.is_empty() {
            break;
        }
        for n in bad {
            guess.remove(&n);
            env.remove(&n);
        }
    }
    // always-Integer locals (analysis.rs `typed_locals`): Fixnums, most likely (their
    // arithmetic then needs no Bignum path, their comparisons no tag checks)
    let mut ints: Vec<Vec<u8>> = Vec::new();
    for name in w.reads.keys().chain(w.writes.keys()) {
        if std::env::var_os("KENNY_NO_SPEC_INTS").is_none() && typed.types.get(name) == Some(&Ty::Int)
            && !typed.bad.contains(name)
            && register(name)
            && !w.writes.get(name).is_some_and(|ws| ws.iter().any(|(_, stmt)| !stmt))
            && !guess.contains_key(name)
            && !ints.contains(name)
        {
            ints.push(name.clone());
        }
    }
    for n in &ints {
        guess.insert(n.clone(), Ty::Int);
    }
    if guess.is_empty() {
        return None;
    }
    // keeps a box: read other than as an operand, or copied into a local that keeps one
    // (or isn't a Float register)
    let mut boxed: HashSet<Vec<u8>> = w.reads.iter().filter(|(_, rs)| rs.iter().any(|(_, operand)| !operand)).map(|(n, _)| n.clone()).collect();
    loop {
        let mut changed = false;
        for (x, y) in &w.copies {
            let float_reg = env.get(x) == Some(&Ty::Float);
            if (boxed.contains(x) || !float_reg) && !boxed.contains(y) {
                boxed.insert(y.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut out = Vec::new();
    let mut names: Vec<&Vec<u8>> = guess.keys().collect();
    names.sort();
    for name in names {
        let refs = typed.refs.get(name).cloned().unwrap_or_default();
        // used only inside the loop, assigned (by a statement) before every use there
        let private = !refs.is_empty() && refs.iter().all(|&r| r >= ls && r < le) && {
            let first = *refs.iter().min().unwrap();
            w.stmt_writes.get(&first).is_some_and(|&(s, e)| refs.iter().all(|&r| r == first || (r >= s && r < e)))
        };
        out.push(SpecLocal { name: name.clone(), ty: guess[name], live: !private, boxed: boxed.contains(name), assigned: w.writes.contains_key(name) });
    }
    Some(out)
}

// ---------- code generation ----------

impl<M: Module> Compiler<M> {
    /// Compile loop `lp` as a fast speculative copy plus the generic one, if it has Float
    /// locals to speculate on. `generic` compiles the loop normally (with `exit` as its
    /// exit block); returns false when the loop isn't speculated.
    pub(crate) fn spec_loop(&mut self, fx: &mut Fx, lp: &Node, compile: &mut dyn FnMut(&mut Self, &mut Fx, Block)) -> Option<Value> {
        if fx.spec_depth > 0 || !self.inline_ctx.is_empty() || !fx.inline_levels.is_empty() || self.refine_scope.is_some() || self.in_dependency() || std::env::var_os("KENNY_NO_SPEC").is_some() {
            return None;
        }
        let register = |n: &[u8]| matches!(fx.locals.get(n), Some(Local::Var(_))) && !fx.flocals.contains_key(n);
        let locals = plan(lp, &fx.typed_info, &register)?;
        // worth a second copy: a Float register, a Fixnum that would otherwise carry an
        // unboxed-Float shadow through the loop, or one the loop computes (no overflow joins)
        if !locals.iter().any(|l| l.ty == Ty::Float || fx.num_locals.contains_key(&l.name) || l.assigned) {
            return None;
        }
        if std::env::var_os("KENNY_DEBUG_SPEC").is_some() {
            let v: Vec<String> = locals.iter().map(|l| format!("{}:{:?}{}{}", String::from_utf8_lossy(&l.name), l.ty, if l.live { "+live" } else { "" }, if l.boxed { "+box" } else { "" })).collect();
            let line = self.line_of(lp.location().start_offset());
            eprintln!("spec {}:{}: {}", self.file.display(), line, v.join(" "));
        }
        fx.spec_depth += 1;
        let exit = fx.b.create_block();
        fx.b.append_block_param(exit, I64);
        let generic = fx.b.create_block();
        let fast = fx.b.create_block();

        // entry: the live speculated locals have the expected types
        for l in locals.iter().filter(|l| l.live) {
            let v = self.local_get(fx, &l.name, 0);
            let next = fx.b.create_block();
            match l.ty {
                Ty::Int => {
                    let low = fx.b.ins().band_imm_s(v, 1);
                    fx.b.ins().brif(low, next, &[], generic, &[]);
                }
                Ty::Float => {
                    let tag = fx.b.ins().band_imm_s(v, 3);
                    let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
                    let heap = fx.b.create_block();
                    fx.b.set_cold_block(heap);
                    fx.b.ins().brif(flo, next, &[], heap, &[]);
                    fx.b.switch_to_block(heap);
                    let is = self.call(fx, "kn_is_float", &[v]);
                    fx.b.ins().brif(is, next, &[], generic, &[]);
                }
            }
            fx.b.switch_to_block(next);
        }
        fx.b.ins().jump(fast, &[]);

        // the fast copy
        fx.b.switch_to_block(fast);
        let mut fvars = HashMap::new();
        let mut shadows = HashMap::new();
        for l in &locals {
            if let Some(s) = fx.num_locals.remove(&l.name) {
                shadows.insert(l.name.clone(), s);
            }
        }
        for l in &locals {
            match l.ty {
                Ty::Float => {
                    let raw = fx.b.declare_var(F64);
                    let cache = l.boxed.then(|| fx.b.declare_var(I64));
                    if l.live {
                        let v = self.local_get(fx, &l.name, 0);
                        let f = self.float_unbox(fx, v);
                        fx.b.def_var(raw, f);
                        if let Some(c) = cache {
                            fx.b.def_var(c, v);
                        }
                    } else {
                        let z = fx.b.ins().f64const(0.0);
                        fx.b.def_var(raw, z);
                        if let Some(c) = cache {
                            let z = fx.b.ins().iconst(I64, 0);
                            fx.b.def_var(c, z);
                        }
                    }
                    fvars.insert(l.name.clone(), (raw, cache));
                }
                Ty::Int => {
                    fx.int_spec.insert(l.name.clone());
                }
            }
        }
        // (the shadows of the Fixnum locals no longer have a valid f64)
        for (name, (_, kind)) in &shadows {
            if fvars.contains_key(name) {
                continue;
            }
            let k = fx.b.ins().iconst(I8, 0);
            fx.b.def_var(*kind, k);
        }
        for (n, v) in &fvars {
            fx.flocals.insert(n.clone(), *v);
        }
        fx.spec = Some(SpecCtx { locals: locals.clone(), fvars: fvars.clone(), shadows: shadows.clone() });
        let fast_exit = fx.b.create_block();
        fx.b.append_block_param(fast_exit, I64);
        let fast_unwind = fx.b.create_block();
        fx.b.set_cold_block(fast_unwind);
        let outer_unwind = *fx.unwind.last().unwrap();
        fx.unwind.push(fast_unwind);
        compile(self, fx, fast_exit);
        fx.unwind.pop();
        // leaving: box the live locals again
        fx.b.switch_to_block(fast_exit);
        let v = fx.b.block_params(fast_exit)[0];
        self.spec_handover(fx, None, |l| l.live);
        fx.b.ins().jump(exit, &[v.into()]);
        fx.b.switch_to_block(fast_unwind);
        self.spec_handover(fx, None, |l| l.live);
        fx.b.ins().jump(outer_unwind, &[]);
        let ctx = fx.spec.take().unwrap();
        for n in ctx.fvars.keys() {
            fx.flocals.remove(n);
        }
        for l in &ctx.locals {
            fx.int_spec.remove(&l.name);
        }
        for (n, s) in ctx.shadows {
            fx.num_locals.insert(n, s);
        }

        // the generic copy, with the places to continue at
        fx.b.switch_to_block(generic);
        let resume = std::mem::take(&mut fx.resume_req);
        fx.resume = resume.clone();
        compile(self, fx, exit);
        let placed = std::mem::take(&mut fx.resume);
        fx.spec_depth -= 1;
        // (a place the generic copy didn't reach can't be entered either; never expected)
        for (_, b) in placed {
            fx.b.switch_to_block(b);
            fx.b.ins().trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
        }
        fx.b.switch_to_block(exit);
        Some(fx.b.block_params(exit)[0])
    }

    /// Store the fast copy's speculated locals (those `which` picks) back into their normal
    /// variables: Floats boxed, the shadows marked "boxed only". `skip`: a local the caller
    /// stores itself.
    pub(crate) fn spec_handover(&mut self, fx: &mut Fx, skip: Option<&[u8]>, which: impl Fn(&SpecLocal) -> bool) {
        let ctx = fx.spec.as_ref().unwrap();
        let locals: Vec<SpecLocal> = ctx.locals.iter().filter(|l| which(l) && Some(l.name.as_slice()) != skip).cloned().collect();
        let fvars = ctx.fvars.clone();
        let shadows = ctx.shadows.clone();
        for l in &locals {
            if let Some(&(raw, cache)) = fvars.get(&l.name) {
                let v = self.flocal_box(fx, raw, cache);
                if let Some(Local::Var(var)) = fx.locals.get(&l.name) {
                    fx.b.def_var(*var, v);
                }
            }
            if let Some(&(_, kind)) = shadows.get(&l.name) {
                let k = fx.b.ins().iconst(I8, 0);
                fx.b.def_var(kind, k);
            }
        }
    }

    /// In the fast copy: `name = a op b` (or `name op= b`) for a Fixnum-speculated local and
    /// `op` one of + - *: the Fixnum operation stored as is; anything else (an operand that
    /// isn't a Fixnum, an overflow) computes the operation in full and hands over to the
    /// generic copy with that value. None (nothing compiled): not such an assignment.
    /// (`a`: None for `name op= b`)
    pub(crate) fn spec_int_arith(&mut self, fx: &mut Fx, name: &[u8], op: &[u8], a: Option<&Node>, b: &Node, at: usize) -> Option<crate::fastpath::Num> {
        if !matches!(op, b"+" | b"-" | b"*") || fx.spec.as_ref()?.fvars.contains_key(name) || !fx.spec.as_ref()?.locals.iter().any(|l| l.name == name && l.ty == Ty::Int) {
            return None;
        }
        let x = match a {
            Some(a) => self.expr(fx, a),
            None => self.local_get(fx, name, fx.inline_levels.len() as u32),
        };
        let y = self.expr(fx, b);
        self.cur_offset = at;
        let deopt = fx.b.create_block();
        fx.b.append_block_param(deopt, I64);
        fx.b.set_cold_block(deopt);
        let slow = fx.b.create_block();
        fx.b.set_cold_block(slow);
        let fix = |fx: &Fx, v: Value| fx.sure_fix.contains(&v) || crate::fastpath::const_of(fx, v).is_some_and(|c| c & 1 == 1);
        if !(fix(fx, x) && fix(fx, y)) {
            let both = fx.b.ins().band(x, y);
            let bit = fx.b.ins().band_imm_s(both, 1);
            let cont = fx.b.create_block();
            fx.b.ins().brif(bit, cont, &[], slow, &[]);
            fx.b.switch_to_block(cont);
        }
        let (r, of) = match op {
            b"+" => {
                let ym1 = fx.b.ins().iadd_imm_s(y, -1);
                fx.b.ins().sadd_overflow(x, ym1)
            }
            b"-" => {
                // (x - y) + 1 without a separate step when y is a constant
                match crate::fastpath::const_of(fx, y).filter(|&c| c != i64::MIN + 1) {
                    Some(c) => {
                        let k = fx.b.ins().iconst(I64, 1 - c);
                        fx.b.ins().sadd_overflow(x, k)
                    }
                    None => {
                        let (d, of) = fx.b.ins().ssub_overflow(x, y);
                        (fx.b.ins().iadd_imm_s(d, 1), of)
                    }
                }
            }
            _ => {
                let xs = fx.b.ins().sshr_imm_s(x, 1);
                let ym1 = fx.b.ins().iadd_imm_s(y, -1);
                let (p, of) = fx.b.ins().smul_overflow(xs, ym1);
                (fx.b.ins().iadd_imm_s(p, 1), of)
            }
        };
        let ok = fx.b.create_block();
        fx.b.ins().brif(of, slow, &[], ok, &[]);
        fx.b.switch_to_block(slow);
        let v = self.send(fx, x, op, &[y], None);
        fx.b.ins().jump(deopt, &[v.into()]);
        fx.b.switch_to_block(ok);
        if let Some(Local::Var(var)) = fx.locals.get(name) {
            fx.b.def_var(*var, r);
        }
        fx.sure_fix.insert(r);
        fx.sure_ints.insert(r);
        self.spec_deopt(fx, name, deopt, at);
        Some(crate::fastpath::Num::Boxed(r))
    }

    /// In the fast copy: assign `n` to speculated local `name` (the statement at `at`),
    /// or hand over to the generic copy when it doesn't have the expected type.
    pub(crate) fn spec_assign(&mut self, fx: &mut Fx, name: &[u8], n: crate::fastpath::Num, at: usize) -> crate::fastpath::Num {
        let ctx = fx.spec.as_ref().unwrap();
        let fv = ctx.fvars.get(name).copied();
        let deopt = fx.b.create_block();
        fx.b.append_block_param(deopt, I64);
        fx.b.set_cold_block(deopt);
        let r = match fv {
            Some((raw, cache)) => {
                let (f, boxed) = self.spec_f64(fx, n, deopt);
                fx.b.def_var(raw, f);
                if let Some(c) = cache {
                    fx.b.def_var(c, boxed);
                }
                crate::fastpath::Num::F64(f)
            }
            None => {
                let v = self.materialize(fx, n);
                let low = fx.b.ins().band_imm_s(v, 1);
                let ok = fx.b.create_block();
                fx.b.ins().brif(low, ok, &[], deopt, &[v.into()]);
                fx.b.switch_to_block(ok);
                if let Some(Local::Var(var)) = fx.locals.get(name) {
                    fx.b.def_var(*var, v);
                }
                fx.sure_fix.insert(v);
                fx.sure_ints.insert(v);
                crate::fastpath::Num::Boxed(v)
            }
        };
        self.spec_deopt(fx, name, deopt, at);
        r
    }

    /// Fill `deopt` (one parameter: `name`'s new value) with the hand-over: everything boxed,
    /// the new value in place, on to the generic copy after the assignment at `at`.
    fn spec_deopt(&mut self, fx: &mut Fx, name: &[u8], deopt: Block, at: usize) {
        let cont = fx.b.current_block().unwrap();
        fx.b.switch_to_block(deopt);
        let v = fx.b.block_params(deopt)[0];
        self.spec_handover(fx, Some(name), |_| true);
        if let Some(Local::Var(var)) = fx.locals.get(name) {
            fx.b.def_var(*var, v);
        }
        if let Some(&(_, kind)) = fx.spec.as_ref().unwrap().shadows.get(name) {
            let k = fx.b.ins().iconst(I8, 0);
            fx.b.def_var(kind, k);
        }
        let target = *fx.resume_req.entry(at).or_insert_with(|| {
            let b = fx.b.create_block();
            fx.b.append_block_param(b, I64);
            b
        });
        fx.b.ins().jump(target, &[v.into()]);
        fx.b.switch_to_block(cont);
    }

    /// (f64, box or 0) of `n` for a speculated Float local: to `deopt` with the value when
    /// it isn't a Float.
    fn spec_f64(&mut self, fx: &mut Fx, n: crate::fastpath::Num, deopt: Block) -> (Value, Value) {
        use crate::fastpath::Num;
        let boxed_v = match n {
            Num::F64(r) => return (r, fx.b.ins().iconst(I64, 0)),
            Num::Lazy { raw, boxed, lit: true, .. } => return (raw, boxed),
            Num::Lazy { boxed, raw, kind, .. } => {
                // kind 1: a raw f64 (from a Float operation); else the box is the value
                let only_raw = fx.b.ins().icmp_imm_s(IntCC::Equal, kind, 1);
                let rb = fx.b.create_block();
                let bb = fx.b.create_block();
                let done = fx.b.create_block();
                fx.b.append_block_param(done, F64);
                fx.b.append_block_param(done, I64);
                fx.b.ins().brif(only_raw, rb, &[], bb, &[]);
                fx.b.switch_to_block(rb);
                let z = fx.b.ins().iconst(I64, 0);
                fx.b.ins().jump(done, &[raw.into(), z.into()]);
                fx.b.switch_to_block(bb);
                let (f, b) = self.spec_f64_boxed(fx, boxed, deopt);
                fx.b.ins().jump(done, &[f.into(), b.into()]);
                fx.b.switch_to_block(done);
                let p = fx.b.block_params(done);
                return (p[0], p[1]);
            }
            Num::Boxed(v) => v,
        };
        self.spec_f64_boxed(fx, boxed_v, deopt)
    }
    fn spec_f64_boxed(&mut self, fx: &mut Fx, v: Value, deopt: Block) -> (Value, Value) {
        let tag = fx.b.ins().band_imm_s(v, 3);
        let flo = fx.b.ins().icmp_imm_s(IntCC::Equal, tag, 2);
        let fb = fx.b.create_block();
        let hb = fx.b.create_block();
        let hb2 = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, F64);
        fx.b.set_cold_block(hb);
        fx.b.set_cold_block(hb2);
        fx.b.ins().brif(flo, fb, &[], hb, &[]);
        fx.b.switch_to_block(fb);
        let d = self.flo_decode(fx, v);
        fx.b.ins().jump(done, &[d.into()]);
        fx.b.switch_to_block(hb);
        let is = self.call(fx, "kn_is_float", &[v]);
        fx.b.ins().brif(is, hb2, &[], deopt, &[v.into()]);
        fx.b.switch_to_block(hb2);
        let call = self.rt_ref(fx, "kn_float_unbox");
        let inst = fx.b.ins().call(call, &[v]);
        let r = fx.b.inst_results(inst)[0];
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        (fx.b.block_params(done)[0], v)
    }
}
