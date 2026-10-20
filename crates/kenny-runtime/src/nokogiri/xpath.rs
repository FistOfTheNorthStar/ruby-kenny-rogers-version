//! XPath 1.0: parser and evaluator over a [`Dom`], with Nokogiri's extensions
//! (`nokogiri-builtin:css-class`, `nokogiri-builtin:local-name-is`, `*:name` wildcard
//! namespaces) and custom functions resolved through a [`Host`] (Nokogiri's handler objects).

use super::dom::*;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Axis {
    Child,
    Descendant,
    DescendantOrSelf,
    Parent,
    Ancestor,
    AncestorOrSelf,
    FollowingSibling,
    PrecedingSibling,
    Following,
    Preceding,
    Attribute,
    SelfAxis,
    Namespace,
}

impl Axis {
    fn reverse(self) -> bool {
        matches!(self, Axis::Ancestor | Axis::AncestorOrSelf | Axis::PrecedingSibling | Axis::Preceding)
    }
}

#[derive(Clone, Debug)]
pub enum Test {
    Node,
    Text,
    Comment,
    PI(Option<String>),
    /// `*`
    Any,
    /// `prefix:*`
    NsAny(String),
    /// `*:local` (any namespace or none)
    LocalAny(String),
    /// `name` / `prefix:name`
    Name(Option<String>, String),
}

