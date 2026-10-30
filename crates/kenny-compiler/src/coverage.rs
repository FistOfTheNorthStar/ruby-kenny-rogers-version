//! Coverage (the `coverage` library): which lines, branches and methods of a file CRuby
//! measures, worked out from the AST the way CRuby's compiler (prism_compile.c) lays out
//! a file's instructions.
//!
//! A program is instrumented only when it uses the coverage library (`require "coverage"`,
//! see `compile_with`); other programs compile exactly as before. Then every compiled file
//! gets a table of counters (kenny-runtime coverage.rs): its statements, branch targets and
//! methods increment a counter each, a plain add into the table its data word points to.
//!
//! Lines. CRuby counts a line each time a "line event" fires: the event belongs to a
//! statement (a node prism flags as a new line) that starts on another line than the last
//! event compiled in the same instruction sequence (method, block, class body, rescue
//! clause ...), and fires at the next instruction emitted after it, whose line is the one
//! counted (a statement whose first instruction comes from a later line, like `[\n  1,\n
//! foo]`, counts that line). The lines with an event are the "relevant" ones (0 rather than
//! nil). `Walker` follows CRuby's compilation order and emits a pseudo-instruction where
//! CRuby's compiler would, to find each event's line; the counter is incremented where the
//! statement starts (`Place::Before`) or, when the statement itself emits nothing (a literal
//! whose value is unused), where it ends (`Place::After`).
//!
//! Branch coverage changes the instructions a little (each branch target gets a `nop`, which
//! keeps some jumps CRuby otherwise removes), so the lines are worked out for both modes.

use ruby_prism::*;
use std::collections::{BTreeSet, HashMap};

