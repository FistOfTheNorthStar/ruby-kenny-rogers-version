//! Enumerator::Lazy — chained operations evaluated one element at a time, so infinite
//! sources like `(1..).lazy.map { }.select { }.first(3)` work.

use crate::core::*;
use crate::gc::RVec;
use crate::tri;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Map,
    Select,
    Reject,
    FilterMap,
    TakeWhile,
    DropWhile,
    Take,
    Drop,
    FlatMap,
    WithIndex,
    Compact,
    Uniq,
}

fn op_code(o: Op) -> i64 {
    o as i64
}
fn op_from(i: i64) -> Op {
    [Op::Map, Op::Select, Op::Reject, Op::FilterMap, Op::TakeWhile, Op::DropWhile, Op::Take, Op::Drop, Op::FlatMap, Op::WithIndex, Op::Compact, Op::Uniq][i as usize]
}

static LAZY: G<*mut Class> = G::new(std::ptr::null_mut());

fn lazy_new(src: V, ops: Vec<V>) -> V {
    let l = object_new(*LAZY.get());
    ivar_set(l, sym("@__src"), src);
    ivar_set(l, sym("@__ops"), ary_new(ops));
    l
}
fn chain(s: V, op: Op, arg: V) -> V {
    let mut ops = rary(ivar_get(s, sym("@__ops"))).v.clone();
    ops.push(ary_new(vec![mkfix(op_code(op)), arg]));
    lazy_new(ivar_get(s, sym("@__src")), ops)
}

struct State {
    ops: Vec<(Op, V)>,
    counters: Vec<i64>,
    flags: Vec<bool>,
    seen: Vec<indexmap::IndexSet<HKey>>,
    seen_roots: RVec,
    stop_after: bool,
}

const CONTINUE: V = QFALSE;
const STOP: V = QTRUE;

/// Push one element through ops[i..]; returns UNDEF (error), STOP or CONTINUE.
fn feed(st: &Rc<RefCell<State>>, i: usize, mut x: V, sink: V) -> V {
    let n = st.borrow().ops.len();
    let mut k = i;
    while k < n {
        let (op, f) = st.borrow().ops[k];
        match op {
            Op::Map => x = tri!(call_block(f, &[x])),
            Op::Select => {
                if !truthy(tri!(call_block(f, &[x]))) {
                    return CONTINUE;
                }
            }
            Op::Reject => {
                if truthy(tri!(call_block(f, &[x]))) {
                    return CONTINUE;
                }
            }
            Op::FilterMap => {
                x = tri!(call_block(f, &[x]));
                if !truthy(x) {
                    return CONTINUE;
                }
            }
            Op::TakeWhile => {
                if !truthy(tri!(call_block(f, &[x]))) {
                    return STOP;
                }
            }
            Op::DropWhile => {
                let dropping = !st.borrow().flags[k];
                if dropping {
                    if truthy(tri!(call_block(f, &[x]))) {
                        return CONTINUE;
                    }
                    st.borrow_mut().flags[k] = true;
                }
            }
            Op::Take => {
                let limit = fix(f);
                let c = st.borrow().counters[k];
                if c >= limit {
                    return STOP;
                }
                st.borrow_mut().counters[k] = c + 1;
                if c + 1 >= limit {
                    st.borrow_mut().stop_after = true;
                }
            }
            Op::Drop => {
                let c = st.borrow().counters[k];
                if c < fix(f) {
                    st.borrow_mut().counters[k] = c + 1;
                    return CONTINUE;
                }
            }
            Op::FlatMap => {
                let r = tri!(call_block(f, &[x]));
                if is_ary(r) {
                    let items = RVec::from(rary(r).v.clone());
                    for j in 0..items.len() {
                        let res = tri!(feed(st, k + 1, items[j], sink));
                        if res == STOP {
                            return STOP;
                        }
                    }
                    return CONTINUE;
                }
                x = r;
            }
            Op::WithIndex => {
                let c = st.borrow().counters[k];
                st.borrow_mut().counters[k] = c + 1;
                if is_proc(f) {
                    tri!(call_block(f, &[x, mkfix(c)]));
                } else {
                    x = ary_new(vec![x, mkfix(c)]);
                }
            }
            Op::Compact => {
                if x == QNIL {
                    return CONTINUE;
                }
            }
            Op::Uniq => {
                let key = if is_proc(f) { tri!(call_block(f, &[x])) } else { x };
                let mut s = st.borrow_mut();
                if !s.seen[k].insert(HKey(key)) {
                    return CONTINUE;
                }
                s.seen_roots.push(key);
            }
        }
        k += 1;
    }
    tri!(call_block(sink, &[x]));
    if st.borrow().stop_after { STOP } else { CONTINUE }
}

