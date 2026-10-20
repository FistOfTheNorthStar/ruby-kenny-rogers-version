//! Marshal: CRuby's binary serialization format 4.8, byte-compatible with `marshal.c`.
//!
//! Dump keeps an identity table of every object written (`@` links preserve sharing and
//! cycles) and a separate symbol table (`;` links). Like CRuby, a Fixnum outside 31 bits is
//! written as a Bignum and takes an object index, and Floats that CRuby keeps as immediate
//! "flonums" are linked by value rather than identity, so indices line up with CRuby's.
//!
//! Native types: Range is written as an Object with `excl`/`begin`/`end` ivars, Rational and
//! Complex as `U` with `[a, b]`, Date/DateTime as `U` with CRuby's 6-element array, Exceptions
//! as Objects with `mesg`/`bt`, and Time through `Time#_dump`/`Time._load` (defined here, with
//! CRuby's packed 8-byte layout plus `nano_num`/`nano_den`/`submicro`/`offset`/`zone`).
//! Hidden runtime ivars (`@__...`) are never written.

use crate::compat::FL_BINARY;
use crate::core::*;
use crate::gc::RVec;
use crate::tri;
use num_bigint::{BigInt, Sign};
use std::collections::HashMap;

type R<T> = Result<T, V>;

const FL_USASCII: u32 = FL_BINARY << 1;
const NS: i128 = 1_000_000_000;

#[inline]
fn chk(v: V) -> R<V> {
    if v == UNDEF { Err(UNDEF) } else { Ok(v) }
}
fn terr<T>(msg: &str) -> R<T> {
    Err(type_error(msg))
}
fn aerr<T>(msg: &str) -> R<T> {
    Err(arg_error(msg))
}
fn global_class(name: &str) -> *mut Class {
    match unsafe { (*rt().c.object).consts.get(&sym(name)) } {
        Some(&v) if is_class(v) => v as *mut Class,
        _ => std::ptr::null_mut(),
    }
}
fn class_le(mut k: *mut Class, base: *mut Class) -> bool {
    if base.is_null() {
        return false;
    }
    unsafe {
        while !k.is_null() {
            if k == base {
                return true;
            }
            k = (*k).superclass;
        }
    }
    false
}
fn is_a_named(v: V, name: &str) -> bool {
    let c = global_class(name);
    !c.is_null() && is_a(v, c)
}
fn has_method(v: V, name: &str) -> bool {
    !find_method(class_of(v), sym(name)).is_null()
}
fn regexp_class() -> *mut Class {
    global_class("Regexp")
}
fn is_regexp_obj(v: V) -> bool {
    is_kind(v, Kind::Data) && class_le(real_class(v), regexp_class())
}
/// Generic (non-Object) ivars that Marshal writes: everything but hidden `@__` ones, in a
/// stable order (Time's internal ivars last, in CRuby's insertion order).
fn generic_ivars(v: V) -> Vec<(u32, V)> {
    let mut l: Vec<(u32, V)> = ivar_list(v).into_iter().filter(|(n, _)| !sym_name(*n).starts_with(b"@__")).collect();
    let order = ["nano_num", "nano_den", "submicro", "offset", "zone"];
    let rank = |id: u32| order.iter().position(|o| sym_name(id) == o.as_bytes()).map(|p| p + 1).unwrap_or(0);
    l.sort_by_key(|(n, _)| (rank(*n), *n));
    l
}