/// a node: its span and type (a statement and the StatementsNode around it can share a span)
pub(crate) type Key = (usize, usize, u16);
/// (first line, first column, last line, last column), columns in bytes as CRuby's
pub(crate) type Loc = (u32, u32, u32, u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Place {
    Before(Key),
    After(Key),
}

const NEWLINE: u16 = 1;
const STATIC_LITERAL: u16 = 2;

fn base_ptr(n: &Node) -> *const u16 {
    match n {
        Node::AliasGlobalVariableNode { pointer, .. } => *pointer as *const u16,
        Node::AliasMethodNode { pointer, .. } => *pointer as *const u16,
        Node::AlternationPatternNode { pointer, .. } => *pointer as *const u16,
        Node::AndNode { pointer, .. } => *pointer as *const u16,
        Node::ArgumentsNode { pointer, .. } => *pointer as *const u16,
        Node::ArrayNode { pointer, .. } => *pointer as *const u16,
        Node::ArrayPatternNode { pointer, .. } => *pointer as *const u16,
        Node::AssocNode { pointer, .. } => *pointer as *const u16,
        Node::AssocSplatNode { pointer, .. } => *pointer as *const u16,
        Node::BackReferenceReadNode { pointer, .. } => *pointer as *const u16,
        Node::BeginNode { pointer, .. } => *pointer as *const u16,
        Node::BlockArgumentNode { pointer, .. } => *pointer as *const u16,
        Node::BlockLocalVariableNode { pointer, .. } => *pointer as *const u16,
        Node::BlockNode { pointer, .. } => *pointer as *const u16,
        Node::BlockParameterNode { pointer, .. } => *pointer as *const u16,
        Node::BlockParametersNode { pointer, .. } => *pointer as *const u16,
        Node::BreakNode { pointer, .. } => *pointer as *const u16,
        Node::CallAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::CallNode { pointer, .. } => *pointer as *const u16,
        Node::CallOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::CallOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::CallTargetNode { pointer, .. } => *pointer as *const u16,
        Node::CapturePatternNode { pointer, .. } => *pointer as *const u16,
        Node::CaseMatchNode { pointer, .. } => *pointer as *const u16,
        Node::CaseNode { pointer, .. } => *pointer as *const u16,
        Node::ClassNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableReadNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableTargetNode { pointer, .. } => *pointer as *const u16,
        Node::ClassVariableWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathTargetNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantPathWriteNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantReadNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantTargetNode { pointer, .. } => *pointer as *const u16,
        Node::ConstantWriteNode { pointer, .. } => *pointer as *const u16,
        Node::DefNode { pointer, .. } => *pointer as *const u16,
        Node::DefinedNode { pointer, .. } => *pointer as *const u16,
        Node::ElseNode { pointer, .. } => *pointer as *const u16,
        Node::EmbeddedStatementsNode { pointer, .. } => *pointer as *const u16,
        Node::EmbeddedVariableNode { pointer, .. } => *pointer as *const u16,
        Node::EnsureNode { pointer, .. } => *pointer as *const u16,
        Node::FalseNode { pointer, .. } => *pointer as *const u16,
        Node::FindPatternNode { pointer, .. } => *pointer as *const u16,
        Node::FlipFlopNode { pointer, .. } => *pointer as *const u16,
        Node::FloatNode { pointer, .. } => *pointer as *const u16,
        Node::ForNode { pointer, .. } => *pointer as *const u16,
        Node::ForwardingArgumentsNode { pointer, .. } => *pointer as *const u16,
        Node::ForwardingParameterNode { pointer, .. } => *pointer as *const u16,
        Node::ForwardingSuperNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableReadNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableTargetNode { pointer, .. } => *pointer as *const u16,
        Node::GlobalVariableWriteNode { pointer, .. } => *pointer as *const u16,
        Node::HashNode { pointer, .. } => *pointer as *const u16,
        Node::HashPatternNode { pointer, .. } => *pointer as *const u16,
        Node::IfNode { pointer, .. } => *pointer as *const u16,
        Node::ImaginaryNode { pointer, .. } => *pointer as *const u16,
        Node::ImplicitNode { pointer, .. } => *pointer as *const u16,
        Node::ImplicitRestNode { pointer, .. } => *pointer as *const u16,
        Node::InNode { pointer, .. } => *pointer as *const u16,
        Node::IndexAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::IndexOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::IndexOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::IndexTargetNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableReadNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableTargetNode { pointer, .. } => *pointer as *const u16,
        Node::InstanceVariableWriteNode { pointer, .. } => *pointer as *const u16,
        Node::IntegerNode { pointer, .. } => *pointer as *const u16,
        Node::InterpolatedMatchLastLineNode { pointer, .. } => *pointer as *const u16,
        Node::InterpolatedRegularExpressionNode { pointer, .. } => *pointer as *const u16,
        Node::InterpolatedStringNode { pointer, .. } => *pointer as *const u16,
        Node::InterpolatedSymbolNode { pointer, .. } => *pointer as *const u16,
        Node::InterpolatedXStringNode { pointer, .. } => *pointer as *const u16,
        Node::ItLocalVariableReadNode { pointer, .. } => *pointer as *const u16,
        Node::ItParametersNode { pointer, .. } => *pointer as *const u16,
        Node::KeywordHashNode { pointer, .. } => *pointer as *const u16,
        Node::KeywordRestParameterNode { pointer, .. } => *pointer as *const u16,
        Node::LambdaNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableAndWriteNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableOperatorWriteNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableOrWriteNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableReadNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableTargetNode { pointer, .. } => *pointer as *const u16,
        Node::LocalVariableWriteNode { pointer, .. } => *pointer as *const u16,
        Node::MatchLastLineNode { pointer, .. } => *pointer as *const u16,
        Node::MatchPredicateNode { pointer, .. } => *pointer as *const u16,
        Node::MatchRequiredNode { pointer, .. } => *pointer as *const u16,
        Node::MatchWriteNode { pointer, .. } => *pointer as *const u16,
        Node::MissingNode { pointer, .. } => *pointer as *const u16,
        Node::ModuleNode { pointer, .. } => *pointer as *const u16,
        Node::MultiTargetNode { pointer, .. } => *pointer as *const u16,
        Node::MultiWriteNode { pointer, .. } => *pointer as *const u16,
        Node::NextNode { pointer, .. } => *pointer as *const u16,
        Node::NilNode { pointer, .. } => *pointer as *const u16,
        Node::NoKeywordsParameterNode { pointer, .. } => *pointer as *const u16,
        Node::NumberedParametersNode { pointer, .. } => *pointer as *const u16,
        Node::NumberedReferenceReadNode { pointer, .. } => *pointer as *const u16,
        Node::OptionalKeywordParameterNode { pointer, .. } => *pointer as *const u16,
        Node::OptionalParameterNode { pointer, .. } => *pointer as *const u16,
        Node::OrNode { pointer, .. } => *pointer as *const u16,
        Node::ParametersNode { pointer, .. } => *pointer as *const u16,
        Node::ParenthesesNode { pointer, .. } => *pointer as *const u16,
        Node::PinnedExpressionNode { pointer, .. } => *pointer as *const u16,
        Node::PinnedVariableNode { pointer, .. } => *pointer as *const u16,
        Node::PostExecutionNode { pointer, .. } => *pointer as *const u16,
        Node::PreExecutionNode { pointer, .. } => *pointer as *const u16,
        Node::ProgramNode { pointer, .. } => *pointer as *const u16,
        Node::RangeNode { pointer, .. } => *pointer as *const u16,
        Node::RationalNode { pointer, .. } => *pointer as *const u16,
        Node::RedoNode { pointer, .. } => *pointer as *const u16,
        Node::RegularExpressionNode { pointer, .. } => *pointer as *const u16,
        Node::RequiredKeywordParameterNode { pointer, .. } => *pointer as *const u16,
        Node::RequiredParameterNode { pointer, .. } => *pointer as *const u16,
        Node::RescueModifierNode { pointer, .. } => *pointer as *const u16,
        Node::RescueNode { pointer, .. } => *pointer as *const u16,
        Node::RestParameterNode { pointer, .. } => *pointer as *const u16,
        Node::RetryNode { pointer, .. } => *pointer as *const u16,
        Node::ReturnNode { pointer, .. } => *pointer as *const u16,
        Node::SelfNode { pointer, .. } => *pointer as *const u16,
        Node::ShareableConstantNode { pointer, .. } => *pointer as *const u16,
        Node::SingletonClassNode { pointer, .. } => *pointer as *const u16,
        Node::SourceEncodingNode { pointer, .. } => *pointer as *const u16,
        Node::SourceFileNode { pointer, .. } => *pointer as *const u16,
        Node::SourceLineNode { pointer, .. } => *pointer as *const u16,
        Node::SplatNode { pointer, .. } => *pointer as *const u16,
        Node::StatementsNode { pointer, .. } => *pointer as *const u16,
        Node::StringNode { pointer, .. } => *pointer as *const u16,
        Node::SuperNode { pointer, .. } => *pointer as *const u16,
        Node::SymbolNode { pointer, .. } => *pointer as *const u16,
        Node::TrueNode { pointer, .. } => *pointer as *const u16,
        Node::UndefNode { pointer, .. } => *pointer as *const u16,
        Node::UnlessNode { pointer, .. } => *pointer as *const u16,
        Node::UntilNode { pointer, .. } => *pointer as *const u16,
        Node::WhenNode { pointer, .. } => *pointer as *const u16,
        Node::WhileNode { pointer, .. } => *pointer as *const u16,
        Node::XStringNode { pointer, .. } => *pointer as *const u16,
        Node::YieldNode { pointer, .. } => *pointer as *const u16,
    }
}

/// The node's flags (`pm_node_t`: a u16 type, then the u16 flags).
fn flags(n: &Node) -> u16 {
    unsafe { *base_ptr(n).add(1) }
}

pub(crate) fn key(n: &Node) -> Key {
    let l = n.location();
    (l.start_offset(), l.end_offset(), unsafe { *base_ptr(n) })
}

/// Nodes that compile to nothing when their value is unused.
fn quiet(n: &Node) -> bool {
    matches!(
        n,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::SelfNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
            | Node::SourceEncodingNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::LocalVariableReadNode { .. }
            | Node::ItLocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::BackReferenceReadNode { .. }
            | Node::NumberedReferenceReadNode { .. }
    ) || (flags(n) & STATIC_LITERAL != 0
        && matches!(
            n,
            Node::ArrayNode { .. } | Node::HashNode { .. } | Node::RangeNode { .. } | Node::InterpolatedStringNode { .. } | Node::InterpolatedSymbolNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
        ))
}

/// A branch base (`if`, `case`, `while` ...) and its targets, in CRuby's order.
pub(crate) struct Base {
    pub key: Key,
    pub kind: &'static str,
    pub loc: Loc,
    /// (target key, kind, location)
    pub targets: Vec<(u8, &'static str, Loc)>,
}

struct Walker<'s> {
    starts: Vec<usize>,
    src_len: usize,
    branch_mode: bool,
    last_line: u32,
    pending: Option<Place>,
    sites: Vec<(Place, u32)>,
    record: bool,
    bases: Vec<Base>,
    base_index: HashMap<Key, usize>,
    /// (key: the def's start offset, or a `define_method` block's; location; name)
    methods: Vec<(usize, Loc, Vec<u8>)>,
    _src: std::marker::PhantomData<&'s ()>,
}

impl<'s> Walker<'s> {
    fn line(&self, off: usize) -> u32 {
        self.starts.partition_point(|&s| s <= off.min(self.src_len)) as u32
    }
    fn lc(&self, off: usize) -> (u32, u32) {
        let l = self.line(off);
        (l, (off - self.starts[l as usize - 1]) as u32)
    }
    fn loc_of(&self, start: usize, end: usize) -> Loc {
        let (a, b) = self.lc(start);
        let (c, d) = self.lc(end);
        (a, b, c, d)
    }
    fn loc(&self, n: &Node) -> Loc {
        let l = n.location();
        self.loc_of(l.start_offset(), l.end_offset())
    }
    fn own(&self, n: &Node) -> u32 {
        self.line(n.location().start_offset())
    }
    /// An instruction at `line`: it carries the pending line event.
    fn emit(&mut self, line: u32) {
        if let Some(p) = self.pending.take() {
            self.sites.push((p, line));
        }
    }
    /// A child instruction sequence (method, block, class body, rescue/ensure clause).
    fn iseq(&mut self, f: impl FnOnce(&mut Self)) {
        let saved = (self.last_line, self.pending.take());
        self.last_line = 0;
        f(self);
        self.last_line = saved.0;
        self.pending = saved.1;
    }
    /// Code compiled now but laid out elsewhere: the pending event isn't its.
    fn chunk(&mut self, f: impl FnOnce(&mut Self)) {
        let saved = self.pending.take();
        f(self);
        self.pending = saved;
    }
    fn base(&mut self, n: &Node, kind: &'static str, loc: Loc) -> Option<usize> {
        if loc.0 == 0 {
            return None;
        }
        let k = key(n);
        if let Some(&i) = self.base_index.get(&k) {
            return Some(i);
        }
        if !self.record {
            return None;
        }
        self.bases.push(Base { key: k, kind, loc, targets: Vec::new() });
        self.base_index.insert(k, self.bases.len() - 1);
        Some(self.bases.len() - 1)
    }
    fn target(&mut self, base: Option<usize>, tkey: u8, kind: &'static str, loc: Loc) {
        let Some(b) = base else { return };
        if !self.record || loc.0 == 0 {
            return;
        }
        let t = &mut self.bases[b].targets;
        if !t.iter().any(|x| x.0 == tkey) {
            t.push((tkey, kind, loc));
        }
    }

    /// Whether compiling `n` emits any instruction.
    fn emits(&self, n: &Node, popped: bool) -> bool {
        if let Some(s) = n.as_statements_node() {
            let v: Vec<Node> = s.body().iter().collect();
            let last = v.len().saturating_sub(1);
            return v.iter().enumerate().any(|(i, c)| self.emits(c, popped || i < last));
        }
        if let Some(e) = n.as_else_node() {
            return match e.statements() {
                Some(s) => self.emits(&s.as_node(), popped),
                None => !popped,
            };
        }
        if let Some(p) = n.as_parentheses_node() {
            return match p.body() {
                Some(b) => self.emits(&b, popped),
                None => !popped,
            };
        }
        if quiet(n) {
            return !popped;
        }
        true
    }

    fn stmts(&mut self, s: Option<StatementsNode>, popped: bool) {
        match s {
            Some(s) => self.walk(&s.as_node(), popped),
            None => {
                if !popped {
                    let l = self.last_line;
                    self.emit(l);
                }
            }
        }
    }

    fn params(&mut self, p: Option<ParametersNode>) {
        let Some(p) = p else { return };
        for o in p.optionals().iter() {
            if let Some(o) = o.as_optional_parameter_node() {
                self.walk(&o.value(), false);
            }
        }
        for k in p.keywords().iter() {
            if let Some(o) = k.as_optional_keyword_parameter_node() {
                self.walk(&o.value(), false);
            }
        }
    }

    fn block_params(&mut self, p: Option<Node>) {
        if let Some(bp) = p.as_ref().and_then(|p| p.as_block_parameters_node()) {
            self.params(bp.parameters());
        }
    }

    fn block(&mut self, b: &BlockNode) {
        self.iseq(|w| {
            w.block_params(b.parameters());
            match b.body() {
                Some(body) => w.walk(&body, false),
                None => {}
            }
        });
    }

    /// (then reachable, else reachable) of a condition: a literal one jumps one way.
    fn static_cond(n: &Node) -> (bool, bool) {
        match n {
            Node::FalseNode { .. } | Node::NilNode { .. } => (false, true),
            Node::FloatNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::IntegerNode { .. }
            | Node::LambdaNode { .. }
            | Node::RationalNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::TrueNode { .. }
            | Node::SourceLineNode { .. }
            | Node::SourceEncodingNode { .. } => (true, false),
            Node::RangeNode { .. } if flags(n) & STATIC_LITERAL != 0 => (true, false),
            Node::ParenthesesNode { .. } => {
                // a single `putobject`
                let p = n.as_parentheses_node().unwrap();
                let inner = p.body().and_then(|b| match b.as_statements_node() {
                    Some(s) => {
                        let v: Vec<Node> = s.body().iter().collect();
                        if v.len() == 1 { v.into_iter().next() } else { None }
                    }
                    None => Some(b),
                });
                match inner {
                    Some(Node::FalseNode { .. }) => (false, true),
                    Some(
                        Node::TrueNode { .. }
                        | Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::SymbolNode { .. }
                        | Node::RationalNode { .. }
                        | Node::ImaginaryNode { .. }
                        | Node::RegularExpressionNode { .. }
                        | Node::SourceLineNode { .. }
                        | Node::SourceEncodingNode { .. },
                    ) => (true, false),
                    _ => (true, true),
                }
            }
            _ => (true, true),
        }
    }

    /// A branch condition (`pm_compile_branch_condition`): whether it jumps to the then and
    /// the else part (a literal jumps one way only; the jump itself is removed again).
    fn cond(&mut self, n: &Node) -> (bool, bool) {
        match n {
            Node::AndNode { .. } => {
                let a = n.as_and_node().unwrap();
                let (_, le) = self.cond(&a.left());
                let (rt, re) = self.cond(&a.right());
                (rt, le || re)
            }
            Node::OrNode { .. } => {
                let o = n.as_or_node().unwrap();
                let (lt, _) = self.cond(&o.left());
                let (rt, re) = self.cond(&o.right());
                (lt || rt, re)
            }
            Node::DefinedNode { .. } => {
                let l = self.own(n);
                self.emit(l);
                (true, true)
            }
            _ => {
                let r = Self::static_cond(n);
                if r == (true, true) {
                    self.walk(n, false);
                }
                r
            }
        }
    }

    fn conditional(&mut self, n: &Node, unless: bool, statements: Option<StatementsNode>, subsequent: Option<Node>, predicate: Node, popped: bool) {
        let line = self.own(n);
        let (t_ref, e_ref) = self.cond(&predicate);
        let both = t_ref && e_ref;
        let cloc = self.loc(n);
        let base = if both { self.base(n, if unless { "unless" } else { "if" }, cloc) } else { None };
        if t_ref {
            let then_loc = match &statements {
                Some(s) => self.loc(&s.as_node()),
                None if !unless => {
                    let e = predicate.location().end_offset();
                    let (l, c) = self.lc(e);
                    (l, c, l, c)
                }
                None => cloc,
            };
            self.stmts(statements, popped);
            if e_ref {
                self.target(base, 0, if unless { "else" } else { "then" }, then_loc);
                let else_emits = match &subsequent {
                    Some(s) => self.emits(s, popped),
                    None => !popped,
                };
                if self.branch_mode || else_emits {
                    self.emit(line);
                }
            }
        }
        if e_ref {
            let else_loc = match &subsequent {
                None => cloc,
                Some(s) => match s.as_else_node() {
                    Some(e) => match e.statements() {
                        Some(st) => self.loc(&st.as_node()),
                        None => self.loc(s),
                    },
                    None => self.loc(s),
                },
            };
            match &subsequent {
                Some(s) => self.walk(s, popped),
                None => {
                    if !popped {
                        let l = self.last_line;
                        self.emit(l);
                    }
                }
            }
            if t_ref {
                self.target(base, 1, if unless { "then" } else { "else" }, else_loc);
            }
        }
    }

    fn loop_node(&mut self, n: &Node, until: bool, statements: Option<StatementsNode>, predicate: Node) {
        let line = self.own(n);
        self.emit(line);
        let lloc = self.loc(n);
        let base = self.base(n, if until { "until" } else { "while" }, lloc);
        let bloc = match &statements {
            Some(s) => self.loc(&s.as_node()),
            None => lloc,
        };
        self.target(base, 0, "body", bloc);
        if let Some(s) = statements {
            self.walk(&s.as_node(), true);
        }
        self.cond(&predicate);
        self.emit(line);
    }

    fn case_node(&mut self, n: &CaseNode, popped: bool) {
        let node = n.as_node();
        let line = self.own(&node);
        let cloc = self.loc(&node);
        let base = self.base(&node, "case", cloc);
        let clauses: Vec<WhenNode> = n.conditions().iter().filter_map(|c| c.as_when_node()).collect();
        let case_event = self.pending.take();
        let mut trailing = None;
        let mut id = 0u8;
        match n.predicate() {
            None => {
                for (i, w) in clauses.iter().enumerate() {
                    let wn = w.as_node();
                    let bloc = match w.statements() {
                        Some(s) => self.loc(&s.as_node()),
                        None => self.loc(&wn),
                    };
                    self.target(base, id, "when", bloc);
                    id += 1;
                    self.stmts(w.statements(), popped);
                    if i + 1 < clauses.len() {
                        self.emit(line);
                    } else {
                        trailing = self.pending.take();
                    }
                    if i == 0 {
                        self.pending = case_event;
                    }
                    for c in w.conditions().iter() {
                        if c.as_splat_node().is_some() {
                            let l = self.own(&c);
                            self.emit(l);
                            self.walk(&c, false);
                        } else {
                            self.cond(&c);
                        }
                    }
                }
                if clauses.is_empty() {
                    self.pending = case_event;
                }
                self.else_clause(n.else_clause(), base, id, cloc, line, popped);
            }
            Some(pred) => {
                for (i, w) in clauses.iter().enumerate() {
                    let wn = w.as_node();
                    let wline = self.own(&wn);
                    self.chunk(|s| {
                        for c in w.conditions().iter() {
                            let l = s.own(&c);
                            if c.as_splat_node().is_some() {
                                s.emit(l);
                            }
                            if c.as_string_node().is_some() {
                                s.emit(l);
                            } else {
                                s.walk(&c, false);
                            }
                            s.emit(l);
                        }
                    });
                    self.emit(wline);
                    let bloc = match w.statements() {
                        Some(s) => self.loc(&s.as_node()),
                        None => self.loc(&wn),
                    };
                    self.target(base, id, "when", bloc);
                    id += 1;
                    self.stmts(w.statements(), popped);
                    if i + 1 < clauses.len() {
                        self.emit(wline);
                    } else {
                        trailing = self.pending.take();
                    }
                }
                self.pending = case_event;
                self.walk(&pred, false);
                self.emit(line);
                match n.else_clause() {
                    Some(e) => {
                        let en = e.as_node();
                        let eloc = match e.statements() {
                            Some(s) => self.loc(&s.as_node()),
                            None => self.loc(&en),
                        };
                        let eline = eloc.0;
                        self.emit(eline);
                        self.target(base, id, "else", eloc);
                        self.walk(&en, popped);
                        self.emit(eline);
                    }
                    None => {
                        self.emit(line);
                        self.target(base, id, "else", cloc);
                        self.emit(line);
                    }
                }
            }
        }
        if trailing.is_some() {
            self.pending = trailing;
        }
    }

    fn else_clause(&mut self, e: Option<ElseNode>, base: Option<usize>, id: u8, cloc: Loc, line: u32, popped: bool) {
        let eloc = match &e {
            None => cloc,
            Some(e) => match e.statements() {
                Some(s) => self.loc(&s.as_node()),
                None => self.loc(&e.as_node()),
            },
        };
        self.target(base, id, "else", eloc);
        match e {
            Some(e) => self.walk(&e.as_node(), popped),
            None => {
                if !popped {
                    let l = self.last_line;
                    self.emit(l);
                }
            }
        }
        self.emit(line);
    }

    fn case_match(&mut self, n: &CaseMatchNode, popped: bool) {
        let node = n.as_node();
        let line = self.own(&node);
        let cloc = self.loc(&node);
        let base = self.base(&node, "case", cloc);
        self.emit(line);
        if let Some(p) = n.predicate() {
            self.walk(&p, false);
        }
        let clauses: Vec<InNode> = n.conditions().iter().filter_map(|c| c.as_in_node()).collect();
        let mut id = 0u8;
        let mut trailing = None;
        for (i, c) in clauses.iter().enumerate() {
            let cn = c.as_node();
            let iline = self.own(&cn);
            self.emit(iline);
            let bloc = match c.statements() {
                Some(s) => self.loc(&s.as_node()),
                None => self.loc(&cn),
            };
            self.target(base, id, "in", bloc);
            id += 1;
            self.stmts(c.statements(), popped);
            if i + 1 < clauses.len() || n.else_clause().is_some() {
                self.emit(iline);
            } else {
                trailing = self.pending.take();
            }
            let pat = c.pattern();
            let pl = self.own(&pat);
            self.chunk(|s| {
                s.emit(pl);
                s.pattern(&pat);
            });
        }
        match n.else_clause() {
            Some(e) => {
                let en = e.as_node();
                let eloc = match e.statements() {
                    Some(s) => self.loc(&s.as_node()),
                    None => self.loc(&en),
                };
                self.target(base, id, "else", eloc);
                self.walk(&en, popped);
                self.emit(line);
            }
            None => {
                self.target(base, id, "else", cloc);
            }
        }
        if trailing.is_some() {
            self.pending = trailing;
        }
    }

    fn begin_node(&mut self, b: &BeginNode, popped: bool) {
        let node = b.as_node();
        if let Some(en) = b.ensure_clause() {
            let est = en.statements();
            // the ensure clause first (laid out after the body), then the body, then the
            // ensure clause again as its own sequence
            self.chunk(|s| {
                if let Some(st) = &est {
                    s.walk(&st.as_node(), true);
                }
            });
            if b.rescue_clause().is_some() {
                self.rescue(b, popped);
            } else {
                match b.statements() {
                    Some(st) => self.walk(&st.as_node(), popped),
                    None => {
                        if !popped {
                            let l = self.last_line;
                            self.emit(l);
                        }
                    }
                }
            }
            self.iseq(|s| {
                if let Some(st) = &est {
                    s.walk(&st.as_node(), true);
                }
            });
        } else if b.rescue_clause().is_some() {
            self.rescue(b, popped);
        } else {
            match b.statements() {
                Some(st) => self.walk(&st.as_node(), popped),
                None => {
                    if !popped {
                        let l = self.last_line;
                        self.emit(l);
                    }
                }
            }
        }
        let _ = node;
    }

    fn rescue(&mut self, b: &BeginNode, popped: bool) {
        let r = b.rescue_clause().unwrap();
        self.iseq(|s| s.walk(&r.as_node(), false));
        match b.statements() {
            Some(st) => self.walk(&st.as_node(), false),
            None => {
                let l = self.own(&r.as_node());
                self.emit(l);
            }
        }
        if let Some(e) = b.else_clause() {
            self.walk(&e.as_node(), popped);
        }
    }

    fn rescue_clause(&mut self, r: &RescueNode) {
        let node = r.as_node();
        let line = self.own(&node);
        let ex: Vec<Node> = r.exceptions().iter().collect();
        if ex.is_empty() {
            self.emit(line);
        }
        for e in &ex {
            self.emit(line);
            self.walk(e, false);
        }
        if let Some(st) = r.statements() {
            self.walk(&st.as_node(), false);
        } else {
            self.emit(line);
        }
        self.emit(line);
        if let Some(sub) = r.subsequent() {
            self.walk(&sub.as_node(), false);
        }
    }

    fn call(&mut self, c: &CallNode, popped: bool) {
        let node = c.as_node();
        let msg_line = match c.message_loc() {
            Some(m) => self.line(m.start_offset()),
            None => self.own(&node),
        };
        if c.is_attribute_write() && !popped {
            self.emit(msg_line);
        }
        // `[literal].include?(x)` loses its `duparray` (peephole): x comes first
        let dup_include = c.name().as_slice() == b"include?"
            && c.block().is_none()
            && c.receiver().map_or(false, |r| r.as_array_node().is_some() && flags(&r) & STATIC_LITERAL != 0)
            && c.arguments().map_or(false, |a| {
                let v: Vec<Node> = a.arguments().iter().collect();
                fn simple(n: &Node) -> bool {
                    match n {
                        Node::LocalVariableReadNode { .. } | Node::InstanceVariableReadNode { .. } | Node::SelfNode { .. } => true,
                        Node::CallNode { .. } => {
                            let c = n.as_call_node().unwrap();
                            c.arguments().is_none() && c.block().is_none() && !c.is_safe_navigation() && c.receiver().map_or(true, |r| simple(&r))
                        }
                        _ => false,
                    }
                }
                v.len() == 1 && simple(&v[0])
            });
        match c.receiver() {
            None => self.emit(msg_line),
            Some(_) if dup_include => {}
            Some(r) => self.walk(&r, false),
        }
        let mut base = None;
        let mut cloc = (0, 0, 0, 0);
        if c.is_safe_navigation() {
            let cursors = [c.closing_loc().map(|l| l.end_offset()), c.arguments().map(|a| a.location().end_offset()), c.message_loc().map(|l| l.end_offset())];
            let mut e = cursors[0];
            e = if e.is_none() || cursors[1].is_none() { cursors[1] } else { e.max(cursors[1]) };
            e = if e.is_none() || cursors[2].is_none() { cursors[2] } else { e.max(cursors[2]) };
            let e = e.or(cursors[0]).unwrap_or(node.location().end_offset());
            let (l1, c1) = self.lc(node.location().start_offset());
            let (l2, c2) = self.lc(e);
            cloc = (l1, c1, l2, c2);
            base = self.base(&node, "&.", cloc);
            self.emit(msg_line);
            self.target(base, 0, "then", cloc);
        }
        if let Some(a) = c.arguments() {
            for arg in a.arguments().iter() {
                self.walk(&arg, false);
            }
        }
        match c.block() {
            Some(b) => match b.as_block_node() {
                Some(bn) => {
                    if matches!(c.name().as_slice(), b"define_method" | b"define_singleton_method") && self.record {
                        let bl = self.loc(&b);
                        let name = c
                            .arguments()
                            .and_then(|a| a.arguments().iter().next())
                            .and_then(|a| match &a {
                                Node::SymbolNode { .. } => Some(a.as_symbol_node().unwrap().unescaped().to_vec()),
                                Node::StringNode { .. } => Some(a.as_string_node().unwrap().unescaped().to_vec()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        self.methods.push((b.location().start_offset(), bl, name));
                    }
                    self.block(&bn);
                }
                None => self.walk(&b, false),
            },
            None => {}
        }
        self.emit(msg_line);
        if c.is_safe_navigation() {
            self.target(base, 1, "else", cloc);
        }
    }

    fn walk(&mut self, n: &Node, popped: bool) {
        let k = key(n);
        let mut line = self.own(n);
        if let Some(b) = n.as_begin_node() {
            if b.statements().is_none() {
                if let Some(r) = b.rescue_clause() {
                    line = self.own(&r.as_node());
                }
            }
        }
        if flags(n) & NEWLINE != 0 && line != self.last_line {
            self.last_line = line;
            self.pending = Some(Place::Before(k));
        }
        self.walk_inner(n, popped);
        if self.pending == Some(Place::Before(k)) {
            self.pending = Some(Place::After(k));
        }
    }

    fn walk_inner(&mut self, n: &Node, popped: bool) {
        let line = self.own(n);
        if quiet(n) {
            if !popped {
                self.emit(line);
            }
            return;
        }
        match n {
            Node::StatementsNode { .. } => {
                let s = n.as_statements_node().unwrap();
                let v: Vec<Node> = s.body().iter().collect();
                let last = v.len().saturating_sub(1);
                for (i, c) in v.iter().enumerate() {
                    self.walk(c, popped || i < last);
                }
            }
            Node::ParenthesesNode { .. } => match n.as_parentheses_node().unwrap().body() {
                Some(b) => self.walk(&b, popped),
                None => {
                    if !popped {
                        self.emit(line);
                    }
                }
            },
            Node::BeginNode { .. } => self.begin_node(&n.as_begin_node().unwrap(), popped),
            Node::RescueNode { .. } => self.rescue_clause(&n.as_rescue_node().unwrap()),
            Node::ElseNode { .. } => {
                let e = n.as_else_node().unwrap();
                self.stmts(e.statements(), popped);
            }
            Node::EnsureNode { .. } => {
                let e = n.as_ensure_node().unwrap();
                self.stmts(e.statements(), true);
            }
            Node::IfNode { .. } => {
                let i = n.as_if_node().unwrap();
                self.conditional(n, false, i.statements(), i.subsequent(), i.predicate(), popped);
            }
            Node::UnlessNode { .. } => {
                let u = n.as_unless_node().unwrap();
                let st = u.else_clause().and_then(|e| e.statements());
                self.conditional(n, true, st, u.statements().map(|s| s.as_node()), u.predicate(), popped);
            }
            Node::WhileNode { .. } => {
                let w = n.as_while_node().unwrap();
                self.loop_node(n, false, w.statements(), w.predicate());
            }
            Node::UntilNode { .. } => {
                let w = n.as_until_node().unwrap();
                self.loop_node(n, true, w.statements(), w.predicate());
            }
            Node::CaseNode { .. } => self.case_node(&n.as_case_node().unwrap(), popped),
            Node::CaseMatchNode { .. } => self.case_match(&n.as_case_match_node().unwrap(), popped),
            Node::CallNode { .. } => self.call(&n.as_call_node().unwrap(), popped),
            Node::DefNode { .. } => {
                let d = n.as_def_node().unwrap();
                if self.record {
                    let l = self.loc(n);
                    self.methods.push((n.location().start_offset(), l, d.name().as_slice().to_vec()));
                }
                self.iseq(|s| {
                    s.params(d.parameters());
                    if let Some(b) = d.body() {
                        s.walk(&b, false);
                    }
                });
                match d.receiver() {
                    Some(r) => self.walk(&r, false),
                    None => self.emit(line),
                }
                self.emit(line);
            }
            Node::ClassNode { .. } => {
                let c = n.as_class_node().unwrap();
                self.iseq(|s| {
                    if let Some(b) = c.body() {
                        s.walk(&b, false);
                    }
                });
                match c.constant_path().as_constant_path_node().and_then(|p| p.parent()) {
                    Some(p) => self.walk(&p, false),
                    None => self.emit(line),
                }
                if let Some(sc) = c.superclass() {
                    self.walk(&sc, false);
                }
                self.emit(line);
            }
            Node::ModuleNode { .. } => {
                let m = n.as_module_node().unwrap();
                self.iseq(|s| {
                    if let Some(b) = m.body() {
                        s.walk(&b, false);
                    }
                });
                match m.constant_path().as_constant_path_node().and_then(|p| p.parent()) {
                    Some(p) => self.walk(&p, false),
                    None => self.emit(line),
                }
                self.emit(line);
            }
            Node::SingletonClassNode { .. } => {
                let s = n.as_singleton_class_node().unwrap();
                self.iseq(|w| {
                    if let Some(b) = s.body() {
                        w.walk(&b, false);
                    }
                });
                self.walk(&s.expression(), false);
                self.emit(line);
            }
            Node::LambdaNode { .. } => {
                let l = n.as_lambda_node().unwrap();
                self.iseq(|s| {
                    s.block_params(l.parameters());
                    if let Some(b) = l.body() {
                        s.walk(&b, false);
                    }
                });
                self.emit(line);
            }
            Node::ForNode { .. } => {
                let f = n.as_for_node().unwrap();
                self.walk(&f.collection(), false);
                self.iseq(|s| {
                    if let Some(st) = f.statements() {
                        s.walk(&st.as_node(), false);
                    }
                });
                self.emit(line);
            }
            Node::SuperNode { .. } => {
                let s = n.as_super_node().unwrap();
                self.emit(line);
                if let Some(a) = s.arguments() {
                    for arg in a.arguments().iter() {
                        self.walk(&arg, false);
                    }
                }
                match s.block() {
                    Some(b) => match b.as_block_node() {
                        Some(bn) => self.block(&bn),
                        None => self.walk(&b, false),
                    },
                    None => {}
                }
                self.emit(line);
            }
            Node::ForwardingSuperNode { .. } => {
                let s = n.as_forwarding_super_node().unwrap();
                self.emit(line);
                if let Some(bn) = s.block() {
                    self.block(&bn);
                }
                self.emit(line);
            }
            Node::RescueModifierNode { .. } => {
                let r = n.as_rescue_modifier_node().unwrap();
                self.iseq(|s| s.walk(&r.rescue_expression(), false));
                self.walk(&r.expression(), false);
                self.emit(line);
            }
            Node::AndNode { .. } => {
                let a = n.as_and_node().unwrap();
                self.walk(&a.left(), false);
                self.walk(&a.right(), popped);
            }
            Node::OrNode { .. } => {
                let o = n.as_or_node().unwrap();
                self.walk(&o.left(), false);
                self.walk(&o.right(), popped);
            }
            Node::InterpolatedStringNode { .. } | Node::InterpolatedSymbolNode { .. } | Node::InterpolatedXStringNode { .. } | Node::InterpolatedRegularExpressionNode { .. } | Node::InterpolatedMatchLastLineNode { .. } => {
                // a leading string part is pushed first (at the node's line), else the
                // first interpolated code comes first
                let parts: Vec<Node> = match n {
                    Node::InterpolatedStringNode { .. } => n.as_interpolated_string_node().unwrap().parts().iter().collect(),
                    Node::InterpolatedSymbolNode { .. } => n.as_interpolated_symbol_node().unwrap().parts().iter().collect(),
                    Node::InterpolatedXStringNode { .. } => {
                        self.emit(line);
                        n.as_interpolated_x_string_node().unwrap().parts().iter().collect()
                    }
                    Node::InterpolatedRegularExpressionNode { .. } => n.as_interpolated_regular_expression_node().unwrap().parts().iter().collect(),
                    _ => n.as_interpolated_match_last_line_node().unwrap().parts().iter().collect(),
                };
                let string_part = |p: &Node| {
                    p.as_string_node().is_some()
                        || p.as_embedded_statements_node().and_then(|e| e.statements()).map_or(false, |s| {
                            let v: Vec<Node> = s.body().iter().collect();
                            v.len() == 1 && v[0].as_string_node().is_some()
                        })
                };
                if parts.first().map_or(true, |p| string_part(p)) {
                    self.emit(line);
                }
                for p in &parts {
                    if !string_part(p) {
                        self.walk(p, false);
                    }
                }
                self.emit(line);
            }
            Node::XStringNode { .. } => self.emit(line),
            Node::MultiWriteNode { .. } => {
                let m = n.as_multi_write_node().unwrap();
                for t in m.lefts().iter().chain(m.rest()).chain(m.rights().iter()) {
                    self.target_parts(&t);
                }
                self.walk(&m.value(), false);
                self.emit(line);
            }
            Node::HashNode { .. } | Node::KeywordHashNode { .. } => {
                let first = match n {
                    Node::HashNode { .. } => n.as_hash_node().unwrap().elements().iter().next(),
                    _ => n.as_keyword_hash_node().unwrap().elements().iter().next(),
                };
                // a leading run of literal pairs is one `duphash`
                let only_kw = matches!(n, Node::KeywordHashNode { .. }) && n.as_keyword_hash_node().unwrap().elements().iter().count() == 1;
                if first.map_or(false, |f| (f.as_assoc_node().is_some() && flags(&f) & STATIC_LITERAL != 0) || (f.as_assoc_splat_node().is_some() && !only_kw)) {
                    self.emit(line);
                }
                self.kids(n);
                self.emit(line);
            }
            Node::LocalVariableWriteNode { .. } => {
                let w = n.as_local_variable_write_node().unwrap();
                let v = w.value();
                // `x = x` compiles to nothing (peephole)
                let same = v.as_local_variable_read_node().map_or(false, |r| r.name().as_slice() == w.name().as_slice() && r.depth() == w.depth());
                if same {
                    if !popped {
                        self.emit(line);
                    }
                } else {
                    self.walk(&v, false);
                    self.emit(line);
                }
            }
            Node::EmbeddedStatementsNode { .. } => {
                let e = n.as_embedded_statements_node().unwrap();
                if let Some(s) = e.statements() {
                    self.walk(&s.as_node(), false);
                }
            }
            Node::DefinedNode { .. } | Node::AliasMethodNode { .. } | Node::AliasGlobalVariableNode { .. } | Node::UndefNode { .. } => {
                self.emit(line);
            }
            Node::LocalVariableOperatorWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantAndWriteNode { .. } => {
                self.emit(line);
                self.kids(n);
            }
            Node::BlockNode { .. } => self.block(&n.as_block_node().unwrap()),
            Node::MissingNode { .. } => {}
            Node::ArrayPatternNode { .. } | Node::HashPatternNode { .. } | Node::FindPatternNode { .. } | Node::CapturePatternNode { .. } | Node::AlternationPatternNode { .. } | Node::PinnedVariableNode { .. } | Node::PinnedExpressionNode { .. } => {
                self.emit(line);
                self.pattern(n);
            }
            Node::MatchPredicateNode { .. } | Node::MatchRequiredNode { .. } => {
                let (v, pat) = match n {
                    Node::MatchPredicateNode { .. } => {
                        let m = n.as_match_predicate_node().unwrap();
                        (m.value(), m.pattern())
                    }
                    _ => {
                        let m = n.as_match_required_node().unwrap();
                        (m.value(), m.pattern())
                    }
                };
                self.emit(line);
                self.walk(&v, false);
                self.emit(line);
                self.pattern(&pat);
            }
            _ => {
                self.kids(n);
                self.emit(line);
            }
        }
    }

    /// The receivers (and index arguments) of a multiple assignment's targets, compiled
    /// before its value.
    fn target_parts(&mut self, t: &Node) {
        match t {
            Node::CallTargetNode { .. } => {
                let c = t.as_call_target_node().unwrap();
                self.walk(&c.receiver(), false);
            }
            Node::IndexTargetNode { .. } => {
                let c = t.as_index_target_node().unwrap();
                self.walk(&c.receiver(), false);
                if let Some(a) = c.arguments() {
                    for x in a.arguments().iter() {
                        self.walk(&x, false);
                    }
                }
            }
            Node::ConstantPathTargetNode { .. } => {
                if let Some(p) = t.as_constant_path_target_node().unwrap().parent() {
                    self.walk(&p, false);
                }
            }
            Node::SplatNode { .. } => {
                if let Some(e) = t.as_splat_node().unwrap().expression() {
                    self.target_parts(&e);
                }
            }
            Node::MultiTargetNode { .. } => {
                let m = t.as_multi_target_node().unwrap();
                for x in m.lefts().iter().chain(m.rest()).chain(m.rights().iter()) {
                    self.target_parts(&x);
                }
            }
            _ => {}
        }
    }

    /// The code in a pattern (its values, guard, pinned expressions): only its blocks and
    /// branches matter, the pattern's own instructions are at its line.
    fn pattern(&mut self, n: &Node) {
        match n {
            Node::IfNode { .. } | Node::UnlessNode { .. } => {
                let (st, pred) = match n {
                    Node::IfNode { .. } => {
                        let i = n.as_if_node().unwrap();
                        (i.statements(), i.predicate())
                    }
                    _ => {
                        let u = n.as_unless_node().unwrap();
                        (u.statements(), u.predicate())
                    }
                };
                if let Some(st) = st {
                    for x in st.body().iter() {
                        self.pattern(&x);
                    }
                }
                self.walk(&pred, false);
            }
            Node::ArrayPatternNode { .. }
            | Node::HashPatternNode { .. }
            | Node::FindPatternNode { .. }
            | Node::CapturePatternNode { .. }
            | Node::AlternationPatternNode { .. }
            | Node::AssocNode { .. }
            | Node::AssocSplatNode { .. }
            | Node::SplatNode { .. }
            | Node::ImplicitNode { .. } => {
                for c in children(n) {
                    self.pattern(&c);
                }
            }
            Node::PinnedExpressionNode { .. } => self.walk(&n.as_pinned_expression_node().unwrap().expression(), false),
            Node::PinnedVariableNode { .. } | Node::LocalVariableTargetNode { .. } => {}
            _ => self.walk(n, false),
        }
    }

    /// The node's children in order (not popped).
    fn kids(&mut self, n: &Node) {
        for c in children(n) {
            self.walk(&c, false);
        }
    }
}

/// A node's direct children, in order.
fn children<'pr>(n: &Node<'pr>) -> Vec<Node<'pr>> {
    struct Kids<'pr> {
        depth: usize,
        out: Vec<Node<'pr>>,
    }
    impl<'pr> Visit<'pr> for Kids<'pr> {
        fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
            if self.depth == 1 {
                self.out.push(node);
            }
            self.depth += 1;
        }
        fn visit_branch_node_leave(&mut self) {
            self.depth -= 1;
        }
        fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
            if self.depth == 1 {
                self.out.push(node);
            }
        }
    }
    let mut k = Kids { depth: 0, out: Vec::new() };
    k.visit(n);
    k.out
}

