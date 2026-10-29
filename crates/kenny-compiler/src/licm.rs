//! Attribute reads remembered across a loop's iterations: in
//!
//! ```ruby
//! while j < m.cols
//!   s += m.data[i * m.cols + j]
//! ```
//!
//! `m.data` and `m.cols` are read through the attr fast path once, then reused while
//! nothing could have changed them.
//!
//! - **Which reads.** `x.m` with no arguments or block, where `x` is a local that nothing in
//!   the loop assigns (a block in it, inlined or not, included); `@x` (of `self`) that
//!   nothing in the loop assigns.
//! - **Array facts.** For `x[i]` / `x[i] = v` / `x[i] op= v` with such an `x`, no value but
//!   the facts the inline index paths check (fastpath.rs `ary_guard`: a plain Array whose
//!   `[]` (and for `[]=` entries `[]=`) is the builtin, and for writes not frozen): entries
//!   `[]` and `[]=`. The values `local_get` reads from `x` are noted (`licm_vals`): an index
//!   path on one of those values skips the checks while the flag is set. The same for the
//!   value of a remembered read (`x.m[i]`, `@a[i]`: entries for `x.m` / `@a`): while no call
//!   or write has cleared the flags, the read gives the same object each time.
//! - **Valid flag.** Each such read has a value variable and a flag. Before the outermost loop
//!   that contains it, the flag is cleared. The attr fast path (`attr_fast`: an attr_reader
//!   of the object's class; for `@x`, `ivar_fast` / `civar_fast`) records the value and sets
//!   the flag.
//! - **What clears it.** Anything that could run Ruby code or change an instance variable:
//!   - every call the compiled code makes (runtime functions, compiled methods, stubs),
//!     except a few that can't;
//!   - every inline instance variable write;
//!   - the GC safepoint.
//!   A path where the flag was never set (the fast copy of a speculated loop entering the
//!   generic one) reads it as 0: Cranelift gives an undefined variable zero.
//! - **Threads.** The flag is only set while one OS thread runs Ruby code
//!   (`KN_DIRECT.call_limit` isn't `usize::MAX`): another thread could change the object
//!   at any time.
//!
//! - **Inlined receivers.** For `x.m(...)` with such an `x`, the instance variables that the
//!   bodies of the inlinable methods named `m` read (and don't assign): entries `x`, `@iv`.
//!   When a method is inlined with a receiver read from `x` (`licm_note`), its `@iv` reads
//!   go through them (`inl_self`).
//!
//! An `x.m` whose method isn't an attr_reader never sets the flag, so it is called each
//! time, as before.

use crate::fastpath::{attr_name, may_be_ruby_method};
use crate::*;
use cranelift_codegen::ir::{condcodes::IntCC, types::I64, InstBuilder};
use ruby_prism::*;
use std::collections::HashSet;

/// One remembered read, `local.meth`, or `self`'s instance variable `meth` (`local` empty).
#[derive(Clone)]
pub struct Entry {
    pub local: Vec<u8>,
    pub meth: Vec<u8>,
    pub val: Variable,
    pub valid: Variable,
    /// `inline_ctx` depth of the method whose local `local` is
    pub level: usize,
}

/// `KN_DIRECT.call_limit`
const KD_CALL_LIMIT: i32 = 8;