#[derive(Clone, Debug)]
pub struct Step {
    pub axis: Axis,
    pub test: Test,
    pub preds: Vec<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Union,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Bin(Op, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    /// absolute?, steps
    Path(bool, Vec<Step>),
    /// primary, predicates, trailing steps
    Filter(Box<Expr>, Vec<Expr>, Vec<Step>),
    Lit(String),
    Num(f64),
    Var(String),
    Call(Option<String>, String, Vec<Expr>),
}

// ---------------------------------------------------------------------------
// tokenizer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LParen,
    RParen,
    LBrack,
    RBrack,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    SlashSlash,
    Pipe,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Star,
    /// operator names and the multiply operator
    OpName(String),
    Lit(String),
    Num(f64),
    Var(String),
    /// NameTest: (prefix, local) where local may be "*"; prefix "*" means any namespace
    Name(Option<String>, String),
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || !c.is_ascii()
}
fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    // per XPath 1.0 3.7: is the next `*` / name an operator?
    let op_ctx = |out: &Vec<Tok>| match out.last() {
        None => false,
        Some(t) => !matches!(
            t,
            Tok::At
                | Tok::ColonColon
                | Tok::LParen
                | Tok::LBrack
                | Tok::Comma
                | Tok::OpName(_)
                | Tok::Slash
                | Tok::SlashSlash
                | Tok::Pipe
                | Tok::Plus
                | Tok::Minus
                | Tok::Eq
                | Tok::Ne
                | Tok::Lt
                | Tok::Le
                | Tok::Gt
                | Tok::Ge
        ),
    };
    let name_at = |i: usize| -> usize {
        let mut j = i;
        while j < cs.len() && is_name_char(cs[j]) {
            j += 1;
        }
        j
    };
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let next = cs.get(i + 1).copied();
        let t = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBrack,
            ']' => Tok::RBrack,
            '@' => Tok::At,
            ',' => Tok::Comma,
            '|' => Tok::Pipe,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '=' => Tok::Eq,
            '!' if next == Some('=') => {
                i += 1;
                Tok::Ne
            }
            '<' if next == Some('=') => {
                i += 1;
                Tok::Le
            }
            '<' => Tok::Lt,
            '>' if next == Some('=') => {
                i += 1;
                Tok::Ge
            }
            '>' => Tok::Gt,
            ':' if next == Some(':') => {
                i += 1;
                Tok::ColonColon
            }
            '/' if next == Some('/') => {
                i += 1;
                Tok::SlashSlash
            }
            '/' => Tok::Slash,
            '.' if next == Some('.') => {
                i += 1;
                Tok::DotDot
            }
            '.' if !next.is_some_and(|n| n.is_ascii_digit()) => Tok::Dot,
            '"' | '\'' => {
                let mut j = i + 1;
                while j < cs.len() && cs[j] != c {
                    j += 1;
                }
                if j >= cs.len() {
                    return Err("Unfinished literal".into());
                }
                let lit: String = cs[i + 1..j].iter().collect();
                i = j;
                Tok::Lit(lit)
            }
            '$' => {
                let j = name_at(i + 1);
                let mut name: String = cs[i + 1..j].iter().collect();
                let mut k = j;
                if k < cs.len() && cs[k] == ':' && k + 1 < cs.len() && is_name_start(cs[k + 1]) {
                    let e = name_at(k + 1);
                    name = cs[i + 1..e].iter().collect();
                    k = e;
                }
                i = k - 1;
                Tok::Var(name)
            }
            '*' => {
                if op_ctx(&out) {
                    Tok::OpName("*".into())
                } else if next == Some(':') && cs.get(i + 2).is_some_and(|&c| is_name_start(c)) {
                    let j = name_at(i + 2);
                    let local: String = cs[i + 2..j].iter().collect();
                    i = j - 1;
                    Tok::Name(Some("*".into()), local)
                } else {
                    Tok::Star
                }
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut j = i;
                while j < cs.len() && cs[j].is_ascii_digit() {
                    j += 1;
                }
                if j < cs.len() && cs[j] == '.' {
                    j += 1;
                    while j < cs.len() && cs[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                let s: String = cs[i..j].iter().collect();
                i = j - 1;
                Tok::Num(s.parse::<f64>().unwrap_or(f64::NAN))
            }
            c if is_name_start(c) => {
                let j = name_at(i);
                let first: String = cs[i..j].iter().collect();
                if op_ctx(&out) && matches!(first.as_str(), "and" | "or" | "mod" | "div") {
                    i = j - 1;
                    Tok::OpName(first)
                } else if j < cs.len() && cs[j] == ':' && cs.get(j + 1) != Some(&':') {
                    // QName or prefix:*
                    if cs.get(j + 1) == Some(&'*') {
                        i = j + 1;
                        Tok::Name(Some(first), "*".into())
                    } else if cs.get(j + 1).is_some_and(|&c| is_name_start(c)) {
                        let e = name_at(j + 1);
                        let local: String = cs[j + 1..e].iter().collect();
                        i = e - 1;
                        Tok::Name(Some(first), local)
                    } else {
                        return Err(format!("Invalid QName at {}", j));
                    }
                } else {
                    i = j - 1;
                    Tok::Name(None, first)
                }
            }
            _ => return Err(format!("Invalid expression: unexpected '{c}'")),
        };
        out.push(t);
        i += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// parser
// ---------------------------------------------------------------------------

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn peek2(&self) -> Option<&Tok> {
        self.t.get(self.i + 1)
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok) -> Result<(), String> {
        if self.eat(t) { Ok(()) } else { Err(format!("Invalid expression: expected {t:?}")) }
    }
    fn op_name(&self, n: &str) -> bool {
        matches!(self.peek(), Some(Tok::OpName(x)) if x == n)
    }

    fn expr(&mut self) -> Result<Expr, String> {
        self.or()
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.op_name("or") {
            self.i += 1;
            let r = self.and()?;
            l = Expr::Bin(Op::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.equality()?;
        while self.op_name("and") {
            self.i += 1;
            let r = self.equality()?;
            l = Expr::Bin(Op::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn equality(&mut self) -> Result<Expr, String> {
        let mut l = self.relational()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => Op::Eq,
                Some(Tok::Ne) => Op::Ne,
                _ => break,
            };
            self.i += 1;
            let r = self.relational()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn relational(&mut self) -> Result<Expr, String> {
        let mut l = self.additive()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Lt) => Op::Lt,
                Some(Tok::Le) => Op::Le,
                Some(Tok::Gt) => Op::Gt,
                Some(Tok::Ge) => Op::Ge,
                _ => break,
            };
            self.i += 1;
            let r = self.additive()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn additive(&mut self) -> Result<Expr, String> {
        let mut l = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => Op::Add,
                Some(Tok::Minus) => Op::Sub,
                _ => break,
            };
            self.i += 1;
            let r = self.multiplicative()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::OpName(n)) if n == "*" => Op::Mul,
                Some(Tok::OpName(n)) if n == "div" => Op::Div,
                Some(Tok::OpName(n)) if n == "mod" => Op::Mod,
                _ => break,
            };
            self.i += 1;
            let r = self.unary()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.union()
    }
    fn union(&mut self) -> Result<Expr, String> {
        let mut l = self.path()?;
        while self.eat(&Tok::Pipe) {
            let r = self.path()?;
            l = Expr::Bin(Op::Union, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn starts_step(&self) -> bool {
        match self.peek() {
            Some(Tok::Dot | Tok::DotDot | Tok::At | Tok::Star) => true,
            Some(Tok::Name(..)) => !matches!(self.peek2(), Some(Tok::LParen)) || self.node_type_ahead(),
            _ => false,
        }
    }
    fn node_type_ahead(&self) -> bool {
        matches!(self.peek(), Some(Tok::Name(None, n)) if matches!(n.as_str(), "node" | "text" | "comment" | "processing-instruction"))
            && matches!(self.peek2(), Some(Tok::LParen))
    }

    fn path(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Slash) => {
                self.i += 1;
                let steps = if self.starts_step() { self.rel_steps()? } else { Vec::new() };
                Ok(Expr::Path(true, steps))
            }
            Some(Tok::SlashSlash) => {
                self.i += 1;
                let mut steps = vec![Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] }];
                steps.extend(self.rel_steps()?);
                Ok(Expr::Path(true, optimize(steps)))
            }
            Some(Tok::Var(_) | Tok::LParen | Tok::Lit(_) | Tok::Num(_)) => self.filter(),
            Some(Tok::Name(..)) if matches!(self.peek2(), Some(Tok::LParen)) && !self.node_type_ahead() => self.filter(),
            _ => {
                let steps = self.rel_steps()?;
                Ok(Expr::Path(false, optimize(steps)))
            }
        }
    }
    fn filter(&mut self) -> Result<Expr, String> {
        let prim = self.primary()?;
        let mut preds = Vec::new();
        while self.eat(&Tok::LBrack) {
            preds.push(self.expr()?);
            self.expect(&Tok::RBrack)?;
        }
        let mut steps = Vec::new();
        loop {
            if self.eat(&Tok::Slash) {
                steps.push(self.step()?);
            } else if self.eat(&Tok::SlashSlash) {
                steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] });
                steps.push(self.step()?);
            } else {
                break;
            }
        }
        if preds.is_empty() && steps.is_empty() {
            return Ok(prim);
        }
        Ok(Expr::Filter(Box::new(prim), preds, optimize(steps)))
    }
    fn primary(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Tok::Var(v)) => {
                self.i += 1;
                Ok(Expr::Var(v))
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Lit(s)) => {
                self.i += 1;
                Ok(Expr::Lit(s))
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Name(prefix, name)) => {
                self.i += 1;
                self.expect(&Tok::LParen)?;
                let mut args = Vec::new();
                if !self.eat(&Tok::RParen) {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(&Tok::Comma) {
                            continue;
                        }
                        self.expect(&Tok::RParen)?;
                        break;
                    }
                }
                Ok(Expr::Call(prefix, name, args))
            }
            t => Err(format!("Invalid expression: unexpected {t:?}")),
        }
    }
    fn rel_steps(&mut self) -> Result<Vec<Step>, String> {
        let mut steps = vec![self.step()?];
        loop {
            if self.eat(&Tok::Slash) {
                steps.push(self.step()?);
            } else if self.eat(&Tok::SlashSlash) {
                steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: vec![] });
                steps.push(self.step()?);
            } else {
                break;
            }
        }
        Ok(steps)
    }
    fn step(&mut self) -> Result<Step, String> {
        if self.eat(&Tok::Dot) {
            return Ok(Step { axis: Axis::SelfAxis, test: Test::Node, preds: vec![] });
        }
        if self.eat(&Tok::DotDot) {
            return Ok(Step { axis: Axis::Parent, test: Test::Node, preds: vec![] });
        }
        let mut axis = Axis::Child;
        if self.eat(&Tok::At) {
            axis = Axis::Attribute;
        } else if let (Some(Tok::Name(None, a)), Some(Tok::ColonColon)) = (self.peek().cloned(), self.peek2()) {
            axis = match a.as_str() {
                "child" => Axis::Child,
                "descendant" => Axis::Descendant,
                "descendant-or-self" => Axis::DescendantOrSelf,
                "parent" => Axis::Parent,
                "ancestor" => Axis::Ancestor,
                "ancestor-or-self" => Axis::AncestorOrSelf,
                "following-sibling" => Axis::FollowingSibling,
                "preceding-sibling" => Axis::PrecedingSibling,
                "following" => Axis::Following,
                "preceding" => Axis::Preceding,
                "attribute" => Axis::Attribute,
                "self" => Axis::SelfAxis,
                "namespace" => Axis::Namespace,
                _ => return Err(format!("Invalid expression: unknown axis {a}")),
            };
            self.i += 2;
        }
        let test = match self.peek().cloned() {
            Some(Tok::Star) => {
                self.i += 1;
                Test::Any
            }
            Some(Tok::Name(None, n)) if self.node_type_ahead() => {
                self.i += 2;
                let t = match n.as_str() {
                    "node" => Test::Node,
                    "text" => Test::Text,
                    "comment" => Test::Comment,
                    _ => {
                        if let Some(Tok::Lit(l)) = self.peek().cloned() {
                            self.i += 1;
                            Test::PI(Some(l))
                        } else {
                            Test::PI(None)
                        }
                    }
                };
                self.expect(&Tok::RParen)?;
                t
            }
            Some(Tok::Name(p, n)) => {
                self.i += 1;
                match (p, n.as_str()) {
                    (Some(p), "*") if p != "*" => Test::NsAny(p),
                    (Some(p), _) if p == "*" => Test::LocalAny(n),
                    (p, _) => Test::Name(p, n),
                }
            }
            t => return Err(format!("Invalid expression: unexpected {t:?}")),
        };
        let mut preds = Vec::new();
        while self.eat(&Tok::LBrack) {
            preds.push(self.expr()?);
            self.expect(&Tok::RBrack)?;
        }
        Ok(Step { axis, test, preds })
    }
}