/// What the compiler and the runtime need to know of one file.
pub(crate) struct FileCov {
    /// The file's number (its counters' data word is `__kn_cov_<id>`).
    pub id: usize,
    pub path: std::path::PathBuf,
    pub ncounters: u32,
    /// line events: where they are counted -> counter
    pub line_sites: HashMap<Place, u32>,
    /// branch targets: (base node, target key) -> counter
    pub branches: HashMap<(Key, u8), u32>,
    /// methods: def (or define_method block) start offset -> counter
    pub methods: HashMap<usize, u32>,
    /// the runtime's description (kenny-runtime coverage.rs `parse_meta`)
    pub meta: String,
    /// what each counter counts (KENNY_DEBUG_COV)
    pub what: Vec<String>,
}

/// Work out a file's coverage structure.
pub(crate) fn analyze(src: &[u8], prog: &ProgramNode, id: usize, path: &std::path::Path) -> FileCov {
    let mut starts = vec![0];
    starts.extend(src.iter().enumerate().filter(|(_, c)| **c == b'\n').map(|(i, _)| i + 1));
    // CRuby's lines array has one entry per newline (a last line without one isn't measured)
    let nlines = src.iter().filter(|c| **c == b'\n').count() as u32;
    let mut runs: Vec<Vec<(Place, u32)>> = Vec::new();
    let mut bases = Vec::new();
    let mut methods = Vec::new();
    for mode in [false, true] {
        let mut w = Walker {
            starts: starts.clone(),
            src_len: src.len(),
            branch_mode: mode,
            last_line: 0,
            pending: None,
            sites: Vec::new(),
            record: !mode,
            bases: std::mem::take(&mut bases),
            base_index: HashMap::new(),
            methods: std::mem::take(&mut methods),
            _src: std::marker::PhantomData,
        };
        w.iseq(|w| w.walk(&prog.statements().as_node(), false));
        bases = std::mem::take(&mut w.bases);
        methods = std::mem::take(&mut w.methods);
        runs.push(std::mem::take(&mut w.sites));
    }
    let mut line_sites: HashMap<Place, u32> = HashMap::new();
    let mut n = 0u32;
    for run in &runs {
        for (p, _) in run {
            if !line_sites.contains_key(p) {
                line_sites.insert(*p, n);
                n += 1;
            }
        }
    }
    let mut meta = format!("N {nlines}\n");
    for (mi, run) in runs.iter().enumerate() {
        meta.push_str(if mi == 0 { "S" } else { "T" });
        let mut seen = BTreeSet::new();
        for (p, l) in run {
            let c = line_sites[p];
            if *l > 0 && *l <= nlines && seen.insert(c) {
                meta.push_str(&format!(" {c}:{l}"));
            }
        }
        meta.push('\n');
    }
    // branch targets: counters in base order
    let mut branches = HashMap::new();
    for b in &bases {
        meta.push_str(&format!("B {} {} {} {} {}", b.kind, b.loc.0, b.loc.1, b.loc.2, b.loc.3));
        for (tk, kind, l) in &b.targets {
            branches.insert((b.key, *tk), n);
            meta.push_str(&format!(" | {} {} {} {} {} {}", kind, n, l.0, l.1, l.2, l.3));
            n += 1;
        }
        meta.push('\n');
    }
    let mut mcount = HashMap::new();
    for (off, l, name) in &methods {
        if mcount.contains_key(off) {
            continue;
        }
        mcount.insert(*off, n);
        // (the name last: it may contain spaces; empty when not known statically)
        meta.push_str(&format!("M {} {} {} {} {} {}\n", n, l.0, l.1, l.2, l.3, String::from_utf8_lossy(name)));
        n += 1;
    }
    let mut places = vec![Place::Before((0, 0, 0)); line_sites.len()];
    for (p, c) in &line_sites {
        places[*c as usize] = *p;
    }
    let line_at = |off: usize| starts.partition_point(|&s| s <= off.min(src.len())) as u32;
    let mut what: Vec<String> = places.iter().map(|p| match p {
        Place::Before(k) => format!("line {} (before)", line_at(k.0)),
        Place::After(k) => format!("line {} (after)", line_at(k.0)),
    }).collect();
    what.resize(n as usize, String::new());
    for b in &bases {
        for (tk, kind, l) in &b.targets {
            if let Some(&c) = branches.get(&(b.key, *tk)) {
                what[c as usize] = format!("branch {} {} at line {}", b.kind, kind, l.0);
            }
        }
    }
    for (off, l, _) in &methods {
        what[mcount[off] as usize] = format!("method at line {}", l.0);
    }
    FileCov { id, path: path.to_path_buf(), ncounters: n, line_sites, branches, methods: mcount, meta, what }
}

