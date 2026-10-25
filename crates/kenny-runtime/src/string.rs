//! String and Symbol. Strings are UTF-8 or binary byte vectors (see docs/LANGUAGE.md).

use crate::core::*;
use crate::fmt::*;
use crate::kernel::{new_str_v, sym_arg};
use crate::tri;
use std::collections::HashMap;

fn b(v: V) -> &'static mut SVec<u8> {
    &mut rstr(v).b
}
fn s_of(v: V) -> String {
    String::from_utf8_lossy(&rstr(v).b).into_owned()
}
/// A String argument's bytes, converted with `to_str` (Err: the exception raised).
fn want_str(v: V) -> Result<Vec<u8>, V> {
    if is_str(v) {
        return Ok(rstr(v).b.clone());
    }
    if is_heap(v) && respond_to(v, sym("to_str")) {
        let r = send0(v, "to_str");
        if r == UNDEF {
            return Err(r);
        }
        if is_str(r) {
            return Ok(rstr(r).b.clone());
        }
        let c = nil_name(v);
        return Err(type_error(&format!("can't convert {c} to String ({c}#to_str gives {})", nil_name(r))));
    }
    Err(type_error(&format!("no implicit conversion of {} into String", nil_name(v))))
}
fn nil_name(v: V) -> String {
    if v == QNIL { "nil".into() } else { class_name(real_class(v)) }
}
fn chars(v: V) -> Vec<String> {
    s_of(v).chars().map(|c| c.to_string()).collect()
}
/// Characters in UTF-8 bytes (invalid bytes count one each, as `from_utf8_lossy` gives).
fn char_count(b: &[u8]) -> usize {
    if b.is_ascii() { b.len() } else { String::from_utf8_lossy(b).chars().count() }
}
/// The byte offset of character `n` (the end when there are fewer).
fn char_byte(b: &[u8], n: usize) -> usize {
    if b.is_ascii() {
        return n.min(b.len());
    }
    let mut i = 0;
    for _ in 0..n {
        if i >= b.len() {
            break;
        }
        let len = match b[i] {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        i = (i + len).min(b.len());
    }
    i
}
/// An Integer position argument.
fn int_arg(v: V) -> Result<i64, V> {
    if is_fix(v) {
        return Ok(fix(v));
    }
    Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(v)))))
}
pub(crate) fn char_len(v: V) -> usize {
    let bytes = &rstr(v).b;
    if ascii_only(v) || is_binary(v) { bytes.len() } else { String::from_utf8_lossy(bytes).chars().count() }
}
/// The bytes are all ASCII (remembered for a frozen String: `FL_STR_ASCII`).
fn ascii_only(v: V) -> bool {
    let h = hdr(v);
    if h.flags & FL_STR_ASCII != 0 {
        return true;
    }
    let a = rstr(v).b.is_ascii();
    if a && h.flags & FL_FROZEN != 0 {
        h.flags |= FL_STR_ASCII;
    }
    a
}
/// ASCII-8BIT strings (`str.b`, digests, ciphertext) index by byte.
fn is_binary(v: V) -> bool {
    hdr(v).flags & crate::compat::FL_BINARY != 0
}
fn mutate(s: V) -> Result<&'static mut SVec<u8>, V> {
    if is_frozen(s) {
        let e = check_frozen(s);
        return Err(e);
    }
    Ok(b(s))
}

/// String#+ of `s` and the bytes `o` of `arg`.
fn str_plus(s: V, o: &[u8], arg: V) -> V {
    // the result's encoding: CRuby's rules for mixing two (see transcode::concat_enc)
    let enc = if is_str(arg) && crate::transcode::same_native(s, arg) {
        None
    } else {
        let xe = if is_str(arg) { crate::transcode::enc_of(arg) } else { crate::transcode::UTF8 };
        match crate::transcode::concat_enc(s, o, xe) {
            Ok(e) => e,
            Err(e) => return e,
        }
    };
    let mut v = Vec::with_capacity(rstr(s).b.len() + o.len());
    v.extend_from_slice(&rstr(s).b);
    v.extend_from_slice(o);
    let r = str_new(v);
    crate::transcode::set_enc(r, enc.unwrap_or_else(|| crate::transcode::enc_of(s)));
    r
}

/// `s = s + e` in compiled code, where the local `s` is only ever built this way
/// (kenny-compiler owned.rs): when `own` (the local holds the only reference to a String
/// made by a literal or by this function) `s` is appended to in place, else this is the
/// builtin String#+. UNDEF when String#+ is not the builtin one or an operand is not a
/// String: the caller then calls `+`.
#[unsafe(no_mangle)]
pub extern "C" fn kn_str_append_own(s: V, e: V, own: u64) -> V {
    if !is_str(s) || !is_str(e) || KN_DIRECT.flags.load(std::sync::atomic::Ordering::Relaxed) & DIRECT_STR_PLUS == 0 {
        return UNDEF;
    }
    if own != 0 && s != e && hdr(s).klass == rt().c.string && !is_frozen(s) {
        let add: &[u8] = &rstr(e).b;
        if !crate::transcode::same_native(s, e) {
            match crate::transcode::concat_enc(s, add, crate::transcode::enc_of(e)) {
                Ok(Some(ne)) => crate::transcode::set_enc(s, ne),
                Ok(None) => {}
                Err(x) => return x,
            }
        }
        // (no one else can see `s`: a replaced buffer can be freed now)
        b(s).extend_private(add);
        return s;
    }
    str_plus(s, &rstr(e).b, e)
}

/// s[start, len] in characters
fn substr(s: V, start: i64, len: i64) -> V {
    let n = char_len(s) as i64;
    let start = if start < 0 { start + n } else { start };
    if start < 0 || start > n || len < 0 {
        return QNIL;
    }
    let end = (start + len).min(n);
    let bytes = &rstr(s).b;
    if is_binary(s) {
        let r = str_new(bytes[start as usize..end as usize].to_vec());
        hdr(r).flags |= crate::compat::FL_BINARY;
        return r;
    }
    if ascii_only(s) {
        return str_new(bytes[start as usize..end as usize].to_vec());
    }
    let cs: String = String::from_utf8_lossy(bytes).chars().skip(start as usize).take((end - start) as usize).collect();
    new_str_v(cs)
}
pub fn range_bounds(r: V, len: i64) -> Option<(i64, i64)> {
    let rg = rrange(r);
    let lo = if rg.lo == QNIL { 0 } else if is_fix(rg.lo) { fix(rg.lo) } else { return None };
    let hi = if rg.hi == QNIL { -1 } else if is_fix(rg.hi) { fix(rg.hi) } else { return None };
    let lo = if lo < 0 { lo + len } else { lo };
    if lo < 0 || lo > len {
        return None;
    }
    let mut hi = if hi < 0 { hi + len } else { hi };
    if rg.excl && rg.hi != QNIL {
        hi -= 1;
    }
    let count = (hi - lo + 1).max(0);
    Some((lo, count))
}

