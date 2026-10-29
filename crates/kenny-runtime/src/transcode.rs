//! String encodings beyond the three Kenny strings carry natively (UTF-8, US-ASCII,
//! ASCII-8BIT): `String#encode` transcoding, the encoding a String reports, and CRuby's
//! encoding rules for `<<` / `concat` / `+`.
//!
//! A String converted (or `force_encoding`'d) to another encoding holds its bytes in that
//! encoding and is flagged binary, so its other methods see bytes; it remembers the
//! encoding's name for `encoding`, `encode` and `valid_encoding?` (docs/LANGUAGE.md).
//! Transcoding covers UTF-8, UTF-16/32 (LE, BE, with a BOM), US-ASCII, ASCII-8BIT,
//! ISO-8859-1 and Windows-1252; ASCII-only text converts to and from any ASCII-compatible
//! encoding, other text in those raises NotImplementedError.

use crate::compat::FL_BINARY;
use crate::core::*;

const FL_ASCII: u32 = FL_BINARY << 1;
const ENC_MASK: u32 = FL_BINARY | FL_ASCII;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Codec {
    Utf8,
    Ascii,
    Binary,
    /// Some(true) big-endian, Some(false) little-endian, None: a BOM says (big-endian out)
    Utf16(Option<bool>),
    Utf32(Option<bool>),
    Latin1,
    Cp1252,
    /// any other encoding: only its ASCII range is understood
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct Enc {
    pub name: &'static str,
    pub codec: Codec,
    pub ascii_compat: bool,
}
impl PartialEq for Enc {
    fn eq(&self, o: &Self) -> bool {
        self.name == o.name
    }
}

pub const UTF8: Enc = Enc { name: "UTF-8", codec: Codec::Utf8, ascii_compat: true };
pub const ASCII: Enc = Enc { name: "US-ASCII", codec: Codec::Ascii, ascii_compat: true };
pub const BINARY: Enc = Enc { name: "ASCII-8BIT", codec: Codec::Binary, ascii_compat: true };

/// The encoding named `name` (any case, aliases too).
pub fn lookup(name: &str) -> Option<Enc> {
    let up = name.to_ascii_uppercase();
    match up.as_str() {
        "UTF-8" | "CP65001" | "LOCALE" | "EXTERNAL" | "FILESYSTEM" => return Some(UTF8),
        "ASCII-8BIT" | "BINARY" => return Some(BINARY),
        "US-ASCII" | "ASCII" | "ANSI_X3.4-1968" | "646" => return Some(ASCII),
        _ => {}
    }
    for &(n, aliases, _, _dummy, ascii_compat) in crate::enc_table::ENCODINGS {
        if n.eq_ignore_ascii_case(&up) || aliases.iter().any(|a| a.eq_ignore_ascii_case(&up)) {
            let codec = match n {
                "UTF-16BE" => Codec::Utf16(Some(true)),
                "UTF-16LE" => Codec::Utf16(Some(false)),
                "UTF-16" => Codec::Utf16(None),
                "UTF-32BE" => Codec::Utf32(Some(true)),
                "UTF-32LE" => Codec::Utf32(Some(false)),
                "UTF-32" => Codec::Utf32(None),
                "ISO-8859-1" => Codec::Latin1,
                "Windows-1252" => Codec::Cp1252,
                _ => Codec::Other,
            };
            return Some(Enc { name: n, codec, ascii_compat });
        }
    }
    None
}

fn enc_ivar() -> u32 {
    static ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ID.get_or_init(|| sym("@__enc"))
}

/// The encoding of String `v`.
pub fn enc_of(v: V) -> Enc {
    let f = hdr(v).flags;
    if f & FL_BINARY != 0 {
        let n = ivar_get(v, enc_ivar());
        if is_str(n) {
            if let Some(e) = lookup(&String::from_utf8_lossy(&rstr(n).b)) {
                return e;
            }
        }
        BINARY
    } else if f & FL_ASCII != 0 {
        ASCII
    } else {
        UTF8
    }
}

/// The encoding of `sub`/`gsub`'s result: the subject's, or (as CRuby's rb_enc_check)
/// a non-ASCII replacement's when the subject is ASCII-only.
pub fn note_piece(enc: &mut Enc, src_ascii: bool, piece: V) {
    if src_ascii && is_str(piece) && !rstr(piece).b.is_ascii() {
        *enc = enc_of(piece);
    }
}

/// Give String `v` encoding `e`.
pub fn set_enc(v: V, e: Enc) {
    let h = hdr(v);
    let was_foreign = h.flags & FL_BINARY != 0 && ivar_get(v, enc_ivar()) != QNIL;
    h.flags &= !ENC_MASK;
    match e.codec {
        Codec::Utf8 => {}
        Codec::Ascii => h.flags |= FL_ASCII,
        Codec::Binary => h.flags |= FL_BINARY,
        _ => {
            h.flags |= FL_BINARY;
            // (force_encoding applies to frozen Strings too: extra.rs)
            gen_ivar_set(v, enc_ivar(), str_from(e.name));
            return;
        }
    }
    if was_foreign {
        gen_ivar_set(v, enc_ivar(), QNIL);
    }
}

/// The Encoding object for `e`.
pub fn encoding_obj(e: Enc) -> V {
    let Some(ec) = const_lookup(rt().c.object, sym("Encoding")) else { return QNIL };
    let name = if e.codec == Codec::Binary { "BINARY" } else { e.name };
    send(ec, sym("find"), &[str_from(name)], QNIL)
}

/// An encoding argument (a String name or an Encoding object): Err(name) if unknown.
pub fn enc_arg(v: V) -> Result<Enc, String> {
    let name = if is_str(v) {
        String::from_utf8_lossy(&rstr(v).b).into_owned()
    } else {
        let n = ivar_get(v, sym("@__name"));
        if is_str(n) {
            String::from_utf8_lossy(&rstr(n).b).into_owned()
        } else {
            match crate::fmt::to_s(v) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(()) => String::new(),
            }
        }
    };
    lookup(&name).ok_or(name)
}