/// `kenny --coverage-dump`: the structure CRuby's `Coverage.peek_result` gives a file that
/// was compiled with lines and branches (`branches`) or lines only, for comparing.
pub fn dump(src: &[u8], path: &std::path::Path, branches: bool) -> String {
    let result = ruby_prism::parse(src);
    let prog = result.node();
    let Some(prog) = prog.as_program_node() else { return String::new() };
    let fc = analyze(src, &prog, 0, path);
    let mut out = format!("{}\n", path.display());
    let mut lines = BTreeSet::new();
    let mut nlines = 0;
    let mut bid = 0;
    let mut bout = String::new();
    for l in fc.meta.lines() {
        if let Some(r) = l.strip_prefix("N ") {
            nlines = r.parse().unwrap_or(0);
        } else if let Some(r) = l.strip_prefix(if branches { "T" } else { "S" }) {
            for p in r.split_whitespace() {
                if let Some((_, line)) = p.split_once(':') {
                    lines.insert(line.parse::<u32>().unwrap_or(0));
                }
            }
        } else if let Some(r) = l.strip_prefix("B ") {
            let mut parts = r.split(" | ");
            let b: Vec<&str> = parts.next().unwrap_or("").split(' ').collect();
            let kind = if b[0] == "&." { "\"&.\"" } else { b[0] };
            bout.push_str(&format!("B [:{}, {}, {}, {}, {}, {}]\n", kind, bid, b[1], b[2], b[3], b[4]));
            bid += 1;
            for t in parts {
                let t: Vec<&str> = t.split(' ').collect();
                bout.push_str(&format!("  [:{}, {}, {}, {}, {}, {}]\n", t[0], bid, t[2], t[3], t[4], t[5]));
                bid += 1;
            }
        }
    }
    out.push_str(&format!("L {}: {}\n", nlines, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" ")));
    if branches {
        out.push_str(&bout);
    }
    out
}

