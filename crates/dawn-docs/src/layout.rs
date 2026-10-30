//! Where each text segment of a document sits ("Slide 4 · Notes", "Sheet Sales · B5", "Page 2") and how it is
//! formatted, for an editor that shows the source and its translation side by side.
//!
//! Nothing here changes how a document is translated or written; it only describes what was found.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

use quick_xml::events::Event;
use zip::ZipArchive;

/// Where a segment is. The UI turns it into a localized label; `part` is one of
/// `body`, `header`, `footer`, `footnotes`, `endnotes`, `comments`, `slide`, `notes`, `sheet`, `comment`,
/// `drawing`, `page`, `paragraph`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub part: &'static str,
    /// A sheet's name.
    pub name: Option<String>,
    /// The slide, page or paragraph number (1-based).
    pub number: Option<usize>,
    /// A cell reference such as `B5`.
    pub cell: Option<String>,
}

/// The formatting of one stretch of a segment that has markers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub link: bool,
}

/// The tag `<prefix:name ...>` or `<name .../>` of the first element called `name` (any prefix) in `key`.
fn first_tag<'a>(key: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = key;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let end = after.find('>')?;
        let tag = &after[..end];
        let element = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        let local = element.rsplit(':').next().unwrap_or("");
        if local == name && !tag.starts_with('/') {
            return Some(tag);
        }
        rest = &after[end + 1..];
    }
    None
}

/// `<w:b/>` or `<b/>` is on; `<w:b w:val="0"/>` and `val="false"` are off.
fn flag(key: &str, name: &str) -> bool {
    match first_tag(key, name) {
        Some(tag) => !(tag.contains("val=\"0\"") || tag.contains("val=\"false\"") || tag.contains("val=\"none\"")),
        None => false,
    }
}

/// The formatting a run-properties string stands for. Word and Excel write `<w:b/>`, PowerPoint `b="1"`.
pub fn style_from_key(key: &str) -> SpanStyle {
    if key.is_empty() {
        return SpanStyle::default();
    }
    let head = key.split('>').next().unwrap_or("");
    let attr = |name: &str, on: &str| head.contains(&format!(" {name}=\"{on}\""));
    SpanStyle {
        bold: flag(key, "b") || attr("b", "1") || attr("b", "true"),
        italic: flag(key, "i") || attr("i", "1") || attr("i", "true"),
        underline: flag(key, "u") || head.contains(" u=\"sng\"") || head.contains(" u=\"dbl\""),
        link: key.contains("Hyperlink") || first_tag(key, "hlinkClick").is_some(),
    }
}

fn read_text(zip: &mut ZipArchive<File>, name: &str) -> Option<String> {
    let mut xml = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut xml).ok()?;
    Some(xml)
}

/// `Id -> target` of a relationships part.
fn relationships(zip: &mut ZipArchive<File>, rels: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(xml) = read_text(zip, rels) else { return map };
    let mut reader = quick_xml::Reader::from_str(&xml);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                let (mut id, mut target) = (None, None);
                for a in e.attributes().flatten() {
                    let value = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                    match a.key.as_ref() {
                        b"Id" => id = Some(value),
                        b"Target" => target = Some(value),
                        _ => {}
                    }
                }
                if let (Some(id), Some(target)) = (id, target) {
                    map.insert(id, target);
                }
            }
            _ => {}
        }
    }
    map
}

/// A relationship target as a part name: absolute (`/xl/a.xml`) or relative to the folder of the source part.
pub fn resolve_target(source_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut dir: Vec<&str> = source_part.split('/').collect();
    dir.pop();
    for piece in target.split('/') {
        match piece {
            ".." => {
                dir.pop();
            }
            "." | "" => {}
            other => dir.push(other),
        }
    }
    dir.join("/")
}

fn rels_name(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// The `element`s of `part` in order (`sheet` of a workbook, `sldId` of a presentation) with their `name`
/// attribute and the part their `r:id` points to.
pub fn ordered_targets(zip: &mut ZipArchive<File>, part: &str, element: &str) -> Vec<(Option<String>, String)> {
    let Some(xml) = read_text(zip, part) else { return Vec::new() };
    let rels = relationships(zip, &rels_name(part));
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = Vec::new();
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == element.as_bytes() => {
                let (mut name, mut rid) = (None, None);
                for a in e.attributes().flatten() {
                    let value = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                    let key = a.key.as_ref();
                    if key == b"name" {
                        name = Some(value);
                    } else if key.contains(&b':') && a.key.local_name().as_ref() == b"id" {
                        rid = Some(value);
                    }
                }
                if let Some(target) = rid.and_then(|r| rels.get(&r)) {
                    out.push((name, resolve_target(part, target)));
                }
            }
            _ => {}
        }
    }
    out
}