/// How an encoding is named in an error about mixing encodings.
fn mix_name(e: Enc) -> &'static str {
    if e.codec == Codec::Binary { "BINARY (ASCII-8BIT)" } else { e.name }
}
/// How an encoding is named in a conversion error.
/// The name CRuby's transcoder for `e` goes by in error messages: upper-case for the
/// single-byte encodings (WINDOWS-1252, MACROMAN, IBM437 ...), the encoding's own name for
/// the others (Shift_JIS, EUC-JP, UTF-16LE).
fn conv_name(e: Enc) -> String {
    let n = e.name;
    if n.starts_with("Windows-12") || n == "Windows-874" || n.starts_with("mac") { n.to_ascii_uppercase() } else { n.to_string() }
}

fn enc_error(class: &str, msg: &str) -> V {
    let ec = const_lookup(rt().c.object, sym("Encoding")).unwrap_or(QNIL);
    let c = const_get_under(ec as *mut Class, sym(class)).filter(|c| is_class(*c)).unwrap_or(rt().c.standard_error as V);
    raise(c as *mut Class, msg)
}

/// The encoding of `s` after appending bytes `xb` in encoding `xe`: None keeps `s`'s; raises
/// Encoding::CompatibilityError (UNDEF) as CRuby's rb_enc_check does.
pub fn concat_enc(s: V, xb: &[u8], xe: Enc) -> Result<Option<Enc>, V> {
    let se = enc_of(s);
    if se == xe || xb.is_empty() {
        return Ok(None);
    }
    let sb = &rstr(s).b;
    if sb.is_empty() {
        return Ok(if se.ascii_compat && xb.is_ascii() { None } else { Some(xe) });
    }
    if se.ascii_compat && xe.ascii_compat {
        if xb.is_ascii() {
            return Ok(None);
        }
        if sb.is_ascii() {
            return Ok(Some(xe));
        }
    }
    Err(enc_error("CompatibilityError", &format!("incompatible character encodings: {} and {}", mix_name(se), mix_name(xe))))
}

/// The encoding flags of `s` and `x` say nothing more is needed for `s << x` (both UTF-8,
/// or both US-ASCII).
#[inline]
pub fn same_native(s: V, x: V) -> bool {
    let (a, b) = (hdr(s).flags, hdr(x).flags);
    (a ^ b) & ENC_MASK == 0 && a & FL_BINARY == 0
}