/// `s[i]`, an Integer index.
pub(crate) fn aref_idx(s: V, i: i64) -> V {
    let r = substr(s, i, 1);
    if r != QNIL && rstr(r).b.is_empty() { QNIL } else { r }
}

/// `s[i] == lit` / `s[i] != lit` (`neq`) in compiled code (kenny-compiler fastpath.rs
/// `str_idx_eq`), `lit` the bytes of a String literal: without making `s[i]` or the literal.
/// UNDEF when `s` is not a plain String, `i` not a Fixnum or a method involved is not the
/// builtin one (`DIRECT_STR_IDX_EQ`): the caller then makes both and calls `==` / `!=`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_str_idx_eq(s: V, i: V, p: *const u8, n: u64, neq: u64) -> V {
    if !is_str(s) || !is_fix(i) || hdr(s).klass != rt().c.string || KN_DIRECT.flags.load(std::sync::atomic::Ordering::Relaxed) & DIRECT_STR_IDX_EQ == 0 {
        return UNDEF;
    }
    let lit = unsafe { std::slice::from_raw_parts(p, n as usize) };
    let eq = if ascii_only(s) || is_binary(s) {
        // one byte (nil when out of range: never equal)
        let b = &rstr(s).b;
        let len = b.len() as i64;
        let i = fix(i);
        let i = if i < 0 { i + len } else { i };
        i >= 0 && i < len && lit.len() == 1 && b[i as usize] == lit[0]
    } else {
        let c = aref_idx(s, fix(i));
        c != QNIL && rstr(c).b[..] == *lit
    };
    bool_v(eq != (neq != 0))
}

fn aref(s: V, a: &[V]) -> V {
    if crate::regex::is_regexp(a[0]) {
        return crate::regex::str_aref(s, a[0], a.get(1).copied());
    }
    if a.len() == 2 {
        return substr(s, fix(a[0]), fix(a[1]));
    }
    let i = a[0];
    if is_fix(i) {
        aref_idx(s, fix(i))
    } else if is_range(i) {
        match range_bounds(i, char_len(s) as i64) {
            Some((st, n)) => substr(s, st, n),
            None => QNIL,
        }
    } else if is_str(i) {
        if find(&rstr(s).b, &rstr(i).b, 0).is_some() { str_new(rstr(i).b.clone()) } else { QNIL }
    } else if is_float(i) {
        let f = float_val(i);
        if f.is_finite() {
            aref_idx(s, f.trunc() as i64)
        } else {
            raise(rt().c.float_domain_error, &float_to_s(f))
        }
    } else if is_heap(i) && respond_to(i, sym("to_int")) {
        let r = crate::tri!(send0(i, "to_int"));
        if is_fix(r) { aref_idx(s, fix(r)) } else { type_error("no implicit conversion into Integer") }
    } else {
        type_error(&format!("no implicit conversion of {} into Integer", nil_name(i)))
    }
}

pub fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
    if n.is_empty() {
        return if from <= h.len() { Some(from) } else { None };
    }
    if n.len() > h.len() || from > h.len() - n.len() {
        return None;
    }
    // candidates by the first byte, then the rest compared
    let last = h.len() - n.len();
    let mut i = from;
    while i <= last {
        let j = i + h[i..=last].iter().position(|&b| b == n[0])?;
        if h[j + 1..j + n.len()] == n[1..] {
            return Some(j);
        }
        i = j + 1;
    }
    None
}
fn rfind(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.len() > h.len() {
        return None;
    }
    (0..=h.len() - n.len()).rev().find(|&i| &h[i..i + n.len()] == n)
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | 0)
}
fn strip_l(x: &[u8]) -> &[u8] {
    let i = x.iter().position(|c| !is_space(*c)).unwrap_or(x.len());
    &x[i..]
}
fn strip_r(x: &[u8]) -> &[u8] {
    let i = x.iter().rposition(|c| !is_space(*c)).map(|i| i + 1).unwrap_or(0);
    &x[..i]
}

fn split(s: V, sep: V, limit: i64) -> V {
    // (no Ruby code runs here: s and sep stay as they are)
    let bytes: &[u8] = &rstr(s).b;
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let awk = sep == QNIL || (is_str(sep) && rstr(sep).b == b" ");
    if awk {
        let mut rest: &[u8] = strip_l(bytes);
        while !rest.is_empty() {
            if limit > 0 && parts.len() as i64 == limit - 1 {
                parts.push(rest.to_vec());
                break;
            }
            let end = rest.iter().position(|c| is_space(*c)).unwrap_or(rest.len());
            parts.push(rest[..end].to_vec());
            rest = strip_l(&rest[end..]);
        }
    } else if is_str(sep) && rstr(sep).b.is_empty() {
        for c in s_of(s).chars() {
            parts.push(c.to_string().into_bytes());
        }
    } else if is_str(sep) {
        let sp: &[u8] = &rstr(sep).b;
        let mut start = 0;
        while let Some(i) = find(bytes, sp, start) {
            if limit > 0 && parts.len() as i64 == limit - 1 {
                break;
            }
            parts.push(bytes[start..i].to_vec());
            start = i + sp.len();
        }
        parts.push(bytes[start..].to_vec());
    } else {
        return type_error("wrong argument type (expected String)");
    }
    if limit == 0 {
        while parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
    }
    ary_new(parts.into_iter().map(str_new).collect())
}

fn tr_expand(set: &str) -> (bool, Vec<char>) {
    let (neg, set) = match set.strip_prefix('^') {
        Some(r) if !r.is_empty() => (true, r),
        _ => (false, set),
    };
    let cs: Vec<char> = set.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        if i + 2 < cs.len() && cs[i + 1] == '-' {
            for c in cs[i]..=cs[i + 2] {
                out.push(c);
            }
            i += 3;
        } else {
            out.push(cs[i]);
            i += 1;
        }
    }
    (neg, out)
}
fn tr(s: &str, from: &str, to: &str) -> String {
    let (neg, f) = tr_expand(from);
    let (_, t) = tr_expand(to);
    s.chars()
        .filter_map(|c| {
            let pos = f.iter().position(|x| *x == c);
            match (neg, pos) {
                (false, Some(i)) => {
                    if t.is_empty() {
                        None
                    } else {
                        Some(*t.get(i).unwrap_or(t.last().unwrap()))
                    }
                }
                (true, None) => t.last().copied(),
                _ => Some(c),
            }
        })
        .collect()
}
fn in_sets(c: char, sets: &[(bool, Vec<char>)]) -> bool {
    sets.iter().all(|(neg, s)| s.contains(&c) != *neg)
}
fn sets_of(a: &[V]) -> Result<Vec<(bool, Vec<char>)>, V> {
    a.iter().map(|x| want_str(*x).map(|b| tr_expand(&String::from_utf8_lossy(&b)))).collect()
}

