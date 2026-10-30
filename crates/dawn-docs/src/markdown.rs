//! Markdown: text is translated, code, links, addresses and the markup itself are kept.
//!
//! Line by line: fenced and indented code blocks, front matter, HTML and reference definitions are
//! copied. For every other line the markup at the start (`#`, `>`, list markers, task boxes) is kept
//! and the rest is translated, with inline code, link targets, addresses, HTML tags and emphasis
//! marks swapped for placeholders while the model works.

use std::path::Path;

use crate::protect::{is_translatable, protect_addresses, push_token, restore, Tokens};
use crate::text::{decode, encode, split_lines, Encoding, Lines};
use crate::{DocError, Loaded, Writer};

/// One piece of a line: kept as it is, or text to translate.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Keep(String),
    /// A unit; `tokens` are what is put back after translation.
    Translate { index: usize, tokens: Tokens },
}

struct MarkdownWriter {
    lines: Lines,
    /// Per line, its pieces.
    pieces: Vec<Vec<Piece>>,
    encoding: Encoding,
}

pub(crate) fn load(path: &Path) -> Result<Loaded, DocError> {
    let (text, encoding) = decode(&std::fs::read(path)?);
    let lines = split_lines(&text);
    let mut units: Vec<String> = Vec::new();
    let pieces = parse(&lines.lines, &mut units);
    Ok(Loaded { units, parts: None, skipped: Vec::new(), writer: Box::new(MarkdownWriter { lines, pieces, encoding }) })
}

fn is_fence(trimmed: &str) -> Option<(char, usize)> {
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn is_reference_definition(trimmed: &str) -> bool {
    trimmed.starts_with('[') && trimmed.find("]:").is_some_and(|i| trimmed[..i].chars().filter(|c| *c == '[').count() == 1)
}

/// The markup at the start of a line (indent, quote marks, list marker, task box, heading hashes).
fn leading_markup(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0;
    loop {
        let start = i;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'>' {
            i += 1;
            continue;
        }
        let rest = &line[i..];
        // bullet
        if rest.len() > 1 && matches!(rest.as_bytes()[0], b'-' | b'*' | b'+') && rest.as_bytes()[1] == b' ' {
            i += 2;
            continue;
        }
        // numbered
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && digits < 10 && rest.len() > digits + 1 && matches!(rest.as_bytes()[digits], b'.' | b')') && rest.as_bytes()[digits + 1] == b' ' {
            i += digits + 2;
            continue;
        }
        // task box
        if rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ") {
            i += 4;
            continue;
        }
        // heading
        let hashes = rest.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && rest.as_bytes().get(hashes) == Some(&b' ') {
            i += hashes + 1;
            continue;
        }
        if i == start {
            break;
        }
        break;
    }
    i
}