/// `descendant-or-self::node()/child::x` (no predicates) is `descendant::x`.
fn optimize(steps: Vec<Step>) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::with_capacity(steps.len());
    for s in steps {
        if let Some(prev) = out.last() {
            if prev.axis == Axis::DescendantOrSelf
                && matches!(prev.test, Test::Node)
                && prev.preds.is_empty()
                && s.axis == Axis::Child
                && s.preds.is_empty()
            {
                out.pop();
                out.push(Step { axis: Axis::Descendant, test: s.test, preds: vec![] });
                continue;
            }
        }
        out.push(s);
    }
    out
}

pub fn parse(s: &str) -> Result<Expr, String> {
    let toks = tokenize(s)?;
    let mut p = P { t: toks, i: 0 };
    if p.t.is_empty() {
        return Err("Invalid expression".into());
    }
    let e = p.expr()?;
    if p.i != p.t.len() {
        return Err("Invalid expression".into());
    }
    Ok(e)
}

thread_local! {
    static CACHE: std::cell::RefCell<HashMap<String, Rc<Expr>>> = std::cell::RefCell::new(HashMap::new());
}

pub fn parse_cached(s: &str) -> Result<Rc<Expr>, String> {
    if let Some(e) = CACHE.with(|c| c.borrow().get(s).cloned()) {
        return Ok(e);
    }
    let e = Rc::new(parse(s)?);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 2000 {
            c.clear();
        }
        c.insert(s.to_string(), e.clone());
    });
    Ok(e)
}

