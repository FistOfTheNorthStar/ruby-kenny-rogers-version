//! Source maps (added for Kenny, see KENNY.md): where each rule, declaration and value of
//! the CSS came from.

use std::sync::Arc;

use codemap::{File, Span};

/// One file the CSS came from.
#[derive(Clone, Debug)]
pub struct Source {
    /// The file's path as it was loaded
    pub name: String,
    pub contents: String,
}

/// A position in the CSS and the source position it came from. Lines and columns are
/// 0-based; columns count UTF-16 code units, as source maps do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub generated_line: u32,
    pub generated_column: u32,
    /// Index into [`SourceMap::sources`]
    pub source: u32,
    pub original_line: u32,
    pub original_column: u32,
}

/// What [`crate::from_path_with_source_map`] returns beside the CSS.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    /// The entry file first, then the files used, in load order
    pub sources: Vec<Source>,
    /// Ordered by generated position
    pub mappings: Vec<Mapping>,
}

impl SourceMap {
    /// Builds the map from the serializer's marks (offsets into `css`, ordered) and the
    /// files loaded (`files[0]` is the entry file). A mark in no file is left out.
    pub(crate) fn build(css: &str, marks: &[(usize, Span)], files: &[Arc<File>]) -> Self {
        // (generated line, generated column, file, original line, original column)
        let mut located: Vec<(u32, u32, usize, u32, u32)> = Vec::with_capacity(marks.len());
        let (mut at, mut line, mut column) = (0usize, 0u32, 0u32);
        for &(offset, span) in marks {
            let Some(rest) = css.get(at..offset.max(at)) else { continue };
            for c in rest.chars() {
                if c == '\n' {
                    line += 1;
                    column = 0;
                } else {
                    column += c.len_utf16() as u32;
                }
            }
            at = offset.max(at);
            if let Some((file, original_line, original_column)) = locate(files, span) {
                located.push((line, column, file, original_line, original_column));
            }
        }

        // the entry file and the files used, in load order; a file loaded twice is one source
        let mut used = vec![false; files.len()];
        if let Some(entry) = used.first_mut() {
            *entry = true;
        }
        for l in &located {
            used[l.2] = true;
        }
        let mut sources: Vec<Source> = Vec::new();
        let mut index = vec![0u32; files.len()];
        for (i, file) in files.iter().enumerate().filter(|&(i, _)| used[i]) {
            index[i] = match sources.iter().position(|s| s.name == file.name()) {
                Some(j) => j as u32,
                None => {
                    sources.push(Source { name: file.name().to_owned(), contents: file.source().to_owned() });
                    sources.len() as u32 - 1
                }
            };
        }

        let mut mappings: Vec<Mapping> = Vec::with_capacity(located.len());
        for (generated_line, generated_column, file, original_line, original_column) in located {
            let mapping = Mapping { generated_line, generated_column, source: index[file], original_line, original_column };
            // two marks at one place (an empty selector, say): the later one is more specific
            match mappings.last_mut() {
                Some(last) if last.generated_line == generated_line && last.generated_column == generated_column => *last = mapping,
                _ => mappings.push(mapping),
            }
        }

        SourceMap { sources, mappings }
    }

    /// The mappings in the source map format (version 3): Base64 VLQ, `;` between lines.
    pub fn encode_mappings(&self) -> String {
        let mut out = String::new();
        let (mut line, mut prev_column) = (0u32, 0i64);
        let (mut prev_source, mut prev_line, mut prev_original_column) = (0i64, 0i64, 0i64);
        for (i, m) in self.mappings.iter().enumerate() {
            if m.generated_line != line {
                for _ in line..m.generated_line {
                    out.push(';');
                }
                line = m.generated_line;
                prev_column = 0;
            } else if i > 0 {
                out.push(',');
            }
            vlq(&mut out, i64::from(m.generated_column) - prev_column);
            vlq(&mut out, i64::from(m.source) - prev_source);
            vlq(&mut out, i64::from(m.original_line) - prev_line);
            vlq(&mut out, i64::from(m.original_column) - prev_original_column);
            prev_column = i64::from(m.generated_column);
            prev_source = i64::from(m.source);
            prev_line = i64::from(m.original_line);
            prev_original_column = i64::from(m.original_column);
        }
        out
    }
}

/// The file (index into `files`), line and UTF-16 column where `span` starts.
fn locate(files: &[Arc<File>], span: Span) -> Option<(usize, u32, u32)> {
    let pos = span.low();
    // files are added in order, so their spans ascend
    let index = files.partition_point(|f| f.span.high() < pos);
    let file = files.get(index)?;
    if pos < file.span.low() {
        return None;
    }
    let line = file.find_line(pos);
    let start = (file.line_span(line).low() - file.span.low()) as usize;
    let end = (pos - file.span.low()) as usize;
    let column = file.source().get(start..end)?.encode_utf16().count();
    Some((index, line as u32, column as u32))
}

fn vlq(out: &mut String, value: i64) {
    const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut v = if value < 0 { ((-value) << 1) | 1 } else { value << 1 } as u64;
    loop {
        let mut digit = (v & 31) as u8;
        v >>= 5;
        if v > 0 {
            digit |= 32;
        }
        out.push(BASE64[digit as usize] as char);
        if v == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_matches_the_spec_examples() {
        let mut s = String::new();
        for v in [0, 1, -1, 15, 16, -16, 123456] {
            vlq(&mut s, v);
            s.push(' ');
        }
        assert_eq!(s, "A C D e gB hB gkxH ");
    }
}