fn lazy_each(s: V, sink: V) -> V {
    let ops: Vec<(Op, V)> = rary(ivar_get(s, sym("@__ops")))
        .v
        .iter()
        .map(|o| (op_from(fix(rary(*o).v[0])), rary(*o).v[1]))
        .collect();
    let n = ops.len();
    // take(0) never pulls anything
    if ops.iter().any(|(o, f)| *o == Op::Take && fix(*f) <= 0) {
        return s;
    }
    let st = Rc::new(RefCell::new(State {
        ops,
        counters: vec![0; n],
        flags: vec![false; n],
        seen: (0..n).map(|_| indexmap::IndexSet::new()).collect(),
        seen_roots: RVec::new(),
        stop_after: false,
    }));
    let src = ivar_get(s, sym("@__src"));
    let st2 = st.clone();
    let r = crate::misc::each_until(src, move |x| {
        let res = feed(&st2, 0, x, sink);
        if res == UNDEF {
            return Err(UNDEF);
        }
        Ok(res == STOP)
    });
    match r {
        Ok(()) => s,
        Err(e) => e,
    }
}

pub fn init() {
    let en = rt().c.enumerator;
    let l = new_class("Enumerator::Lazy", en);
    const_set(en, sym("Lazy"), l as V);
    *LAZY.get() = l;
    defn(rt().c.enumerable, "lazy", 0, 0, |s, _, _| lazy_new(s, vec![]));
    defn(l, "each", 0, 0, |s, _, b| if is_proc(b) { lazy_each(s, b) } else { s });
    macro_rules! op {
        ($name:expr, $op:expr) => {
            defn(l, $name, 0, 0, |s, _, b| {
                if !is_proc(b) {
                    return arg_error(concat!("tried to call lazy ", $name, " without a block"));
                }
                chain(s, $op, b)
            });
        };
    }
    op!("map", Op::Map);
    op!("collect", Op::Map);
    op!("select", Op::Select);
    op!("filter", Op::Select);
    op!("reject", Op::Reject);
    op!("filter_map", Op::FilterMap);
    op!("take_while", Op::TakeWhile);
    op!("drop_while", Op::DropWhile);
    op!("flat_map", Op::FlatMap);
    defn(l, "take", 1, 1, |s, a, _| chain(s, Op::Take, a[0]));
    defn(l, "drop", 1, 1, |s, a, _| chain(s, Op::Drop, a[0]));
    defn(l, "with_index", 0, 1, |s, _, b| chain(s, Op::WithIndex, b));
    defn(l, "each_with_index", 0, 0, |s, _, b| chain(s, Op::WithIndex, b));
    defn(l, "compact", 0, 0, |s, _, _| chain(s, Op::Compact, QNIL));
    defn(l, "uniq", 0, 0, |s, _, b| chain(s, Op::Uniq, b));
    defn(l, "lazy", 0, 0, |s, _, _| s);
    defn(l, "force", 0, 1, |s, _, _| match crate::misc::each_to_vec(s) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(l, "to_a", 0, 0, |s, _, _| match crate::misc::each_to_vec(s) {
        Ok(v) => ary_new(v),
        Err(e) => e,
    });
    defn(l, "eager", 0, 0, |s, _, _| crate::misc::enum_for(s, "each", &[]));
    defn(l, "inspect", 0, 0, |s, _, _| {
        let src = ivar_get(s, sym("@__src"));
        crate::kernel::new_str_v(format!("#<Enumerator::Lazy: {}>", crate::fmt::inspect_lossy(src)))
    });
}