/// The byte offsets of every start tag called `name` (any prefix) in `xml`, in order, with the value of `attr`.
pub fn starts_of(xml: &str, name: &str, attr: Option<&str>) -> Vec<(usize, Option<String>)> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        let at = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == name.as_bytes() => {
                let value = attr.and_then(|wanted| {
                    e.attributes().flatten().find(|a| a.key.as_ref() == wanted.as_bytes()).and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
                });
                out.push((at, value));
            }
            _ => {}
        }
    }
    out
}

/// For every shared string index: the first cell that shows it, as `(sheet name, reference)`.
pub fn shared_string_cells(zip: &mut ZipArchive<File>) -> HashMap<usize, (String, String)> {
    let mut map = HashMap::new();
    for (name, part) in ordered_targets(zip, "xl/workbook.xml", "sheet") {
        let Some(xml) = read_text(zip, &part) else { continue };
        let mut reader = quick_xml::Reader::from_str(&xml);
        let mut cell: Option<(String, bool)> = None;
        let mut in_value = false;
        while let Ok(event) = reader.read_event() {
            match event {
                Event::Eof => break,
                Event::Start(e) if e.local_name().as_ref() == b"c" => {
                    let (mut r, mut shared) = (String::new(), false);
                    for a in e.attributes().flatten() {
                        match a.key.as_ref() {
                            b"r" => r = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
                            b"t" => shared = a.value.as_ref() == b"s",
                            _ => {}
                        }
                    }
                    cell = Some((r, shared));
                }
                Event::Start(e) if e.local_name().as_ref() == b"v" => in_value = cell.as_ref().is_some_and(|c| c.1),
                Event::Text(t) if in_value => {
                    if let (Some((r, _)), Ok(text)) = (cell.as_ref(), t.decode()) {
                        if let Ok(index) = text.trim().parse::<usize>() {
                            map.entry(index).or_insert_with(|| (name.clone().unwrap_or_default(), r.clone()));
                        }
                    }
                }
                Event::End(e) if e.local_name().as_ref() == b"v" => in_value = false,
                Event::End(e) if e.local_name().as_ref() == b"c" => cell = None,
                _ => {}
            }
        }
    }
    map
}

/// `1`-based position of every presentation slide part (`ppt/slides/slide3.xml` -> 1), in presentation order.
pub fn slide_positions(zip: &mut ZipArchive<File>) -> HashMap<String, usize> {
    ordered_targets(zip, "ppt/presentation.xml", "sldId").into_iter().enumerate().map(|(i, (_, part))| (part, i + 1)).collect()
}

/// The slide a notes page belongs to.
pub fn notes_slide(zip: &mut ZipArchive<File>, notes_part: &str) -> Option<String> {
    let rels = relationships(zip, &rels_name(notes_part));
    rels.values().find(|t| t.contains("slides/slide")).map(|t| resolve_target(notes_part, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_properties_become_styles_whatever_the_prefix() {
        assert_eq!(style_from_key(""), SpanStyle::default());
        assert!(style_from_key("<w:rPr><w:b/></w:rPr>").bold);
        assert!(style_from_key("<x:rPr><x:i/></x:rPr>").italic);
        assert!(style_from_key("<rPr><b/><u/></rPr>").underline);
        assert!(!style_from_key(r#"<w:rPr><w:b w:val="0"/></w:rPr>"#).bold);
        assert!(style_from_key(r#"<a:rPr lang="en-US" b="1" i="1"><a:solidFill/></a:rPr>"#).bold);
        assert!(style_from_key(r#"<a:rPr b="1" i="1"/>"#).italic);
        assert!(style_from_key(r#"<w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr>"#).link);
        assert!(!style_from_key("<w:rPr><w:sz w:val=\"22\"/></w:rPr>").bold, "a size is not bold");
        assert!(!style_from_key("<w:rPr><w:bCs/></w:rPr>").bold, "complex-script bold is another element");
    }

    #[test]
    fn relationship_targets_resolve_like_the_package_format_says() {
        assert_eq!(resolve_target("ppt/presentation.xml", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve_target("ppt/notesSlides/notesSlide1.xml", "../slides/slide5.xml"), "ppt/slides/slide5.xml");
        assert_eq!(resolve_target("xl/workbook.xml", "/xl/worksheets/sheet2.xml"), "xl/worksheets/sheet2.xml");
        assert_eq!(rels_name("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
    }

    #[test]
    fn start_tags_are_found_with_their_offsets_and_an_attribute() {
        let xml = r#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c><x:c r="B5"><v>1</v></x:c></row></sheetData></worksheet>"#;
        let found = starts_of(xml, "c", Some("r"));
        assert_eq!(found.iter().map(|(_, r)| r.clone().unwrap()).collect::<Vec<_>>(), ["A1", "B5"]);
        assert!(found[0].0 < found[1].0);
        assert!(xml[found[1].0..].starts_with("<x:c"));
    }
}