/// `s << n` for an Integer: the codepoint's bytes in `s`'s encoding, and the encoding the
/// result has (US-ASCII + 128..255 gives ASCII-8BIT).
pub fn codepoint_bytes(s: V, n: i64) -> Result<(Vec<u8>, Option<Enc>), V> {
    let e = enc_of(s);
    let range = || raise(rt().c.range_error, &format!("{n} out of char range"));
    if n < 0 {
        return Err(range());
    }
    match e.codec {
        Codec::Binary | Codec::Latin1 | Codec::Cp1252 if n <= 255 => Ok((vec![n as u8], None)),
        Codec::Ascii if n < 128 => Ok((vec![n as u8], None)),
        Codec::Ascii if n <= 255 => Ok((vec![n as u8], Some(BINARY))),
        Codec::Utf8 | Codec::Utf16(_) | Codec::Utf32(_) => {
            let mut out = Vec::new();
            match char::from_u32(n as u32) {
                Some(_) if encode_cp(e.codec, n as u32, &mut out, false) => Ok((out, None)),
                _ => Err(if e.codec == Codec::Utf8 { range() } else { raise(rt().c.range_error, &format!("invalid codepoint 0x{n:X} in {}", e.name)) }),
            }
        }
        Codec::Other if n < 128 => Ok((vec![n as u8], None)),
        _ => Err(range()),
    }
}

// ---------- decoding ----------

enum Unit {
    Ch(u32),
    /// invalid bytes; `incomplete`: cut off by the end; `next`: the byte after (not consumed)
    Bad { bytes: Vec<u8>, incomplete: bool, next: Option<u8> },
    /// a byte the source encoding leaves undefined (ASCII-8BIT's 0x80.., Windows-1252's holes)
    Undef(u8),
}

const CP1252: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0,
    0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

fn decode_utf8(b: &[u8], out: &mut Vec<Unit>) {
    let n = b.len();
    let mut i = 0;
    'outer: while i < n {
        let c = b[i];
        if c < 0x80 {
            out.push(Unit::Ch(c as u32));
            i += 1;
            continue;
        }
        let len = match c {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => {
                out.push(Unit::Bad { bytes: vec![c], incomplete: false, next: None });
                i += 1;
                continue;
            }
        };
        for j in 1..len {
            if i + j >= n {
                out.push(Unit::Bad { bytes: b[i..n].to_vec(), incomplete: true, next: None });
                break 'outer;
            }
            let cc = b[i + j];
            let (lo, hi) = if j == 1 {
                match c {
                    0xE0 => (0xA0, 0xBF),
                    0xED => (0x80, 0x9F),
                    0xF0 => (0x90, 0xBF),
                    0xF4 => (0x80, 0x8F),
                    _ => (0x80, 0xBF),
                }
            } else {
                (0x80, 0xBF)
            };
            if cc < lo || cc > hi {
                out.push(Unit::Bad { bytes: b[i..i + j].to_vec(), incomplete: false, next: Some(cc) });
                i += j;
                continue 'outer;
            }
        }
        let s = std::str::from_utf8(&b[i..i + len]).ok().and_then(|s| s.chars().next());
        out.push(Unit::Ch(s.map(|c| c as u32).unwrap_or(0xFFFD)));
        i += len;
    }
}

