//! Folded constants: `H = 40` (or `SIZE = H * W`, `EPS = 0.5`) at the top level of a file,
//! assigned nowhere else in it, read in that file's top-level code and top-level `def`s and
//! blocks as the literal itself.
//!
//! A word per constant says the value holds: the assignment sets it (kenny-runtime abi.rs
//! `kn_const_fold`, after checking Object really got that value) and the runtime clears it
//! when a constant of that name is set again or removed anywhere. A read checks the word and
//! takes the normal lookup while it is 0 (before the assignment ran, or after a change).
//!
//! Only values that are immediates (Fixnums, flonums) are folded: nothing for the GC to see
//! or move.

use crate::*;
use ruby_prism::*;

/// A folded constant of one file.
pub(crate) struct CFold {
    pub word: DataId,
    /// the tagged value
    pub val: i64,
    /// the assignment (its node's offset)
    pub at: usize,
}

/// Counts assignments by constant name.
struct Writes(HashMap<Vec<u8>, usize>);
impl Writes {
    fn add(&mut self, n: &[u8]) {
        *self.0.entry(n.to_vec()).or_insert(0) += 1;
    }
}
impl<'pr> Visit<'pr> for Writes {
    fn visit_constant_write_node(&mut self, node: &ConstantWriteNode<'pr>) {
        self.add(node.name().as_slice());
        visit_constant_write_node(self, node);
    }
    fn visit_constant_or_write_node(&mut self, node: &ConstantOrWriteNode<'pr>) {
        self.add(node.name().as_slice());
        visit_constant_or_write_node(self, node);
    }
    fn visit_constant_and_write_node(&mut self, node: &ConstantAndWriteNode<'pr>) {
        self.add(node.name().as_slice());
        visit_constant_and_write_node(self, node);
    }
    fn visit_constant_operator_write_node(&mut self, node: &ConstantOperatorWriteNode<'pr>) {
        self.add(node.name().as_slice());
        visit_constant_operator_write_node(self, node);
    }
    fn visit_constant_target_node(&mut self, node: &ConstantTargetNode<'pr>) {
        self.add(node.name().as_slice());
    }
}

/// The tagged immediate `n` evaluates to, given the constants folded so far.
fn fold_value(n: &Node, known: &HashMap<Vec<u8>, i64>) -> Option<i64> {
    const FIX_MAX: i64 = (1 << 62) - 1;
    const FIX_MIN: i64 = -(1 << 62);
    let int = |n: &Node| -> Option<i64> {
        let v = fold_value(n, known)?;
        (v & 1 == 1).then_some(v >> 1)
    };
    let tag = |i: i64| -> Option<i64> { (FIX_MIN..=FIX_MAX).contains(&i).then_some((i << 1) | 1) };
    if let Some(i) = n.as_integer_node() {
        return tag(crate::expr::int_value(&i)?);
    }
    if let Some(f) = n.as_float_node() {
        return crate::fastpath::f64_to_flonum(f.value());
    }
    if let Some(c) = n.as_constant_read_node() {
        return known.get(c.name().as_slice()).copied();
    }
    if let Some(p) = n.as_parentheses_node() {
        let b = p.body()?;
        let s = b.as_statements_node()?;
        let v: Vec<Node> = s.body().iter().collect();
        return match v.as_slice() {
            [x] => fold_value(x, known),
            _ => None,
        };
    }
    let c = n.as_call_node()?;
    if c.block().is_some() || c.is_safe_navigation() {
        return None;
    }
    let args: Vec<Node> = c.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
    let [b] = args.as_slice() else { return None };
    let a = int(&c.receiver()?)?;
    let b = int(b)?;
    tag(match c.name().as_slice() {
        b"+" => a.checked_add(b)?,
        b"-" => a.checked_sub(b)?,
        b"*" => a.checked_mul(b)?,
        _ => return None,
    })
}

impl<M: Module> Compiler<M> {
    /// Find the foldable constants of a file's top level (`slot`: its cref slot).
    pub(crate) fn cfold_scan(&mut self, slot: usize, prog: &ProgramNode) {
        if self.eval_unit || std::env::var_os("KENNY_NO_CFOLD").is_some() {
            return;
        }
        let mut w = Writes(HashMap::new());
        w.visit(&prog.as_node());
        let mut known = HashMap::new();
        for st in prog.statements().body().iter() {
            let Some(cw) = st.as_constant_write_node() else { continue };
            let name = cw.name().as_slice().to_vec();
            if w.0.get(&name) != Some(&1) {
                continue;
            }
            let Some(val) = fold_value(&cw.value(), &known) else { continue };
            known.insert(name.clone(), val);
            let label: String = String::from_utf8_lossy(&name).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            let word = self.module.declare_data(&format!("__kn_cfold_{slot}_{label}"), Linkage::Local, true, false).expect("cfold word");
            let mut d = DataDescription::new();
            d.define(Box::new(0u64.to_le_bytes()));
            d.set_align(8);
            self.module.define_data(word, &d).expect("cfold word");
            self.cfold.insert((slot, name), CFold { word, val, at: cw.location().start_offset() });
        }
    }

    /// After the folded constant's assignment (`v` the value assigned).
    pub(crate) fn cfold_assigned(&mut self, fx: &mut Fx, name: &[u8], at: usize, sym: Value, v: Value) {
        let Some(cf) = self.cfold.get(&(fx.cref_slot, name.to_vec())) else { return };
        if cf.at != at {
            return;
        }
        let (word, val) = (cf.word, cf.val);
        let g = self.module.declare_data_in_func(word, fx.b.func);
        let wp = fx.b.ins().symbol_value(I64, g);
        let e = self.iconst(fx, val);
        self.call(fx, "kn_const_fold", &[wp, sym, e, v]);
    }

    /// A read of a folded constant: the literal while its word says so, else `slow`.
    pub(crate) fn cfold_read(&mut self, fx: &mut Fx, name: &[u8], slow: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Option<Value> {
        let cf = self.cfold.get(&(fx.cref_slot, name.to_vec()))?;
        let (word, val) = (cf.word, cf.val);
        let g = self.module.declare_data_in_func(word, fx.b.func);
        let wp = fx.b.ins().symbol_value(I64, g);
        let ok = fx.b.ins().load(I64, mem(), wp, 0);
        let fast = fx.b.create_block();
        let slow_b = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        fx.b.set_cold_block(slow_b);
        fx.b.ins().brif(ok, fast, &[], slow_b, &[]);
        fx.b.switch_to_block(slow_b);
        let r = slow(self, fx);
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(fast);
        let k = self.iconst(fx, val);
        fx.b.ins().jump(done, &[k.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        Some(r)
    }
}