// ---------- instrumentation ----------

use crate::{mem, Compiler, Fx};
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, InstBuilder};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use std::rc::Rc;

/// The file being compiled: its structure, its counters' data word and its number.
pub(crate) type Cur = (Rc<FileCov>, DataId, usize);

/// Compiler::cov: the coverage build.
#[derive(Default)]
pub(crate) struct Build {
    /// every instrumented file: its counters' data word, its path and its description
    pub files: Vec<(DataId, String, String)>,
    pub all: Vec<Rc<FileCov>>,
    pub cur: Option<Cur>,
    /// the nodes whose line events are being compiled: an entry point (`expr_void`,
    /// `expr`, `num_expr`, `cond_br`) that hands the node to another counts it only once
    pub active: Vec<Key>,
    /// (a method's function, its file's word, its counter)
    pub methods: Vec<(FuncId, DataId, u32)>,
    /// (file number, counter) of the counters compiled (KENNY_DEBUG_COV)
    pub emitted: std::collections::HashSet<(usize, u32)>,
}

impl Build {
    /// KENNY_DEBUG_COV: the counters no code increments (the model found a line event, a
    /// branch or a method that the code generator doesn't compile where expected).
    pub(crate) fn report_unemitted(&self) {
        if std::env::var_os("KENNY_DEBUG_COV").is_none() {
            return;
        }
        for f in &self.all {
            for c in 0..f.ncounters {
                if !self.emitted.contains(&(f.id, c)) {
                    eprintln!("coverage: {}: counter {c} not compiled: {}", f.path.display(), f.what.get(c as usize).map(|s| s.as_str()).unwrap_or(""));
                }
            }
        }
    }
}

