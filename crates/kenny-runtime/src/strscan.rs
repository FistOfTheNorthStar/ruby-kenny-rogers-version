//! StringScanner (the `strscan` C extension), on top of the Oniguruma-backed Regexp.
//!
//! State lives in hidden ivars: `@__str` (the scanned String, shared like CRuby),
//! `@__pos` / `@__prev` (byte offsets), `@__m` (absolute group offsets of the last match,
//! or nil) and `@__re` (the last Regexp, for named groups).
//!
//! Like CRuby's default (`fixed_anchor: false`), matching runs on the rest of the string,
//! so `\A` and `^` match at the scan pointer.

use crate::core::*;
use crate::compat::FL_BINARY;
use crate::regex::{is_regexp, re_of};
use crate::tri;
use onig::{EncodedChars, Region, SearchOptions};

static SCANNER: G<*mut Class> = G::new(std::ptr::null_mut());
static SCAN_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());

/// Raw bytes handed to Oniguruma as UTF-8 (binary strings need not be valid UTF-8).
struct Bytes<'a>(&'a [u8]);
impl EncodedChars for Bytes<'_> {
    fn start_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }
    fn limit_ptr(&self) -> *const u8 {
        unsafe { self.0.as_ptr().add(self.0.len()) }
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

fn s_str(s: V) -> V {
    ivar_get(s, sym("@__str"))
}
fn s_bytes(s: V) -> &'static mut SVec<u8> {
    &mut rstr(s_str(s)).b
}
fn s_pos(s: V) -> usize {
    let p = ivar_get(s, sym("@__pos"));
    if is_fix(p) { (fix(p).max(0) as usize).min(s_bytes(s).len()) } else { 0 }
}
fn set_pos(s: V, p: usize) {
    ivar_set(s, sym("@__pos"), mkfix(p as i64));
}
fn s_match(s: V) -> Option<Vec<i64>> {
    let m = ivar_get(s, sym("@__m"));
    if is_ary(m) { Some(rary(m).v.iter().map(|x| fix(*x)).collect()) } else { None }
}
fn set_match(s: V, m: Option<Vec<i64>>, re: V) {
    match m {
        Some(v) => {
            ivar_set(s, sym("@__m"), ary_new(v.into_iter().map(mkfix).collect()));
            ivar_set(s, sym("@__re"), re);
        }
        None => {
            ivar_set(s, sym("@__m"), QNIL);
        }
    }
}
/// A new String with the scanned string's encoding.
fn sub(s: V, a: usize, b: usize) -> V {
    let bytes = s_bytes(s);
    let (a, b) = (a.min(bytes.len()), b.min(bytes.len()));
    let r = str_new(bytes[a..b.max(a)].to_vec());
    hdr(r).flags |= hdr(s_str(s)).flags & (FL_BINARY | (FL_BINARY << 1));
    r
}

/// CRuby's strscan_do_scan: `adv` moves the pointer, `getstr` returns the string (else
/// the length), `head` anchors the match at the pointer.
fn do_scan(s: V, pat: V, adv: bool, getstr: bool, head: bool) -> V {
    let pos = s_pos(s);
    let (rest_len, m) = {
        let bytes = &s_bytes(s)[..];
        let rest = &bytes[pos..];
        let m: Option<Vec<i64>> = if is_regexp(pat) {
            let re = &re_of(pat).re;
            let mut region = Region::new();
            let hit = if head {
                re.match_with_encoding(Bytes(rest), 0, SearchOptions::SEARCH_OPTION_NONE, Some(&mut region)).is_some()
            } else {
                re.search_with_encoding(Bytes(rest), 0, rest.len(), SearchOptions::SEARCH_OPTION_NONE, Some(&mut region)).is_some()
            };
            if hit {
                let mut v = Vec::with_capacity(region.len() * 2);
                for i in 0..region.len() {
                    match region.pos(i) {
                        Some((b, e)) => {
                            v.push((b + pos) as i64);
                            v.push((e + pos) as i64);
                        }
                        None => {
                            v.push(-1);
                            v.push(-1);
                        }
                    }
                }
                Some(v)
            } else {
                None
            }
        } else if is_str(pat) {
            let p = &rstr(pat).b;
            let at = if head {
                rest.starts_with(p).then_some(0)
            } else {
                crate::string::find(rest, p, 0)
            };
            at.map(|i| vec![(pos + i) as i64, (pos + i + p.len()) as i64])
        } else {
            return type_error(&format!("wrong argument type {} (expected Regexp)", class_name(real_class(pat))));
        };
        (rest.len(), m)
    };
    let _ = rest_len;
    let Some(m) = m else {
        set_match(s, None, QNIL);
        return QNIL;
    };
    let end = m[1] as usize;
    ivar_set(s, sym("@__prev"), mkfix(pos as i64));
    set_match(s, Some(m), if is_regexp(pat) { pat } else { QNIL });
    if adv {
        set_pos(s, end);
    }
    if getstr { sub(s, pos, end) } else { mkfix((end - pos) as i64) }
}