/// Replace inline markup by placeholders; returns the text to translate and what to put back.
fn protect_inline(content: &str, counter: &mut usize) -> (String, Tokens) {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::new();
    let mut tokens: Tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            // `code` spans, with any number of backticks
            '`' => {
                let n = chars[i..].iter().take_while(|x| **x == '`').count();
                let fence: String = "`".repeat(n);
                let rest: String = chars[i + n..].iter().collect();
                if let Some(end) = rest.find(&fence) {
                    let span: String = chars[i..i + n].iter().collect::<String>() + &rest[..end + n];
                    let len = span.chars().count();
                    out.push_str(&push_token(counter, &mut tokens, &span));
                    i += len;
                    continue;
                }
                out.push_str(&fence);
                i += n;
            }
            // link or image target: ](...)
            ']' if chars.get(i + 1) == Some(&'(') => {
                let mut depth = 0;
                let mut end = None;
                for (k, ch) in chars.iter().enumerate().skip(i + 1) {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(k);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(end) = end {
                    let span: String = chars[i..=end].iter().collect();
                    // "]" stays visible text-wise; the target is protected, so the model sees "word]Uq1x".
                    out.push(']');
                    out.push_str(&push_token(counter, &mut tokens, &span[1..]));
                    i = end + 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            // <https://...> autolinks and inline HTML tags
            '<' => {
                if let Some(end) = chars[i..].iter().position(|x| *x == '>') {
                    let span: String = chars[i..=i + end].iter().collect();
                    let looks_like_tag = span.len() > 2 && !span.contains(' ') || span.starts_with("</") || span.starts_with("<a ") || span.starts_with("<img ") || span.starts_with("<span ");
                    if looks_like_tag && !span[1..span.len() - 1].contains('<') {
                        out.push_str(&push_token(counter, &mut tokens, &span));
                        i += end + 1;
                        continue;
                    }
                }
                out.push(c);
                i += 1;
            }
            // emphasis marks at word edges: **, *, __, _, ~~
            '*' | '_' | '~' => {
                let n = chars[i..].iter().take_while(|x| **x == c).count();
                let before = i.checked_sub(1).map(|k| chars[k]);
                let after = chars.get(i + n).copied();
                let edge_before = before.is_none_or(|b| !b.is_alphanumeric());
                let edge_after = after.is_none_or(|a| !a.is_alphanumeric());
                if (edge_before ^ edge_after) && n <= 3 {
                    let span: String = std::iter::repeat_n(c, n).collect();
                    out.push_str(&push_token(counter, &mut tokens, &span));
                } else {
                    out.extend(std::iter::repeat_n(c, n));
                }
                i += n;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    let (text, addresses) = protect_addresses(&out, counter);
    tokens.extend(addresses);
    (text, tokens)
}

fn parse(lines: &[String], units: &mut Vec<String>) -> Vec<Vec<Piece>> {
    let mut pieces = Vec::with_capacity(lines.len());
    let mut counter = 0usize;
    let mut fence: Option<(char, usize)> = None;
    let mut in_front_matter = lines.first().is_some_and(|l| l.trim_end() == "---") && lines.iter().skip(1).any(|l| l.trim_end() == "---" || l.trim_end() == "...");
    let mut in_comment = false;
    let mut in_list = false;
    let mut previous_blank = true;

    let translate = |content: &str, units: &mut Vec<String>, counter: &mut usize| -> Vec<Piece> {
        let leading: String = content.chars().take_while(|c| c.is_whitespace()).collect();
        let trailing: String = content.chars().rev().take_while(|c| c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
        let core = content.trim();
        let (protected, tokens) = protect_inline(core, counter);
        if !is_translatable(&restore_for_check(&protected)) {
            return vec![Piece::Keep(content.to_string())];
        }
        units.push(protected);
        let index = units.len() - 1;
        let mut out = Vec::new();
        if !leading.is_empty() {
            out.push(Piece::Keep(leading));
        }
        out.push(Piece::Translate { index, tokens });
        if !trailing.is_empty() {
            out.push(Piece::Keep(trailing));
        }
        out
    };

    for (n, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let keep = |line: &str| vec![Piece::Keep(line.to_string())];

        if in_front_matter {
            if n > 0 && (trimmed == "---" || trimmed == "...") {
                in_front_matter = false;
            }
            pieces.push(keep(line));
            continue;
        }
        if let Some((c, len)) = fence {
            if is_fence(trimmed).is_some_and(|(c2, n2)| c2 == c && n2 >= len) {
                fence = None;
            }
            pieces.push(keep(line));
            continue;
        }
        if let Some(f) = is_fence(trimmed) {
            fence = Some(f);
            pieces.push(keep(line));
            continue;
        }
        if in_comment {
            if trimmed.contains("-->") {
                in_comment = false;
            }
            pieces.push(keep(line));
            continue;
        }
        if trimmed.starts_with("<!--") {
            in_comment = !trimmed.contains("-->");
            pieces.push(keep(line));
            continue;
        }
        if trimmed.is_empty() {
            previous_blank = true;
            in_list = in_list && false;
            pieces.push(keep(line));
            continue;
        }
        let is_list_line = leading_markup(line) > 0 && (trimmed.starts_with(['-', '*', '+']) || trimmed.starts_with(|c: char| c.is_ascii_digit()));
        let indented_code = (line.starts_with("    ") || line.starts_with('\t')) && previous_blank && !in_list;
        let html_block = trimmed.starts_with('<') && trimmed.ends_with('>') && !trimmed.starts_with("<http");
        if indented_code || html_block || is_reference_definition(trimmed) || is_table_separator(line) {
            previous_blank = false;
            pieces.push(keep(line));
            continue;
        }
        in_list = is_list_line || (in_list && line.starts_with(' '));
        previous_blank = false;

        if trimmed.contains('|') && trimmed.matches('|').count() >= 2 {
            // A table row: every cell is a unit.
            let mut row: Vec<Piece> = Vec::new();
            let cells: Vec<&str> = line.split('|').collect();
            for (k, cell) in cells.iter().enumerate() {
                if k > 0 {
                    row.push(Piece::Keep("|".into()));
                }
                if cell.trim().is_empty() {
                    row.push(Piece::Keep(cell.to_string()));
                } else {
                    row.extend(translate(cell, units, &mut counter));
                }
            }
            pieces.push(row);
            continue;
        }

        let markup = leading_markup(line);
        let mut row = Vec::new();
        if markup > 0 {
            row.push(Piece::Keep(line[..markup].to_string()));
        }
        row.extend(translate(&line[markup..], units, &mut counter));
        pieces.push(row);
    }
    pieces
}

/// The text of a protected line without placeholders, to decide whether anything is left to translate.
fn restore_for_check(protected: &str) -> String {
    let mut out = String::new();
    let mut rest = protected;
    while let Some(at) = rest.find("Uq") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with('x') {
            rest = &after[digits + 1..];
        } else {
            out.push_str("Uq");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

impl Writer for MarkdownWriter {
    fn write(&self, translations: &[String], out: &Path) -> Result<Vec<String>, DocError> {
        let mut lines = Vec::with_capacity(self.pieces.len());
        for row in &self.pieces {
            let mut line = String::new();
            for piece in row {
                match piece {
                    Piece::Keep(text) => line.push_str(text),
                    Piece::Translate { index, tokens } => {
                        let text = translations[*index].replace(['\r', '\n'], " ");
                        line.push_str(&restore(&text, tokens));
                    }
                }
            }
            lines.push(line);
        }
        std::fs::write(out, encode(&self.lines.join(&lines), self.encoding))?;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::translate_document;

    fn run(markdown: &str) -> String {
        let dir = temp_dir("md");
        let input = dir.join("a.md");
        std::fs::write(&input, markdown).unwrap();
        let out = dir.join("b.md");
        translate_document(&input, &out, &mut Tag("de"), &mut |_| {}).unwrap();
        std::fs::read_to_string(out).unwrap()
    }

    #[test]
    fn code_blocks_inline_code_and_urls_are_not_translated() {
        let md = "# Install guide\n\nRun `npm install --save` first, see https://example.org/docs.\n\n```bash\necho hello world\n```\n\n~~~\nplain code fence\n~~~\n\nThe end.\n";
        let out = run(md);
        assert!(out.contains("# [de]Install guide"), "{out}");
        assert!(out.contains("`npm install --save`"));
        assert!(out.contains("https://example.org/docs"));
        assert!(out.contains("```bash\necho hello world\n```"), "fenced code is copied");
        assert!(out.contains("~~~\nplain code fence\n~~~"));
        assert!(out.contains("[de]The end."));
    }

    #[test]
    fn list_markers_quotes_and_task_boxes_stay_and_the_text_is_translated() {
        let out = run("- first item\n  - nested item\n1. numbered\n> quoted words\n- [x] done task\n");
        assert_eq!(out, "- [de]first item\n  - [de]nested item\n1. [de]numbered\n> [de]quoted words\n- [x] [de]done task\n");
    }

    #[test]
    fn links_keep_their_target_and_images_their_path() {
        let out = run("Read the [user guide](docs/guide.md) and look at ![a picture](img/pic.png).\n");
        assert!(out.contains("](docs/guide.md)") && out.contains("](img/pic.png)"), "{out}");
    }

    #[test]
    fn table_cells_are_translated_and_the_separator_row_is_kept() {
        let out = run("| Name | Value |\n|------|-------|\n| apple | 12 |\n");
        assert_eq!(out, "| [de]Name | [de]Value |\n|------|-------|\n| [de]apple | 12 |\n");
    }

    #[test]
    fn front_matter_html_comments_and_reference_definitions_are_kept() {
        let md = "---\ntitle: My page\n---\n\n<!-- a comment\nspanning lines -->\n\nText here.\n\n[ref]: https://example.org \"A title\"\n";
        let out = run(md);
        assert!(out.starts_with("---\ntitle: My page\n---\n"));
        assert!(out.contains("<!-- a comment\nspanning lines -->"));
        assert!(out.contains("[de]Text here."));
        assert!(out.ends_with("[ref]: https://example.org \"A title\"\n"));
    }

    #[test]
    fn emphasis_marks_survive_as_placeholders() {
        let out = run("This is **very important** and _slanted_ text.\n");
        assert_eq!(out, "[de]This is **very important** and _slanted_ text.\n");
    }

    #[test]
    fn snake_case_words_are_not_emphasis() {
        let (text, tokens) = protect_inline("call my_function_name now", &mut 0);
        assert_eq!(text, "call my_function_name now");
        assert!(tokens.is_empty());
    }

    #[test]
    fn indented_code_after_a_blank_line_is_kept_but_list_continuations_are_translated() {
        let out = run("Paragraph words.\n\n    code line stays\n\n- item\n\n  continued item text\n");
        assert!(out.contains("    code line stays"), "{out}");
        assert!(out.contains("  [de]continued item text"), "{out}");
    }

    #[test]
    fn numbers_and_symbols_only_lines_are_copied() {
        let out = run("---\n\n***\n\n12 345\n");
        assert_eq!(out, "---\n\n***\n\n12 345\n");
    }

    #[test]
    fn leading_markup_is_measured_correctly() {
        assert_eq!(leading_markup("# Title"), 2);
        assert_eq!(leading_markup("  - [ ] task"), 8);
        assert_eq!(leading_markup("> - quoted item"), 4);
        assert_eq!(leading_markup("10. ten"), 4);
        assert_eq!(leading_markup("plain"), 0);
        assert_eq!(leading_markup("#hashtag"), 0);
    }
}