/// What `cov_enter` leaves for `cov_leave`.
pub(crate) struct Enter {
    after: Option<u32>,
    /// the node was pushed on `active`
    pushed: bool,
}

impl<M: Module> Compiler<M> {
    /// A source file's top level is being compiled: its counters. The previous file's state,
    /// to restore at the end (None: not instrumented).
    pub(crate) fn cov_begin(&mut self, src: &[u8], prog: &ProgramNode, path: &std::path::Path) -> Option<Option<Cur>> {
        if self.eval_unit || self.cov.is_none() {
            return None;
        }
        let idx = self.cov.as_ref().unwrap().files.len();
        let fc = Rc::new(analyze(src, prog, idx, path));
        let id = self.module.declare_data(&format!("__kn_cov_{idx}"), Linkage::Local, true, false).ok()?;
        let mut d = DataDescription::new();
        d.define_zeroinit(8);
        d.set_align(8);
        self.module.define_data(id, &d).ok()?;
        let b = self.cov.as_mut().unwrap();
        b.files.push((id, path.display().to_string(), fc.meta.clone()));
        b.all.push(fc.clone());
        Some(b.cur.replace((fc, id, idx)))
    }

    pub(crate) fn cov_end(&mut self, saved: Option<Option<Cur>>) {
        if let (Some(s), Some(b)) = (saved, self.cov.as_mut()) {
            b.cur = s;
        }
    }

