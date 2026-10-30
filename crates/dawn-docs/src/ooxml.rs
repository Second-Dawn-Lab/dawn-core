//! The text of Office XML parts (Word, PowerPoint, Excel), found and replaced in place.
//!
//! A part is scanned once: every text element (`<w:t>`, `<a:t>`, `<t>`) is recorded with the exact
//! byte range of its content, together with the formatting of the run it belongs to. Translated text
//! is then spliced into those ranges, so everything else in the part (attributes, namespaces, field
//! codes, properties) stays byte for byte as it was.
//!
//! Inline formatting. A paragraph is a row of runs ("This is a ", bold "very important", " sentence").
//! Translating the runs one by one would cut sentences in pieces, so the runs of a paragraph are
//! translated together, with a marker pair around each differently formatted stretch:
//!
//! ```text
//! {1}This is a {/1}{2}very important{/2}{3} sentence{/3}   ->   {1}Das ist ein {/1}{2}sehr wichtiger{/2}{3} Satz{/3}
//! ```
//!
//! The words between the markers go back into the runs with the matching formatting. If the model
//! lost or mixed up a marker, the plain translation is cut at word boundaries in proportion to the
//! original run lengths instead (formatting then lands approximately, never in a wrong order of
//! characters, and never outside the paragraph).

use std::collections::HashMap;
use std::ops::Range;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::{NsReader, Reader};

use crate::protect;

/// The elements that give the text structure of one kind of part.
///
/// Elements are matched by namespace URI and local name, never by prefix: files written by the .NET
/// OpenXML SDK or ClosedXML use `x:`, other writers `w:`, `a:` or no prefix at all, and all of them are valid.
#[derive(Debug)]
pub struct Dialect {
    /// The namespace URIs of the markup language (the 2006 transitional one and the ISO strict one).
    pub namespaces: &'static [&'static str],
    /// A paragraph-like container: `p`, or `si` / `is` / `text` in Excel.
    pub para: &'static [&'static str],
    pub run: &'static [&'static str],
    pub text: &'static [&'static str],
    /// Run properties (the formatting of a run).
    pub props: &'static [&'static str],
    /// Elements that end a stretch of text: tabs, line breaks, pictures.
    pub breaks: &'static [&'static str],
    /// Property elements that hide a run.
    pub hidden: &'static [&'static str],
    /// Text below these elements (any namespace) is not content (math, phonetic guides).
    pub skip_inside: &'static [&'static str],
    /// Worksheets: a cell of type `str` without a formula holds its text in `<v>`.
    pub str_cells: bool,
}

pub const NS_WORD: &[&str] = &["http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"];
pub const NS_DRAWING: &[&str] = &["http://schemas.openxmlformats.org/drawingml/2006/main", "http://purl.oclc.org/ooxml/drawingml/main"];
pub const NS_SHEET: &[&str] = &["http://schemas.openxmlformats.org/spreadsheetml/2006/main", "http://purl.oclc.org/ooxml/spreadsheetml/main"];

pub const WORD: Dialect = Dialect {
    namespaces: NS_WORD,
    para: &["p"],
    run: &["r"],
    text: &["t"],
    props: &["rPr"],
    breaks: &["tab", "br", "cr", "drawing", "pict", "AlternateContent", "object", "sym", "ptab"],
    hidden: &["vanish"],
    skip_inside: &["oMath", "oMathPara", "instrText"],
    str_cells: false,
};

/// PowerPoint, and the DrawingML text boxes that Word and Excel embed.
pub const DRAWING: Dialect = Dialect {
    namespaces: NS_DRAWING,
    para: &["p"],
    run: &["r"],
    text: &["t"],
    props: &["rPr"],
    breaks: &["br", "fld"],
    hidden: &[],
    skip_inside: &[],
    str_cells: false,
};

