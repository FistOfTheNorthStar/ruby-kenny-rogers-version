//! Regexp and MatchData, backed by Oniguruma with Ruby syntax.
//!
//! `$~` (and `$1`, `$&`, ...) is the last match of the current thread.

use crate::core::*;
use crate::fmt::*;
use crate::gc::RVec;
use crate::kernel::new_str_v;
use crate::tri;
use onig::{Regex, RegexOptions, Region, SearchOptions, Syntax};

pub struct Re {
    pub re: Regex,
    pub source: Vec<u8>,
    pub opts: u32,
    pub names: Vec<(String, Vec<u32>)>,
    /// compiled as ASCII-8BIT (a `/n` regexp with byte escapes): matches the raw bytes
    pub binary: bool,
    /// `/\A[[:space:]]*\z/` (ActiveSupport's String#blank?): `match?` checks the characters
    /// directly
    pub all_space: bool,
}
pub const OPT_I: u32 = 1;
pub const OPT_X: u32 = 2;
pub const OPT_M: u32 = 4;
/// `/n`: no encoding (ASCII-8BIT)
pub const OPT_N: u32 = 32;

static REGEXP: G<*mut Class> = G::new(std::ptr::null_mut());
static MATCHDATA: G<*mut Class> = G::new(std::ptr::null_mut());
static REGEXP_ERROR: G<*mut Class> = G::new(std::ptr::null_mut());

unsafe extern "C" fn free_re(p: *mut std::ffi::c_void) {
    unsafe { drop(Box::from_raw(p as *mut Re)) };
}

pub fn is_regexp(v: V) -> bool {
    is_kind(v, Kind::Data) && real_class(v) == *REGEXP.get()
}
pub fn re_of(v: V) -> &'static Re {
    unsafe { &*((*(v as *mut RData)).data as *const Re) }
}