fn group_index(s: V, k: V, m: &[i64]) -> Result<Option<usize>, V> {
    let n = m.len() / 2;
    if is_fix(k) {
        let mut i = fix(k);
        if i < 0 {
            i += n as i64;
        }
        return Ok(if i < 0 || i as usize >= n { None } else { Some(i as usize) });
    }
    if is_sym(k) || is_str(k) {
        let name = String::from_utf8_lossy(&crate::fmt::to_s(k).unwrap_or_default()).into_owned();
        let re = ivar_get(s, sym("@__re"));
        let found = if is_regexp(re) {
            re_of(re).names.iter().find(|(nm, _)| *nm == name).map(|(_, g)| {
                g.iter().rev().map(|x| *x as usize).find(|g| m.get(g * 2).is_some_and(|b| *b >= 0)).unwrap_or(*g.last().unwrap() as usize)
            })
        } else {
            None
        };
        return match found {
            Some(g) => Ok(Some(g)),
            None => Err(raise(rt().c.index_error, &format!("undefined group name reference: {name}"))),
        };
    }
    if is_float(k) {
        return group_index(s, mkfix(float_val(k) as i64), m);
    }
    Err(type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(k)))))
}
fn group(s: V, m: &[i64], i: usize) -> V {
    if m[i * 2] < 0 { QNIL } else { sub(s, m[i * 2] as usize, m[i * 2 + 1] as usize) }
}

fn inspect(s: V) -> V {
    let st = s_str(s);
    if !is_str(st) {
        return str_from("#<StringScanner (uninitialized)>");
    }
    let b = s_bytes(s).clone();
    let pos = s_pos(s);
    if pos >= b.len() {
        return str_from("#<StringScanner fin>");
    }
    let show = |x: &[u8]| crate::fmt::str_inspect(x);
    let after = if b.len() - pos > 5 {
        let mut v = b[pos..pos + 5].to_vec();
        v.extend_from_slice(b"...");
        show(&v)
    } else {
        show(&b[pos..])
    };
    if pos == 0 {
        crate::kernel::new_str_v(format!("#<StringScanner {}/{} @ {}>", pos, b.len(), after))
    } else {
        let before = if pos > 5 {
            let mut v = b"...".to_vec();
            v.extend_from_slice(&b[pos - 5..pos]);
            show(&v)
        } else {
            show(&b[..pos])
        };
        crate::kernel::new_str_v(format!("#<StringScanner {}/{} {} @ {}>", pos, b.len(), before, after))
    }
}