fn sub_impl(s: V, a: &[V], blk: V, global: bool) -> Result<Option<Vec<u8>>, V> {
    if a.is_empty() {
        return Err(arity_error(0, 1, 2));
    }
    if crate::regex::is_regexp(a[0]) {
        return crate::regex::sub(s, a[0], a.get(1).copied(), blk, global);
    }
    if !is_str(a[0]) {
        return Err(type_error("wrong argument type (expected Regexp)"));
    }
    let pat = rstr(a[0]).b.clone();
    let src = rstr(s).b.clone();
    let mut out = Vec::with_capacity(src.len());
    let mut start = 0;
    let mut changed = false;
    // a String replacement: converted once; `\\0`, `\\&`, `\\\\` etc. are expanded as with a Regexp
    let rep_str = match a.get(1) {
        Some(&r) if !is_hash(r) => Some(want_str(r)?),
        _ => None,
    };
    while let Some(i) = find(&src, &pat, start) {
        out.extend_from_slice(&src[start..i]);
        let rep: Vec<u8> = if let Some(rb) = &rep_str {
            if rb.contains(&b'\\') {
                crate::regex::expand(rb, &src, &[i as i64, (i + pat.len()) as i64], QNIL)
            } else {
                rb.clone()
            }
        } else if a.len() > 1 {
            let v = crate::hash::hget(a[1], str_new(pat.clone()));
            if v == UNDEF {
                return Err(UNDEF);
            }
            to_s(v).map_err(|_| UNDEF)?
        } else if is_proc(blk) {
            let matched = str_new(pat.clone());
            let v = call_block(blk, &[matched]);
            if v == UNDEF {
                return Err(UNDEF);
            }
            to_s(v).map_err(|_| UNDEF)?
        } else {
            return Err(arity_error(1, 2, 2));
        };
        out.extend_from_slice(&rep);
        changed = true;
        start = i + pat.len();
        if pat.is_empty() {
            if start < src.len() {
                out.push(src[start]);
            }
            start += 1;
        }
        if !global || start > src.len() {
            break;
        }
    }
    if start < src.len() {
        out.extend_from_slice(&src[start..]);
    }
    Ok(if changed { Some(out) } else { None })
}

fn succ(x: &[u8]) -> Vec<u8> {
    let mut v = x.to_vec();
    if v.is_empty() {
        return v;
    }
    let mut i = v.len();
    loop {
        if i == 0 {
            let first = v[0];
            let ins = match first {
                b'0'..=b'9' => b'1',
                b'a'..=b'z' => b'a',
                b'A'..=b'Z' => b'A',
                _ => 1,
            };
            v.insert(0, ins);
            return v;
        }
        i -= 1;
        match v[i] {
            b'z' => v[i] = b'a',
            b'Z' => v[i] = b'A',
            b'9' => v[i] = b'0',
            c if c.is_ascii_alphanumeric() => {
                v[i] = c + 1;
                return v;
            }
            c => {
                if v.iter().any(|c| c.is_ascii_alphanumeric()) {
                    continue;
                }
                v[i] = c.wrapping_add(1);
                return v;
            }
        }
    }
}

fn str_cmp(a: V, o: V) -> V {
    if !is_str(o) {
        return QNIL;
    }
    mkfix(rstr(a).b.cmp(&rstr(o).b) as i64)
}

static SYM_PROCS: crate::mt::L<Option<HashMap<u32, V>>> = crate::mt::L::new(None);

pub fn static_roots(out: &mut Vec<V>) {
    if let Some(m) = SYM_PROCS.raw() {
        out.extend(m.values().copied());
    }
}

pub fn sym_to_proc(id: u32) -> V {
    if let Some(&p) = SYM_PROCS.get().as_ref().and_then(|m| m.get(&id)) {
        return p;
    }
    let p = proc_new_native(
        Box::new(move |_, args, blk| {
            if args.is_empty() {
                return arg_error("no receiver given");
            }
            send(args[0], id, &args[1..], blk)
        }),
        1,
        -1,
    );
    rproc(p).lambda = true;
    rproc(p).autosplat = false;
    SYM_PROCS.get().get_or_insert_with(HashMap::new).insert(id, p);
    p
}