/// Ruby's `\u{HHHH ...}` / `\uHHHH` escapes, which Oniguruma doesn't parse: like CRuby,
/// ASCII code points become `\xHH` and others the literal UTF-8 character.
fn unescape_unicode(pat: &str) -> std::borrow::Cow<'_, str> {
    if !pat.contains("\\u") {
        return std::borrow::Cow::Borrowed(pat);
    }
    let push = |out: &mut String, cp: u32| match char::from_u32(cp) {
        Some(c) if cp < 0x80 => out.push_str(&format!("\\x{:02X}", c as u32)),
        Some(c) => out.push(c),
        None => out.push_str(&format!("\\x{{{cp:X}}}")),
    };
    let mut out = String::with_capacity(pat.len());
    let mut it = pat.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.peek().map(|&(_, n)| n) {
            Some('u') => {
                let rest = &pat[i + 2..];
                if let Some(body) = rest.strip_prefix('{').and_then(|r| r.split_once('}')).map(|(b, _)| b) {
                    let cps: Option<Vec<u32>> = body.split_whitespace().map(|h| u32::from_str_radix(h, 16).ok()).collect();
                    if let Some(cps) = cps.filter(|v| !v.is_empty()) {
                        cps.into_iter().for_each(|cp| push(&mut out, cp));
                        let skip = 1 + 1 + body.len() + 1; // u { body }
                        for _ in 0..rest[..skip - 1].chars().count() + 1 {
                            it.next();
                        }
                        continue;
                    }
                } else if rest.len() >= 4 && rest[..4].chars().all(|h| h.is_ascii_hexdigit()) {
                    push(&mut out, u32::from_str_radix(&rest[..4], 16).unwrap());
                    for _ in 0..5 {
                        it.next();
                    }
                    continue;
                }
                out.push(c);
            }
            Some(n) => {
                out.push(c);
                out.push(n);
                it.next();
            }
            None => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Rewrites every `(group)?` as `(?:(group)|)` and `(group)??` as `(?:|(group))`.
fn optional_groups_as_alternation(pat: &str) -> String {
    let cs: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 16);
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    let mut class = 0; // nesting depth of [...]
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' {
            out.push(c);
            if let Some(n) = cs.get(i + 1) {
                out.push(*n);
            }
            i += 2;
            continue;
        }
        if class > 0 {
            if c == '[' {
                class += 1;
            } else if c == ']' {
                class -= 1;
            }
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '[' => class = 1,
            '(' => starts.push(out.len()),
            ')' => {
                if let Some(s) = starts.pop() {
                    out.push(')');
                    i += 1;
                    if cs.get(i) == Some(&'?') && cs.get(i + 1) != Some(&'+') {
                        let group = out.split_off(s);
                        if cs.get(i + 1) == Some(&'?') {
                            out.push_str(&format!("(?:|{group})"));
                            i += 2;
                        } else {
                            out.push_str(&format!("(?:{group}|)"));
                            i += 1;
                        }
                    }
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    out
}

pub fn regexp_new(src: &[u8], opts: u32) -> V {
    let raw = String::from_utf8_lossy(src);
    let pat = unescape_unicode(&raw).into_owned();
    let mut o = RegexOptions::REGEX_OPTION_NONE;
    if opts & OPT_I != 0 {
        o |= RegexOptions::REGEX_OPTION_IGNORECASE;
    }
    if opts & OPT_X != 0 {
        o |= RegexOptions::REGEX_OPTION_EXTEND;
    }
    if opts & OPT_M != 0 {
        o |= RegexOptions::REGEX_OPTION_MULTILINE;
    }
    // `/n` with `\x80`-`\xff` escapes: a byte regexp (CRuby makes it ASCII-8BIT); UTF-8
    // Oniguruma would reject the lone bytes as broken multibyte characters
    let binary = opts & OPT_N != 0 && has_high_byte_escape(&pat);
    let mut binary = binary;
    let compiled = if binary {
        Regex::with_options_and_encoding(onig::EncodedBytes::ascii(pat.as_bytes()), o, Syntax::ruby())
    } else {
        match Regex::with_options(&pat, o, Syntax::ruby()) {
            // a byte regexp's source reused without its /n (to_s, Regexp.union, interpolation)
            Err(_) if has_high_byte_escape(&pat) => {
                binary = true;
                Regex::with_options_and_encoding(onig::EncodedBytes::ascii(pat.as_bytes()), o, Syntax::ruby())
            }
            // Oniguruma rejects `?` on a group that can match only an anchor, e.g. `(?:x|$)?`
            // (uglifier); Onigmo accepts it. `(g)?` is `(?:(g)|)`
            Err(e) if e.description().contains("target of repeat operator is invalid") => {
                Regex::with_options(&optional_groups_as_alternation(&pat), o, Syntax::ruby())
            }
            r => r,
        }
    };
    match compiled {
        Ok(re) => {
            let mut names = Vec::new();
            re.foreach_name(|n, g| {
                names.push((n.to_string(), g.to_vec()));
                true
            });
            names.sort_by_key(|(_, g)| g.iter().min().copied().unwrap_or(0));
            let all_space = !binary && opts & (OPT_I | OPT_X | OPT_M) == 0 && src == br"\A[[:space:]]*\z";
            let data = Box::into_raw(Box::new(Re { re, source: src.to_vec(), opts, names, binary, all_space }));
            let v = alloc(RData {
                h: Header { klass: *REGEXP.get(), kind: Kind::Data, flags: FL_FROZEN },
                data: data as *mut std::ffi::c_void,
                dmark: None,
                dfree: Some(free_re),
                roots: Vec::new(),
            });
            v
        }
        Err(e) => raise(*REGEXP_ERROR.get(), &format!("{}: /{}/", e.description(), pat)),
    }
}

/// `\xHH` with HH >= 0x80 in a pattern (outside of `\\` escapes).
fn has_high_byte_escape(pat: &str) -> bool {
    let b = pat.as_bytes();
    let mut i = 0;
    while i + 3 < b.len() {
        if b[i] == b'\\' {
            if b[i + 1] == b'x' && matches!(b[i + 2], b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F') && b[i + 3].is_ascii_hexdigit() {
                return true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

fn text(s: V) -> String {
    String::from_utf8_lossy(&rstr(s).b).into_owned()
}
pub fn char_index(b: &[u8], byte: usize) -> usize {
    let byte = byte.min(b.len());
    if b[..byte].is_ascii() { byte } else { String::from_utf8_lossy(&b[..byte]).chars().count() }
}
fn byte_index(t: &str, ch: usize) -> usize {
    if t.is_ascii() {
        return ch.min(t.len());
    }
    t.char_indices().nth(ch).map(|(i, _)| i).unwrap_or(t.len())
}

/// Search from byte offset `start`; returns group offsets (begin,end pairs; -1 if unset).
pub fn search(re: V, t: &str, start: usize) -> Option<Vec<i64>> {
    let mut region = Region::new();
    if !search_region(re_of(re), t, start, &mut region) {
        return None;
    }
    Some(region_pos(&region))
}

/// Search from byte offset `start` into a caller-owned region (reused across a loop).
fn search_region(r: &Re, t: &str, start: usize, region: &mut Region) -> bool {
    if start > t.len() {
        return false;
    }
    if r.binary {
        r.re.search_with_encoding(onig::EncodedBytes::ascii(t.as_bytes()), start, t.len(), SearchOptions::SEARCH_OPTION_NONE, Some(region)).is_some()
    } else {
        r.re.search_with_options(t, start, t.len(), SearchOptions::SEARCH_OPTION_NONE, Some(region)).is_some()
    }
}

/// Group offsets of a match (begin,end pairs; -1 if the group didn't take part).
fn region_pos(region: &Region) -> Vec<i64> {
    let mut pos = Vec::with_capacity(region.len() * 2);
    for i in 0..region.len() {
        match region.pos(i) {
            Some((b, e)) => {
                pos.push(b as i64);
                pos.push(e as i64);
            }
            None => {
                pos.push(-1);
                pos.push(-1);
            }
        }
    }
    pos
}

/// Whether `re` matches string `s` from character `pos` (negative: from the end), without
/// building match positions or copying the string (Regexp#match?, String#blank? in Rails).
pub fn matches(re: V, s: V, pos: i64) -> bool {
    let b: &[u8] = &rstr(s).b;
    let r = re_of(re);
    let owned;
    let t: &str = match std::str::from_utf8(b) {
        Ok(t) => t,
        Err(_) if r.binary => "",
        Err(_) => {
            owned = String::from_utf8_lossy(b).into_owned();
            &owned
        }
    };
    let len = if r.binary { b.len() } else { t.len() };
    if r.all_space && pos == 0 {
        // [[:space:]] is Unicode White_Space, as char::is_whitespace
        return t.chars().all(char::is_whitespace);
    }
    let start = if pos == 0 {
        0
    } else {
        let nchars = if r.binary { b.len() as i64 } else { t.chars().count() as i64 };
        let p = if pos < 0 { pos + nchars } else { pos };
        if p < 0 || p > nchars {
            return false;
        }
        if r.binary { p as usize } else { byte_index(t, p as usize) }
    };
    if r.binary {
        r.re.search_with_encoding(onig::EncodedBytes::ascii(b), start, len, SearchOptions::SEARCH_OPTION_NONE, None).is_some()
    } else {
        r.re.search_with_options(t, start, len, SearchOptions::SEARCH_OPTION_NONE, None).is_some()
    }
}

fn set_last(m: V) {
    pst().last_match = m;
}
pub fn last_match() -> V {
    pst().last_match
}

fn matchdata_new(re: V, s_bytes: Vec<u8>, pos: Vec<i64>) -> V {
    let m = object_new(*MATCHDATA.get());
    let s = str_new(s_bytes);
    hdr(s).flags |= FL_FROZEN;
    ivar_set(m, sym("@__str"), s);
    ivar_set(m, sym("@__re"), re);
    ivar_set(m, sym("@__pos"), ary_new(pos.into_iter().map(mkfix).collect()));
    m
}
fn md_pos(m: V) -> Vec<i64> {
    rary(ivar_get(m, sym("@__pos"))).v.iter().map(|x| fix(*x)).collect()
}
fn md_str(m: V) -> V {
    ivar_get(m, sym("@__str"))
}
pub fn md_group(m: V, i: usize) -> V {
    let pos = md_pos(m);
    if i * 2 + 1 >= pos.len() || pos[i * 2] < 0 {
        return QNIL;
    }
    let b = &rstr(md_str(m)).b;
    str_new(b[pos[i * 2] as usize..pos[i * 2 + 1] as usize].to_vec())
}
fn md_named_index(m: V, name: &str) -> Option<usize> {
    let re = ivar_get(m, sym("@__re"));
    let pos = md_pos(m);
    let (_, groups) = re_of(re).names.iter().find(|(n, _)| n == name)?;
    // the last group with this name that participated
    groups.iter().rev().map(|g| *g as usize).find(|g| pos.get(g * 2).is_some_and(|b| *b >= 0)).or(groups.last().map(|g| *g as usize))
}

/// Match `re` against string `s` from char position `pos`; sets `$~`.
pub fn do_match(re: V, s: V, pos: usize, set: bool) -> V {
    let t = text(s);
    let start = byte_index(&t, pos);
    match search(re, &t, start) {
        Some(p) => {
            let m = matchdata_new(re, t.into_bytes(), p);
            if set {
                set_last(m);
            }
            m
        }
        None => {
            if set {
                set_last(QNIL);
            }
            QNIL
        }
    }
}

fn to_re(v: V) -> Result<V, V> {
    if is_regexp(v) {
        Ok(v)
    } else if is_str(v) {
        Ok(tri_r(regexp_new(escape(&rstr(v).b).as_bytes(), 0))?)
    } else {
        Err(type_error(&format!("wrong argument type {} (expected Regexp)", class_name(real_class(v)))))
    }
}
fn tri_r(v: V) -> Result<V, V> {
    if v == UNDEF { Err(UNDEF) } else { Ok(v) }
}

pub fn escape(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '.' | '*' | '?' | '+' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '-' | '#' => {
                out.push('\\');
                out.push(c)
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' ' => out.push_str("\\ "),
            c => out.push(c),
        }
    }
    out
}

fn opts_str(o: u32) -> String {
    let mut s = String::new();
    if o & OPT_M != 0 {
        s.push('m');
    }
    if o & OPT_I != 0 {
        s.push('i');
    }
    if o & OPT_X != 0 {
        s.push('x');
    }
    s
}

/// Expand \0 \1 \k<name> \& \` \' \\ in a sub/gsub replacement.
pub fn expand(rep: &[u8], src: &[u8], pos: &[i64], re: V) -> Vec<u8> {
    let group = |i: usize| -> &[u8] {
        if i * 2 + 1 < pos.len() && pos[i * 2] >= 0 { &src[pos[i * 2] as usize..pos[i * 2 + 1] as usize] } else { b"" }
    };
    let mut out = Vec::with_capacity(rep.len());
    let mut i = 0;
    while i < rep.len() {
        let c = rep[i];
        if c == b'\\' && i + 1 < rep.len() {
            let n = rep[i + 1];
            match n {
                b'0'..=b'9' => {
                    out.extend_from_slice(group((n - b'0') as usize));
                    i += 2;
                    continue;
                }
                b'&' => {
                    out.extend_from_slice(group(0));
                    i += 2;
                    continue;
                }
                b'`' => {
                    out.extend_from_slice(&src[..pos[0] as usize]);
                    i += 2;
                    continue;
                }
                b'\'' => {
                    out.extend_from_slice(&src[pos[1] as usize..]);
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                b'k' if rep.get(i + 2) == Some(&b'<') => {
                    if let Some(end) = rep[i + 3..].iter().position(|c| *c == b'>') {
                        let name = String::from_utf8_lossy(&rep[i + 3..i + 3 + end]).into_owned();
                        if let Some((_, g)) = is_regexp(re).then(|| re_of(re).names.iter().find(|(nm, _)| *nm == name)).flatten() {
                            if let Some(gi) = g.last() {
                                out.extend_from_slice(group(*gi as usize));
                            }
                        }
                        i += 4 + end;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// String#sub/gsub with a Regexp pattern. Ok(None) = no change.
pub fn sub(s: V, re: V, rep: Option<V>, blk: V, global: bool) -> Result<Option<Vec<u8>>, V> {
    let t = text(s);
    let r = re_of(re);
    // a String replacement is converted once, and expanded per match only if it has `\`
    let rep_bytes: Option<Vec<u8>> = match rep {
        Some(x) if !is_hash(x) => Some(to_s(x).map_err(|_| UNDEF)?),
        _ => None,
    };
    let rep_plain = rep_bytes.as_ref().is_some_and(|b| !b.contains(&b'\\'));
    let hash = rep.filter(|x| is_hash(*x));
    // a Hash without a default proc gives the same value for the same key: look each distinct
    // match up once (URI encoding maps every byte through a table)
    let cacheable = hash.is_some_and(|h| rhash(h).default_proc == QNIL);
    let mut cache: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut region = Region::new();
    let mut out = Vec::with_capacity(t.len());
    let mut start = 0usize;
    let mut last_end = 0usize;
    let mut last_begin: Option<usize> = None;
    while start <= t.len() {
        if !search_region(r, &t, start, &mut region) {
            break;
        }
        let src = t.as_bytes();
        let (b, e) = region.pos(0).unwrap_or((start, start));
        out.extend_from_slice(&src[last_end..b]);
        if let Some(h) = hash {
            let matched = &src[b..e];
            match cache.iter().find(|(k, _)| k[..] == *matched) {
                Some((_, piece)) => out.extend_from_slice(piece),
                None => {
                    let v = crate::hash::hget(h, str_new(matched.to_vec()));
                    if v == UNDEF {
                        return Err(UNDEF);
                    }
                    let piece = to_s(v).map_err(|_| UNDEF)?;
                    out.extend_from_slice(&piece);
                    if cacheable && cache.len() < 64 {
                        cache.push((matched.to_vec(), piece));
                    }
                }
            }
        } else if let Some(rb) = &rep_bytes {
            if rep_plain {
                out.extend_from_slice(rb);
            } else {
                out.extend_from_slice(&expand(rb, src, &region_pos(&region), re));
            }
        } else {
            if !is_proc(blk) {
                return Err(arity_error(1, 2, 2));
            }
            let m = matchdata_new(re, src.to_vec(), region_pos(&region));
            set_last(m);
            let v = call_block(blk, &[str_new(src[b..e].to_vec())]);
            if v == UNDEF {
                return Err(UNDEF);
            }
            out.extend_from_slice(&to_s(v).map_err(|_| UNDEF)?);
        }
        last_begin = Some(b);
        last_end = e;
        if e == b {
            // empty match: copy one char and move on
            if e < src.len() {
                let ch_len = t[e..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                out.extend_from_slice(&src[e..e + ch_len]);
                last_end = e + ch_len;
                start = e + ch_len;
            } else {
                start = e + 1;
            }
        } else {
            start = e;
        }
        if !global {
            break;
        }
    }
    let Some(last_begin) = last_begin else {
        set_last(QNIL);
        return Ok(None);
    };
    if last_end < t.len() {
        out.extend_from_slice(&t.as_bytes()[last_end..]);
    }
    if rep.is_some() {
        // $~ is the last match (as in CRuby): found again from where it began
        if let Some(pos) = search(re, &t, last_begin) {
            set_last(matchdata_new(re, t.into_bytes(), pos));
        }
    }
    Ok(Some(out))
}

pub fn scan(s: V, re: V, blk: V) -> V {
    let t = text(s);
    let src = t.as_bytes().to_vec();
    let mut out = RVec::new();
    let mut start = 0;
    while start <= src.len() {
        let Some(pos) = search(re, &t, start) else { break };
        let (b, e) = (pos[0] as usize, pos[1] as usize);
        let item = if pos.len() > 2 {
            let groups: Vec<V> = (1..pos.len() / 2)
                .map(|i| if pos[i * 2] >= 0 { str_new(src[pos[i * 2] as usize..pos[i * 2 + 1] as usize].to_vec()) } else { QNIL })
                .collect();
            ary_new(groups)
        } else {
            str_new(src[b..e].to_vec())
        };
        let m = matchdata_new(re, src.clone(), pos);
        set_last(m);
        if is_proc(blk) {
            tri!(call_block(blk, &[item]));
        } else {
            out.push(item);
        }
        start = if e == b { e + t[e..].chars().next().map(|c| c.len_utf8()).unwrap_or(1) } else { e };
    }
    if is_proc(blk) { s } else { ary_new(out.take()) }
}

pub fn split(s: V, re: V, limit: i64) -> V {
    let t = text(s);
    let src = t.as_bytes();
    let mut parts: Vec<V> = Vec::new();
    let mut start = 0;
    let mut field_start = 0;
    if src.is_empty() {
        return ary_new(vec![]);
    }
    while start <= src.len() {
        if limit > 0 && parts.len() as i64 >= limit - 1 {
            break;
        }
        let Some(pos) = search(re, &t, start) else { break };
        let (b, e) = (pos[0] as usize, pos[1] as usize);
        if e == b {
            if b >= src.len() {
                break;
            }
            if b == field_start {
                start = b + t[b..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                if b == 0 {
                    continue;
                }
                // split between chars
                parts.push(str_new(src[field_start..b].to_vec()));
                field_start = b;
                continue;
            }
        }
        parts.push(str_new(src[field_start..b].to_vec()));
        for i in 1..pos.len() / 2 {
            if pos[i * 2] >= 0 {
                parts.push(str_new(src[pos[i * 2] as usize..pos[i * 2 + 1] as usize].to_vec()));
            }
        }
        field_start = e;
        start = if e == b { e + t[e..].chars().next().map(|c| c.len_utf8()).unwrap_or(1) } else { e };
    }
    parts.push(str_new(src[field_start..].to_vec()));
    if limit == 0 {
        while parts.last().is_some_and(|p| rstr(*p).b.is_empty()) {
            parts.pop();
        }
    }
    ary_new(parts)
}

/// String#=~ / Regexp#=~ : char index of the match or nil.
pub fn match_index(re: V, s: V) -> V {
    if s == QNIL {
        set_last(QNIL);
        return QNIL;
    }
    let m = do_match(re, s, 0, true);
    if m == QNIL {
        return QNIL;
    }
    let pos = md_pos(m);
    mkfix(char_index(&rstr(md_str(m)).b, pos[0] as usize) as i64)
}

pub fn str_index(s: V, re: V, from: usize) -> V {
    let m = do_match(re, s, from, true);
    if m == QNIL {
        return QNIL;
    }
    mkfix(char_index(&rstr(md_str(m)).b, md_pos(m)[0] as usize) as i64)
}

pub fn str_aref(s: V, re: V, cap: Option<V>) -> V {
    let m = do_match(re, s, 0, true);
    if m == QNIL {
        return QNIL;
    }
    match cap {
        None => md_group(m, 0),
        Some(c) => md_aref(m, c),
    }
}

pub fn partition(s: V, re: V, last: bool) -> V {
    let t = text(s);
    let src = t.as_bytes();
    let found = if last {
        let mut best = None;
        let mut start = 0;
        while let Some(p) = search(re, &t, start) {
            start = if p[1] > p[0] { p[1] as usize } else { p[1] as usize + 1 };
            best = Some(p);
            if start > src.len() {
                break;
            }
        }
        best
    } else {
        search(re, &t, 0)
    };
    match found {
        Some(p) => {
            let (b, e) = (p[0] as usize, p[1] as usize);
            ary_new(vec![str_new(src[..b].to_vec()), str_new(src[b..e].to_vec()), str_new(src[e..].to_vec())])
        }
        None if last => ary_new(vec![str_new(vec![]), str_new(vec![]), str_new(src.to_vec())]),
        None => ary_new(vec![str_new(src.to_vec()), str_new(vec![]), str_new(vec![])]),
    }
}

fn md_aref(m: V, k: V) -> V {
    if is_range(k) {
        let all = ary_new((0..md_pos(m).len() / 2).map(|i| md_group(m, i)).collect());
        return crate::array::aref(all, &[k]);
    }
    if is_fix(k) {
        let n = md_pos(m).len() as i64 / 2;
        let i = fix(k);
        let i = if i < 0 { i + n } else { i };
        if i < 0 {
            return QNIL;
        }
        return md_group(m, i as usize);
    }
    let name = if is_sym(k) {
        sym_str(sym_id(k))
    } else if is_str(k) {
        String::from_utf8_lossy(&rstr(k).b).into_owned()
    } else {
        return type_error("no implicit conversion into Integer");
    };
    match md_named_index(m, &name) {
        Some(i) => md_group(m, i),
        None => raise(rt().c.index_error, &format!("undefined group name reference: {name}")),
    }
}

// ---------- ABI for compiled code ----------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kn_regexp_lit(slot: *mut V, ptr: *const u8, len: u64, opts: u64) -> V {
    let s = unsafe { &mut *slot };
    if *s != 0 {
        return *s;
    }
    let src = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    let v = regexp_new(src, opts as u32);
    if v != UNDEF {
        crate::gc::pin(v);
        *s = v;
    }
    v
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_regexp_new(s: V, opts: u64) -> V {
    regexp_new(&rstr(s).b.clone(), opts as u32)
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_last_match() -> V {
    last_match()
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_nth_ref(n: u64) -> V {
    let m = last_match();
    if m == QNIL { QNIL } else { md_group(m, n as usize) }
}
#[unsafe(no_mangle)]
pub extern "C" fn kn_backref(c: u64) -> V {
    let m = last_match();
    if m == QNIL {
        return QNIL;
    }
    let pos = md_pos(m);
    let b = rstr(md_str(m)).b.clone();
    match c as u8 {
        b'&' => md_group(m, 0),
        b'`' => str_new(b[..pos[0] as usize].to_vec()),
        b'\'' => str_new(b[pos[1] as usize..].to_vec()),
        _ => {
            // $+ : last matched group
            let n = pos.len() / 2;
            (1..n).rev().find(|i| pos[i * 2] >= 0).map(|i| md_group(m, i)).unwrap_or(QNIL)
        }
    }
}
/// Named capture after `/(?<x>..)/ =~ str` (assigns local x).
#[unsafe(no_mangle)]
pub extern "C" fn kn_match_named(name: V) -> V {
    let m = last_match();
    if m == QNIL {
        return QNIL;
    }
    match md_named_index(m, &sym_str(sym_id(name))) {
        Some(i) => md_group(m, i),
        None => QNIL,
    }
}

pub fn static_roots(out: &mut Vec<V>) {
    out.push(pst().last_match);
}

pub fn init() {
    crate::gc::heap().extra_roots.push(static_roots);
    let o = rt().c.object;
    let rc = new_class("Regexp", o);
    set_global_const("Regexp", rc as V);
    *REGEXP.get() = rc;
    const_set(rc, sym("IGNORECASE"), mkfix(OPT_I as i64));
    const_set(rc, sym("EXTENDED"), mkfix(OPT_X as i64));
    const_set(rc, sym("MULTILINE"), mkfix(OPT_M as i64));
    const_set(rc, sym("FIXEDENCODING"), mkfix(16));
    const_set(rc, sym("NOENCODING"), mkfix(32));
    let re_err = new_class("RegexpError", rt().c.standard_error);
    set_global_const("RegexpError", re_err as V);
    *REGEXP_ERROR.get() = re_err;
    let md = new_class("MatchData", o);
    set_global_const("MatchData", md as V);
    *MATCHDATA.get() = md;

    fn new_re(a: &[V]) -> V {
        let src = a[0];
        if is_regexp(src) {
            return src;
        }
        if !is_str(src) {
            return type_error("no implicit conversion into String");
        }
        let opts = match a.get(1) {
            Some(&x) if is_fix(x) => fix(x) as u32,
            Some(&x) if is_str(x) => {
                let b = rstr(x).b.clone();
                (if b.contains(&b'i') { OPT_I } else { 0 }) | (if b.contains(&b'x') { OPT_X } else { 0 }) | (if b.contains(&b'm') { OPT_M } else { 0 })
            }
            Some(&x) if truthy(x) => OPT_I,
            _ => 0,
        };
        regexp_new(&rstr(src).b.clone(), opts)
    }
    defs(rc, "new", 1, 3, |_, a, _| new_re(a));
    defs(rc, "compile", 1, 3, |_, a, _| new_re(a));
    defs(rc, "escape", 1, 1, |_, a, _| match crate::fmt::to_s(a[0]) {
        Ok(b) => new_str_v(escape(&b)),
        Err(()) => UNDEF,
    });
    defs(rc, "quote", 1, 1, |_, a, _| match crate::fmt::to_s(a[0]) {
        Ok(b) => new_str_v(escape(&b)),
        Err(()) => UNDEF,
    });
    defs(rc, "union", 0, -1, |_, a, _| {
        let items: Vec<V> = if a.len() == 1 && is_ary(a[0]) { rary(a[0]).v.clone() } else { a.to_vec() };
        let parts: Vec<String> = items
            .iter()
            .map(|x| {
                if is_regexp(*x) {
                    let r = re_of(*x);
                    format!("(?{}-mix:{})", opts_str(r.opts), String::from_utf8_lossy(&r.source))
                } else {
                    escape(&crate::fmt::to_s(*x).unwrap_or_default())
                }
            })
            .collect();
        regexp_new(parts.join("|").as_bytes(), 0)
    });
    defs(rc, "last_match", 0, 1, |_, a, _| {
        let m = last_match();
        match a.first() {
            Some(&k) if m != QNIL => md_aref(m, k),
            _ => m,
        }
    });
    defn(rc, "source", 0, 0, |s, _, _| str_new(re_of(s).source.clone()));
    defn(rc, "to_s", 0, 0, |s, _, _| {
        let r = re_of(s);
        let on = opts_str(r.opts);
        let off: String = "mix".chars().filter(|c| !on.contains(*c)).collect();
        new_str_v(format!("(?{}{}{}:{})", on, if off.is_empty() { "" } else { "-" }, off, String::from_utf8_lossy(&r.source)))
    });
    defn(rc, "inspect", 0, 0, |s, _, _| {
        let r = re_of(s);
        new_str_v(format!("/{}/{}", String::from_utf8_lossy(&r.source), opts_str(r.opts)))
    });
    defn(rc, "options", 0, 0, |s, _, _| mkfix(re_of(s).opts as i64));
    defn(rc, "casefold?", 0, 0, |s, _, _| bool_v(re_of(s).opts & OPT_I != 0));
    defn(rc, "names", 0, 0, |s, _, _| ary_new(re_of(s).names.iter().map(|(n, _)| str_from(n)).collect()));
    defn(rc, "=~", 1, 1, |s, a, _| {
        if a[0] == QNIL {
            return QNIL;
        }
        if is_sym(a[0]) {
            return match_index(s, str_from(&sym_str(sym_id(a[0]))));
        }
        if !is_str(a[0]) {
            return type_error("no implicit conversion into String");
        }
        match_index(s, a[0])
    });
    defn(rc, "match", 1, 2, |s, a, b| {
        if a[0] == QNIL {
            pst().last_match = QNIL;
            return QNIL;
        }
        let pos = a.get(1).map(|x| fix(*x).max(0) as usize).unwrap_or(0);
        let target = if is_sym(a[0]) { str_from(&sym_str(sym_id(a[0]))) } else { a[0] };
        let m = do_match(s, target, pos, true);
        if m != QNIL && is_proc(b) { call_block(b, &[m]) } else { m }
    });
    defn(rc, "match?", 1, 2, |s, a, _| {
        if a[0] == QNIL {
            return QFALSE;
        }
        let target = if is_sym(a[0]) { str_from(&sym_str(sym_id(a[0]))) } else { a[0] };
        if !is_str(target) {
            return QFALSE;
        }
        let pos = match a.get(1) {
            Some(&p) if is_fix(p) => fix(p),
            _ => 0,
        };
        bool_v(matches(s, target, pos))
    });
    defn(rc, "===", 1, 1, |s, a, _| {
        let target = if is_sym(a[0]) { str_from(&sym_str(sym_id(a[0]))) } else { a[0] };
        if !is_str(target) {
            return QFALSE;
        }
        bool_v(do_match(s, target, 0, true) != QNIL)
    });
    defn(rc, "==", 1, 1, |s, a, _| bool_v(is_regexp(a[0]) && re_of(s).source == re_of(a[0]).source && re_of(s).opts == re_of(a[0]).opts));
    defn(rc, "eql?", 1, 1, |s, a, _| bool_v(is_regexp(a[0]) && re_of(s).source == re_of(a[0]).source && re_of(s).opts == re_of(a[0]).opts));
    defn(rc, "hash", 0, 0, |s, _, _| {
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        re_of(s).source.hash(&mut h);
        mkfix((h.finish() >> 2) as i64)
    });
    defn(rc, "freeze", 0, 0, |s, _, _| s);
    defn(rc, "named_captures", 0, 0, |s, _, _| {
        let h = hash_new();
        for (n, g) in &re_of(s).names {
            crate::hash::hset(h, str_from(n), ary_new(g.iter().map(|x| mkfix(*x as i64)).collect()));
        }
        h
    });

    // ---- MatchData ----
    defn(md, "[]", 1, 2, |s, a, _| {
        if a.len() == 2 {
            let all = ary_new((0..md_pos(s).len() / 2).map(|i| md_group(s, i)).collect());
            return crate::array::aref(all, a);
        }
        md_aref(s, a[0])
    });
    defn(md, "to_a", 0, 0, |s, _, _| ary_new((0..md_pos(s).len() / 2).map(|i| md_group(s, i)).collect()));
    defn(md, "deconstruct", 0, 0, |s, _, _| ary_new((1..md_pos(s).len() / 2).map(|i| md_group(s, i)).collect()));
    defn(md, "captures", 0, 0, |s, _, _| ary_new((1..md_pos(s).len() / 2).map(|i| md_group(s, i)).collect()));
    defn(md, "named_captures", 0, 1, |s, _, _| {
        let h = hash_new();
        let re = ivar_get(s, sym("@__re"));
        for (n, _) in re_of(re).names.clone() {
            let v = md_named_index(s, &n).map(|i| md_group(s, i)).unwrap_or(QNIL);
            crate::hash::hset(h, str_from(&n), v);
        }
        h
    });
    defn(md, "deconstruct_keys", 1, 1, |s, _, _| {
        let h = hash_new();
        let re = ivar_get(s, sym("@__re"));
        for (n, _) in re_of(re).names.clone() {
            let v = md_named_index(s, &n).map(|i| md_group(s, i)).unwrap_or(QNIL);
            crate::hash::hset(h, mksym(sym(&n)), v);
        }
        h
    });
    defn(md, "names", 0, 0, |s, _, _| {
        let re = ivar_get(s, sym("@__re"));
        ary_new(re_of(re).names.iter().map(|(n, _)| str_from(n)).collect())
    });
    defn(md, "pre_match", 0, 0, |s, _, _| str_new(rstr(md_str(s)).b[..md_pos(s)[0] as usize].to_vec()));
    defn(md, "post_match", 0, 0, |s, _, _| str_new(rstr(md_str(s)).b[md_pos(s)[1] as usize..].to_vec()));
    fn group_index(s: V, k: V) -> Option<usize> {
        if is_fix(k) {
            Some(fix(k) as usize)
        } else if is_sym(k) || is_str(k) {
            let n = if is_sym(k) { sym_str(sym_id(k)) } else { String::from_utf8_lossy(&rstr(k).b).into_owned() };
            md_named_index(s, &n)
        } else {
            None
        }
    }
    defn(md, "begin", 1, 1, |s, a, _| match group_index(s, a[0]) {
        Some(i) if md_pos(s).get(i * 2).is_some_and(|b| *b >= 0) => mkfix(char_index(&rstr(md_str(s)).b, md_pos(s)[i * 2] as usize) as i64),
        _ => QNIL,
    });
    defn(md, "end", 1, 1, |s, a, _| match group_index(s, a[0]) {
        Some(i) if md_pos(s).get(i * 2).is_some_and(|b| *b >= 0) => mkfix(char_index(&rstr(md_str(s)).b, md_pos(s)[i * 2 + 1] as usize) as i64),
        _ => QNIL,
    });
    defn(md, "offset", 1, 1, |s, a, _| {
        let b = tri!(send(s, sym("begin"), a, QNIL));
        let e = tri!(send(s, sym("end"), a, QNIL));
        ary_new(vec![b, e])
    });
    defn(md, "size", 0, 0, |s, _, _| mkfix(md_pos(s).len() as i64 / 2));
    defn(md, "length", 0, 0, |s, _, _| mkfix(md_pos(s).len() as i64 / 2));
    defn(md, "to_s", 0, 0, |s, _, _| md_group(s, 0));
    defn(md, "string", 0, 0, |s, _, _| md_str(s));
    defn(md, "regexp", 0, 0, |s, _, _| ivar_get(s, sym("@__re")));
    defn(md, "values_at", 0, -1, |s, a, _| ary_new(a.iter().map(|k| md_aref(s, *k)).collect()));
    defn(md, "inspect", 0, 0, |s, _, _| {
        let n = md_pos(s).len() / 2;
        let re = ivar_get(s, sym("@__re"));
        let names = &re_of(re).names;
        let mut out = format!("#<MatchData {}", inspect_lossy(md_group(s, 0)));
        for i in 1..n {
            let label = names.iter().find(|(_, g)| g.contains(&(i as u32))).map(|(nm, _)| nm.clone()).unwrap_or(i.to_string());
            out.push_str(&format!(" {}:{}", label, inspect_lossy(md_group(s, i))));
        }
        out.push('>');
        new_str_v(out)
    });
    defn(md, "==", 1, 1, |s, a, _| bool_v(s == a[0]));

    // Kernel#!~
    defn(rt().c.kernel, "!~", 1, 1, |s, a, _| {
        let r = tri!(send1(s, "=~", a[0]));
        bool_v(!truthy(r))
    });
    let _ = to_re;
}

pub fn pattern_arg(v: V) -> Result<V, V> {
    to_re(v)
}