/// Shared strings, inline strings, comments and `str` cells of Excel.
pub const EXCEL_STRINGS: Dialect = Dialect {
    namespaces: NS_SHEET,
    para: &["si", "is", "text"],
    run: &["r"],
    text: &["t"],
    props: &["rPr"],
    breaks: &[],
    hidden: &[],
    skip_inside: &["rPh", "phoneticPr"],
    str_cells: true,
};

#[derive(Debug, Clone)]
pub struct TextNode {
    /// The start tag (`<w:t xml:space="preserve">`).
    pub tag: Range<usize>,
    /// The (escaped) content between the start and end tag.
    pub content: Range<usize>,
    pub preserve: bool,
    /// The unescaped text.
    pub text: String,
    /// Index into [`Scan::keys`]: the run's formatting.
    pub key: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Segment {
    /// Indices into [`Scan::nodes`], in reading order.
    pub nodes: Vec<usize>,
}

#[derive(Debug, Default)]
pub struct Scan {
    /// The part has formulas (worksheets): their text compared with fixed text may need adjusting.
    pub formulas: bool,
    pub nodes: Vec<TextNode>,
    pub segments: Vec<Segment>,
    /// Run formatting by index; 0 is "no properties".
    pub keys: Vec<String>,
}

fn local(name: &[u8]) -> &[u8] {
    match name.iter().position(|b| *b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// The local name is in `list`, whatever the namespace.
fn local_in(name: &[u8], list: &[&str]) -> bool {
    let l = local(name);
    list.iter().any(|n| n.as_bytes() == l)
}

fn has_preserve(e: &BytesStart) -> bool {
    e.attributes().flatten().any(|a| a.key.as_ref() == b"xml:space" && a.value.as_ref() == b"preserve")
}

fn attribute_is(e: &BytesStart, key: &[u8], value: &[u8]) -> bool {
    e.attributes().flatten().any(|a| local(a.key.as_ref()) == key && a.value.as_ref() == value)
}

struct RunState {
    key: usize,
    hidden: bool,
}

/// A worksheet cell of type `str`: its `<v>` is text unless the cell has a formula (then it is a result).
struct StrCell {
    formula: bool,
}

/// Find the text of one part.
pub fn scan(xml: &str, d: &Dialect) -> Result<Scan, String> {
    let mut reader = NsReader::from_str(xml);
    let mut out = Scan { keys: vec![String::new()], ..Default::default() };
    let mut paras: Vec<Vec<usize>> = Vec::new();
    let mut run: Option<RunState> = None;
    let mut props_start: Option<usize> = None;
    let mut in_text: Option<(Range<usize>, bool)> = None;
    let mut skip_depth = 0usize;
    let mut cell: Option<StrCell> = None;

    fn close(paras: &mut [Vec<usize>], segments: &mut Vec<Segment>) {
        if let Some(top) = paras.last_mut() {
            if !top.is_empty() {
                segments.push(Segment { nodes: std::mem::take(top) });
            }
        }
    }

    loop {
        let before = reader.buffer_position() as usize;
        let (resolved, event) = reader.read_resolved_event().map_err(|e| format!("XML error at byte {before}: {e}"))?;
        // Is the element in this markup language's namespace?
        let in_ns = matches!(&resolved, ResolveResult::Bound(Namespace(ns)) if d.namespaces.iter().any(|n| n.as_bytes() == *ns));
        let after = reader.buffer_position() as usize;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name();
                let name = name.as_ref();
                let is = |list: &[&str]| in_ns && local_in(name, list);
                if local_in(name, d.skip_inside) {
                    skip_depth += 1;
                }
                if is(d.para) {
                    paras.push(Vec::new());
                } else if d.str_cells && is(&["c"]) {
                    if attribute_is(&e, b"t", b"str") {
                        cell = Some(StrCell { formula: false });
                        paras.push(Vec::new());
                    }
                } else if d.str_cells && is(&["f"]) {
                    out.formulas = true;
                    if let Some(c) = cell.as_mut() {
                        c.formula = true;
                    }
                } else if d.str_cells && is(&["v"]) && cell.as_ref().is_some_and(|c| !c.formula) {
                    // xml:space is not allowed on <v>, so the node counts as already preserving.
                    in_text = Some((before..after, true));
                } else if is(d.run) {
                    run = Some(RunState { key: 0, hidden: false });
                } else if is(d.props) && run.is_some() {
                    props_start = Some(before);
                } else if is(d.hidden) && props_start.is_some() {
                    if let Some(r) = run.as_mut() {
                        r.hidden = true;
                    }
                } else if is(d.text) && !paras.is_empty() {
                    in_text = Some((before..after, has_preserve(&e)));
                } else if is(d.breaks) {
                    close(&mut paras, &mut out.segments);
                }
            }
            Event::Empty(e) => {
                let name = e.name();
                let name = name.as_ref();
                let is = |list: &[&str]| in_ns && local_in(name, list);
                if d.str_cells && is(&["f"]) {
                    out.formulas = true;
                    if let Some(c) = cell.as_mut() {
                        c.formula = true;
                    }
                } else if is(d.hidden) && props_start.is_some() {
                    if let Some(r) = run.as_mut() {
                        r.hidden = true;
                    }
                } else if is(d.breaks) {
                    close(&mut paras, &mut out.segments);
                }
            }
            Event::End(e) => {
                let name = e.name();
                let name = name.as_ref();
                let is = |list: &[&str]| in_ns && local_in(name, list);
                if is(d.text) || (d.str_cells && is(&["v"])) {
                    if let Some((tag, preserve)) = in_text.take() {
                        let content = tag.end..before;
                        let raw = &xml[content.clone()];
                        let hidden = run.as_ref().is_some_and(|r| r.hidden);
                        if skip_depth == 0 && !hidden && !raw.contains("<![CDATA[") {
                            if let (Some(top), Ok(text)) = (paras.last_mut(), quick_xml::escape::unescape(raw)) {
                                top.push(out.nodes.len());
                                out.nodes.push(TextNode { tag, content, preserve, text: text.into_owned(), key: run.as_ref().map_or(0, |r| r.key) });
                            }
                        } else {
                            close(&mut paras, &mut out.segments);
                        }
                    }
                } else if is(d.props) {
                    if let (Some(start), Some(r)) = (props_start.take(), run.as_mut()) {
                        let key = &xml[start..after];
                        r.key = match out.keys.iter().position(|k| k == key) {
                            Some(i) => i,
                            None => {
                                out.keys.push(key.to_string());
                                out.keys.len() - 1
                            }
                        };
                    }
                } else if is(d.run) {
                    run = None;
                } else if d.str_cells && is(&["c"]) {
                    if cell.take().is_some() {
                        close(&mut paras, &mut out.segments);
                        paras.pop();
                    }
                } else if is(d.para) {
                    close(&mut paras, &mut out.segments);
                    paras.pop();
                }
                if local_in(name, d.skip_inside) {
                    skip_depth = skip_depth.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Is `xml` well formed? Checked on every part before it is written, so a bug can never produce a
/// file Word refuses to open.
pub fn is_well_formed(xml: &str) -> bool {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

/// Replace the text of some nodes; everything else is copied as it was.
pub fn rewrite(xml: &str, scan: &Scan, new_text: &HashMap<usize, String>) -> String {
    struct Edit {
        range: Range<usize>,
        with: String,
    }
    let mut edits: Vec<Edit> = Vec::new();
    for (&index, text) in new_text {
        let node = &scan.nodes[index];
        edits.push(Edit { range: node.content.clone(), with: quick_xml::escape::escape(text).into_owned() });
        // Leading or trailing spaces only survive when the tag says so.
        let needs_preserve = text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace);
        if needs_preserve && !node.preserve {
            let tag = &xml[node.tag.clone()];
            if let Some(close) = tag.rfind('>') {
                edits.push(Edit { range: node.tag.clone(), with: format!("{} xml:space=\"preserve\"{}", &tag[..close], &tag[close..]) });
            }
        }
    }
    edits.sort_by_key(|e| e.range.start);
    let mut out = String::with_capacity(xml.len() + 256);
    let mut cursor = 0;
    for e in edits {
        out.push_str(&xml[cursor..e.range.start]);
        out.push_str(&e.with);
        cursor = e.range.end;
    }
    out.push_str(&xml[cursor..]);
    out
}

// --- units: what is translated, and how the translation goes back ------------------------------

/// One stretch of text sent to the translator.
#[derive(Debug, Clone)]
pub struct Unit {
    /// What is translated (placeholders and markers included).
    pub text: String,
    /// The segment's text as it was in the document, trimmed.
    original: String,
    /// The nodes of each formatting span, in order. One span means no markers were used.
    spans: Vec<Vec<usize>>,
    /// Whitespace around the segment, put back around the translation.
    lead: String,
    trail: String,
    /// The text of every span without markers, to cut the translation in proportion if needed.
    span_lengths: Vec<usize>,
    tokens: protect::Tokens,
}

const MARKER_OPEN: fn(usize) -> String = |k| format!("{{{k}}}");
const MARKER_CLOSE: fn(usize) -> String = |k| format!("{{/{k}}}");

/// The units of a scanned part (segments with something to translate).
pub fn plan(scan: &Scan) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut counter = 0usize;
    for segment in &scan.segments {
        // Adjacent nodes with the same formatting form one span; whitespace-only nodes join the span before.
        let mut spans: Vec<(usize, Vec<usize>)> = Vec::new();
        for &n in &segment.nodes {
            let node = &scan.nodes[n];
            match spans.last_mut() {
                Some((key, nodes)) if *key == node.key || node.text.trim().is_empty() => nodes.push(n),
                _ => spans.push((node.key, vec![n])),
            }
        }
        let whole: String = segment.nodes.iter().map(|&n| scan.nodes[n].text.as_str()).collect();
        if !protect::is_translatable(&whole) {
            continue;
        }
        let lead: String = whole.chars().take_while(|c| c.is_whitespace()).collect();
        let trail: String = whole.chars().rev().take_while(|c| c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
        let use_markers = spans.len() > 1 && !whole.contains(['{', '}']);

        let span_texts: Vec<String> = spans.iter().map(|(_, nodes)| nodes.iter().map(|&n| scan.nodes[n].text.as_str()).collect()).collect();
        let mut tokens = Vec::new();
        let mut text = String::new();
        if use_markers {
            for (k, span) in span_texts.iter().enumerate() {
                let piece = if k == 0 { span.trim_start() } else { span.as_str() };
                let piece = if k + 1 == span_texts.len() { piece.trim_end() } else { piece };
                let (protected, t) = protect::protect_addresses(piece, &mut counter);
                tokens.extend(t);
                text.push_str(&MARKER_OPEN(k + 1));
                text.push_str(&protected);
                text.push_str(&MARKER_CLOSE(k + 1));
            }
        } else {
            let (protected, t) = protect::protect_addresses(whole.trim(), &mut counter);
            tokens = t;
            text = protected;
        }
        let span_lengths = span_texts.iter().map(|s| s.trim().chars().count().max(1)).collect();
        units.push(Unit {
            text,
            original: whole.trim().to_string(),
            spans: if use_markers { spans.into_iter().map(|(_, n)| n).collect() } else { vec![segment.nodes.clone()] },
            lead,
            trail,
            span_lengths: if use_markers { span_lengths } else { vec![1] },
            tokens,
        });
    }
    units
}

/// Split a marked-up translation into the text of each of `count` spans. `None` when a marker is
/// missing, repeated or out of order.
pub fn split_markers(translated: &str, count: usize) -> Option<Vec<String>> {
    let mut pieces = Vec::with_capacity(count);
    let mut rest = translated;
    for k in 1..=count {
        let (open, close) = (MARKER_OPEN(k), MARKER_CLOSE(k));
        let start = rest.find(&open)?;
        // Text outside any marker (the model moved a space or punctuation) stays with the previous span.
        let before = &rest[..start];
        if k > 1 {
            if let Some(last) = pieces.last_mut() {
                let last: &mut String = last;
                last.push_str(before);
            }
        } else if !before.trim().is_empty() {
            return None;
        }
        let after_open = &rest[start + open.len()..];
        let end = after_open.find(&close)?;
        pieces.push(after_open[..end].to_string());
        rest = &after_open[end + close.len()..];
    }
    if let Some(last) = pieces.last_mut() {
        last.push_str(rest);
    }
    // No stray markers may be left.
    let joined: String = pieces.concat();
    if joined.contains(['{', '}']) {
        return None;
    }
    Some(pieces)
}

/// Cut `text` into `lengths.len()` pieces at word boundaries, in proportion to `lengths`.
pub fn split_proportionally(text: &str, lengths: &[usize]) -> Vec<String> {
    let total: usize = lengths.iter().sum::<usize>().max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut acc = 0;
    for (i, len) in lengths.iter().enumerate() {
        acc += len;
        if i + 1 == lengths.len() {
            pieces.push(chars[start..].iter().collect());
            break;
        }
        let target = (chars.len() * acc / total).clamp(start, chars.len());
        // The nearest space at or after the target, else before it.
        let mut cut = (target..chars.len()).find(|&j| chars[j].is_whitespace()).unwrap_or(chars.len());
        if cut == chars.len() {
            cut = (start..target).rev().find(|&j| chars[j].is_whitespace()).unwrap_or(start);
        }
        // Keep the space with the following piece so words do not run together.
        pieces.push(chars[start..cut].iter().collect());
        start = cut;
    }
    pieces
}

impl Unit {
    pub fn spans(&self) -> usize {
        self.spans.len()
    }

    /// The text as it stood in the document.
    pub fn original(&self) -> &str {
        &self.original
    }

    /// The new text of the nodes of this unit, given the translation of [`Unit::text`].
    pub fn apply(&self, translated: &str) -> Vec<(usize, String)> {
        let restore = |s: &str| protect::restore(s, &self.tokens);
        let mut pieces: Vec<String> = if self.spans.len() > 1 {
            match split_markers(translated, self.spans.len()) {
                Some(p) => p.iter().map(|s| restore(s)).collect(),
                None => {
                    // The markers did not survive: strip what is left of them and cut by proportion.
                    let mut plain = translated.to_string();
                    for k in 1..=self.spans.len() {
                        plain = plain.replace(&MARKER_OPEN(k), "").replace(&MARKER_CLOSE(k), "");
                    }
                    split_proportionally(&restore(plain.trim()), &self.span_lengths)
                }
            }
        } else {
            vec![restore(translated.trim())]
        };
        // Models often put a space before punctuation that follows a marker ("Handbuch{/2} , bevor").
        for piece in pieces.iter_mut().skip(1) {
            let trimmed = piece.trim_start();
            if trimmed.len() < piece.len() && trimmed.starts_with([',', '.', ';', ':', '!', '?', ')']) {
                *piece = trimmed.to_string();
            }
        }
        if let Some(first) = pieces.first_mut() {
            *first = format!("{}{}", self.lead, first.trim_start());
        }
        if let Some(last) = pieces.last_mut() {
            *last = format!("{}{}", last.trim_end(), self.trail);
        }
        let mut edits = Vec::new();
        for (span, piece) in self.spans.iter().zip(pieces) {
            // The first node of the span carries the text, the others are emptied.
            for (i, &node) in span.iter().enumerate() {
                edits.push((node, if i == 0 { piece.clone() } else { String::new() }));
            }
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Hello world</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">This is a </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>very important</w:t></w:r><w:r><w:t xml:space="preserve"> sentence &amp; more.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>Name</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>Value</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>hidden text</w:t></w:r><w:r><w:t>shown text</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>12345</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:t>Page</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:drawing><wp:inline><w:txbxContent><w:p><w:r><w:t>Inside a text box</w:t></w:r></w:p></w:txbxContent></wp:inline></w:drawing></w:r><w:r><w:t>After the box</w:t></w:r></w:p>"#,
        r#"</w:body></w:document>"#
    );

    fn texts(scan: &Scan) -> Vec<String> {
        scan.segments.iter().map(|s| s.nodes.iter().map(|&n| scan.nodes[n].text.as_str()).collect::<String>()).collect()
    }

    #[test]
    fn finds_the_text_of_every_paragraph() {
        let scan = scan(DOC, &WORD).unwrap();
        let all = texts(&scan);
        assert!(all.contains(&"Hello world".to_string()));
        assert!(all.contains(&"This is a very important sentence & more.".to_string()), "{all:?}");
        assert!(all.contains(&"Inside a text box".to_string()), "text boxes are paragraphs of their own");
        assert!(all.contains(&"After the box".to_string()));
    }

    #[test]
    fn tabs_split_a_paragraph_and_hidden_runs_and_fields_are_left_out() {
        let scan = scan(DOC, &WORD).unwrap();
        let all = texts(&scan);
        assert!(all.contains(&"Name".to_string()) && all.contains(&"Value".to_string()));
        assert!(!all.iter().any(|t| t.contains("hidden")));
        assert!(all.contains(&"shown text".to_string()));
        assert!(!all.iter().any(|t| t.contains("PAGE")), "field codes are not text");
    }

    #[test]
    fn rewriting_changes_only_the_text() {
        let scan = scan(DOC, &WORD).unwrap();
        let hello = scan.nodes.iter().position(|n| n.text == "Hello world").unwrap();
        let mut edits = HashMap::new();
        edits.insert(hello, "Hallo Welt & <Co>".to_string());
        let out = rewrite(DOC, &scan, &edits);
        assert!(out.contains("<w:t>Hallo Welt &amp; &lt;Co&gt;</w:t>"));
        assert_eq!(out.replace("Hallo Welt &amp; &lt;Co&gt;", "Hello world"), DOC, "nothing else moved");
        assert!(is_well_formed(&out));
    }

    #[test]
    fn leading_spaces_get_xml_space_preserve() {
        let scan = scan(DOC, &WORD).unwrap();
        let hello = scan.nodes.iter().position(|n| n.text == "Hello world").unwrap();
        let mut edits = HashMap::new();
        edits.insert(hello, " Hallo ".to_string());
        let out = rewrite(DOC, &scan, &edits);
        assert!(out.contains(r#"<w:t xml:space="preserve"> Hallo </w:t>"#), "{out}");
        assert!(is_well_formed(&out));
    }

    #[test]
    fn formatting_spans_get_markers_and_the_translation_goes_back_into_the_runs() {
        let scan = scan(DOC, &WORD).unwrap();
        let units = plan(&scan);
        let unit = units.iter().find(|u| u.text.contains("important")).unwrap();
        assert_eq!(unit.text, "{1}This is a {/1}{2}very important{/2}{3} sentence & more.{/3}");
        assert_eq!(unit.spans(), 3);
        let edits = unit.apply("{1}Das ist ein {/1}{2}sehr wichtiger{/2}{3} Satz & mehr.{/3}");
        let by_node: HashMap<usize, String> = edits.into_iter().collect();
        let node = |text: &str| scan.nodes.iter().position(|n| n.text == text).unwrap();
        assert_eq!(by_node[&node("This is a ")], "Das ist ein ");
        assert_eq!(by_node[&node("very important")], "sehr wichtiger");
        assert_eq!(by_node[&node(" sentence & more.")], " Satz & mehr.");
        let out = rewrite(DOC, &scan, &by_node);
        assert!(out.contains("<w:rPr><w:b/></w:rPr><w:t>sehr wichtiger</w:t>"), "the bold run keeps its formatting");
        assert!(is_well_formed(&out));
    }

    #[test]
    fn lost_markers_fall_back_to_cutting_by_proportion() {
        let scan = scan(DOC, &WORD).unwrap();
        let units = plan(&scan);
        let unit = units.iter().find(|u| u.text.contains("important")).unwrap();
        let edits = unit.apply("Das ist ein sehr wichtiger Satz und mehr.");
        let joined: String = edits.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(joined.split_whitespace().collect::<Vec<_>>().join(" "), "Das ist ein sehr wichtiger Satz und mehr.");
        // Nothing is lost and no marker text leaks.
        assert!(!joined.contains('{'));
    }

    #[test]
    fn marker_parsing_rejects_damaged_output() {
        assert_eq!(split_markers("{1}a{/1}{2}b{/2}", 2), Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(split_markers("{1}a{/1} y {2}b{/2} z", 2), Some(vec!["a y ".to_string(), "b z".to_string()]), "text between and after the markers stays with the span before it");
        assert_eq!(split_markers("x {1}a{/1}{2}b{/2}", 2), None, "text before the first marker is not guessed at");
        assert_eq!(split_markers("{1}a{/1}", 2), None);
        assert_eq!(split_markers("{2}b{/2}{1}a{/1}", 2), None, "out of order");
        assert_eq!(split_markers("{1}a{2}{/1}b{/2}", 2), None);
    }

    #[test]
    fn a_space_the_model_put_before_punctuation_is_removed() {
        let scan = scan(DOC, &WORD).unwrap();
        let units = plan(&scan);
        let unit = units.iter().find(|u| u.text.contains("important")).unwrap();
        let edits = unit.apply("{1}Das ist ein {/1}{2}sehr wichtiger{/2}{3} , Satz.{/3}");
        assert_eq!(edits.iter().map(|(_, t)| t.as_str()).collect::<String>(), "Das ist ein sehr wichtiger, Satz.");
    }

    #[test]
    fn proportional_cuts_happen_at_spaces() {
        let pieces = split_proportionally("one two three four five six", &[10, 10, 20]);
        assert_eq!(pieces.concat(), "one two three four five six");
        assert_eq!(pieces.len(), 3);
    }

    #[test]
    fn addresses_survive_the_round_trip() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Visit https://example.org/a now.</w:t></w:r></w:p></w:document>"#;
        let scan = scan(xml, &WORD).unwrap();
        let units = plan(&scan);
        assert_eq!(units[0].text, "Visit Uq1x now.");
        let edits = units[0].apply("Besuchen Sie Uq1x jetzt.");
        assert_eq!(edits[0].1, "Besuchen Sie https://example.org/a jetzt.");
    }

    #[test]
    fn excel_shared_strings_rich_text_and_phonetics() {
        let xml = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Total</t></si><si><r><rPr><b/></rPr><t>Net</t></r><r><t xml:space="preserve"> price</t></r></si><si><t>Hello</t><rPh sb="0" eb="1"><t>ハロー</t></rPh></si></sst>"#;
        let scan = scan(xml, &EXCEL_STRINGS).unwrap();
        assert_eq!(texts(&scan), vec!["Total", "Net price", "Hello"]);
    }

    #[test]
    fn malformed_xml_is_reported_not_guessed() {
        assert!(scan("<w:p><w:r><w:t>oops</w:p>", &WORD).is_err());
        assert!(!is_well_formed("<a><b></a>"));
    }
}