    /// The address of the current file's counters word.
    fn cov_word(&mut self, fx: &mut Fx) -> Option<cranelift_codegen::ir::Value> {
        let id = self.cov.as_ref()?.cur.as_ref()?.1;
        let g = match fx.cov_gv {
            Some(g) => g,
            None => {
                let g = self.module.declare_data_in_func(id, fx.b.func);
                fx.cov_gv = Some(g);
                g
            }
        };
        Some(fx.b.ins().symbol_value(I64, g))
    }

    /// counters[c] += 1 (`first`: a line event, which tells the runtime when it first runs,
    /// for oneshot_lines)
    fn cov_count(&mut self, fx: &mut Fx, c: u32, first: bool) {
        let Some(w) = self.cov_word(fx) else { return };
        let b = self.cov.as_mut().unwrap();
        let fid = b.cur.as_ref().unwrap().2;
        b.emitted.insert((fid, c));
        let p = fx.b.ins().load(I64, mem(), w, 0);
        let off = (c as i32) * 8;
        let v = fx.b.ins().load(I64, mem(), p, off);
        let v1 = fx.b.ins().iadd_imm_s(v, 1);
        fx.b.ins().store(mem(), v1, p, off);
        if first {
            let z = fx.b.ins().icmp_imm_s(IntCC::Equal, v, 0);
            let cold = fx.b.create_block();
            let cont = fx.b.create_block();
            fx.b.set_cold_block(cold);
            fx.b.ins().brif(z, cold, &[], cont, &[]);
            fx.b.switch_to_block(cold);
            let f = self.rt_ref(fx, "kn_cov_first");
            let ci = fx.b.ins().iconst(I64, c as i64);
            fx.b.ins().call(f, &[w, ci]);
            fx.b.ins().jump(cont, &[]);
            fx.b.switch_to_block(cont);
        }
    }