fn decode(e: Enc, b: &[u8]) -> Result<Vec<Unit>, V> {
    let mut out = Vec::with_capacity(b.len());
    match e.codec {
        Codec::Utf8 => decode_utf8(b, &mut out),
        Codec::Ascii => out.extend(b.iter().map(|&c| if c < 0x80 { Unit::Ch(c as u32) } else { Unit::Bad { bytes: vec![c], incomplete: false, next: None } })),
        Codec::Binary => out.extend(b.iter().map(|&c| if c < 0x80 { Unit::Ch(c as u32) } else { Unit::Undef(c) })),
        Codec::Latin1 => out.extend(b.iter().map(|&c| Unit::Ch(c as u32))),
        Codec::Cp1252 => out.extend(b.iter().map(|&c| match c {
            0x80..=0x9F => match CP1252[(c - 0x80) as usize] {
                0 => Unit::Undef(c),
                u => Unit::Ch(u as u32),
            },
            _ => Unit::Ch(c as u32),
        })),
        Codec::Utf16(order) => {
            let (mut be, mut i) = (order.unwrap_or(true), 0);
            if order.is_none() && b.len() >= 2 {
                match (b[0], b[1]) {
                    (0xFE, 0xFF) => i = 2,
                    (0xFF, 0xFE) => {
                        be = false;
                        i = 2
                    }
                    _ => {}
                }
            }
            let unit = |k: usize| if be { u16::from_be_bytes([b[k], b[k + 1]]) } else { u16::from_le_bytes([b[k], b[k + 1]]) };
            while i < b.len() {
                if i + 1 >= b.len() {
                    out.push(Unit::Bad { bytes: b[i..].to_vec(), incomplete: true, next: None });
                    break;
                }
                let u = unit(i);
                match u {
                    0xD800..=0xDBFF => {
                        if i + 3 >= b.len() {
                            out.push(Unit::Bad { bytes: b[i..].to_vec(), incomplete: true, next: None });
                            break;
                        }
                        let l = unit(i + 2);
                        if (0xDC00..=0xDFFF).contains(&l) {
                            out.push(Unit::Ch(0x10000 + (((u as u32) - 0xD800) << 10) + ((l as u32) - 0xDC00)));
                            i += 4;
                        } else {
                            out.push(Unit::Bad { bytes: b[i..i + 2].to_vec(), incomplete: false, next: None });
                            i += 2;
                        }
                    }
                    0xDC00..=0xDFFF => {
                        out.push(Unit::Bad { bytes: b[i..i + 2].to_vec(), incomplete: false, next: None });
                        i += 2;
                    }
                    _ => {
                        out.push(Unit::Ch(u as u32));
                        i += 2;
                    }
                }
            }
        }
        Codec::Utf32(order) => {
            let (mut be, mut i) = (order.unwrap_or(true), 0);
            if order.is_none() && b.len() >= 4 {
                match (b[0], b[1], b[2], b[3]) {
                    (0, 0, 0xFE, 0xFF) => i = 4,
                    (0xFF, 0xFE, 0, 0) => {
                        be = false;
                        i = 4
                    }
                    _ => {}
                }
            }
            while i < b.len() {
                if i + 3 >= b.len() {
                    out.push(Unit::Bad { bytes: b[i..].to_vec(), incomplete: true, next: None });
                    break;
                }
                let w = [b[i], b[i + 1], b[i + 2], b[i + 3]];
                let u = if be { u32::from_be_bytes(w) } else { u32::from_le_bytes(w) };
                if char::from_u32(u).is_some() {
                    out.push(Unit::Ch(u));
                } else {
                    out.push(Unit::Bad { bytes: w.to_vec(), incomplete: false, next: None });
                }
                i += 4;
            }
        }
        Codec::Other => {
            if !e.ascii_compat || !b.is_ascii() {
                return Err(not_supported(e));
            }
            out.extend(b.iter().map(|&c| Unit::Ch(c as u32)));
        }
    }
    Ok(out)
}

fn not_supported(e: Enc) -> V {
    raise(rt().c.not_implemented_error, &format!("Kenny converts text to and from {} only when it is ASCII (see docs/LANGUAGE.md)", e.name))
}

// ---------- encoding ----------