/// Float text the way CRuby's `w_float` writes it (shortest round-trip digits).
fn float_repr(d: f64) -> String {
    if d.is_infinite() {
        return if d < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if d.is_nan() {
        return "nan".into();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let e = format!("{:e}", d.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let decpt: i64 = exp.parse::<i64>().unwrap() + 1;
    let digs = digits.len() as i64;
    let mut out = String::new();
    if d < 0.0 {
        out.push('-');
    }
    if decpt < -3 || decpt > digs {
        out.push_str(&digits[..1]);
        if digs > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push_str(&format!("e{}", decpt - 1));
    } else if decpt > 0 {
        out.push_str(&digits[..decpt as usize]);
        if digs > decpt {
            out.push('.');
            out.push_str(&digits[decpt as usize..]);
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-decpt) {
            out.push('0');
        }
        out.push_str(&digits);
    }
    out
}

// =====================================================================================
// dump
// =====================================================================================

struct Dumper {
    out: Vec<u8>,
    syms: HashMap<u32, usize>,
    objs: HashMap<V, usize>,
    flonums: HashMap<u64, usize>,
    nobj: usize,
    /// Keeps every registered object (and `marshal_dump`/`_dump` results) alive, so an
    /// address in `objs` is never reused by a new object during the dump.
    keep: RVec,
}

impl Dumper {
    fn byte(&mut self, b: u8) {
        self.out.push(b);
    }
    fn long(&mut self, x: i64) {
        if x == 0 {
            self.out.push(0);
        } else if 0 < x && x < 123 {
            self.out.push((x + 5) as u8);
        } else if -124 < x && x < 0 {
            self.out.push(((x - 5) & 0xff) as u8);
        } else {
            let mut buf = [0u8; 9];
            let mut x = x;
            for i in 1..9 {
                buf[i] = (x & 0xff) as u8;
                x >>= 8;
                if x == 0 {
                    buf[0] = i as u8;
                    self.out.extend_from_slice(&buf[..=i]);
                    return;
                }
                if x == -1 {
                    buf[0] = (-(i as i64)) as u8;
                    self.out.extend_from_slice(&buf[..=i]);
                    return;
                }
            }
        }
    }
    fn bytes(&mut self, b: &[u8]) {
        self.long(b.len() as i64);
        self.out.extend_from_slice(b);
    }
    fn symbol(&mut self, id: u32) {
        if let Some(&i) = self.syms.get(&id) {
            self.byte(b';');
            self.long(i as i64);
            return;
        }
        let name = sym_name(id);
        let enc = !name.is_ascii();
        if enc {
            self.byte(b'I');
        }
        self.byte(b':');
        self.bytes(name);
        let n = self.syms.len();
        self.syms.insert(id, n);
        if enc {
            self.long(1);
            self.symbol(sym("E"));
            self.byte(b'T');
        }
    }
    fn remember(&mut self, v: V) {
        let n = self.nobj;
        self.objs.insert(v, n);
        self.nobj += 1;
        self.keep.push(v);
    }
    fn class_path(&mut self, c: *mut Class) -> R<u32> {
        let name = unsafe { (*c).name.clone() };
        if name.is_empty() || name.starts_with("#<") {
            return terr(&format!("can't dump anonymous {} {}", if unsafe { (*c).is_module } { "module" } else { "class" }, class_name(c)));
        }
        Ok(sym(&name))
    }
    /// `e` records for modules an object was extended with.
    fn extended(&mut self, klass: *mut Class, check: bool) -> R<()> {
        unsafe {
            if !(*klass).is_singleton {
                return Ok(());
            }
            if check && (!(*klass).methods.is_empty() || !(*klass).civars.is_empty() || !(*klass).prepends.is_empty()) {
                return terr("singleton can't be dumped");
            }
        }
        let anc = ancestors(klass);
        let pos = anc.iter().position(|&k| k == klass).unwrap_or(0);
        for &m in &anc[pos + 1..] {
            if !unsafe { (*m).is_module } {
                break;
            }
            self.byte(b'e');
            let p = self.class_path(m)?;
            self.symbol(p);
        }
        Ok(())
    }
    fn class_rec(&mut self, ty: u8, v: V, check: bool) -> R<()> {
        self.extended(class_of(v), check)?;
        self.byte(ty);
        let p = self.class_path(real_class(v))?;
        self.symbol(p);
        Ok(())
    }
    fn uclass(&mut self, v: V, base: *mut Class) -> R<()> {
        self.extended(class_of(v), true)?;
        let k = real_class(v);
        if k != base {
            self.byte(b'C');
            let p = self.class_path(k)?;
            self.symbol(p);
        }
        Ok(())
    }
    /// The `I` ivar block: encoding first (`E`), then the ivars.
    fn ivars(&mut self, enc: Option<bool>, iv: &[(u32, V)], limit: i64) -> R<()> {
        self.long(iv.len() as i64 + enc.is_some() as i64);
        if let Some(e) = enc {
            self.symbol(sym("E"));
            self.byte(if e { b'T' } else { b'F' });
        }
        for &(n, x) in iv {
            self.symbol(n);
            self.object(x, limit)?;
        }
        Ok(())
    }
    fn bignum(&mut self, n: &BigInt) {
        self.byte(b'l');
        let (sign, mut mag) = n.to_bytes_le();
        self.byte(if sign == Sign::Minus { b'-' } else { b'+' });
        if mag.len() % 2 == 1 {
            mag.push(0);
        }
        self.long(mag.len() as i64 / 2);
        self.out.extend_from_slice(&mag);
    }

    fn object(&mut self, v: V, limit: i64) -> R<()> {
        if limit == 0 {
            return aerr("exceed depth limit");
        }
        match v {
            QNIL => return Ok(self.byte(b'0')),
            QTRUE => return Ok(self.byte(b'T')),
            QFALSE => return Ok(self.byte(b'F')),
            _ => {}
        }
        if is_fix(v) {
            let i = fix(v);
            if (-(1i64 << 30)..(1i64 << 30)).contains(&i) {
                self.byte(b'i');
                self.long(i);
            } else {
                // CRuby converts to a (fresh, never linked) Bignum, which takes an index.
                self.nobj += 1;
                self.bignum(&BigInt::from(i));
            }
            return Ok(());
        }
        if is_sym(v) {
            self.symbol(sym_id(v));
            return Ok(());
        }
        let lim = if limit > 0 { limit - 1 } else { limit };
        // Floats CRuby keeps as flonums are linked by value (whether Kenny boxed them or not)
        if is_float(v) && f64_to_flonum(float_val(v)).is_some() {
            let bits = float_val(v).to_bits();
            if let Some(&i) = self.flonums.get(&bits) {
                self.byte(b'@');
                self.long(i as i64);
                return Ok(());
            }
            self.flonums.insert(bits, self.nobj);
            self.nobj += 1;
            self.keep.push(v);
            self.byte(b'f');
            self.bytes(float_repr(float_val(v)).as_bytes());
            return Ok(());
        }
        if !is_heap(v) {
            return terr(&format!("can't dump {}", class_name(real_class(v))));
        }
        if let Some(&i) = self.objs.get(&v) {
            self.byte(b'@');
            self.long(i as i64);
            return Ok(());
        }
        let k = kind(v).unwrap();
        if k != Kind::Class {
            if has_method(v, "marshal_dump") {
                self.remember(v);
                let r = chk(send0(v, "marshal_dump"))?;
                self.keep.push(r);
                self.class_rec(b'U', v, false)?;
                return self.object(r, lim);
            }
            if has_method(v, "_dump") {
                let r = chk(send(v, sym("_dump"), &[mkfix(limit)], QNIL))?;
                if !is_str(r) {
                    return terr("_dump() must return string");
                }
                self.keep.push(r);
                let enc = str_enc(r);
                let iv = generic_ivars(r);
                let hasiv = enc.is_some() || !iv.is_empty();
                if hasiv {
                    self.byte(b'I');
                }
                self.class_rec(b'u', v, false)?;
                let b = rstr(r).b.clone();
                self.bytes(&b);
                if hasiv {
                    self.ivars(enc, &iv, lim)?;
                }
                self.remember(v);
                return Ok(());
            }
        }
        self.remember(v);
        match k {
            Kind::Class => {
                let c = v as *mut Class;
                if unsafe { (*c).is_singleton } {
                    return terr("singleton class can't be dumped");
                }
                let p = self.class_path(c)?;
                self.byte(if unsafe { (*c).is_module } { b'm' } else { b'c' });
                self.bytes(sym_name(p));
            }
            Kind::Float => {
                self.byte(b'f');
                self.bytes(float_repr(float_val(v)).as_bytes());
            }
            Kind::Bignum => {
                let n = rbig(v).n.clone();
                let iv = generic_ivars(v);
                if !iv.is_empty() {
                    self.byte(b'I');
                }
                self.bignum(&n);
                if !iv.is_empty() {
                    self.ivars(None, &iv, lim)?;
                }
            }
            Kind::String => {
                let enc = str_enc(v);
                let iv = generic_ivars(v);
                let hasiv = enc.is_some() || !iv.is_empty();
                if hasiv {
                    self.byte(b'I');
                }
                self.uclass(v, rt().c.string)?;
                self.byte(b'"');
                let b = rstr(v).b.clone();
                self.bytes(&b);
                if hasiv {
                    self.ivars(enc, &iv, lim)?;
                }
            }
            Kind::Array => {
                let iv = generic_ivars(v);
                if !iv.is_empty() {
                    self.byte(b'I');
                }
                self.uclass(v, rt().c.array)?;
                self.byte(b'[');
                let n = rary(v).v.len();
                self.long(n as i64);
                let mut i = 0;
                while i < rary(v).v.len() {
                    let x = rary(v).v[i];
                    self.object(x, lim)?;
                    i += 1;
                }
                if !iv.is_empty() {
                    self.ivars(None, &iv, lim)?;
                }
            }
            Kind::Hash => {
                let iv = generic_ivars(v);
                if !iv.is_empty() {
                    self.byte(b'I');
                }
                self.uclass(v, rt().c.hash)?;
                let h = rhash(v);
                if h.default_proc != QNIL {
                    return terr("can't dump hash with default proc");
                }
                let def = h.default;
                self.byte(if def == QNIL { b'{' } else { b'}' });
                let pairs = crate::hash::pairs(v);
                self.long(pairs.len() as i64);
                for (a, b) in pairs {
                    self.object(a, lim)?;
                    self.object(b, lim)?;
                }
                if def != QNIL {
                    self.object(def, lim)?;
                }
                if !iv.is_empty() {
                    self.ivars(None, &iv, lim)?;
                }
            }
            Kind::Range => {
                // CRuby's compat dumper: an Object of class Range with excl/begin/end ivars.
                self.class_rec(b'o', v, true)?;
                let rg = rrange(v);
                let (lo, hi, excl) = (rg.lo, rg.hi, rg.excl);
                let iv = generic_ivars(v);
                self.long(iv.len() as i64 + 3);
                for &(n, x) in &iv {
                    self.symbol(n);
                    self.object(x, lim)?;
                }
                self.symbol(sym("excl"));
                self.object(bool_v(excl), lim)?;
                self.symbol(sym("begin"));
                self.object(lo, lim)?;
                self.symbol(sym("end"));
                self.object(hi, lim)?;
            }
            Kind::Data if is_regexp_obj(v) => {
                let re = crate::regex::re_of(v);
                let src = re.source.clone();
                let ascii = src.is_ascii();
                let opts = (re.opts & 7) | (re.opts & 32) | if ascii { 0 } else { 16 };
                let iv = generic_ivars(v);
                self.byte(b'I');
                self.uclass(v, regexp_class())?;
                self.byte(b'/');
                self.bytes(&src);
                self.byte(opts as u8);
                self.ivars(Some(!ascii && opts & 32 == 0), &iv, lim)?;
            }
            Kind::Data | Kind::Proc | Kind::Env => {
                return terr(&format!("no _dump_data is defined for class {}", class_name(real_class(v))));
            }
            Kind::Object => self.plain_object(v, lim)?,
        }
        Ok(())
    }

    fn plain_object(&mut self, v: V, lim: i64) -> R<()> {
        let r = rt();
        if crate::rational::is_rational(v) {
            self.class_rec(b'U', v, false)?;
            let a = ary_new(vec![ivar_get(v, sym("@__num")), ivar_get(v, sym("@__den"))]);
            self.keep.push(a);
            return self.object(a, lim);
        }
        let cc = global_class("Complex");
        if !cc.is_null() && real_class(v) == cc {
            self.class_rec(b'U', v, false)?;
            let a = ary_new(vec![ivar_get(v, sym("@__re")), ivar_get(v, sym("@__im"))]);
            self.keep.push(a);
            return self.object(a, lim);
        }
        if is_a_named(v, "Date") && ivar_defined(v, sym("@__jd")) {
            let g = |n: &str| {
                let x = ivar_get(v, sym(n));
                if is_fix(x) { fix(x) } else { 0 }
            };
            let (jd, df, sf, of) = (g("@__jd"), g("@__df"), g("@__sf"), g("@__of"));
            let t = jd * 86400 + df - of;
            self.class_rec(b'U', v, false)?;
            let a = ary_new(vec![mkfix(0), mkfix(t.div_euclid(86400)), mkfix(t.rem_euclid(86400)), mkfix(sf), mkfix(of), float_new(2299161.0)]);
            self.keep.push(a);
            return self.object(a, lim);
        }
        if is_a(v, r.c.exception) {
            self.class_rec(b'o', v, true)?;
            let iv = ivar_list(v);
            self.long(iv.len() as i64 + 2);
            self.symbol(sym("mesg"));
            self.object(ivar_get(v, sym("@__message")), lim)?;
            self.symbol(sym("bt"));
            self.object(ivar_get(v, sym("@__backtrace")), lim)?;
            for (n, x) in iv {
                self.symbol(n);
                self.object(x, lim)?;
            }
            return Ok(());
        }
        let cls = real_class(v);
        let anc_names: Vec<String> = ancestors(cls).iter().map(|&c| unsafe { (*c).name.clone() }).collect();
        let has = |n: &str| anc_names.iter().any(|a| a == n);
        if is_a(v, r.c.io) || ["MatchData", "Queue", "Thread::Queue", "SizedQueue", "Thread::SizedQueue", "ConditionVariable", "Thread::ConditionVariable"].iter().any(|n| has(n)) {
            return terr(&format!("can't dump {}", class_name(cls)));
        }
        const NO_DUMP: [&str; 20] = [
            "Proc", "Method", "UnboundMethod", "Binding", "Thread", "Mutex", "Thread::Mutex", "Monitor", "Enumerator", "Enumerator::Yielder",
            "Enumerator::Lazy", "Fiber", "StringIO", "Dir", "File::Stat", "ObjectSpace::WeakMap", "StringScanner", "Thread::Backtrace::Location",
            "Encoding::Converter", "Enumerator::Generator",
        ];
        if is_a(v, r.c.mutex) || is_a(v, r.c.enumerator) || NO_DUMP.iter().any(|n| has(n)) {
            return terr(&format!("no _dump_data is defined for class {}", class_name(cls)));
        }
        if is_a(v, r.c.struct_) {
            let members = unsafe { (*cls).struct_members.clone() };
            let mids: Vec<u32> = members.iter().map(|m| sym(&format!("@{}", sym_str(*m)))).collect();
            let iv: Vec<(u32, V)> = ivar_list(v).into_iter().filter(|(n, _)| !mids.contains(n)).collect();
            if !iv.is_empty() {
                self.byte(b'I');
            }
            self.class_rec(b'S', v, true)?;
            self.long(members.len() as i64);
            for (i, &m) in members.iter().enumerate() {
                self.symbol(m);
                self.object(ivar_get(v, mids[i]), lim)?;
            }
            if !iv.is_empty() {
                self.ivars(None, &iv, lim)?;
            }
            return Ok(());
        }
        self.class_rec(b'o', v, true)?;
        let iv = ivar_list(v);
        self.long(iv.len() as i64);
        for (n, x) in iv {
            self.symbol(n);
            self.object(x, lim)?;
        }
        Ok(())
    }
}

/// The `E` encoding ivar of a string: Some(true) UTF-8, Some(false) US-ASCII, None binary.
fn str_enc(s: V) -> Option<bool> {
    let f = hdr(s).flags;
    if f & FL_BINARY != 0 {
        None
    } else if f & FL_USASCII != 0 {
        Some(false)
    } else {
        Some(true)
    }
}

fn int_arg(v: V) -> Option<i64> {
    if is_fix(v) { Some(fix(v)) } else { None }
}

fn marshal_dump(_: V, a: &[V], _: V) -> V {
    let obj = a[0];
    let mut io = QNIL;
    let mut limit = -1i64;
    match a.get(1) {
        Some(&x) if is_fix(x) => limit = fix(x),
        Some(&x) if x != QNIL => {
            if !has_method(x, "write") {
                return type_error("instance of IO needed");
            }
            io = x;
        }
        _ => {}
    }
    if let Some(&x) = a.get(2) {
        match int_arg(x) {
            Some(l) => limit = l,
            None => return type_error(&format!("no implicit conversion of {} into Integer", class_name(real_class(x)))),
        }
    }
    let mut d = Dumper { out: vec![4, 8], syms: HashMap::new(), objs: HashMap::new(), flonums: HashMap::new(), nobj: 0, keep: RVec::new() };
    d.keep.push(obj);
    if d.object(obj, limit).is_err() {
        return UNDEF;
    }
    let s = str_new(std::mem::take(&mut d.out));
    hdr(s).flags |= FL_BINARY;
    if io != QNIL {
        if has_method(io, "binmode") && send0(io, "binmode") == UNDEF {
            return UNDEF;
        }
        if send1(io, "write", s) == UNDEF {
            return UNDEF;
        }
        return io;
    }
    s
}

// =====================================================================================
// load
// =====================================================================================

struct Loader {
    src: Vec<u8>,
    pos: usize,
    io: V,
    io_getbyte: bool,
    syms: Vec<u32>,
    objs: RVec,
    proc_: V,
    freeze: bool,
}

impl Loader {
    fn too_short<T>() -> R<T> {
        aerr("marshal data too short")
    }
    fn byte(&mut self) -> R<u8> {
        if self.io != QNIL {
            if !self.io_getbyte {
                return Ok(self.take(1)?[0]);
            }
            let b = chk(send0(self.io, "getbyte"))?;
            if !is_fix(b) {
                let eof = global_class("EOFError");
                return Err(raise(eof, "end of file reached"));
            }
            return Ok(fix(b) as u8);
        }
        match self.src.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Self::too_short(),
        }
    }
    fn take(&mut self, n: usize) -> R<Vec<u8>> {
        if n == 0 {
            return Ok(Vec::new());
        }
        if self.io != QNIL {
            let s = chk(send1(self.io, "read", mkfix(n as i64)))?;
            if !is_str(s) || rstr(s).b.len() < n {
                return Self::too_short();
            }
            return Ok(rstr(s).b.clone());
        }
        if self.pos + n > self.src.len() {
            return Self::too_short();
        }
        let b = self.src[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(b)
    }
    fn long(&mut self) -> R<i64> {
        let c = self.byte()? as i8 as i64;
        if c == 0 {
            return Ok(0);
        }
        if c > 0 {
            if 4 < c && c < 128 {
                return Ok(c - 5);
            }
            let mut x: i64 = 0;
            for i in 0..c {
                x |= (self.byte()? as i64) << (8 * i);
            }
            Ok(x)
        } else {
            if -129 < c && c < -4 {
                return Ok(c + 5);
            }
            let c = -c;
            let mut x: i64 = -1;
            for i in 0..c {
                x &= !(0xffi64 << (8 * i));
                x |= (self.byte()? as i64) << (8 * i);
            }
            Ok(x)
        }
    }
    fn bytes(&mut self) -> R<Vec<u8>> {
        let n = self.long()?;
        if n < 0 {
            return Self::too_short();
        }
        self.take(n as usize)
    }
    fn symbol(&mut self) -> R<u32> {
        let mut ivar = false;
        loop {
            match self.byte()? {
                b'I' => ivar = true,
                b':' => return self.symreal(ivar),
                b';' => {
                    if ivar {
                        return aerr("dump format error (symlink with encoding)");
                    }
                    let i = self.long()?;
                    return match self.syms.get(i as usize) {
                        Some(&s) => Ok(s),
                        None => aerr("bad symbol"),
                    };
                }
                t => return aerr(&format!("dump format error for symbol(0x{:x})", t)),
            }
        }
    }
    fn symreal(&mut self, ivar: bool) -> R<u32> {
        let b = self.bytes()?;
        let id = intern(&b);
        self.syms.push(id);
        if ivar {
            let n = self.long()?;
            for _ in 0..n {
                self.symbol()?;
                self.object()?;
            }
        }
        Ok(id)
    }
    fn reserve(&mut self) -> usize {
        self.objs.push(QNIL);
        self.objs.len() - 1
    }
    fn entry(&mut self, v: V) -> V {
        self.objs.push(v);
        v
    }
    fn path2class(&mut self, name: &[u8]) -> R<V> {
        let s = String::from_utf8_lossy(name).into_owned();
        let mut cur = rt().c.object;
        let parts: Vec<&str> = s.split("::").collect();
        for (i, &part) in parts.iter().enumerate() {
            // CRuby names the path up to the failing segment (with a trailing "::" if more follow).
            let upto = || {
                let mut p = parts[..=i].join("::");
                if i + 1 < parts.len() {
                    p.push_str("::");
                }
                p
            };
            if part.is_empty() {
                return aerr(&format!("undefined class/module {}", upto()));
            }
            let id = sym(part);
            let found = unsafe { (*cur).consts.get(&id).copied() }.or_else(|| if cur == rt().c.object { None } else { const_get_under(cur, id) });
            let v = match found {
                Some(v) => v,
                None if crate::abi::autoload_path(cur, id).is_some() => {
                    let v = crate::abi::const_missing(cur, id);
                    if v == UNDEF {
                        return Err(UNDEF);
                    }
                    v
                }
                None => return aerr(&format!("undefined class/module {}", upto())),
            };
            if !is_class(v) {
                return terr(&format!("{} does not refer to class/module", upto()));
            }
            cur = v as *mut Class;
        }
        Ok(cur as V)
    }
    fn want_class(&mut self, name: &[u8]) -> R<*mut Class> {
        let c = self.path2class(name)?;
        if rclass(c).is_module {
            return aerr(&format!("{} does not refer to class", String::from_utf8_lossy(name)));
        }
        Ok(c as *mut Class)
    }
    fn unique(&mut self) -> R<Vec<u8>> {
        let id = self.symbol()?;
        Ok(sym_name(id).to_vec())
    }

    /// One object, after ivars, with `proc` applied (`r_object` + `r_leave`).
    fn object(&mut self) -> R<V> {
        let mut ivp = false;
        let (v, link) = self.object0(&mut ivp)?;
        self.leave(v, link)
    }
    fn leave(&mut self, v: V, link: bool) -> R<V> {
        let mut v = v;
        if self.freeze && !link && is_heap(v) && !is_class(v) {
            hdr(v).flags |= FL_FROZEN;
        }
        if self.proc_ != QNIL {
            v = chk(send1(self.proc_, "call", v))?;
        }
        Ok(v)
    }
    /// Read ivars into `obj` (encoding ivars set the string encoding).
    fn ivar(&mut self, obj: V) -> R<()> {
        let n = self.long()?;
        let is_exc = is_kind(obj, Kind::Object) && is_a(obj, rt().c.exception);
        for _ in 0..n {
            let id = self.symbol()?;
            let val = self.object()?;
            let name = sym_name(id);
            if name == b"E" || name == b"encoding" {
                if is_str(obj) {
                    let h = hdr(obj);
                    h.flags &= !(FL_BINARY | FL_USASCII);
                    if name == b"E" {
                        if val == QFALSE {
                            h.flags |= FL_USASCII;
                        }
                    } else if is_str(val) {
                        let e = String::from_utf8_lossy(&rstr(val).b).to_ascii_uppercase();
                        if e == "ASCII-8BIT" || e == "BINARY" {
                            h.flags |= FL_BINARY;
                        } else if e == "US-ASCII" {
                            h.flags |= FL_USASCII;
                        }
                    }
                }
                continue;
            }
            if is_exc && name == b"mesg" {
                ivar_set(obj, sym("@__message"), val);
                continue;
            }
            if is_exc && name == b"bt" {
                if val != QNIL {
                    ivar_set(obj, sym("@__backtrace"), val);
                }
                continue;
            }
            if name.starts_with(b"@__") {
                continue;
            }
            // Non-@ ivars only exist on _dump strings (e.g. Time's `offset`/`zone`).
            if !name.starts_with(b"@") && is_kind(obj, Kind::Object) {
                continue;
            }
            if chk(ivar_set(obj, id, val)).is_err() {
                return Err(UNDEF);
            }
        }
        Ok(())
    }

    /// Returns the object and whether it was a link (`@`) to an earlier one.
    fn object0(&mut self, ivp: &mut bool) -> R<(V, bool)> {
        let t = self.byte()?;
        let v = match t {
            b'@' => {
                let i = self.long()?;
                return match self.objs.get(i as usize) {
                    Some(&v) if i >= 0 => Ok((v, true)),
                    _ => aerr("dump format error (unlinked)"),
                };
            }
            b'I' => {
                let mut iv = true;
                let (v, link) = self.object0(&mut iv)?;
                if iv {
                    self.ivar(v)?;
                }
                return Ok((v, link));
            }
            b'e' => {
                let path = self.unique()?;
                let m = self.path2class(&path)?;
                if !rclass(m).is_module {
                    return aerr(&format!("{} does not refer to module", String::from_utf8_lossy(&path)));
                }
                let (v, link) = self.object0(ivp)?;
                chk(send1(v, "extend", m))?;
                return Ok((v, link));
            }
            b'0' => QNIL,
            b'T' => QTRUE,
            b'F' => QFALSE,
            b'i' => mkfix(self.long()?),
            b'f' => {
                let b = self.bytes()?;
                let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                let s = String::from_utf8_lossy(&b[..end]).into_owned();
                let f = match s.as_str() {
                    "nan" => f64::NAN,
                    "inf" => f64::INFINITY,
                    "-inf" => f64::NEG_INFINITY,
                    _ => s.parse::<f64>().unwrap_or(0.0),
                };
                self.entry(float_new(f))
            }
            b'l' => {
                let sign = self.byte()?;
                let n = self.long()?;
                if n < 0 {
                    return Self::too_short();
                }
                let b = self.take(n as usize * 2)?;
                let mag = BigInt::from_bytes_le(Sign::Plus, &b);
                let v = int_from_big(if sign == b'-' { -mag } else { mag });
                self.entry(v)
            }
            b'"' => {
                let b = self.bytes()?;
                let s = str_new(b);
                hdr(s).flags |= FL_BINARY;
                self.entry(s)
            }
            b'/' => {
                let idx = self.reserve();
                let src = self.bytes()?;
                let opts = self.byte()? as u32;
                if *ivp {
                    let tmp = str_new(Vec::new());
                    self.ivar(tmp)?;
                    *ivp = false;
                }
                let re = chk(crate::regex::regexp_new(&src, opts & 7))?;
                self.objs[idx] = re;
                re
            }
            b'[' => {
                let n = self.long()?;
                let a = self.entry(ary_new(Vec::with_capacity(n.clamp(0, 1 << 16) as usize)));
                for _ in 0..n {
                    let x = self.object()?;
                    rary(a).v.push(x);
                }
                a
            }
            b'{' | b'}' => {
                let n = self.long()?;
                let h = self.entry(hash_new());
                for _ in 0..n {
                    let k = self.object()?;
                    let x = self.object()?;
                    chk(crate::hash::hset(h, k, x))?;
                }
                if t == b'}' {
                    let d = self.object()?;
                    crate::gc::wb_obj(h);
                    rhash(h).default = d;
                }
                h
            }
            b'S' => {
                let name = self.unique()?;
                let k = self.want_class(&name)?;
                let nm = String::from_utf8_lossy(&name).into_owned();
                if !class_le(k, rt().c.struct_) {
                    return terr(&format!("class {} not a struct", nm));
                }
                let n = self.long()?;
                let v = chk(crate::kernel::alloc_instance(k))?;
                self.entry(v);
                let members = unsafe { (*k).struct_members.clone() };
                if n as usize != members.len() {
                    return terr(&format!("struct {} not compatible (struct size differs)", nm));
                }
                for m in members {
                    let slot = self.symbol()?;
                    if slot != m {
                        return terr(&format!("struct {} not compatible (:{} for :{})", nm, sym_str(slot), sym_str(m)));
                    }
                    let x = self.object()?;
                    ivar_set(v, sym(&format!("@{}", sym_str(m))), x);
                }
                v
            }
            b'u' => {
                let name = self.unique()?;
                let k = self.path2class(&name)?;
                let data = str_new(self.bytes()?);
                hdr(data).flags |= FL_BINARY;
                if *ivp {
                    self.ivar(data)?;
                    *ivp = false;
                }
                if !has_method(k, "_load") {
                    return terr(&format!("class {} needs to have method `_load'", String::from_utf8_lossy(&name)));
                }
                let v = chk(send(k, sym("_load"), &[data], QNIL))?;
                self.entry(v)
            }
            b'U' => {
                let name = self.unique()?;
                let k = self.want_class(&name)?;
                if let Some(v) = self.native_usrmarshal(k)? {
                    v
                } else {
                    let v = chk(crate::kernel::alloc_instance(k))?;
                    if !has_method(v, "marshal_load") {
                        return terr(&format!("instance of {} needs to have method `marshal_load'", String::from_utf8_lossy(&name)));
                    }
                    self.entry(v);
                    let data = self.object()?;
                    chk(send1(v, "marshal_load", data))?;
                    v
                }
            }
            b'o' => {
                let name = self.unique()?;
                let k = self.want_class(&name)?;
                if class_le(k, rt().c.range) {
                    self.range_object(k)?
                } else {
                    let v = chk(crate::kernel::alloc_instance(k))?;
                    if !is_kind(v, Kind::Object) {
                        return aerr("dump format error");
                    }
                    self.entry(v);
                    self.ivar(v)?;
                    v
                }
            }
            b'c' | b'm' | b'M' => {
                let name = self.bytes()?;
                let c = self.path2class(&name)?;
                let is_mod = rclass(c).is_module;
                if t == b'c' && is_mod {
                    return aerr(&format!("{} does not refer to class", String::from_utf8_lossy(&name)));
                }
                if t == b'm' && !is_mod {
                    return aerr(&format!("{} does not refer to module", String::from_utf8_lossy(&name)));
                }
                self.entry(c)
            }
            b'C' => {
                let name = self.unique()?;
                let k = self.want_class(&name)?;
                let (v, link) = self.object0(ivp)?;
                match kind(v) {
                    Some(Kind::String) | Some(Kind::Array) | Some(Kind::Hash) | Some(Kind::Data) => hdr(v).klass = k,
                    _ => return aerr("dump format error (user class)"),
                }
                return Ok((v, link));
            }
            b':' => {
                let id = self.symreal(*ivp)?;
                *ivp = false;
                mksym(id)
            }
            b';' => {
                let i = self.long()?;
                match self.syms.get(i as usize) {
                    Some(&s) => mksym(s),
                    None => return aerr("bad symbol"),
                }
            }
            b'd' => {
                let name = self.unique()?;
                return terr(&format!("class {} needs to have instance method `_load_data'", String::from_utf8_lossy(&name)));
            }
            _ => return aerr(&format!("dump format error(0x{:x})", t)),
        };
        Ok((v, false))
    }

    /// `U` records of runtime-native classes (Rational, Complex, Date/DateTime).
    fn native_usrmarshal(&mut self, k: *mut Class) -> R<Option<V>> {
        let rat = global_class("Rational");
        let cpx = global_class("Complex");
        let date = global_class("Date");
        if !(k == rat || k == cpx || class_le(k, date)) || k.is_null() {
            return Ok(None);
        }
        let idx = self.reserve();
        let data = self.object()?;
        let el = |i: usize| if is_ary(data) { rary(data).v.get(i).copied().unwrap_or(QNIL) } else { QNIL };
        if !is_ary(data) {
            return aerr("marshaled data must be an array");
        }
        let v = if k == rat {
            let big = |x: V| if is_fix(x) { Some(BigInt::from(fix(x))) } else if is_big(x) { Some(rbig(x).n.clone()) } else { None };
            match (big(el(0)), big(el(1))) {
                (Some(n), Some(d)) => chk(crate::rational::rational_new(n, d))?,
                _ => return aerr("marshaled rational must have an array whose length is 2"),
            }
        } else if k == cpx {
            crate::rational::complex_new(el(0), el(1))
        } else {
            let g = |i: usize| if is_fix(el(i)) { fix(el(i)) } else { 0 };
            let (jd, df, sf, of) = (g(1), g(2), g(3), g(4));
            let t = jd * 86400 + df + of;
            let o = object_new(k);
            ivar_set(o, sym("@__jd"), mkfix(t.div_euclid(86400)));
            ivar_set(o, sym("@__df"), mkfix(t.rem_euclid(86400)));
            ivar_set(o, sym("@__sf"), mkfix(sf));
            ivar_set(o, sym("@__of"), mkfix(of));
            o
        };
        self.objs[idx] = v;
        Ok(Some(v))
    }

    fn range_object(&mut self, k: *mut Class) -> R<V> {
        let idx = self.reserve();
        let n = self.long()?;
        let (mut lo, mut hi, mut excl) = (QNIL, QNIL, false);
        let mut extra = RVec::new();
        for _ in 0..n {
            let id = self.symbol()?;
            let val = self.object()?;
            match sym_name(id) {
                b"begin" => lo = val,
                b"end" => hi = val,
                b"excl" => excl = truthy(val),
                _ => {
                    extra.push(mksym(id));
                    extra.push(val);
                }
            }
        }
        let r = range_new(lo, hi, excl);
        hdr(r).klass = k;
        for p in extra.chunks(2) {
            if sym_name(sym_id(p[0])).starts_with(b"@") {
                gen_ivar_set(r, sym_id(p[0]), p[1]);
            }
        }
        self.objs[idx] = r;
        Ok(r)
    }
}

fn kwarg(a: &[V], key: &str) -> (Option<V>, usize) {
    match a.last() {
        Some(&h) if is_hash(h) && hdr(h).flags & FL_KWHASH != 0 => (rhash(h).m.get(&HKey(mksym(sym(key)))).copied(), a.len() - 1),
        _ => (None, a.len()),
    }
}

fn marshal_load(_: V, a: &[V], _: V) -> V {
    let (fz, npos) = kwarg(a, "freeze");
    if npos == 0 {
        return arity_error(0, 1, 2);
    }
    let src = a[0];
    let proc_ = if npos > 1 { a[1] } else { QNIL };
    let mut l = Loader { src: Vec::new(), pos: 0, io: QNIL, io_getbyte: false, syms: Vec::new(), objs: RVec::new(), proc_, freeze: fz.is_some_and(truthy) };
    if is_str(src) {
        l.src = rstr(src).b.clone();
    } else if has_method(src, "to_str") {
        let s = send0(src, "to_str");
        if s == UNDEF {
            return UNDEF;
        }
        if !is_str(s) {
            return type_error("instance of IO needed");
        }
        l.src = rstr(s).b.clone();
    } else if has_method(src, "read") {
        l.io = src;
        l.io_getbyte = has_method(src, "getbyte");
        if has_method(src, "binmode") && send0(src, "binmode") == UNDEF {
            return UNDEF;
        }
    } else {
        return type_error("instance of IO needed");
    }
    let mut run = || -> R<V> {
        let major = l.byte()?;
        let minor = l.byte()?;
        if major != 4 || minor > 8 {
            return terr(&format!("incompatible marshal file format (can't be read)\n\tformat version 4.8 required; {}.{} given", major, minor));
        }
        l.object()
    };
    run().unwrap_or(UNDEF)
}

// =====================================================================================
// Time#_dump / Time._load
// =====================================================================================

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
fn ascii_str(b: &[u8]) -> V {
    let s = str_new(b.to_vec());
    hdr(s).flags |= FL_USASCII;
    s
}

fn time_dump(t: V, _: &[V], _: V) -> V {
    let ns = crate::misc::time_ns(t);
    let secs = ns.div_euclid(NS) as i64;
    let frac = ns.rem_euclid(NS) as i64;
    let utc = truthy(ivar_get(t, sym("@__utc")));
    let (y, mon, d) = civil_from_days(secs.div_euclid(86400));
    let sod = secs.rem_euclid(86400);
    let year = y - 1900;
    if !(0..=0xffff).contains(&year) {
        return arg_error(&format!("year too {} to marshal: {} UTC", if year < 0 { "small" } else { "big" }, y));
    }
    let (usec, nsec) = (frac / 1000, frac % 1000);
    let p: u32 = 1 << 31 | (utc as u32) << 30 | (year as u32) << 14 | ((mon - 1) as u32) << 10 | (d as u32) << 5 | (sod / 3600) as u32;
    let s: u32 = ((sod % 3600 / 60) as u32) << 26 | ((sod % 60) as u32) << 20 | usec as u32;
    let mut b = p.to_le_bytes().to_vec();
    b.extend_from_slice(&s.to_le_bytes());
    let str = str_new(b);
    hdr(str).flags |= FL_BINARY;
    for (n, x) in ivar_list(t) {
        ivar_set(str, n, x);
    }
    if nsec != 0 {
        ivar_set(str, sym("nano_num"), mkfix(nsec));
        ivar_set(str, sym("nano_den"), mkfix(1));
        let b1 = ((nsec % 10) << 4) as u8;
        let b0 = ((nsec / 10 % 10) | ((nsec / 100 % 10) << 4)) as u8;
        let sub = if b1 == 0 { str_new(vec![b0]) } else { str_new(vec![b0, b1]) };
        hdr(sub).flags |= FL_BINARY;
        ivar_set(str, sym("submicro"), sub);
    }
    let zone = if utc {
        ascii_str(b"UTC")
    } else {
        let off = tri!(send0(t, "utc_offset"));
        ivar_set(str, sym("offset"), off);
        let z = tri!(send0(t, "zone"));
        if is_str(z) { ascii_str(&rstr(z).b.clone()) } else { QNIL }
    };
    ivar_set(str, sym("zone"), zone);
    str
}

fn set_time(t: V, ns: i128, utc: bool, off: V) {
    ivar_set(t, sym("@__ns"), int_from_i128(ns));
    ivar_set(t, sym("@__t"), float_new(ns.div_euclid(NS) as f64 + ns.rem_euclid(NS) as f64 / 1e9));
    ivar_set(t, sym("@__utc"), bool_v(utc));
    ivar_set(t, sym("@__off"), off);
}

fn time_load(k: V, a: &[V], _: V) -> V {
    let s = a[0];
    if !is_str(s) {
        return type_error("marshaled time format differ");
    }
    let b = rstr(s).b.clone();
    if b.len() != 8 {
        return type_error("marshaled time format differ");
    }
    let p = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let sv = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let cls = if is_class(k) { k as *mut Class } else { rt().c.time };
    let t = tri!(crate::kernel::alloc_instance(cls));
    if p & (1 << 31) == 0 {
        set_time(t, p as i128 * NS + sv as i128 * 1000, false, QNIL);
        return t;
    }
    let utc = (p >> 30) & 1 == 1;
    let year = ((p >> 14) & 0xffff) as i64 + 1900;
    let mon = ((p >> 10) & 0xf) as i64;
    let mday = ((p >> 5) & 0x1f) as i64;
    let hour = (p & 0x1f) as i64;
    let min = ((sv >> 26) & 0x3f) as i64;
    let sec = ((sv >> 20) & 0x3f) as i64;
    let usec = (sv & 0xfffff) as i64;
    let mut nano: i128 = 0;
    let (nn, nd) = (ivar_get(s, sym("nano_num")), ivar_get(s, sym("nano_den")));
    if is_fix(nn) && is_fix(nd) && fix(nd) != 0 {
        nano = (fix(nn) / fix(nd)) as i128;
    } else {
        let sub = ivar_get(s, sym("submicro"));
        if is_str(sub) {
            let sb = &rstr(sub).b;
            let d0 = sb.first().copied().unwrap_or(0);
            let d1 = sb.get(1).copied().unwrap_or(0);
            nano = ((d0 >> 4) as i128) * 100 + ((d0 & 0xf) as i128) * 10 + (d1 >> 4) as i128;
        }
    }
    let secs = days_from_civil(year, mon + 1, mday) * 86400 + hour * 3600 + min * 60 + sec;
    let ns = secs as i128 * NS + usec as i128 * 1000 + nano;
    if utc {
        set_time(t, ns, true, QNIL);
    } else {
        set_time(t, ns, false, QNIL);
        let off = ivar_get(s, sym("offset"));
        if is_fix(off) {
            let local_off = tri!(send0(t, "utc_offset"));
            let zone = ivar_get(s, sym("zone"));
            let same_zone = !is_str(zone) || {
                let z = tri!(send0(t, "zone"));
                is_str(z) && rstr(z).b == rstr(zone).b
            };
            if local_off != off || !same_zone {
                set_time(t, ns, false, off);
            }
        }
    }
    for (n, x) in ivar_list(s) {
        if sym_name(n).starts_with(b"@") {
            ivar_set(t, n, x);
        }
    }
    t
}

pub fn init() {
    // Interned in CRuby's order so these ivars of a Time's _dump string sort stably.
    for n in ["nano_num", "nano_den", "submicro", "offset", "zone", "E", "mesg", "bt", "excl", "begin", "end"] {
        sym(n);
    }
    let m = new_module("Marshal");
    set_global_const("Marshal", m as V);
    const_set(m, sym("MAJOR_VERSION"), mkfix(4));
    const_set(m, sym("MINOR_VERSION"), mkfix(8));
    defs(m, "dump", 1, 3, marshal_dump);
    defs(m, "load", 1, 3, marshal_load);
    defs(m, "restore", 1, 3, marshal_load);
    let tm = rt().c.time;
    defn(tm, "_dump", 0, 1, time_dump);
    defs(tm, "_load", 1, 1, time_load);
}
