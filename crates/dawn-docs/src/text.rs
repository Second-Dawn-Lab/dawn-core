//! Plain text files: paragraphs are translated, line breaks, blank lines and the end of the file stay.

use std::path::Path;

use crate::protect::is_translatable;
use crate::{DocError, Loaded, Writer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Not valid UTF-8: read as Latin-1 (close to Windows-1252) and written as UTF-8.
    Latin1,
}

pub(crate) fn decode(bytes: &[u8]) -> (String, Encoding) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), Encoding::Utf8Bom);
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (utf16(rest, true), Encoding::Utf16Le);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (utf16(rest, false), Encoding::Utf16Be);
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (text, Encoding::Utf8),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    }
}

pub(crate) fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 | Encoding::Latin1 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Le => [&[0xFF, 0xFE][..], &text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<_>>()].concat(),
        Encoding::Utf16Be => [&[0xFE, 0xFF][..], &text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<_>>()].concat(),
    }
}

/// Lines with their line ending style; the original text is rebuilt from them.
pub(crate) struct Lines {
    pub lines: Vec<String>,
    /// "\r\n" or "\n".
    pub eol: &'static str,
    pub final_newline: bool,
}

pub(crate) fn split_lines(text: &str) -> Lines {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let final_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
    if final_newline {
        lines.pop();
    }
    Lines { lines, eol, final_newline }
}

impl Lines {
    pub fn join(&self, lines: &[String]) -> String {
        let mut out = lines.join(self.eol);
        if self.final_newline {
            out.push_str(self.eol);
        }
        out
    }
}

/// A run of non-blank lines and where it starts.
struct Block {
    first: usize,
    count: usize,
}

struct TextWriter {
    lines: Lines,
    blocks: Vec<Block>,
    encoding: Encoding,
}

pub(crate) fn load_text(path: &Path) -> Result<Loaded, DocError> {
    let (text, encoding) = decode(&std::fs::read(path)?);
    let lines = split_lines(&text);
    let mut blocks = Vec::new();
    let mut units = Vec::new();
    let mut i = 0;
    while i < lines.lines.len() {
        if lines.lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let first = i;
        while i < lines.lines.len() && !lines.lines[i].trim().is_empty() {
            i += 1;
        }
        let block = lines.lines[first..i].join("\n");
        if is_translatable(&block) {
            units.push(block);
            blocks.push(Block { first, count: i - first });
        }
    }
    Ok(Loaded { units, layout: None, restorers: Vec::new(), parts: None, skipped: Vec::new(), writer: Box::new(TextWriter { lines, blocks, encoding }) })
}

impl Writer for TextWriter {
    fn write(&self, translations: &[String], out: &Path) -> Result<Vec<String>, DocError> {
        let mut result: Vec<String> = Vec::with_capacity(self.lines.lines.len());
        let mut cursor = 0;
        for (block, translation) in self.blocks.iter().zip(translations) {
            result.extend(self.lines.lines[cursor..block.first].iter().cloned());
            result.extend(translation.split('\n').map(|l| l.trim_end_matches('\r').to_string()));
            cursor = block.first + block.count;
        }
        result.extend(self.lines.lines[cursor..].iter().cloned());
        std::fs::write(out, encode(&self.lines.join(&result), self.encoding))?;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::translate_document;

    fn run(content: &[u8]) -> Vec<u8> {
        let dir = temp_dir("text");
        let input = dir.join("a.txt");
        std::fs::write(&input, content).unwrap();
        let out = dir.join("b.txt");
        translate_document(&input, &out, &mut Tag("de"), &mut |_| {}).unwrap();
        std::fs::read(out).unwrap()
    }

    #[test]
    fn paragraphs_are_translated_and_the_layout_stays() {
        let out = run(b"Title\n\nFirst line\nsecond line\n\n\n   indented words\n\n12345\n");
        assert_eq!(String::from_utf8(out).unwrap(), "[de]Title\n\n[de]First line\nsecond line\n\n\n[de]   indented words\n\n12345\n");
    }

    #[test]
    fn crlf_line_endings_and_a_missing_final_newline_are_kept() {
        let out = run(b"One\r\n\r\nTwo");
        assert_eq!(out, b"[de]One\r\n\r\n[de]Two");
    }

    #[test]
    fn encodings_are_kept() {
        assert_eq!(run(b"\xEF\xBB\xBFHello"), b"\xEF\xBB\xBF[de]Hello");
        let utf16: Vec<u8> = [&[0xFF, 0xFE][..], &"Hallo".encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<_>>()].concat();
        let out = run(&utf16);
        assert_eq!(decode(&out), ("[de]Hallo".to_string(), Encoding::Utf16Le));
        // Latin-1 input comes out as UTF-8 with the same letters.
        assert_eq!(String::from_utf8(run(b"Gr\xfc\xdfe")).unwrap(), "[de]Gr\u{fc}\u{df}e");
    }

    #[test]
    fn an_empty_file_is_copied() {
        assert_eq!(run(b""), b"");
    }
}