// ---------------------------------------------------------------------------
// evaluation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Val {
    Nodes(Vec<u32>),
    Str(String),
    Num(f64),
    Bool(bool),
}

/// Resolves functions the evaluator doesn't know (Nokogiri's custom XPath handlers).
pub trait Host {
    fn call(&mut self, name: &str, args: Vec<Val>) -> Result<Val, String>;
}

pub struct Eval<'h> {
    dom: *const Dom,
    pub ns: HashMap<String, String>,
    pub vars: HashMap<String, Val>,
    pub host: Option<&'h mut dyn Host>,
    order: Option<Vec<u64>>,
}

struct Ctx {
    node: u32,
    pos: usize,
    size: usize,
}

pub fn num_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n == n.trunc() && n.abs() < 1e18 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n}");
        if s.contains('e') { format!("{n:.}") } else { s }
    }
}

pub fn str_to_num(s: &str) -> f64 {
    let t = s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r');
    let body = t.strip_prefix('-').unwrap_or(t);
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.') || body.matches('.').count() > 1 || body == "." {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

fn xpath_round(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() {
        return n;
    }
    if n < 0.0 && n >= -0.5 {
        return -0.0;
    }
    (n + 0.5).floor()
}

impl<'h> Eval<'h> {
    pub fn new(dom: &Dom) -> Eval<'h> {
        Eval { dom: dom as *const Dom, ns: HashMap::new(), vars: HashMap::new(), host: None, order: None }
    }
    #[inline]
    fn d(&self) -> &Dom {
        unsafe { &*self.dom }
    }

    pub fn evaluate(&mut self, e: &Expr, node: u32) -> Result<Val, String> {
        self.order = None;
        self.eval(e, &Ctx { node, pos: 1, size: 1 })
    }

    fn keys(&mut self) -> &Vec<u64> {
        if self.order.as_ref().is_none_or(|o| o.len() != self.d().nodes.len()) {
            let keys = self.d().order_all();
            self.order = Some(keys);
        }
        self.order.as_ref().unwrap()
    }
    fn sort_dedupe(&mut self, v: &mut Vec<u32>) {
        if v.len() < 2 {
            return;
        }
        let keys = self.keys();
        v.sort_by_key(|&n| keys[n as usize]);
        v.dedup();
    }

    pub fn string_value(&self, n: u32) -> String {
        self.d().content(n)
    }

    fn to_str(&self, v: &Val) -> String {
        match v {
            Val::Str(s) => s.clone(),
            Val::Num(n) => num_to_string(*n),
            Val::Bool(b) => if *b { "true".into() } else { "false".into() },
            Val::Nodes(ns) => ns.first().map(|&n| self.string_value(n)).unwrap_or_default(),
        }
    }
    fn to_num(&self, v: &Val) -> f64 {
        match v {
            Val::Num(n) => *n,
            Val::Bool(b) => if *b { 1.0 } else { 0.0 },
            _ => str_to_num(&self.to_str(v)),
        }
    }
    fn to_bool(&self, v: &Val) -> bool {
        match v {
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            Val::Nodes(ns) => !ns.is_empty(),
        }
    }

    fn eval(&mut self, e: &Expr, c: &Ctx) -> Result<Val, String> {
        match e {
            Expr::Lit(s) => Ok(Val::Str(s.clone())),
            Expr::Num(n) => Ok(Val::Num(*n)),
            Expr::Var(v) => self.vars.get(v).cloned().ok_or_else(|| format!("Undefined variable: {v}")),
            Expr::Neg(x) => {
                let v = self.eval(x, c)?;
                Ok(Val::Num(-self.to_num(&v)))
            }
            Expr::Bin(op, l, r) => self.binary(*op, l, r, c),
            Expr::Path(abs, steps) => {
                let start = if *abs { 0 } else { c.node };
                let mut set = vec![start];
                for s in steps {
                    set = self.step(&set, s)?;
                }
                Ok(Val::Nodes(set))
            }
            Expr::Filter(prim, preds, steps) => {
                let v = self.eval(prim, c)?;
                let Val::Nodes(mut set) = v else {
                    if preds.is_empty() && steps.is_empty() {
                        return Ok(v);
                    }
                    return Err("Invalid type: predicate on a non-node-set".into());
                };
                self.sort_dedupe(&mut set);
                for p in preds {
                    set = self.apply_pred(&set, p)?;
                }
                for s in steps {
                    set = self.step(&set, s)?;
                }
                Ok(Val::Nodes(set))
            }
            Expr::Call(prefix, name, args) => self.call(prefix.as_deref(), name, args, c),
        }
    }

    fn binary(&mut self, op: Op, l: &Expr, r: &Expr, c: &Ctx) -> Result<Val, String> {
        match op {
            Op::Or => {
                let a = self.eval(l, c)?;
                if self.to_bool(&a) {
                    return Ok(Val::Bool(true));
                }
                let b = self.eval(r, c)?;
                Ok(Val::Bool(self.to_bool(&b)))
            }
            Op::And => {
                let a = self.eval(l, c)?;
                if !self.to_bool(&a) {
                    return Ok(Val::Bool(false));
                }
                let b = self.eval(r, c)?;
                Ok(Val::Bool(self.to_bool(&b)))
            }
            Op::Union => {
                let a = self.eval(l, c)?;
                let b = self.eval(r, c)?;
                match (a, b) {
                    (Val::Nodes(mut x), Val::Nodes(y)) => {
                        x.extend(y);
                        self.sort_dedupe(&mut x);
                        Ok(Val::Nodes(x))
                    }
                    _ => Err("Invalid type: union of non-node-sets".into()),
                }
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod => {
                let a = self.eval(l, c)?;
                let b = self.eval(r, c)?;
                let (x, y) = (self.to_num(&a), self.to_num(&b));
                Ok(Val::Num(match op {
                    Op::Add => x + y,
                    Op::Sub => x - y,
                    Op::Mul => x * y,
                    Op::Div => x / y,
                    _ => x % y,
                }))
            }
            _ => {
                let a = self.eval(l, c)?;
                let b = self.eval(r, c)?;
                Ok(Val::Bool(self.compare(op, &a, &b)))
            }
        }
    }

    fn cmp_atoms(&self, op: Op, a: &Val, b: &Val) -> bool {
        match op {
            Op::Eq | Op::Ne => {
                let eq = if matches!(a, Val::Bool(_)) || matches!(b, Val::Bool(_)) {
                    self.to_bool(a) == self.to_bool(b)
                } else if matches!(a, Val::Num(_)) || matches!(b, Val::Num(_)) {
                    self.to_num(a) == self.to_num(b)
                } else {
                    self.to_str(a) == self.to_str(b)
                };
                if op == Op::Eq { eq } else { !eq }
            }
            _ => {
                let (x, y) = (self.to_num(a), self.to_num(b));
                match op {
                    Op::Lt => x < y,
                    Op::Le => x <= y,
                    Op::Gt => x > y,
                    _ => x >= y,
                }
            }
        }
    }

    fn compare(&self, op: Op, a: &Val, b: &Val) -> bool {
        match (a, b) {
            (Val::Nodes(x), Val::Nodes(y)) => {
                let ys: Vec<String> = y.iter().map(|&n| self.string_value(n)).collect();
                x.iter().any(|&n| {
                    let s = Val::Str(self.string_value(n));
                    ys.iter().any(|t| {
                        if matches!(op, Op::Eq | Op::Ne) {
                            self.cmp_atoms(op, &s, &Val::Str(t.clone()))
                        } else {
                            self.cmp_atoms(op, &Val::Num(self.to_num(&s)), &Val::Num(str_to_num(t)))
                        }
                    })
                })
            }
            (Val::Nodes(x), other) => match other {
                Val::Bool(_) => self.cmp_atoms(op, &Val::Bool(!x.is_empty()), other),
                Val::Num(_) => x.iter().any(|&n| self.cmp_atoms(op, &Val::Num(str_to_num(&self.string_value(n))), other)),
                _ => x.iter().any(|&n| self.cmp_atoms(op, &Val::Str(self.string_value(n)), other)),
            },
            (other, Val::Nodes(y)) => {
                let flipped = match op {
                    Op::Lt => Op::Gt,
                    Op::Le => Op::Ge,
                    Op::Gt => Op::Lt,
                    Op::Ge => Op::Le,
                    o => o,
                };
                self.compare(flipped, &Val::Nodes(y.clone()), other)
            }
            _ => self.cmp_atoms(op, a, b),
        }
    }

    // ---------- steps ----------

    fn principal(&self, axis: Axis) -> T {
        if axis == Axis::Attribute { T::Attr } else { T::Element }
    }

    fn resolve(&self, prefix: &str) -> Result<String, String> {
        if prefix == "xml" {
            return Ok(XML_NS.to_string());
        }
        self.ns.get(prefix).cloned().ok_or_else(|| "Undefined namespace prefix".to_string())
    }

    fn matches(&self, n: u32, axis: Axis, test: &Test, href: &Option<String>) -> bool {
        let d = self.d();
        let node = d.n(n);
        match test {
            Test::Node => true,
            Test::Text => matches!(node.t, T::Text | T::CData),
            Test::Comment => node.t == T::Comment,
            Test::PI(name) => node.t == T::PI && name.as_ref().is_none_or(|x| *x == node.name),
            Test::Any => node.t == self.principal(axis),
            Test::NsAny(_) => {
                node.t == self.principal(axis) && node.ns != NONE && Some(&d.nss[node.ns as usize].href) == href.as_ref()
            }
            Test::LocalAny(local) => node.t == self.principal(axis) && node.name == *local,
            Test::Name(_, local) => {
                if node.t != self.principal(axis) || node.name != *local {
                    return false;
                }
                match href {
                    None => node.ns == NONE,
                    Some(h) => node.ns != NONE && d.nss[node.ns as usize].href == *h,
                }
            }
        }
    }

    fn axis_nodes(&self, n: u32, axis: Axis, out: &mut Vec<u32>) {
        let d = self.d();
        let node = d.n(n);
        match axis {
            Axis::Child => {
                if node.t != T::Attr {
                    let mut c = node.first;
                    while c != NONE {
                        out.push(c);
                        c = d.n(c).next;
                    }
                }
            }
            Axis::Descendant | Axis::DescendantOrSelf => {
                if axis == Axis::DescendantOrSelf {
                    out.push(n);
                }
                if node.t == T::Attr {
                    return;
                }
                // preorder without recursion
                let mut c = node.first;
                while c != NONE {
                    out.push(c);
                    let cn = d.n(c);
                    if cn.first != NONE {
                        c = cn.first;
                        continue;
                    }
                    let mut x = c;
                    loop {
                        if x == n {
                            c = NONE;
                            break;
                        }
                        let xn = d.n(x);
                        if xn.next != NONE {
                            c = xn.next;
                            break;
                        }
                        x = xn.parent;
                        if x == NONE || x == n {
                            c = NONE;
                            break;
                        }
                    }
                }
            }
            Axis::Parent => {
                if node.parent != NONE {
                    out.push(node.parent);
                }
            }
            Axis::Ancestor | Axis::AncestorOrSelf => {
                if axis == Axis::AncestorOrSelf {
                    out.push(n);
                }
                let mut p = node.parent;
                while p != NONE {
                    out.push(p);
                    p = d.n(p).parent;
                }
            }
            Axis::FollowingSibling => {
                if node.t != T::Attr {
                    let mut c = node.next;
                    while c != NONE {
                        out.push(c);
                        c = d.n(c).next;
                    }
                }
            }
            Axis::PrecedingSibling => {
                if node.t != T::Attr {
                    let mut c = node.prev;
                    while c != NONE {
                        out.push(c);
                        c = d.n(c).prev;
                    }
                }
            }
            Axis::Following => {
                let mut x = n;
                if node.t == T::Attr {
                    // the owner's descendants follow the attribute
                    x = node.parent;
                    if x == NONE {
                        return;
                    }
                    self.axis_nodes(x, Axis::Descendant, out);
                }
                while x != NONE {
                    let mut s = d.n(x).next;
                    while s != NONE {
                        self.axis_nodes(s, Axis::DescendantOrSelf, out);
                        s = d.n(s).next;
                    }
                    x = d.n(x).parent;
                }
            }
            Axis::Preceding => {
                let mut x = if node.t == T::Attr { node.parent } else { n };
                while x != NONE {
                    let mut s = d.n(x).prev;
                    while s != NONE {
                        let mut sub = Vec::new();
                        self.axis_nodes(s, Axis::DescendantOrSelf, &mut sub);
                        out.extend(sub.into_iter().rev());
                        s = d.n(s).prev;
                    }
                    x = d.n(x).parent;
                }
            }
            Axis::Attribute => {
                if node.t == T::Element {
                    out.extend(node.attrs.iter().copied());
                }
            }
            Axis::SelfAxis => out.push(n),
            Axis::Namespace => {}
        }
    }

    fn step(&mut self, set: &[u32], s: &Step) -> Result<Vec<u32>, String> {
        let href = match &s.test {
            Test::Name(Some(p), _) | Test::NsAny(p) => Some(self.resolve(p)?),
            _ => None,
        };
        let mut out = Vec::new();
        let mut cand = Vec::new();
        for &n in set {
            cand.clear();
            self.axis_nodes(n, s.axis, &mut cand);
            cand.retain(|&x| self.matches(x, s.axis, &s.test, &href));
            let mut cur = std::mem::take(&mut cand);
            for p in &s.preds {
                cur = self.apply_pred(&cur, p)?;
            }
            if s.axis.reverse() {
                cur.reverse();
            }
            out.extend(cur.iter().copied());
            cand = cur;
        }
        if set.len() > 1 || s.axis.reverse() {
            self.sort_dedupe(&mut out);
        }
        Ok(out)
    }

    fn apply_pred(&mut self, set: &[u32], p: &Expr) -> Result<Vec<u32>, String> {
        // a constant number predicate is a position
        if let Expr::Num(k) = p {
            let k = *k;
            return Ok(if k >= 1.0 && k == k.trunc() && (k as usize) <= set.len() { vec![set[k as usize - 1]] } else { vec![] });
        }
        let size = set.len();
        let mut out = Vec::new();
        for (i, &n) in set.iter().enumerate() {
            let v = self.eval(p, &Ctx { node: n, pos: i + 1, size })?;
            let keep = match v {
                Val::Num(k) => k == (i + 1) as f64,
                v => self.to_bool(&v),
            };
            if keep {
                out.push(n);
            }
        }
        Ok(out)
    }

    // ---------- functions ----------

    fn arg_str(&mut self, args: &[Expr], i: usize, c: &Ctx) -> Result<String, String> {
        match args.get(i) {
            Some(e) => {
                let v = self.eval(e, c)?;
                Ok(self.to_str(&v))
            }
            None => Ok(self.string_value(c.node)),
        }
    }
    fn arg_num(&mut self, args: &[Expr], i: usize, c: &Ctx) -> Result<f64, String> {
        match args.get(i) {
            Some(e) => {
                let v = self.eval(e, c)?;
                Ok(self.to_num(&v))
            }
            None => Ok(str_to_num(&self.string_value(c.node))),
        }
    }
    fn arg_nodes(&mut self, args: &[Expr], i: usize, c: &Ctx) -> Result<Vec<u32>, String> {
        match args.get(i) {
            Some(e) => match self.eval(e, c)? {
                Val::Nodes(n) => Ok(n),
                _ => Err("Invalid type".into()),
            },
            None => Ok(vec![c.node]),
        }
    }
    fn nargs(&self, args: &[Expr], min: usize, max: usize) -> Result<(), String> {
        if args.len() < min || args.len() > max { Err("Invalid number of arguments".into()) } else { Ok(()) }
    }

    fn call(&mut self, prefix: Option<&str>, name: &str, args: &[Expr], c: &Ctx) -> Result<Val, String> {
        let builtin = prefix.is_none();
        if prefix == Some("nokogiri-builtin") {
            match name {
                "css-class" => {
                    self.nargs(args, 2, 2)?;
                    let hay = self.arg_str(args, 0, c)?;
                    let needle = self.arg_str(args, 1, c)?;
                    return Ok(Val::Bool(hay.split_ascii_whitespace().any(|w| w == needle)));
                }
                "local-name-is" => {
                    self.nargs(args, 1, 1)?;
                    let want = self.arg_str(args, 0, c)?;
                    let n = self.d().n(c.node);
                    return Ok(Val::Bool(n.t == T::Element && n.name == want));
                }
                "next-element" => {
                    let mut x = self.d().n(c.node).next;
                    while x != NONE && self.d().t(x) != T::Element {
                        x = self.d().n(x).next;
                    }
                    return Ok(Val::Nodes(if x == NONE { vec![] } else { vec![x] }));
                }
                _ => {}
            }
        }
        if builtin {
            match name {
                "last" => return Ok(Val::Num(c.size as f64)),
                "position" => return Ok(Val::Num(c.pos as f64)),
                "count" => {
                    self.nargs(args, 1, 1)?;
                    return Ok(Val::Num(self.arg_nodes(args, 0, c)?.len() as f64));
                }
                "id" => {
                    self.nargs(args, 1, 1)?;
                    let v = self.eval(&args[0], c)?;
                    let ids: Vec<String> = match &v {
                        Val::Nodes(ns) => ns.iter().flat_map(|&n| self.string_value(n).split_whitespace().map(String::from).collect::<Vec<_>>()).collect(),
                        v => self.to_str(v).split_whitespace().map(String::from).collect(),
                    };
                    let mut all = Vec::new();
                    self.axis_nodes(0, Axis::Descendant, &mut all);
                    let d = self.d();
                    let out: Vec<u32> = all
                        .into_iter()
                        .filter(|&n| d.t(n) == T::Element && d.get_attr(n, "id").is_some_and(|i| ids.iter().any(|x| x == i)))
                        .collect();
                    return Ok(Val::Nodes(out));
                }
                "local-name" | "name" | "namespace-uri" => {
                    self.nargs(args, 0, 1)?;
                    let ns = self.arg_nodes(args, 0, c)?;
                    let Some(&n) = ns.first() else { return Ok(Val::Str(String::new())) };
                    let d = self.d();
                    let node = d.n(n);
                    let s = match (name, node.t) {
                        (_, T::Element | T::Attr) => match name {
                            "local-name" => node.name.clone(),
                            "name" => d.qname(n),
                            _ => if node.ns == NONE { String::new() } else { d.nss[node.ns as usize].href.clone() },
                        },
                        (_, T::PI) if name != "namespace-uri" => node.name.clone(),
                        _ => String::new(),
                    };
                    return Ok(Val::Str(s));
                }
                "string" => {
                    self.nargs(args, 0, 1)?;
                    return Ok(Val::Str(self.arg_str(args, 0, c)?));
                }
                "concat" => {
                    if args.len() < 2 {
                        return Err("Invalid number of arguments".into());
                    }
                    let mut s = String::new();
                    for i in 0..args.len() {
                        s.push_str(&self.arg_str(args, i, c)?);
                    }
                    return Ok(Val::Str(s));
                }
                "starts-with" | "contains" | "substring-before" | "substring-after" | "ends-with" => {
                    self.nargs(args, 2, 2)?;
                    let a = self.arg_str(args, 0, c)?;
                    let b = self.arg_str(args, 1, c)?;
                    return Ok(match name {
                        "starts-with" => Val::Bool(a.starts_with(&b)),
                        "ends-with" => Val::Bool(a.ends_with(&b)),
                        "contains" => Val::Bool(a.contains(&b)),
                        "substring-before" => Val::Str(a.find(&b).map(|i| a[..i].to_string()).unwrap_or_default()),
                        _ => Val::Str(a.find(&b).map(|i| a[i + b.len()..].to_string()).unwrap_or_default()),
                    });
                }
                "substring" => {
                    self.nargs(args, 2, 3)?;
                    let s = self.arg_str(args, 0, c)?;
                    let start = xpath_round(self.arg_num(args, 1, c)?);
                    let len = if args.len() == 3 { xpath_round(self.arg_num(args, 2, c)?) } else { f64::INFINITY };
                    let out: String = s
                        .chars()
                        .enumerate()
                        .filter(|(i, _)| {
                            let p = (*i + 1) as f64;
                            p >= start && p < start + len
                        })
                        .map(|(_, ch)| ch)
                        .collect();
                    return Ok(Val::Str(out));
                }
                "string-length" => {
                    self.nargs(args, 0, 1)?;
                    return Ok(Val::Num(self.arg_str(args, 0, c)?.chars().count() as f64));
                }
                "normalize-space" => {
                    self.nargs(args, 0, 1)?;
                    let s = self.arg_str(args, 0, c)?;
                    return Ok(Val::Str(s.split([' ', '\t', '\n', '\r']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")));
                }
                "translate" => {
                    self.nargs(args, 3, 3)?;
                    let s = self.arg_str(args, 0, c)?;
                    let from: Vec<char> = self.arg_str(args, 1, c)?.chars().collect();
                    let to: Vec<char> = self.arg_str(args, 2, c)?.chars().collect();
                    let out: String = s
                        .chars()
                        .filter_map(|ch| match from.iter().position(|&f| f == ch) {
                            Some(i) => to.get(i).copied(),
                            None => Some(ch),
                        })
                        .collect();
                    return Ok(Val::Str(out));
                }
                "boolean" => {
                    self.nargs(args, 1, 1)?;
                    let v = self.eval(&args[0], c)?;
                    return Ok(Val::Bool(self.to_bool(&v)));
                }
                "not" => {
                    self.nargs(args, 1, 1)?;
                    let v = self.eval(&args[0], c)?;
                    return Ok(Val::Bool(!self.to_bool(&v)));
                }
                "true" => return Ok(Val::Bool(true)),
                "false" => return Ok(Val::Bool(false)),
                "lang" => {
                    self.nargs(args, 1, 1)?;
                    let want = self.arg_str(args, 0, c)?.to_ascii_lowercase();
                    let d = self.d();
                    let mut x = c.node;
                    while x != NONE {
                        if d.t(x) == T::Element {
                            let a = d.attr_node_ns(x, "lang", Some(XML_NS));
                            let a = if a == NONE { d.attr_node(x, "xml:lang") } else { a };
                            if a != NONE {
                                let l = d.n(a).content.to_ascii_lowercase();
                                return Ok(Val::Bool(l == want || l.starts_with(&format!("{want}-"))));
                            }
                        }
                        x = d.n(x).parent;
                    }
                    return Ok(Val::Bool(false));
                }
                "number" => {
                    self.nargs(args, 0, 1)?;
                    return Ok(Val::Num(self.arg_num(args, 0, c)?));
                }
                "sum" => {
                    self.nargs(args, 1, 1)?;
                    let ns = self.arg_nodes(args, 0, c)?;
                    return Ok(Val::Num(ns.iter().map(|&n| str_to_num(&self.string_value(n))).sum()));
                }
                "floor" => {
                    self.nargs(args, 1, 1)?;
                    return Ok(Val::Num(self.arg_num(args, 0, c)?.floor()));
                }
                "ceiling" => {
                    self.nargs(args, 1, 1)?;
                    return Ok(Val::Num(self.arg_num(args, 0, c)?.ceil()));
                }
                "round" => {
                    self.nargs(args, 1, 1)?;
                    return Ok(Val::Num(xpath_round(self.arg_num(args, 0, c)?)));
                }
                _ => {}
            }
        }
        // a custom function: the handler gets the context node set first for nokogiri: calls
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a, c)?);
        }
        match self.host.as_mut() {
            Some(h) => h.call(name, vals),
            None => Err(format!("Unregistered function: {}{name}", prefix.map(|p| format!("{p}:")).unwrap_or_default())),
        }
    }
}