/// Append codepoint `cp` in `codec`; false if `codec` can't represent it. `bom`: emit the
/// byte order mark first (UTF-16 / UTF-32 without an order).
fn encode_cp(codec: Codec, cp: u32, out: &mut Vec<u8>, bom: bool) -> bool {
    match codec {
        Codec::Utf8 => match char::from_u32(cp) {
            Some(c) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                true
            }
            None => false,
        },
        Codec::Ascii | Codec::Binary | Codec::Other => {
            if cp < 0x80 {
                out.push(cp as u8);
                true
            } else {
                false
            }
        }
        Codec::Latin1 => {
            if cp <= 0xFF {
                out.push(cp as u8);
                true
            } else {
                false
            }
        }
        Codec::Cp1252 => {
            if cp < 0x80 || (0xA0..=0xFF).contains(&cp) {
                out.push(cp as u8);
                true
            } else if let Some(i) = CP1252.iter().position(|&u| u != 0 && u as u32 == cp) {
                out.push(0x80 + i as u8);
                true
            } else {
                false
            }
        }
        Codec::Utf16(order) => {
            let Some(c) = char::from_u32(cp) else { return false };
            let be = order.unwrap_or(true);
            if bom && order.is_none() {
                out.extend_from_slice(&[0xFE, 0xFF]);
            }
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                out.extend_from_slice(&if be { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            true
        }
        Codec::Utf32(order) => {
            if char::from_u32(cp).is_none() {
                return false;
            }
            let be = order.unwrap_or(true);
            if bom && order.is_none() {
                out.extend_from_slice(&[0, 0, 0xFE, 0xFF]);
            }
            out.extend_from_slice(&if be { cp.to_be_bytes() } else { cp.to_le_bytes() });
            true
        }
    }
}

fn is_unicode(c: Codec) -> bool {
    matches!(c, Codec::Utf8 | Codec::Utf16(_) | Codec::Utf32(_))
}

/// `"\xFF"`-style bytes, as conversion errors show them.
fn dump(b: &[u8]) -> String {
    let mut s = String::from("\"");
    for &c in b {
        if (0x20..0x7F).contains(&c) && c != b'"' && c != b'\\' {
            s.push(c as char);
        } else {
            s.push_str(&format!("\\x{c:02X}"));
        }
    }
    s.push('"');
    s
}

#[derive(Default)]
struct Opts {
    invalid_replace: bool,
    undef_replace: bool,
    replace: Option<Vec<u8>>,
    fallback: Option<V>,
    xml: Option<bool>, // Some(false) :text, Some(true) :attr
    universal_nl: bool,
    out_nl: Option<&'static [u8]>,
}
impl Opts {
    fn decorates(&self) -> bool {
        self.xml.is_some() || self.universal_nl || self.out_nl.is_some()
    }
    fn any(&self) -> bool {
        self.decorates() || self.invalid_replace || self.undef_replace || self.fallback.is_some()
    }
}

fn opts_of(h: V) -> Opts {
    let mut o = Opts::default();
    let get = |k: &str| rhash(h).m.get(&HKey(mksym(sym(k)))).copied();
    let is = |v: Option<V>, s: &str| v.is_some_and(|x| x == mksym(sym(s)));
    o.invalid_replace = is(get("invalid"), "replace");
    o.undef_replace = is(get("undef"), "replace");
    o.replace = get("replace").filter(|v| is_str(*v)).map(|v| rstr(v).b.to_vec());
    o.fallback = get("fallback").filter(|v| *v != QNIL);
    o.xml = match get("xml") {
        Some(v) if v == mksym(sym("text")) => Some(false),
        Some(v) if v == mksym(sym("attr")) => Some(true),
        _ => None,
    };
    o.universal_nl = get("universal_newline").is_some_and(truthy) || is(get("newline"), "universal");
    if get("crlf_newline").is_some_and(truthy) || is(get("newline"), "crlf") {
        o.out_nl = Some(b"\r\n");
    } else if get("cr_newline").is_some_and(truthy) || is(get("newline"), "cr") {
        o.out_nl = Some(b"\r");
    } else if get("lf_newline").is_some_and(truthy) || is(get("newline"), "lf") {
        o.out_nl = Some(b"\n");
    }
    o
}

/// The bytes of `s` (encoding `src`) in `dst`.
fn transcode(bytes: &[u8], src: Enc, dst: Enc, o: &Opts) -> Result<Vec<u8>, V> {
    let mut units = decode(src, bytes)?;
    if o.universal_nl {
        let mut u2 = Vec::with_capacity(units.len());
        let mut it = units.into_iter().peekable();
        while let Some(u) = it.next() {
            if let Unit::Ch(13) = u {
                if matches!(it.peek(), Some(Unit::Ch(10))) {
                    it.next();
                }
                u2.push(Unit::Ch(10));
            } else {
                u2.push(u);
            }
        }
        units = u2;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut bom = true;
    let mut emit = |cp: u32, out: &mut Vec<u8>| {
        let ok = encode_cp(dst.codec, cp, out, bom);
        if ok {
            bom = false;
        }
        ok
    };
    let replacement = |out: &mut Vec<u8>, emit: &mut dyn FnMut(u32, &mut Vec<u8>) -> bool| {
        match &o.replace {
            Some(r) => {
                for c in String::from_utf8_lossy(r).chars() {
                    emit(c as u32, out);
                }
            }
            None => {
                emit(if is_unicode(dst.codec) { 0xFFFD } else { b'?' as u32 }, out);
            }
        }
    };
    // CRuby names the conversion steps by their transcoders (upper-case names) and the whole
    // conversion by the encodings: the same names up to 3.4, the encodings' own (macRoman,
    // Windows-1252) in 4.0, where a one-step conversion is then reported in the long form
    let r4 = crate::misc::ruby4();
    let whole = |e: Enc| if r4 { e.name.to_string() } else { conv_name(e) };
    let short = |e: Enc| !r4 || e.name == conv_name(e);
    let via = |what: &str| {
        if src.codec == Codec::Utf8 && short(dst) {
            format!("{what} from UTF-8 to {}", conv_name(dst))
        } else if src.codec == Codec::Utf8 {
            format!("{what} to {} in conversion from UTF-8 to {}", conv_name(dst), conv_name(dst))
        } else if dst.codec == Codec::Utf8 && short(src) {
            format!("{what} from {} to UTF-8", conv_name(src))
        } else if dst.codec == Codec::Utf8 {
            format!("{what} to UTF-8 in conversion from {} to UTF-8", whole(src))
        } else {
            format!("{what} to {} in conversion from {} to UTF-8 to {}", conv_name(dst), whole(src), conv_name(dst))
        }
    };
    if o.xml == Some(true) {
        emit(b'"' as u32, &mut out);
    }
    for u in units {
        match u {
            Unit::Ch(cp) => {
                if cp == 10 {
                    if let Some(nl) = o.out_nl {
                        for &c in nl {
                            emit(c as u32, &mut out);
                        }
                        continue;
                    }
                }
                if let Some(attr) = o.xml {
                    let ent: Option<&str> = match cp {
                        0x26 => Some("&amp;"),
                        0x3C => Some("&lt;"),
                        0x3E => Some("&gt;"),
                        0x22 if attr => Some("&quot;"),
                        _ => None,
                    };
                    if let Some(ent) = ent {
                        for c in ent.bytes() {
                            emit(c as u32, &mut out);
                        }
                        continue;
                    }
                }
                if emit(cp, &mut out) {
                    continue;
                }
                // undefined in the destination
                if let Some(fb) = o.fallback {
                    let ch = str_from(&char::from_u32(cp).map(|c| c.to_string()).unwrap_or_default());
                    let r = if is_hash(fb) {
                        crate::hash::hget(fb, ch)
                    } else if is_proc(fb) {
                        call_block(fb, &[ch])
                    } else if respond_to(fb, sym("call")) && !respond_to(fb, sym("[]")) {
                        send(fb, sym("call"), &[ch], QNIL)
                    } else {
                        send(fb, sym("[]"), &[ch], QNIL)
                    };
                    if r == UNDEF {
                        return Err(UNDEF);
                    }
                    if is_str(r) {
                        let rb = rstr(r).b.to_vec();
                        let mut ok = true;
                        for c in String::from_utf8_lossy(&rb).chars() {
                            ok &= emit(c as u32, &mut out);
                        }
                        if ok {
                            continue;
                        }
                    }
                    if r != QNIL && !is_str(r) {
                        return Err(type_error(&format!("no implicit conversion of {} into String", class_name(real_class(r)))));
                    }
                }
                if o.xml.is_some() {
                    for c in format!("&#x{cp:X};").bytes() {
                        emit(c as u32, &mut out);
                    }
                    continue;
                }
                if o.undef_replace {
                    replacement(&mut out, &mut emit);
                    continue;
                }
                return Err(enc_error("UndefinedConversionError", &via(&format!("U+{cp:04X}"))));
            }
            Unit::Undef(b) => {
                if o.undef_replace {
                    replacement(&mut out, &mut emit);
                    continue;
                }
                return Err(enc_error("UndefinedConversionError", &if dst.codec == Codec::Utf8 && short(src) {
                    format!("{} from {} to UTF-8", dump(&[b]), conv_name(src))
                } else if dst.codec == Codec::Utf8 {
                    format!("{} to UTF-8 in conversion from {} to UTF-8", dump(&[b]), whole(src))
                } else {
                    format!("{} to UTF-8 in conversion from {} to UTF-8 to {}", dump(&[b]), whole(src), conv_name(dst))
                }));
            }
            Unit::Bad { bytes, incomplete, next } => {
                if o.invalid_replace {
                    replacement(&mut out, &mut emit);
                    continue;
                }
                let what = match (incomplete, next) {
                    (true, _) => format!("incomplete {} on {}", dump(&bytes), conv_name(src)),
                    (false, Some(n)) => format!("{} followed by {} on {}", dump(&bytes), dump(&[n]), conv_name(src)),
                    (false, None) => format!("{} on {}", dump(&bytes), conv_name(src)),
                };
                return Err(enc_error("InvalidByteSequenceError", &what));
            }
        }
    }
    if o.xml == Some(true) {
        emit(b'"' as u32, &mut out);
    }
    Ok(out)
}

/// String#encode / encode!: `encode(dst = default_internal, src = encoding, **opts)`.
pub fn encode(s: V, a: &[V], bang: bool) -> V {
    let (pos, opts) = match a.last() {
        Some(&h) if is_hash(h) => (&a[..a.len() - 1], opts_of(h)),
        _ => (a, Opts::default()),
    };
    let not_found = |from: &str, to: &str| enc_error("ConverterNotFoundError", &format!("code converter not found ({from} to {to})"));
    let src = match pos.get(1) {
        Some(&v) => match enc_arg(v) {
            Ok(e) => e,
            Err(n) => {
                let to = pos.first().map(|d| match enc_arg(*d) {
                    Ok(e) => e.name.to_string(),
                    Err(n) => n,
                });
                return not_found(&n, &to.unwrap_or_default());
            }
        },
        None => enc_of(s),
    };
    let dst = match pos.first() {
        Some(&v) => match enc_arg(v) {
            Ok(e) => e,
            Err(n) => return not_found(src.name, &n),
        },
        None => src,
    };
    let bytes = rstr(s).b.to_vec();
    let out = if src == dst && !opts.any() {
        bytes
    } else if bytes.is_ascii() && src.ascii_compat && dst.ascii_compat && !opts.decorates() {
        bytes
    } else {
        match transcode(&bytes, src, dst, &opts) {
            Ok(b) => b,
            Err(e) => return e,
        }
    };
    if bang {
        crate::tri!(check_frozen(s));
        rstr(s).b.set(out);
        set_enc(s, dst);
        return s;
    }
    let r = str_new(out);
    set_enc(r, dst);
    r
}

/// A String in an encoding other than the native three, as UTF-8 (None: not such a
/// String, or not valid in its encoding).
pub fn foreign_as_utf8(v: V) -> Option<Vec<u8>> {
    let e = enc_of(v);
    // (CRuby shows the characters of UTF-16/32 Strings, the bytes of other encodings')
    if !matches!(e.codec, Codec::Utf16(_) | Codec::Utf32(_)) {
        return None;
    }
    let units = decode(e, &rstr(v).b).ok()?;
    let mut out = Vec::new();
    for u in units {
        match u {
            Unit::Ch(cp) => {
                encode_cp(Codec::Utf8, cp, &mut out, false);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// `valid_encoding?` for a String in an encoding other than the native three.
pub fn valid_foreign(v: V) -> Option<bool> {
    let e = enc_of(v);
    match e.codec {
        Codec::Utf16(_) | Codec::Utf32(_) => {
            let u = decode(e, &rstr(v).b).ok()?;
            Some(!u.iter().any(|x| matches!(x, Unit::Bad { .. })))
        }
        Codec::Latin1 | Codec::Cp1252 | Codec::Other => Some(true),
        _ => None,
    }
}

/// Encoding::InvalidByteSequenceError & co (EncodingError subclasses).
pub fn init() {
    let Some(ec) = const_lookup(rt().c.object, sym("Encoding")) else { return };
    let Some(base) = const_lookup(rt().c.object, sym("EncodingError")) else { return };
    for name in ["CompatibilityError", "InvalidByteSequenceError", "UndefinedConversionError", "ConverterNotFoundError"] {
        if const_get_under(ec as *mut Class, sym(name)).is_none() {
            let c = new_class(&format!("Encoding::{name}"), base as *mut Class);
            const_set(ec as *mut Class, sym(name), c as V);
        }
    }
}