    /// At the start of a file's top level: the runtime starts measuring it.
    pub(crate) fn cov_file_start(&mut self, fx: &mut Fx) {
        if let Some(w) = self.cov_word(fx) {
            let f = self.rt_ref(fx, "kn_cov_file_start");
            fx.b.ins().call(f, &[w]);
        }
    }

    fn cov_line_counter(&self, p: Place) -> Option<u32> {
        self.cov.as_ref()?.cur.as_ref()?.0.line_sites.get(&p).copied()
    }

    /// Before compiling a node: its line event, if it is a statement with one.
    pub(crate) fn cov_enter(&mut self, fx: &mut Fx, node: &Node) -> Enter {
        let k = key(node);
        let b = self.cov.as_mut().unwrap();
        if b.active.contains(&k) {
            return Enter { after: None, pushed: false };
        }
        b.active.push(k);
        if let Some(c) = self.cov_line_counter(Place::Before(k)) {
            self.cov_count(fx, c, true);
        }
        Enter { after: self.cov_line_counter(Place::After(k)), pushed: true }
    }

    pub(crate) fn cov_leave(&mut self, fx: &mut Fx, e: Enter) {
        let pushed = self.cov_leave_now(fx, e);
        self.cov_pop(pushed);
    }

    /// `cov_leave`'s count before the node is compiled (a condition: it ends in a branch);
    /// then `cov_pop` after it.
    pub(crate) fn cov_leave_now(&mut self, fx: &mut Fx, e: Enter) -> bool {
        if let Some(c) = e.after {
            self.cov_count(fx, c, true);
        }
        e.pushed
    }

    pub(crate) fn cov_pop(&mut self, pushed: bool) {
        if let (true, Some(b)) = (pushed, self.cov.as_mut()) {
            b.active.pop();
        }
    }

    pub(crate) fn cov_has_branch(&self, node: &Node) -> bool {
        self.cov.as_ref().and_then(|b| b.cur.as_ref()).is_some_and(|c| c.0.branches.contains_key(&(key(node), 0)))
    }

    /// Entering target `t` of the branch `node` (if, unless, case, &.).
    pub(crate) fn cov_branch(&mut self, fx: &mut Fx, node: &Node, t: u8) {
        if self.cov.is_some() {
            self.cov_branch_key(fx, key(node), t);
        }
    }

    pub(crate) fn cov_branch_key(&mut self, fx: &mut Fx, k: Key, t: u8) {
        let c = self.cov.as_ref().and_then(|b| b.cur.as_ref()).and_then(|c| c.0.branches.get(&(k, t)).copied());
        if let Some(c) = c {
            self.cov_count(fx, c, false);
        }
    }

    fn cov_method_counter(&self, at: usize) -> Option<u32> {
        self.cov.as_ref()?.cur.as_ref()?.0.methods.get(&at).copied()
    }

    /// At the start of a method's body (`at`: its key).
    pub(crate) fn cov_method(&mut self, fx: &mut Fx, at: usize) {
        if let Some(c) = self.cov_method_counter(at) {
            self.cov_count(fx, c, false);
        }
    }

    /// The function a method's entry is (`kn_def`'s, or a `define_method` block's).
    pub(crate) fn cov_method_fn(&mut self, fid: FuncId, at: usize) {
        if let Some(c) = self.cov_method_counter(at) {
            let b = self.cov.as_mut().unwrap();
            let w = b.cur.as_ref().unwrap().1;
            b.methods.push((fid, w, c));
        }
    }
}