fn init_scanner() {
    let c = new_class("StringScanner", rt().c.object);
    set_global_const("StringScanner", c as V);
    *SCANNER.get() = c;
    let err = new_class("StringScanner::Error", rt().c.standard_error);
    const_set(c, sym("Error"), err as V);
    *SCAN_ERROR.get() = err;
    set_global_const("ScanError", err as V);
    const_set(c, sym("Version"), str_from("3.1.2"));
    const_set(c, sym("Id"), str_from("$Id$"));
    defs(c, "must_C_version", 0, 0, |s, _, _| s);

    defn(c, "initialize", 1, 3, |s, a, _| {
        if !is_str(a[0]) {
            return type_error(&format!("no implicit conversion of {} into String", class_name(real_class(a[0]))));
        }
        ivar_set(s, sym("@__str"), a[0]);
        set_pos(s, 0);
        ivar_set(s, sym("@__prev"), mkfix(0));
        set_match(s, None, QNIL);
        QNIL
    });
    defn(c, "initialize_copy", 1, 1, |s, a, _| {
        for k in ["@__str", "@__pos", "@__prev", "@__m", "@__re"] {
            ivar_set(s, sym(k), ivar_get(a[0], sym(k)));
        }
        s
    });
    defn(c, "fixed_anchor?", 0, 0, |_, _, _| QFALSE);

    defn(c, "scan", 1, 1, |s, a, _| do_scan(s, a[0], true, true, true));
    defn(c, "match?", 1, 1, |s, a, _| do_scan(s, a[0], false, false, true));
    defn(c, "skip", 1, 1, |s, a, _| do_scan(s, a[0], true, false, true));
    defn(c, "check", 1, 1, |s, a, _| do_scan(s, a[0], false, true, true));
    defn(c, "scan_until", 1, 1, |s, a, _| do_scan(s, a[0], true, true, false));
    defn(c, "skip_until", 1, 1, |s, a, _| do_scan(s, a[0], true, false, false));
    defn(c, "check_until", 1, 1, |s, a, _| do_scan(s, a[0], false, true, false));
    defn(c, "exist?", 1, 1, |s, a, _| do_scan(s, a[0], false, false, false));
    defn(c, "search_full", 3, 3, |s, a, _| do_scan(s, a[0], truthy(a[1]), truthy(a[2]), false));
    defn(c, "scan_full", 3, 3, |s, a, _| do_scan(s, a[0], truthy(a[1]), truthy(a[2]), true));

    defn(c, "getch", 0, 0, |s, _, _| {
        let pos = s_pos(s);
        let b = s_bytes(s);
        if pos >= b.len() {
            set_match(s, None, QNIL);
            return QNIL;
        }
        let binary = hdr(s_str(s)).flags & FL_BINARY != 0;
        let n = if binary {
            1
        } else {
            let lead = b[pos];
            let want = match lead {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => 1,
            };
            if pos + want <= b.len() && std::str::from_utf8(&b[pos..pos + want]).is_ok() { want } else { 1 }
        };
        ivar_set(s, sym("@__prev"), mkfix(pos as i64));
        set_match(s, Some(vec![pos as i64, (pos + n) as i64]), QNIL);
        set_pos(s, pos + n);
        sub(s, pos, pos + n)
    });
    fn get_byte(s: V) -> V {
        let pos = s_pos(s);
        if pos >= s_bytes(s).len() {
            set_match(s, None, QNIL);
            return QNIL;
        }
        ivar_set(s, sym("@__prev"), mkfix(pos as i64));
        set_match(s, Some(vec![pos as i64, (pos + 1) as i64]), QNIL);
        set_pos(s, pos + 1);
        sub(s, pos, pos + 1)
    }
    defn(c, "get_byte", 0, 0, |s, _, _| get_byte(s));
    defn(c, "getbyte", 0, 0, |s, _, _| get_byte(s));
    defn(c, "scan_byte", 0, 0, |s, _, _| {
        let pos = s_pos(s);
        let Some(&b) = s_bytes(s).get(pos) else {
            set_match(s, None, QNIL);
            return QNIL;
        };
        get_byte(s);
        mkfix(b as i64)
    });
    defn(c, "peek_byte", 0, 0, |s, _, _| match s_bytes(s).get(s_pos(s)) {
        Some(&b) => mkfix(b as i64),
        None => QNIL,
    });
    fn peek(s: V, a: &[V]) -> V {
        if !is_fix(a[0]) {
            return type_error("no implicit conversion into Integer");
        }
        let n = fix(a[0]);
        if n < 0 {
            return arg_error("negative string size (or size too big)");
        }
        let pos = s_pos(s);
        sub(s, pos, pos.saturating_add(n as usize))
    }
    defn(c, "peek", 1, 1, |s, a, _| peek(s, a));
    defn(c, "peep", 1, 1, |s, a, _| peek(s, a));

    defn(c, "pos", 0, 0, |s, _, _| mkfix(s_pos(s) as i64));
    defn(c, "pointer", 0, 0, |s, _, _| mkfix(s_pos(s) as i64));
    fn set_pos_m(s: V, a: &[V]) -> V {
        if !is_fix(a[0]) {
            return type_error("no implicit conversion into Integer");
        }
        let len = s_bytes(s).len() as i64;
        let mut i = fix(a[0]);
        if i < 0 {
            i += len;
        }
        if i < 0 || i > len {
            return raise(rt().c.range_error, "index out of range");
        }
        set_pos(s, i as usize);
        mkfix(i)
    }
    defn(c, "pos=", 1, 1, |s, a, _| set_pos_m(s, a));
    defn(c, "pointer=", 1, 1, |s, a, _| set_pos_m(s, a));
    defn(c, "charpos", 0, 0, |s, _, _| {
        let pos = s_pos(s);
        mkfix(crate::regex::char_index(s_bytes(s), pos) as i64)
    });
    defn(c, "rest", 0, 0, |s, _, _| {
        let len = s_bytes(s).len();
        sub(s, s_pos(s), len)
    });
    defn(c, "rest_size", 0, 0, |s, _, _| mkfix((s_bytes(s).len() - s_pos(s)) as i64));
    defn(c, "restsize", 0, 0, |s, _, _| mkfix((s_bytes(s).len() - s_pos(s)) as i64));
    defn(c, "rest?", 0, 0, |s, _, _| bool_v(s_pos(s) < s_bytes(s).len()));
    defn(c, "eos?", 0, 0, |s, _, _| bool_v(s_pos(s) >= s_bytes(s).len()));
    defn(c, "empty?", 0, 0, |s, _, _| bool_v(s_pos(s) >= s_bytes(s).len()));
    fn bol(s: V) -> V {
        let pos = s_pos(s);
        bool_v(pos == 0 || s_bytes(s)[pos - 1] == b'\n')
    }
    defn(c, "beginning_of_line?", 0, 0, |s, _, _| bol(s));
    defn(c, "bol?", 0, 0, |s, _, _| bol(s));

    defn(c, "matched?", 0, 0, |s, _, _| bool_v(s_match(s).is_some()));
    defn(c, "matched", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => sub(s, m[0] as usize, m[1] as usize),
        None => QNIL,
    });
    defn(c, "matched_size", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => mkfix(m[1] - m[0]),
        None => QNIL,
    });
    defn(c, "pre_match", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => sub(s, 0, m[0] as usize),
        None => QNIL,
    });
    defn(c, "post_match", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => {
            let len = s_bytes(s).len();
            sub(s, m[1] as usize, len)
        }
        None => QNIL,
    });
    defn(c, "[]", 1, 1, |s, a, _| {
        let Some(m) = s_match(s) else { return QNIL };
        match group_index(s, a[0], &m) {
            Ok(Some(i)) => group(s, &m, i),
            Ok(None) => QNIL,
            Err(e) => e,
        }
    });
    defn(c, "size", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => mkfix((m.len() / 2) as i64),
        None => QNIL,
    });
    defn(c, "captures", 0, 0, |s, _, _| match s_match(s) {
        Some(m) => ary_new((1..m.len() / 2).map(|i| group(s, &m, i)).collect()),
        None => QNIL,
    });
    defn(c, "values_at", 0, -1, |s, a, _| {
        let Some(m) = s_match(s) else { return QNIL };
        let mut out = Vec::new();
        for &k in a {
            match group_index(s, k, &m) {
                Ok(Some(i)) => out.push(group(s, &m, i)),
                Ok(None) => out.push(QNIL),
                Err(e) => return e,
            }
        }
        ary_new(out)
    });
    defn(c, "named_captures", 0, 0, |s, _, _| {
        let h = hash_new();
        let re = ivar_get(s, sym("@__re"));
        if let (Some(m), true) = (s_match(s), is_regexp(re)) {
            for (name, _) in re_of(re).names.iter() {
                let k = str_from(name);
                let v = match group_index(s, k, &m) {
                    Ok(Some(i)) => group(s, &m, i),
                    _ => QNIL,
                };
                crate::hash::hset(h, k, v);
            }
        }
        h
    });

    defn(c, "unscan", 0, 0, |s, _, _| {
        if s_match(s).is_none() {
            return raise(*SCAN_ERROR.get(), "unscan error: not scanned yet");
        }
        let p = ivar_get(s, sym("@__prev"));
        ivar_set(s, sym("@__pos"), p);
        set_match(s, None, QNIL);
        s
    });
    defn(c, "reset", 0, 0, |s, _, _| {
        set_pos(s, 0);
        set_match(s, None, QNIL);
        s
    });
    fn terminate(s: V) -> V {
        let len = s_bytes(s).len();
        set_pos(s, len);
        set_match(s, None, QNIL);
        s
    }
    defn(c, "terminate", 0, 0, |s, _, _| terminate(s));
    defn(c, "clear", 0, 0, |s, _, _| terminate(s));
    defn(c, "string", 0, 0, |s, _, _| s_str(s));
    defn(c, "string=", 1, 1, |s, a, _| {
        if !is_str(a[0]) {
            return type_error(&format!("no implicit conversion of {} into String", class_name(real_class(a[0]))));
        }
        ivar_set(s, sym("@__str"), a[0]);
        set_pos(s, 0);
        set_match(s, None, QNIL);
        a[0]
    });
    fn concat(s: V, a: &[V]) -> V {
        if !is_str(a[0]) {
            return type_error(&format!("no implicit conversion of {} into String", class_name(real_class(a[0]))));
        }
        tri!(check_frozen(s_str(s)));
        let add = rstr(a[0]).b.clone();
        s_bytes(s).extend_from_slice(&add);
        s
    }
    defn(c, "concat", 1, 1, |s, a, _| concat(s, a));
    defn(c, "<<", 1, 1, |s, a, _| concat(s, a));
    defn(c, "inspect", 0, 0, |s, _, _| inspect(s));
}

pub fn init() {
    crate::compat::provide_feature(&["strscan"], Some(init_scanner));
}