/// The `x.m` reads of a loop (predicate and body) worth remembering, and the `x.m(...)`
/// calls (receiver, method) whose inlined bodies' instance variable reads may be.
fn scan(pred: &Node, body: Option<&StatementsNode>) -> (Vec<(Vec<u8>, Vec<u8>)>, Vec<(Vec<u8>, Vec<u8>)>) {
    struct S {
        reads: Vec<(Vec<u8>, Vec<u8>)>,
        calls: Vec<(Vec<u8>, Vec<u8>)>,
        written: HashSet<Vec<u8>>,
        ivars_written: HashSet<Vec<u8>>,
        opaque: bool,
        nest: u32,
    }
    impl<'pr> Visit<'pr> for S {
        fn visit_call_node(&mut self, node: &CallNode<'pr>) {
            let name = node.name().as_slice();
            if matches!(name, b"binding" | b"eval" | b"local_variable_set" | b"instance_eval" | b"instance_exec" | b"class_eval" | b"module_eval") {
                self.opaque = true;
            }
            if matches!(name, b"[]" | b"[]=") && node.block().is_none() && !node.is_safe_navigation() {
                self.index(node.receiver(), name == b"[]=");
            }
            if self.nest == 0 && !node.is_safe_navigation() && may_be_ruby_method(name) {
                if let Some(l) = node.receiver().and_then(|r| r.as_local_variable_read_node()) {
                    let k = (l.name().as_slice().to_vec(), name.to_vec());
                    if l.depth() == 0 && !self.calls.contains(&k) {
                        self.calls.push(k);
                    }
                }
            }
            if self.nest == 0 && node.arguments().is_none() && node.block().is_none() && !node.is_safe_navigation() && attr_name(name) && may_be_ruby_method(name) {
                if let Some(l) = node.receiver().and_then(|r| r.as_local_variable_read_node()) {
                    if l.depth() == 0 {
                        let k = (l.name().as_slice().to_vec(), name.to_vec());
                        if !self.reads.contains(&k) {
                            self.reads.push(k);
                        }
                        return;
                    }
                }
            }
            visit_call_node(self, node);
        }
        // any assignment of the name, at any depth (a block's own local of that name, a
        // parameter, a rescue / for / pattern target...): not remembered
        fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_local_variable_write_node(self, node);
        }
        fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
        }
        fn visit_local_variable_operator_write_node(&mut self, node: &LocalVariableOperatorWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_local_variable_operator_write_node(self, node);
        }
        fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_local_variable_or_write_node(self, node);
        }
        fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_local_variable_and_write_node(self, node);
        }
        fn visit_required_parameter_node(&mut self, node: &RequiredParameterNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
        }
        fn visit_optional_parameter_node(&mut self, node: &OptionalParameterNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_optional_parameter_node(self, node);
        }
        fn visit_block_local_variable_node(&mut self, node: &BlockLocalVariableNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
        }
        fn visit_index_operator_write_node(&mut self, node: &IndexOperatorWriteNode<'pr>) {
            self.index(node.receiver(), true);
            visit_index_operator_write_node(self, node);
        }
        fn visit_index_or_write_node(&mut self, node: &IndexOrWriteNode<'pr>) {
            self.index(node.receiver(), true);
            visit_index_or_write_node(self, node);
        }
        fn visit_index_and_write_node(&mut self, node: &IndexAndWriteNode<'pr>) {
            self.index(node.receiver(), true);
            visit_index_and_write_node(self, node);
        }
        fn visit_instance_variable_read_node(&mut self, node: &InstanceVariableReadNode<'pr>) {
            let k = (Vec::new(), node.name().as_slice().to_vec());
            if self.nest == 0 && !self.reads.contains(&k) {
                self.reads.push(k);
            }
        }
        fn visit_instance_variable_write_node(&mut self, node: &InstanceVariableWriteNode<'pr>) {
            self.ivars_written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_write_node(self, node);
        }
        fn visit_instance_variable_target_node(&mut self, node: &InstanceVariableTargetNode<'pr>) {
            self.ivars_written.insert(node.name().as_slice().to_vec());
        }
        fn visit_instance_variable_operator_write_node(&mut self, node: &InstanceVariableOperatorWriteNode<'pr>) {
            self.ivars_written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_operator_write_node(self, node);
        }
        fn visit_instance_variable_or_write_node(&mut self, node: &InstanceVariableOrWriteNode<'pr>) {
            self.ivars_written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_or_write_node(self, node);
        }
        fn visit_instance_variable_and_write_node(&mut self, node: &InstanceVariableAndWriteNode<'pr>) {
            self.ivars_written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_and_write_node(self, node);
        }
        fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
            self.nest += 1;
            visit_block_node(self, node);
            self.nest -= 1;
        }
        fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
            self.nest += 1;
            visit_lambda_node(self, node);
            self.nest -= 1;
        }
        fn visit_def_node(&mut self, _node: &DefNode<'pr>) {}
        fn visit_class_node(&mut self, _node: &ClassNode<'pr>) {}
        fn visit_module_node(&mut self, _node: &ModuleNode<'pr>) {}
        fn visit_singleton_class_node(&mut self, _node: &SingletonClassNode<'pr>) {}
    }
    impl S {
        fn index(&mut self, recv: Option<Node>, write: bool) {
            if self.nest != 0 {
                return;
            }
            let Some(r) = recv else { return };
            // the array: a local, `x.m` (x a local), or `@a`
            let key = if let Some(l) = r.as_local_variable_read_node() {
                if l.depth() != 0 {
                    return;
                }
                l.name().as_slice().to_vec()
            } else if let Some(iv) = r.as_instance_variable_read_node() {
                iv.name().as_slice().to_vec()
            } else if let Some(c) = r.as_call_node() {
                let name = c.name().as_slice();
                let Some(l) = c.receiver().and_then(|x| x.as_local_variable_read_node()) else { return };
                if l.depth() != 0 || c.arguments().is_some() || c.block().is_some() || c.is_safe_navigation() || !attr_name(name) || !may_be_ruby_method(name) {
                    return;
                }
                [l.name().as_slice(), b".", name].concat()
            } else {
                return;
            };
            let mut ks = vec![(key.clone(), b"[]".to_vec())];
            if write {
                ks.push((key, b"[]=".to_vec()));
            }
            for k in ks {
                if !self.reads.contains(&k) {
                    self.reads.push(k);
                }
            }
        }
    }
    let mut s = S { reads: Vec::new(), calls: Vec::new(), written: HashSet::new(), ivars_written: HashSet::new(), opaque: false, nest: 0 };
    s.visit(pred);
    if let Some(b) = body {
        s.visit_statements_node(b);
    }
    if s.opaque {
        return (Vec::new(), Vec::new());
    }
    let (written, ivars_written) = (s.written, s.ivars_written);
    let reads = s
        .reads
        .into_iter()
        .filter(|(l, m)| match l.first() {
            None => !ivars_written.contains(m),
            Some(b'@') => !ivars_written.contains(l),
            _ => !written.contains(base_local(l)),
        })
        .collect();
    let calls = s.calls.into_iter().filter(|(l, _)| !written.contains(l)).collect();
    (reads, calls)
}

/// The instance variables a method body reads and doesn't assign.
fn ivars_read(body: &Node) -> Vec<Vec<u8>> {
    #[derive(Default)]
    struct V {
        read: Vec<Vec<u8>>,
        written: HashSet<Vec<u8>>,
    }
    impl<'pr> Visit<'pr> for V {
        fn visit_instance_variable_read_node(&mut self, node: &InstanceVariableReadNode<'pr>) {
            let n = node.name().as_slice().to_vec();
            if !self.read.contains(&n) {
                self.read.push(n);
            }
        }
        fn visit_instance_variable_write_node(&mut self, node: &InstanceVariableWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_write_node(self, node);
        }
        fn visit_instance_variable_target_node(&mut self, node: &InstanceVariableTargetNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
        }
        fn visit_instance_variable_operator_write_node(&mut self, node: &InstanceVariableOperatorWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_operator_write_node(self, node);
        }
        fn visit_instance_variable_or_write_node(&mut self, node: &InstanceVariableOrWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_or_write_node(self, node);
        }
        fn visit_instance_variable_and_write_node(&mut self, node: &InstanceVariableAndWriteNode<'pr>) {
            self.written.insert(node.name().as_slice().to_vec());
            visit_instance_variable_and_write_node(self, node);
        }
    }
    let mut v = V::default();
    v.visit(body);
    let w = v.written;
    v.read.into_iter().filter(|n| !w.contains(n)).collect()
}

/// The local of an entry's key (`x` of `x.m`).
fn base_local(key: &[u8]) -> &[u8] {
    key.split(|&c| c == b'.').next().unwrap_or(key)
}

impl<M: Module> Compiler<M> {
    /// Before a loop: the reads to remember that an enclosing loop doesn't already, flags
    /// cleared. Returns how many entries to drop after the loop.
    pub(crate) fn licm_enter(&mut self, fx: &mut Fx, pred: &Node, body: Option<&StatementsNode>) -> usize {
        if self.refine_scope.is_some() || self.cov.is_some() || std::env::var_os("KENNY_NO_LICM").is_some() {
            return 0;
        }
        let level = self.inline_ctx.len();
        let mut added = 0;
        let (mut reads, calls) = scan(pred, body);
        // the instance variables of inlined receivers
        for (local, meth) in calls {
            let mut ivs: Vec<Vec<u8>> = Vec::new();
            for d in self.inline_defs.values() {
                if d.name == meth {
                    if let Some(b) = d.body() {
                        for iv in ivars_read(&b) {
                            if !ivs.contains(&iv) {
                                ivs.push(iv);
                            }
                        }
                    }
                }
            }
            for iv in ivs {
                let k = (local.clone(), iv);
                if !reads.contains(&k) {
                    reads.push(k);
                }
            }
        }
        for (local, meth) in reads {
            // a local of this method (not a block's); `self` of an ordinary method
            let bad = match local.first() {
                None | Some(b'@') => fx.vself.is_some(),
                _ => !matches!(fx.locals.get(base_local(&local)), Some(Local::Var(_))),
            };
            if bad {
                continue;
            }
            if fx.licm.iter().any(|e| e.level == level && e.local == local && e.meth == meth) {
                continue;
            }
            if std::env::var_os("KENNY_DEBUG_LICM").is_some() {
                let dot = if local.is_empty() { "" } else { "." };
                eprintln!("kenny: loop-invariant {}{dot}{}", String::from_utf8_lossy(&local), String::from_utf8_lossy(&meth));
            }
            let val = fx.b.declare_var(I64);
            let valid = fx.b.declare_var(I64);
            let z = fx.b.ins().iconst(I64, 0);
            fx.b.def_var(val, z);
            fx.b.def_var(valid, z);
            fx.licm.push(Entry { local, meth, val, valid, level });
            added += 1;
        }
        added
    }
    pub(crate) fn licm_leave(&mut self, fx: &mut Fx, added: usize) {
        let n = fx.licm.len() - added;
        fx.licm.truncate(n);
        let live = &fx.licm;
        fx.licm_vals.retain(|(_, e)| live.iter().any(|x| x.valid == e.valid));
    }
    /// `local_get` read `v` from the local `name`: note it if it is a remembered Array, or a
    /// receiver whose instance variables are remembered.
    pub(crate) fn licm_note(&mut self, fx: &mut Fx, name: &[u8], v: Value) {
        let level = self.inline_ctx.len();
        for e in &fx.licm {
            if e.level == level && e.local == name && (matches!(e.meth.as_slice(), b"[]" | b"[]=") || e.meth.first() == Some(&b'@')) {
                fx.licm_vals.push((v, e.clone()));
            }
        }
    }
    /// Inlining a method with receiver `recv`: the remembered instance variables of it.
    pub(crate) fn licm_inl_self(&self, fx: &Fx, recv: Value) -> Vec<Entry> {
        fx.licm_vals.iter().filter(|(v, e)| *v == recv && e.meth.first() == Some(&b'@')).map(|(_, e)| e.clone()).collect()
    }
    /// The Array-facts entry for `recv`, if it was read from a remembered Array local.
    pub(crate) fn licm_ary(&mut self, fx: &mut Fx, recv: Value, write: bool) -> Option<Entry> {
        let meth: &[u8] = if write { b"[]=" } else { b"[]" };
        fx.licm_vals.iter().rev().find(|(v, e)| *v == recv && e.meth == meth).map(|(_, e)| e.clone())
    }

    /// Something may have run Ruby code or written an instance variable: forget every
    /// remembered value.
    pub(crate) fn licm_kill(&mut self, fx: &mut Fx) {
        if fx.licm.is_empty() {
            return;
        }
        let z = fx.b.ins().iconst(I64, 0);
        // (the value too: nothing old stays live across the call)
        for e in &fx.licm {
            fx.b.def_var(e.valid, z);
            fx.b.def_var(e.val, z);
        }
    }

    /// `x.m` (no arguments, block or `&.`): the remembered value, else the call (whose
    /// attr fast path records it).
    pub(crate) fn licm_read(&mut self, fx: &mut Fx, c: &CallNode) -> Option<Value> {
        let l = c.receiver()?.as_local_variable_read_node()?;
        if l.depth() != fx.inline_levels.len() as u32 {
            return None;
        }
        let e = self.licm_find(fx, l.name().as_slice(), c.name().as_slice())?;
        Some(self.licm_use(fx, e, |s, fx| s.call_node(fx, c)))
    }
    /// `@x` of `self`: the remembered value, else `read` (whose fast path records it).
    pub(crate) fn licm_ivar(&mut self, fx: &mut Fx, name: &[u8], read: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Option<Value> {
        let e = match self.licm_find(fx, b"", name) {
            Some(e) => e,
            // an inlined method's receiver (`inl_self`)
            None if fx.licm_pending.is_none() => fx.inl_self.iter().find(|e| e.meth == name)?.clone(),
            None => return None,
        };
        Some(self.licm_use(fx, e, read))
    }
    fn licm_find(&mut self, fx: &mut Fx, local: &[u8], meth: &[u8]) -> Option<Entry> {
        if fx.licm_pending.is_some() {
            return None;
        }
        let level = self.inline_ctx.len();
        fx.licm.iter().find(|e| e.level == level && e.local == local && e.meth == meth).cloned()
    }
    fn licm_use(&mut self, fx: &mut Fx, e: Entry, compute: impl FnOnce(&mut Self, &mut Fx) -> Value) -> Value {
        let hit = fx.b.create_block();
        let miss = fx.b.create_block();
        let done = fx.b.create_block();
        fx.b.append_block_param(done, I64);
        let valid = fx.b.use_var(e.valid);
        fx.b.ins().brif(valid, hit, &[], miss, &[]);
        fx.b.switch_to_block(hit);
        let v = fx.b.use_var(e.val);
        fx.b.ins().jump(done, &[v.into()]);
        fx.b.switch_to_block(miss);
        let key = if e.local.is_empty() { e.meth.clone() } else { [e.local.as_slice(), b".", e.meth.as_slice()].concat() };
        fx.licm_pending = Some(e);
        let r = compute(self, fx);
        fx.licm_pending = None;
        fx.b.ins().jump(done, &[r.into()]);
        fx.b.switch_to_block(done);
        let r = fx.b.block_params(done)[0];
        fx.checked.insert(r);
        // (Array facts about the value: licm_ary)
        self.licm_note(fx, &key, r);
        r
    }

    /// In the attr fast path of a read, once it has the value `v`: if it is the read being
    /// remembered (same inline depth), record it.
    pub(crate) fn licm_record(&mut self, fx: &mut Fx, e: &Entry, v: Value) {
        let gv = self.module.declare_data_in_func(self.direct_id, fx.b.func);
        let dir = fx.b.ins().symbol_value(I64, gv);
        let lim = fx.b.ins().load(I64, mem(), dir, KD_CALL_LIMIT);
        let one = fx.b.ins().icmp_imm_s(IntCC::NotEqual, lim, -1);
        let one = fx.b.ins().uextend(I64, one);
        fx.b.def_var(e.val, v);
        fx.b.def_var(e.valid, one);
    }
    /// The pending remembered read, if this fast path (`attr_fast`, or for `ivar` an
    /// instance variable's) is now compiling it.
    pub(crate) fn licm_take(&mut self, fx: &mut Fx, ivar: bool) -> Option<Entry> {
        // (an inlined receiver's: taken by the instance variable read it is pending for)
        let inl = |e: &Entry| !e.local.is_empty() && e.meth.first() == Some(&b'@');
        if fx.licm_pending.as_ref().is_some_and(|e| if inl(e) { ivar } else { e.level == self.inline_ctx.len() && e.local.is_empty() == ivar }) {
            fx.licm_pending.take()
        } else {
            None
        }
    }
}