pub fn init() {
    crate::gc::heap().extra_roots.push(static_roots);
    let c = rt().c.string;
    defs(c, "new", 0, 3, |s, a, b| {
        if s as *mut Class != rt().c.string {
            return crate::kernel::class_new_instance(s, a, b);
        }
        let bytes = a.first().filter(|x| is_str(**x)).map(|x| rstr(*x).b.clone()).unwrap_or_default();
        let n = str_new(bytes);
        if let Some(&x) = a.first().filter(|x| is_str(**x)) {
            copy_enc(n, x);
        }
        n
    });
    defn(c, "initialize", 0, 3, |s, a, _| {
        if let Some(&x) = a.first().filter(|x| is_str(**x)) {
            b(s).set(rstr(x).b.clone());
            copy_enc(s, x);
        }
        QNIL
    });
    /// `String.new(str)` keeps str's encoding
    fn copy_enc(dst: V, src: V) {
        let enc = crate::compat::FL_BINARY | (crate::compat::FL_BINARY << 1);
        hdr(dst).flags = (hdr(dst).flags & !enc) | (hdr(src).flags & enc);
    }
    // on an instance of a String subclass (ActiveSupport::StringInquirer, SafeBuffer...) they
    // return a plain String, like Ruby (Sprockets digests `Rails.env.to_s` by class)
    fn plain(s: V) -> V {
        if real_class(s) == rt().c.string {
            return s;
        }
        let n = str_new(rstr(s).b.clone());
        copy_enc(n, s);
        n
    }
    defn(c, "to_s", 0, 0, |s, _, _| plain(s));
    defn(c, "to_str", 0, 0, |s, _, _| plain(s));
    defn(c, "inspect", 0, 0, |s, _, _| new_str_v(str_inspect_v(s)));
    defn(c, "dump", 0, 0, |s, _, _| new_str_v(str_inspect(&rstr(s).b)));
    defn(c, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(c, "==", 1, 1, |s, a, _| bool_v(is_str(a[0]) && rstr(s).b == rstr(a[0]).b));
    defn(c, "===", 1, 1, |s, a, _| bool_v(is_str(a[0]) && rstr(s).b == rstr(a[0]).b));
    defn(c, "eql?", 1, 1, |s, a, _| bool_v(is_str(a[0]) && rstr(s).b == rstr(a[0]).b));
    defn(c, "<=>", 1, 1, |s, a, _| str_cmp(s, a[0]));
    defn(c, "=~", 1, 1, |s, a, _| {
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::match_index(a[0], s);
        }
        if is_str(a[0]) {
            return type_error("wrong argument type String (expected Regexp)");
        }
        send1(a[0], "=~", s)
    });
    defn(c, "match", 1, 2, |s, a, b| {
        let re = match crate::regex::pattern_arg(a[0]) {
            Ok(r) => r,
            Err(e) => return e,
        };
        send(re, sym("match"), &[s, a.get(1).copied().unwrap_or(mkfix(0))], b)
    });
    defn(c, "match?", 1, 2, |s, a, _| {
        let re = match crate::regex::pattern_arg(a[0]) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let mut args = vec![s];
        args.extend(a.get(1).copied());
        send(re, sym("match?"), &args, QNIL)
    });
    defn(c, "+", 1, 1, |s, a, _| {
        let o = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        str_plus(s, &o, a[0])
    });
    defn(c, "*", 1, 1, |s, a, _| {
        // a Float count is truncated (NUM2LONG), anything else non-Integer is a TypeError
        let n = if is_float(a[0]) && float_val(a[0]).is_finite() {
            mkfix(float_val(a[0]).trunc() as i64)
        } else if is_int(a[0]) {
            a[0]
        } else {
            return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(a[0]))));
        };
        let a = [n];
        if !is_fix(a[0]) || fix(a[0]) < 0 {
            return arg_error("negative argument");
        }
        str_new(rstr(s).b.repeat(fix(a[0]) as usize))
    });
    defn(c, "%", 1, 1, |s, a, _| {
        let args = if is_ary(a[0]) { rary(a[0]).v.clone() } else { vec![a[0]] };
        match sprintf(&rstr(s).b.clone(), &args) {
            Ok(x) => str_new(x),
            Err(e) => e,
        }
    });
    /// `<<` / `concat`, with CRuby's encoding rules (an ASCII-only receiver takes the
    /// argument's encoding; incompatible ones raise Encoding::CompatibilityError).
    fn append(s: V, a: &[V]) -> V {
        if let Err(e) = mutate(s) {
            return e;
        }
        // (`concat` with several arguments checks them all before appending any)
        let mut parts: Vec<(Vec<u8>, Option<crate::transcode::Enc>)> = Vec::with_capacity(a.len());
        for &x in a {
            if is_fix(x) {
                match crate::transcode::codepoint_bytes(s, fix(x)) {
                    Ok(p) => parts.push(p),
                    Err(e) => return e,
                }
            } else if is_str(x) && crate::transcode::same_native(s, x) {
                parts.push((rstr(x).b.to_vec(), None));
            } else {
                let o = match want_str(x) {
                    Ok(o) => o,
                    Err(e) => return e,
                };
                let xe = if is_str(x) { crate::transcode::enc_of(x) } else { crate::transcode::UTF8 };
                match crate::transcode::concat_enc(s, &o, xe) {
                    Ok(ne) => parts.push((o, ne)),
                    Err(e) => return e,
                }
            }
        }
        for (bytes, ne) in parts {
            if let Some(ne) = ne {
                crate::transcode::set_enc(s, ne);
            }
            b(s).extend_from_slice(&bytes);
        }
        s
    }
    defn(c, "<<", 1, 1, |s, a, _| append(s, a));
    defn(c, "concat", 0, -1, |s, a, _| append(s, a));
    // (bytes as they are: the receiver keeps its encoding)
    defn(c, "append_as_bytes", 0, -1, |s, a, _| {
        for &x in a {
            let add = if is_fix(x) {
                vec![(fix(x) & 0xFF) as u8]
            } else {
                match want_str(x) {
                    Ok(o) => o,
                    Err(e) => return e,
                }
            };
            match mutate(s) {
                Ok(bs) => bs.extend_from_slice(&add),
                Err(e) => return e,
            }
        }
        s
    });
    defn(c, "prepend", 0, -1, |s, a, _| {
        let mut pre = Vec::new();
        for &x in a {
            match want_str(x) {
                Ok(o) => pre.extend(o),
                Err(e) => return e,
            }
        }
        match mutate(s) {
            Ok(bs) => {
                pre.extend_from_slice(bs);
                bs.set(pre);
                s
            }
            Err(e) => e,
        }
    });
    defn(c, "replace", 1, 1, |s, a, _| match (want_str(a[0]), mutate(s)) {
        (Ok(o), Ok(bs)) => {
            bs.set(o);
            s
        }
        (Err(e), _) | (_, Err(e)) => e,
    });
    defn(c, "clear", 0, 0, |s, _, _| match mutate(s) {
        Ok(bs) => {
            bs.clear();
            s
        }
        Err(e) => e,
    });
    defn(c, "insert", 2, 2, |s, a, _| {
        let o = match want_str(a[1]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let mut cs = chars(s);
        let n = cs.len() as i64;
        let i = fix(a[0]);
        let i = if i < 0 { i + n + 1 } else { i };
        if i < 0 || i > n {
            return raise(rt().c.index_error, &format!("index {} out of string", fix(a[0])));
        }
        cs.insert(i as usize, String::from_utf8_lossy(&o).into_owned());
        match mutate(s) {
            Ok(bs) => {
                bs.set(cs.concat().into_bytes());
                s
            }
            Err(e) => e,
        }
    });
    defn(c, "length", 0, 0, |s, _, _| mkfix(char_len(s) as i64));
    defn(c, "size", 0, 0, |s, _, _| mkfix(char_len(s) as i64));
    defn(c, "bytesize", 0, 0, |s, _, _| mkfix(rstr(s).b.len() as i64));
    defn(c, "empty?", 0, 0, |s, _, _| bool_v(rstr(s).b.is_empty()));
    defn(c, "+@", 0, 0, |s, _, _| if is_frozen(s) { str_new(rstr(s).b.clone()) } else { s });
    defn(c, "-@", 0, 0, |s, _, _| {
        if is_frozen(s) {
            return s;
        }
        let n = str_new(rstr(s).b.clone());
        hdr(n).flags |= FL_FROZEN;
        n
    });
    defn(c, "dedup", 0, 0, |s, _, _| {
        let n = str_new(rstr(s).b.clone());
        hdr(n).flags |= FL_FROZEN;
        n
    });
    macro_rules! map_str {
        ($name:expr, $bang:expr, $f:expr) => {
            defn(c, $name, 0, 1, |s, _, _| new_str_v($f(&s_of(s))));
            defn(c, $bang, 0, 1, |s, _, _| {
                let n: String = $f(&s_of(s));
                if n.as_bytes() == rstr(s).b.as_slice() {
                    return QNIL;
                }
                match mutate(s) {
                    Ok(bs) => {
                        bs.set(n.into_bytes());
                        s
                    }
                    Err(e) => e,
                }
            });
        };
    }
    fn capitalize(x: &str) -> String {
        let mut cs = x.chars();
        match cs.next() {
            Some(f) => f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
            None => String::new(),
        }
    }
    fn swapcase(x: &str) -> String {
        x.chars().map(|c| if c.is_uppercase() { c.to_lowercase().collect::<String>() } else { c.to_uppercase().collect() }).collect()
    }
    map_str!("upcase", "upcase!", |x: &str| x.to_uppercase());
    map_str!("downcase", "downcase!", |x: &str| x.to_lowercase());
    map_str!("capitalize", "capitalize!", capitalize);
    map_str!("swapcase", "swapcase!", swapcase);
    map_str!("reverse", "reverse!", |x: &str| x.chars().rev().collect::<String>());
    map_str!("strip", "strip!", |x: &str| String::from_utf8_lossy(strip_r(strip_l(x.as_bytes()))).into_owned());
    map_str!("lstrip", "lstrip!", |x: &str| String::from_utf8_lossy(strip_l(x.as_bytes())).into_owned());
    map_str!("rstrip", "rstrip!", |x: &str| String::from_utf8_lossy(strip_r(x.as_bytes())).into_owned());
    map_str!("chop", "chop!", |x: &str| {
        let mut s = x.to_string();
        if s.ends_with("\r\n") {
            s.truncate(s.len() - 2);
        } else {
            s.pop();
        }
        s
    });
    // squeeze([set...]): collapse runs of the same character (only characters in the sets, if given)
    fn squeeze(s: V, a: &[V]) -> Result<String, V> {
        let sets = sets_of(a)?;
        let mut out = String::new();
        let mut last = None;
        for ch in s_of(s).chars() {
            if Some(ch) != last || (!sets.is_empty() && !in_sets(ch, &sets)) {
                out.push(ch);
            }
            last = Some(ch);
        }
        Ok(out)
    }
    defn(c, "squeeze", 0, -1, |s, a, _| match squeeze(s, a) {
        Ok(x) => new_str_v(x),
        Err(e) => e,
    });
    defn(c, "squeeze!", 0, -1, |s, a, _| {
        let n = match squeeze(s, a) {
            Ok(x) => x,
            Err(e) => return e,
        };
        if n.as_bytes() == rstr(s).b.as_slice() {
            return QNIL;
        }
        match mutate(s) {
            Ok(bs) => {
                bs.set(n.into_bytes());
                s
            }
            Err(e) => e,
        }
    });
    fn chomp(s: V, a: &[V]) -> Vec<u8> {
        let x = rstr(s).b.clone();
        if let Some(&suf) = a.first() {
            if is_str(suf) {
                let sb = &rstr(suf).b;
                return if x.ends_with(sb) { x[..x.len() - sb.len()].to_vec() } else { x };
            }
        }
        if x.ends_with(b"\r\n") {
            x[..x.len() - 2].to_vec()
        } else if x.ends_with(b"\n") || x.ends_with(b"\r") {
            x[..x.len() - 1].to_vec()
        } else {
            x
        }
    }
    defn(c, "chomp", 0, 1, |s, a, _| str_new(chomp(s, a)));
    defn(c, "chomp!", 0, 1, |s, a, _| {
        let n = chomp(s, a);
        if n == rstr(s).b {
            return QNIL;
        }
        match mutate(s) {
            Ok(bs) => {
                bs.set(n);
                s
            }
            Err(e) => e,
        }
    });
    defn(c, "delete_prefix", 1, 1, |s, a, _| {
        let x = rstr(s).b.clone();
        let p = rstr(a[0]).b.clone();
        str_new(if x.starts_with(&p) { x[p.len()..].to_vec() } else { x })
    });
    defn(c, "delete_suffix", 1, 1, |s, a, _| {
        let x = rstr(s).b.clone();
        let p = rstr(a[0]).b.clone();
        str_new(if x.ends_with(&p) { x[..x.len() - p.len()].to_vec() } else { x })
    });
    defn(c, "include?", 1, 1, |s, a, _| match want_str(a[0]) {
        Ok(o) => bool_v(find(&rstr(s).b, &o, 0).is_some()),
        Err(e) => e,
    });
    defn(c, "start_with?", 0, -1, |s, a, _| {
        for &p in a {
            if crate::regex::is_regexp(p) {
                let m = crate::regex::do_match(p, s, 0, true);
                if m != QNIL && fix(tri!(send1(m, "begin", mkfix(0)))) == 0 {
                    return QTRUE;
                }
            } else if is_str(p) && rstr(s).b.starts_with(&rstr(p).b) {
                return QTRUE;
            }
        }
        QFALSE
    });
    defn(c, "end_with?", 0, -1, |s, a, _| bool_v(a.iter().any(|p| is_str(*p) && rstr(s).b.ends_with(&rstr(*p).b))));
    // index/rindex positions count characters; a negative one from the end
    defn(c, "index", 1, 2, |s, a, _| {
        let n = char_len(s) as i64;
        let mut from = match a.get(1) {
            Some(x) => match int_arg(*x) {
                Ok(i) => i,
                Err(e) => return e,
            },
            None => 0,
        };
        if from < 0 {
            from += n;
        }
        if from < 0 || from > n {
            if crate::regex::is_regexp(a[0]) {
                crate::regex::set_last(QNIL);
            }
            return QNIL;
        }
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::str_index(s, a[0], from as usize);
        }
        let o = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let b = &rstr(s).b;
        match find(b, &o, char_byte(b, from as usize)) {
            Some(i) => mkfix(char_count(&b[..i]) as i64),
            None => QNIL,
        }
    });
    defn(c, "rindex", 1, 2, |s, a, _| {
        let n = char_len(s) as i64;
        let mut pos = match a.get(1) {
            Some(x) => match int_arg(*x) {
                Ok(i) => i,
                Err(e) => return e,
            },
            None => n,
        };
        if pos < 0 {
            pos += n;
        }
        if pos < 0 {
            if crate::regex::is_regexp(a[0]) {
                crate::regex::set_last(QNIL);
            }
            return QNIL;
        }
        let pos = pos.min(n) as usize;
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::str_rindex(s, a[0], pos);
        }
        let o = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let b = &rstr(s).b;
        // the last occurrence starting at character `pos` or before
        let end = (char_byte(b, pos) + o.len()).min(b.len());
        match rfind(&b[..end], &o) {
            Some(i) => mkfix(char_count(&b[..i]) as i64),
            None => QNIL,
        }
    });
    // byteindex/byterindex: byte offsets (an offset inside a character is an IndexError)
    fn byte_off(s: V, a: &[V], dflt: i64) -> Result<Option<usize>, V> {
        let b = &rstr(s).b;
        let n = b.len() as i64;
        let mut off = match a.get(1) {
            Some(x) => int_arg(*x)?,
            None => dflt,
        };
        if off < 0 {
            off += n;
        }
        if off < 0 || off > n {
            return Ok(None);
        }
        let o = off as usize;
        if o < b.len() && (b[o] & 0xc0) == 0x80 && !is_binary(s) {
            return Err(raise(rt().c.index_error, &format!("offset {off} does not land on character boundary")));
        }
        Ok(Some(o))
    }
    defn(c, "byteindex", 1, 2, |s, a, _| {
        let from = match byte_off(s, a, 0) {
            Ok(Some(o)) => o,
            Ok(None) => return QNIL,
            Err(e) => return e,
        };
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::str_byte_search(s, a[0], from, false);
        }
        let o = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        match find(&rstr(s).b, &o, from) {
            Some(i) => mkfix(i as i64),
            None => QNIL,
        }
    });
    defn(c, "byterindex", 1, 2, |s, a, _| {
        let n = rstr(s).b.len() as i64;
        let from = match byte_off(s, a, n) {
            Ok(Some(o)) => o,
            Ok(None) => return QNIL,
            Err(e) => return e,
        };
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::str_byte_search(s, a[0], from, true);
        }
        let o = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        let b = &rstr(s).b;
        let end = (from + o.len()).min(b.len());
        match rfind(&b[..end], &o) {
            Some(i) => mkfix(i as i64),
            None => QNIL,
        }
    });
    // bytesplice(index, length, str [, str_index, str_length]) / (range, str [, str_range])
    defn(c, "bytesplice", 2, 5, |s, a, _| {
        if is_frozen(s) {
            return check_frozen(s);
        }
        let n = rstr(s).b.len() as i64;
        let span = |r: V, n: i64| -> Option<(usize, usize)> {
            let (st, len) = range_bounds(r, n)?;
            Some((st as usize, len.max(0) as usize))
        };
        let (start, len, rest) = if is_range(a[0]) {
            match span(a[0], n) {
                Some((st, l)) => (st, l, &a[1..]),
                None => return raise(rt().c.range_error, "range out of string"),
            }
        } else {
            let (Ok(mut st), Some(l)) = (int_arg(a[0]), a.get(1).map(|x| int_arg(*x))) else {
                return type_error("wrong argument type (expected Integer or Range)");
            };
            let Ok(l) = l else { return type_error("no implicit conversion into Integer") };
            if st < 0 {
                st += n;
            }
            if st < 0 || st > n {
                return raise(rt().c.index_error, &format!("index {} out of string", fix(a[0])));
            }
            (st as usize, (l.max(0) as usize).min((n - st) as usize), &a[2..])
        };
        let Some(&src) = rest.first().filter(|x| is_str(**x)) else {
            return type_error("wrong argument type (expected String)");
        };
        let sb = rstr(src).b.clone();
        let piece = if rest.len() == 1 {
            sb
        } else if is_range(rest[1]) {
            match span(rest[1], sb.len() as i64) {
                Some((st, l)) => sb[st..(st + l).min(sb.len())].to_vec(),
                None => return raise(rt().c.range_error, "range out of string"),
            }
        } else {
            let (Ok(st), Some(Ok(l))) = (int_arg(rest[1]), rest.get(2).map(|x| int_arg(*x))) else {
                return type_error("wrong argument type");
            };
            let st = if st < 0 { st + sb.len() as i64 } else { st };
            if st < 0 || st > sb.len() as i64 {
                return raise(rt().c.index_error, "index out of string");
            }
            let st = st as usize;
            sb[st..(st + l.max(0) as usize).min(sb.len())].to_vec()
        };
        let end = (start + len).min(n as usize);
        rstr(s).b.splice(start..end, piece);
        s
    });
    defn(c, "[]", 1, 2, |s, a, _| aref(s, a));
    defn(c, "slice", 1, 2, |s, a, _| aref(s, a));
    defn(c, "[]=", 2, 3, |s, a, _| {
        let val = *a.last().unwrap();
        let rep = match want_str(val) {
            Ok(o) => String::from_utf8_lossy(&o).into_owned(),
            Err(e) => return e,
        };
        let cs = chars(s);
        let n = cs.len() as i64;
        let (start, len) = if a.len() == 3 {
            (fix(a[0]), fix(a[1]))
        } else if is_fix(a[0]) {
            (fix(a[0]), 1)
        } else if is_range(a[0]) {
            match range_bounds(a[0], n) {
                Some(x) => x,
                None => return raise(rt().c.range_error, "range out of string"),
            }
        } else if is_str(a[0]) {
            let pat = rstr(a[0]).b.clone();
            match find(&rstr(s).b, &pat, 0) {
                Some(i) => (String::from_utf8_lossy(&rstr(s).b[..i]).chars().count() as i64, String::from_utf8_lossy(&pat).chars().count() as i64),
                None => return raise(rt().c.index_error, "string not matched"),
            }
        } else {
            return type_error("no implicit conversion into Integer");
        };
        let start = if start < 0 { start + n } else { start };
        if start < 0 || start > n {
            return raise(rt().c.index_error, &format!("index {} out of string", start));
        }
        let end = (start + len).min(n);
        let out = cs[..start as usize].concat() + &rep + &cs[end as usize..].concat();
        match mutate(s) {
            Ok(bs) => {
                bs.set(out.into_bytes());
                val
            }
            Err(e) => e,
        }
    });
    defn(c, "chars", 0, 0, |s, _, _| ary_new(chars(s).into_iter().map(new_str_v).collect()));
    defn(c, "bytes", 0, 0, |s, _, _| ary_new(rstr(s).b.iter().map(|x| mkfix(*x as i64)).collect()));
    defn(c, "getbyte", 1, 1, |s, a, _| {
        let i = if is_fix(a[0]) {
            fix(a[0])
        } else if is_float(a[0]) {
            float_val(a[0]) as i64
        } else {
            return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(a[0]))));
        };
        let b = &rstr(s).b;
        let i = if i < 0 { i + b.len() as i64 } else { i };
        if i < 0 { QNIL } else { b.get(i as usize).map(|x| mkfix(*x as i64)).unwrap_or(QNIL) }
    });
    fn lines(s: V) -> Vec<V> {
        let x = rstr(s).b.clone();
        let mut out = Vec::new();
        let mut start = 0;
        for (i, ch) in x.iter().enumerate() {
            if *ch == b'\n' {
                out.push(str_new(x[start..=i].to_vec()));
                start = i + 1;
            }
        }
        if start < x.len() {
            out.push(str_new(x[start..].to_vec()));
        }
        out
    }
    defn(c, "lines", 0, 1, |s, _, _| ary_new(lines(s)));
    defn(c, "each_line", 0, 1, |s, _, blk| {
        if !is_proc(blk) {
            return crate::misc::enum_for(s, "each_line", &[]);
        }
        let ls = crate::gc::RVec::from(lines(s));
        for i in 0..ls.len() {
            tri!(call_block(blk, &[ls[i]]));
        }
        s
    });
    defn(c, "each_char", 0, 0, |s, _, blk| {
        if !is_proc(blk) {
            return crate::misc::enum_for(s, "each_char", &[]);
        }
        for ch in chars(s) {
            tri!(call_block(blk, &[new_str_v(ch)]));
        }
        s
    });
    defn(c, "each_byte", 0, 0, |s, _, blk| {
        if !is_proc(blk) {
            return crate::misc::enum_for(s, "each_byte", &[]);
        }
        for x in rstr(s).b.clone() {
            tri!(call_block(blk, &[mkfix(x as i64)]));
        }
        s
    });
    defn(c, "split", 0, 2, |s, a, blk| {
        let r = if a.first().is_some_and(|x| crate::regex::is_regexp(*x)) {
            crate::regex::split(s, a[0], a.get(1).map(|x| fix(*x)).unwrap_or(0))
        } else {
            split(s, a.first().copied().unwrap_or(QNIL), a.get(1).map(|x| fix(*x)).unwrap_or(0))
        };
        let r = tri!(r);
        if is_proc(blk) {
            for x in crate::gc::Rooted::of(rary(r).v.clone()) {
                tri!(call_block(blk, &[x]));
            }
            return s;
        }
        r
    });
    defn(c, "partition", 1, 1, |s, a, _| {
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::partition(s, a[0], false);
        }
        let x = rstr(s).b.clone();
        let p = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        match find(&x, &p, 0) {
            Some(i) => ary_new(vec![str_new(x[..i].to_vec()), str_new(p.clone()), str_new(x[i + p.len()..].to_vec())]),
            None => ary_new(vec![str_new(x), str_new(vec![]), str_new(vec![])]),
        }
    });
    defn(c, "rpartition", 1, 1, |s, a, _| {
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::partition(s, a[0], true);
        }
        let x = rstr(s).b.clone();
        let p = match want_str(a[0]) {
            Ok(o) => o,
            Err(e) => return e,
        };
        match rfind(&x, &p) {
            Some(i) => ary_new(vec![str_new(x[..i].to_vec()), str_new(p.clone()), str_new(x[i + p.len()..].to_vec())]),
            None => ary_new(vec![str_new(vec![]), str_new(vec![]), str_new(x)]),
        }
    });
    defn(c, "to_i", 0, 1, |s, a, _| {
        let base = a.first().map(|x| fix(*x) as u32).unwrap_or(10);
        if base == 10 {
            return crate::kernel::parse_int(&rstr(s).b, false).unwrap_or(mkfix(0));
        }
        let t = s_of(s);
        let t = t.trim();
        let digits: String = t.chars().take_while(|c| c.is_digit(base) || *c == '-').collect();
        num_bigint::BigInt::parse_bytes(digits.as_bytes(), base).map(int_from_big).unwrap_or(mkfix(0))
    });
    defn(c, "hex", 0, 0, |s, _, _| {
        let t = s_of(s);
        let t = t.trim().trim_start_matches("0x");
        let d: String = t.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        num_bigint::BigInt::parse_bytes(d.as_bytes(), 16).map(int_from_big).unwrap_or(mkfix(0))
    });
    defn(c, "oct", 0, 0, |s, _, _| {
        let d: String = s_of(s).trim().chars().take_while(|c| c.is_digit(8)).collect();
        num_bigint::BigInt::parse_bytes(d.as_bytes(), 8).map(int_from_big).unwrap_or(mkfix(0))
    });
    defn(c, "to_f", 0, 0, |s, _, _| {
        let t = s_of(s);
        let t = t.trim();
        let mut end = 0;
        let cs: Vec<char> = t.chars().collect();
        let mut seen_dot = false;
        let mut seen_e = false;
        while end < cs.len() {
            let ch = cs[end];
            let ok = ch.is_ascii_digit()
                || (ch == '-' || ch == '+') && (end == 0 || matches!(cs[end - 1], 'e' | 'E'))
                || ch == '.' && !seen_dot && !seen_e
                || matches!(ch, 'e' | 'E') && !seen_e && end > 0
                || ch == '_';
            if !ok {
                break;
            }
            seen_dot |= ch == '.';
            seen_e |= matches!(ch, 'e' | 'E');
            end += 1;
        }
        let num: String = cs[..end].iter().filter(|c| **c != '_').collect();
        let mut n = num.as_str();
        loop {
            if let Ok(f) = n.parse::<f64>() {
                return float_new(f);
            }
            if n.is_empty() {
                return float_new(0.0);
            }
            n = &n[..n.len() - 1];
        }
    });
    defn(c, "to_sym", 0, 0, |s, _, _| mksym(intern(&rstr(s).b.clone())));
    defn(c, "intern", 0, 0, |s, _, _| mksym(intern(&rstr(s).b.clone())));
    defn(c, "ord", 0, 0, |s, _, _| match if hdr(s).flags & FL_BINARY != 0 { rstr(s).b.first().map(|b| *b as char) } else { s_of(s).chars().next() } {
        Some(ch) => mkfix(ch as i64),
        None => arg_error("empty string"),
    });
    defn(c, "sum", 0, 0, |s, _, _| mkfix(rstr(s).b.iter().map(|x| *x as i64).sum()));
    fn pad(s: V, a: &[V], mode: u8) -> V {
        let w = fix(a[0]).max(0) as usize;
        let padc: Vec<char> = match a.get(1) {
            Some(p) if is_str(*p) => s_of(*p).chars().collect(),
            _ => vec![' '],
        };
        if padc.is_empty() {
            return arg_error("zero width padding");
        }
        let cur = s_of(s);
        let n = cur.chars().count();
        if w <= n {
            return str_new(cur.into_bytes());
        }
        let total = w - n;
        let mk = |k: usize| -> String { (0..k).map(|i| padc[i % padc.len()]).collect() };
        let out = match mode {
            b'l' => cur + &mk(total),
            b'r' => mk(total) + &cur,
            _ => {
                let left = total / 2;
                mk(left) + &cur + &mk(total - left)
            }
        };
        new_str_v(out)
    }
    defn(c, "ljust", 1, 2, |s, a, _| pad(s, a, b'l'));
    defn(c, "rjust", 1, 2, |s, a, _| pad(s, a, b'r'));
    defn(c, "center", 1, 2, |s, a, _| pad(s, a, b'c'));
    defn(c, "tr", 2, 2, |s, a, _| match (want_str(a[0]), want_str(a[1])) {
        (Ok(f), Ok(t)) => new_str_v(tr(&s_of(s), &String::from_utf8_lossy(&f), &String::from_utf8_lossy(&t))),
        (Err(e), _) | (_, Err(e)) => e,
    });
    defn(c, "delete", 1, -1, |s, a, _| match sets_of(a) {
        Ok(sets) => new_str_v(s_of(s).chars().filter(|ch| !in_sets(*ch, &sets)).collect()),
        Err(e) => e,
    });
    defn(c, "count", 1, -1, |s, a, _| match sets_of(a) {
        Ok(sets) => mkfix(s_of(s).chars().filter(|ch| in_sets(*ch, &sets)).count() as i64),
        Err(e) => e,
    });
    defn(c, "sub", 1, 2, |s, a, blk| match sub_impl(s, a, blk, false) {
        Ok(Some(o)) => str_new(o),
        Ok(None) => str_new(rstr(s).b.clone()),
        Err(e) => e,
    });
    defn(c, "gsub", 1, 2, |s, a, blk| match sub_impl(s, a, blk, true) {
        Ok(Some(o)) => str_new(o),
        Ok(None) => str_new(rstr(s).b.clone()),
        Err(e) => e,
    });
    defn(c, "sub!", 1, 2, |s, a, blk| match sub_impl(s, a, blk, false) {
        Ok(Some(o)) => match mutate(s) {
            Ok(bs) => {
                bs.set(o);
                s
            }
            Err(e) => e,
        },
        Ok(None) => QNIL,
        Err(e) => e,
    });
    defn(c, "gsub!", 1, 2, |s, a, blk| match sub_impl(s, a, blk, true) {
        Ok(Some(o)) => match mutate(s) {
            Ok(bs) => {
                bs.set(o);
                s
            }
            Err(e) => e,
        },
        Ok(None) => QNIL,
        Err(e) => e,
    });
    defn(c, "scan", 1, 1, |s, a, blk| {
        if crate::regex::is_regexp(a[0]) {
            return crate::regex::scan(s, a[0], blk);
        }
        let p = match want_str(a[0]) {
            Ok(o) => o,
            Err(_) => return raise(rt().c.not_implemented_error, "Regexp patterns are not supported yet"),
        };
        let x = rstr(s).b.clone();
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(i) = find(&x, &p, start) {
            out.push(str_new(p.clone()));
            start = i + p.len().max(1);
        }
        let out = crate::gc::RVec::from(out);
        if is_proc(blk) {
            for i in 0..out.len() {
                tri!(call_block(blk, &[out[i]]));
            }
            return s;
        }
        ary_new(out.to_vec())
    });
    defn(c, "succ", 0, 0, |s, _, _| str_new(succ(&rstr(s).b)));
    defn(c, "next", 0, 0, |s, _, _| str_new(succ(&rstr(s).b)));
    defn(c, "upto", 1, 2, |s, a, blk| {
        let end = rstr(a[0]).b.clone();
        let mut cur = rstr(s).b.clone();
        loop {
            if cur.len() > end.len() {
                break;
            }
            tri!(call_block(blk, &[str_new(cur.clone())]));
            if cur == end {
                break;
            }
            cur = succ(&cur);
        }
        s
    });
    defn(c, "casecmp", 1, 1, |s, a, _| {
        if !is_str(a[0]) {
            return QNIL;
        }
        mkfix(s_of(s).to_lowercase().cmp(&s_of(a[0]).to_lowercase()) as i64)
    });
    defn(c, "casecmp?", 1, 1, |s, a, _| {
        if !is_str(a[0]) {
            return QNIL;
        }
        bool_v(s_of(s).to_lowercase() == s_of(a[0]).to_lowercase())
    });
    defn(c, "ascii_only?", 0, 0, |s, _, _| bool_v(rstr(s).b.is_ascii()));
    defn(c, "valid_encoding?", 0, 0, |s, _, _| bool_v(std::str::from_utf8(&rstr(s).b).is_ok()));
    defn(c, "force_encoding", 1, 1, |s, _, _| s);
    defn(c, "encode", 0, -1, |s, _, _| str_new(rstr(s).b.clone()));
    defn(c, "scrub", 0, 1, |s, _, _| new_str_v(s_of(s)));
    defn(c, "b", 0, 0, |s, _, _| str_new(rstr(s).b.clone()));
    defn(c, "encoding", 0, 0, |_, _, _| {
        let enc = const_lookup(rt().c.object, sym("Encoding")).unwrap();
        const_get_under(enc as *mut Class, sym("UTF_8")).unwrap()
    });
    defn(c, "freeze", 0, 0, |s, _, _| {
        hdr(s).flags |= FL_FROZEN;
        s
    });
    defn(c, "unpack1", 1, 1, |_, _, _| raise(rt().c.not_implemented_error, "String#unpack1 is not supported yet"));

    let enc = new_class("Encoding", rt().c.object);
    set_global_const("Encoding", enc as V);
    let utf8 = object_new(enc);
    ivar_set(utf8, sym("@__name"), str_from("UTF-8"));
    const_set(enc, sym("UTF_8"), utf8);
    let bin = object_new(enc);
    ivar_set(bin, sym("@__name"), str_from("ASCII-8BIT"));
    const_set(enc, sym("BINARY"), bin);
    const_set(enc, sym("ASCII_8BIT"), bin);
    defn(enc, "to_s", 0, 0, |s, _, _| ivar_get(s, sym("@__name")));
    defn(enc, "name", 0, 0, |s, _, _| ivar_get(s, sym("@__name")));
    defn(enc, "inspect", 0, 0, |s, _, _| {
        let n = s_of(ivar_get(s, sym("@__name")));
        // (Ruby 3.4 names ASCII-8BIT BINARY first)
        let n = if n == "ASCII-8BIT" { "BINARY (ASCII-8BIT)".to_string() } else { n };
        new_str_v(format!("#<Encoding:{n}>"))
    });

    // ---- Symbol ----
    let y = rt().c.symbol;
    defn(y, "to_s", 0, 0, |s, _, _| str_new(sym_name(sym_id(s)).to_vec()));
    defn(y, "id2name", 0, 0, |s, _, _| str_new(sym_name(sym_id(s)).to_vec()));
    defn(y, "name", 0, 0, |s, _, _| {
        let v = str_new(sym_name(sym_id(s)).to_vec());
        hdr(v).flags |= FL_FROZEN;
        v
    });
    defn(y, "to_sym", 0, 0, |s, _, _| s);
    defn(y, "to_proc", 0, 0, |s, _, _| sym_to_proc(sym_id(s)));
    defn(y, "inspect", 0, 0, |s, _, _| new_str_v(sym_inspect(sym_id(s))));
    defn(y, "length", 0, 0, |s, _, _| mkfix(String::from_utf8_lossy(sym_name(sym_id(s))).chars().count() as i64));
    defn(y, "size", 0, 0, |s, _, _| mkfix(String::from_utf8_lossy(sym_name(sym_id(s))).chars().count() as i64));
    defn(y, "<=>", 1, 1, |s, a, _| {
        if !is_sym(a[0]) {
            return QNIL;
        }
        mkfix(sym_name(sym_id(s)).cmp(sym_name(sym_id(a[0]))) as i64)
    });
    defn(y, "==", 1, 1, |s, a, _| bool_v(s == a[0]));
    defn(y, "hash", 0, 0, |s, _, _| mkfix((hash_self(s) >> 2) as i64));
    defn(y, "upcase", 0, 0, |s, _, _| mksym(sym(&sym_str(sym_id(s)).to_uppercase())));
    defn(y, "downcase", 0, 0, |s, _, _| mksym(sym(&sym_str(sym_id(s)).to_lowercase())));
    defn(y, "capitalize", 0, 0, |s, _, _| {
        let x = str_from(&sym_str(sym_id(s)));
        let c = tri!(send0(x, "capitalize"));
        mksym(intern(&rstr(c).b.clone()))
    });
    defn(y, "succ", 0, 0, |s, _, _| mksym(intern(&succ(sym_name(sym_id(s))))));
    defn(y, "[]", 1, 2, |s, a, _| aref(str_from(&sym_str(sym_id(s))), a));
    defn(y, "start_with?", 0, -1, |s, a, _| bool_v(a.iter().any(|p| is_str(*p) && sym_name(sym_id(s)).starts_with(&rstr(*p).b))));
    defn(y, "end_with?", 0, -1, |s, a, _| bool_v(a.iter().any(|p| is_str(*p) && sym_name(sym_id(s)).ends_with(&rstr(*p).b))));
    defn(y, "=~", 1, 1, |s, a, _| send1(str_from(&sym_str(sym_id(s))), "=~", a[0]));
    defn(y, "match", 1, 2, |s, a, _| send(str_from(&sym_str(sym_id(s))), sym("match"), a, QNIL));
    defn(y, "match?", 1, 2, |s, a, _| send(str_from(&sym_str(sym_id(s))), sym("match?"), a, QNIL));
    defn(y, "empty?", 0, 0, |s, _, _| bool_v(sym_name(sym_id(s)).is_empty()));
    let _ = sym_arg;
}
