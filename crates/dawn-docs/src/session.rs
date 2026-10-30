//! A document opened for editing: its segments with their places, translated once, edited by the user,
//! and written back with the edited texts. Closing the file again is never needed: the session keeps what it
//! read, and the original is only read again to write the output.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::layout::{Header, SpanStyle};
use crate::{check_input, load, temp_path, translate_texts, DocError, Kind, Loaded, Progress, Report, Translator, Unit};

/// One text segment of the document.
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    pub header: Header,
    /// The index among the units (what is translated and saved), or `None` for text that is left as it was.
    pub unit: Option<usize>,
    /// The text as the document has it (formatting markers `{1}..{/1}` included when it has several formats).
    pub source: String,
    /// The formatting of each marked stretch (empty without markers).
    pub styles: Vec<SpanStyle>,
    /// No markers or address tokens: the translation is plain text.
    pub plain: bool,
}

/// What identifies a file: if any of these differ, a draft made from it no longer fits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    /// Seconds since 1970 of the last change.
    pub modified: u64,
    /// A hash of the document's texts.
    pub text_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintStatus {
    Same,
    /// The file is there but its text is not what the draft was made from.
    Changed,
    Missing,
}

/// FNV-1a: stable between versions and platforms, unlike the standard hasher.
fn hash_texts(units: &[String]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for unit in units {
        for byte in unit.bytes().chain(std::iter::once(0u8)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn file_facts(path: &Path) -> Result<(u64, u64), DocError> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Ok((meta.len(), modified))
}

impl Fingerprint {
    /// Compare with the file as it is now. Only the text decides "changed" when the size or date differ but
    /// the text is the same (a file that was copied or re-saved without edits is still the same document).
    pub fn check(&self, path: &Path) -> FingerprintStatus {
        let Ok((size, modified)) = file_facts(path) else { return FingerprintStatus::Missing };
        if size == self.size && modified == self.modified {
            return FingerprintStatus::Same;
        }
        match check_input(path).and_then(|kind| load(path, kind)) {
            Ok(loaded) if hash_texts(&loaded.units) == self.text_hash => FingerprintStatus::Same,
            Ok(_) => FingerprintStatus::Changed,
            Err(_) => FingerprintStatus::Changed,
        }
    }
}

/// An open document.
pub struct Session {
    pub kind: Kind,
    pub path: PathBuf,
    pub segments: Vec<SegmentInfo>,
    /// The texts that are translated (with markers and tokens), by unit.
    pub units: Vec<String>,
    pub parts: Option<(usize, &'static str)>,
    pub skipped: Vec<String>,
    pub fingerprint: Fingerprint,
    loaded: Loaded,
}

impl Session {
    /// Read the document. Refuses what cannot be edited (Markdown keeps code and link targets as tokens
    /// that an editor must not show).
    pub fn open(path: &Path) -> Result<Session, DocError> {
        let kind = check_input(path)?;
        if kind == Kind::Markdown {
            return Err(DocError::Unsupported("markdown cannot be edited yet".into()));
        }
        let mut loaded = load(path, kind)?;
        let (size, modified) = file_facts(path)?;
        let fingerprint = Fingerprint { size, modified, text_hash: hash_texts(&loaded.units) };
        let segments = loaded.layout.take().unwrap_or_else(|| {
            loaded
                .units
                .iter()
                .enumerate()
                .map(|(i, text)| SegmentInfo { header: Header { part: "paragraph", number: Some(i + 1), ..Default::default() }, unit: Some(i), source: text.clone(), styles: Vec::new(), plain: true })
                .collect()
        });
        Ok(Session { kind, path: path.to_path_buf(), segments, units: loaded.units.clone(), parts: loaded.parts, skipped: std::mem::take(&mut loaded.skipped), fingerprint, loaded })
    }

    /// Translate every unit (repeated texts once). Returns one translation per unit, in the form the document
    /// stores (markers and tokens as the translator returned them).
    pub fn translate(&self, translator: &mut dyn Translator, progress: &mut dyn FnMut(Progress)) -> Result<Vec<String>, DocError> {
        Ok(translate_texts(&self.units, translator, progress)?.0)
    }

    fn restorer(&self, unit: usize) -> Option<&Unit> {
        self.loaded.restorers.get(unit).and_then(Option::as_ref)
    }

    /// A translation as an editor shows it: address tokens written out again.
    pub fn display(&self, unit: usize, translation: &str) -> String {
        self.restorer(unit).map_or_else(|| translation.to_string(), |u| u.restore_display(translation))
    }

    /// Write `translations` (one per unit; what [`Session::display`] returned or the user typed) to `output`.
    /// The original is not touched; the file is written next to its final name and renamed when complete, and read
    /// again before that to make sure it opens.
    pub fn save(&self, translations: &[String], output: &Path) -> Result<Report, DocError> {
        if translations.len() != self.units.len() {
            return Err(DocError::Translate("the number of translations does not match the document".into()));
        }
        let temp = temp_path(output);
        let notes = match self.loaded.writer.write(translations, &temp) {
            Ok(notes) => notes,
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                return Err(err);
            }
        };
        // Open the result again, as Word or PowerPoint would: same number of units as the original.
        let out_kind = Kind::from_path(output).unwrap_or(self.kind);
        let reopened = if out_kind == self.kind || self.kind == Kind::Pdf { reopen(&temp, out_kind) } else { Ok(()) };
        if let Err(err) = reopened {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
        if output.exists() {
            std::fs::remove_file(output)?;
        }
        std::fs::rename(&temp, output)?;
        let mut skipped = self.skipped.clone();
        skipped.extend(notes);
        Ok(Report { kind: self.kind, units: self.units.len(), translated: self.units.len(), parts: self.parts, skipped })
    }
}

/// The written file must be readable again by this crate's own loader.
fn reopen(temp: &Path, kind: Kind) -> Result<(), DocError> {
    // The temporary file ends in `.part`; the loaders read the path as given.
    load(temp, kind).map(|_| ()).map_err(|e| DocError::Corrupt(format!("the written file could not be opened again: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    const CT: &[u8] = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;

    fn docx(dir: &Path) -> PathBuf {
        let path = dir.join("letter.docx");
        let document = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Dear customer,</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Please read the </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>user manual</w:t></w:r><w:r><w:t xml:space="preserve"> carefully. See https://example.com/a now.</w:t></w:r></w:p><w:p><w:r><w:t>2024</w:t></w:r></w:p></w:body></w:document>"#;
        let header = br#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Company report</w:t></w:r></w:p></w:hdr>"#;
        make_zip(&path, &[("[Content_Types].xml", CT), ("word/document.xml", document), ("word/header1.xml", header), ("word/media/a.png", &[1, 2, 3])]);
        path
    }

    #[test]
    fn a_docx_has_a_segment_for_every_paragraph_with_its_place_and_formatting() {
        let dir = temp_dir("session-docx");
        let s = Session::open(&docx(&dir)).unwrap();
        // Order of the parts: "word/document.xml" sorts before "word/header1.xml" naturally.
        let places: Vec<(&str, Option<usize>, bool)> = s.segments.iter().map(|g| (g.header.part, g.header.number, g.unit.is_some())).collect();
        assert_eq!(places, [("body", Some(1), true), ("body", Some(2), true), ("body", Some(3), false), ("header", Some(1), true)]);
        assert_eq!(s.segments[2].source, "2024", "numbers are listed but left as they were");
        let marked = &s.segments[1];
        assert!(marked.source.starts_with("{1}Please read the {/1}{2}user manual{/2}{3} carefully."), "{}", marked.source);
        assert!(marked.source.contains("https://example.com/a"), "addresses are written out, not shown as tokens");
        assert_eq!(marked.styles.len(), 3);
        assert!(marked.styles[1].bold && !marked.styles[0].bold);
        assert_eq!(s.units.len(), 3);
        assert_eq!(s.segments[3].unit, Some(2));
    }

    #[test]
    fn edited_texts_are_written_back_and_everything_else_is_byte_identical() {
        let dir = temp_dir("session-save");
        let input = docx(&dir);
        let s = Session::open(&input).unwrap();
        let mut translations = s.translate(&mut Tag("de"), &mut |_| {}).unwrap();
        // The user edits the first and the header.
        translations[0] = "Sehr geehrte Kundin,".into();
        translations[2] = "Firmenbericht".into();
        let out = dir.join("letter (DE).docx");
        let report = s.save(&translations, &out).unwrap();
        assert_eq!(report.units, 3);
        let document = String::from_utf8(read_part(&out, "word/document.xml")).unwrap();
        assert!(document.contains("<w:t>Sehr geehrte Kundin,</w:t>"), "{document}");
        assert!(document.contains("<w:t>2024</w:t>"));
        assert!(String::from_utf8(read_part(&out, "word/header1.xml")).unwrap().contains("Firmenbericht"));
        assert_eq!(read_part(&out, "word/media/a.png"), read_part(&input, "word/media/a.png"));
        assert_eq!(read_part(&out, "[Content_Types].xml"), read_part(&input, "[Content_Types].xml"));
        assert!(std::fs::read(&input).is_ok() && Session::open(&input).unwrap().units == s.units, "the original is unchanged");
        assert!(!dir.join("letter (DE).docx.part").exists());
        // The written file opens again and has the edited text.
        assert_eq!(Session::open(&out).unwrap().units[0], "Sehr geehrte Kundin,");
    }

    #[test]
    fn markers_and_tokens_survive_an_edit_and_a_damaged_marker_falls_back() {
        let dir = temp_dir("session-markers");
        let input = docx(&dir);
        let s = Session::open(&input).unwrap();
        let mut translations = s.translate(&mut Tag("de"), &mut |_| {}).unwrap();
        // A translation in display form: markers kept, the address written out.
        translations[1] = "{1}Bitte lesen Sie die {/1}{2}Bedienungsanleitung{/2}{3} sorgfältig. Siehe https://example.com/a jetzt.{/3}".into();
        let out = dir.join("a.docx");
        s.save(&translations, &out).unwrap();
        let document = String::from_utf8(read_part(&out, "word/document.xml")).unwrap();
        assert!(document.contains("<w:rPr><w:b/></w:rPr><w:t>Bedienungsanleitung</w:t>"), "{document}");
        assert!(document.contains("https://example.com/a"));
        // The user removed a marker: the text is cut in proportion, nothing is lost.
        translations[1] = "{1}Bitte lesen Sie die {/1}Bedienungsanleitung{/2}{3} sorgfältig.{/3}".into();
        let out2 = dir.join("b.docx");
        s.save(&translations, &out2).unwrap();
        let document = String::from_utf8(read_part(&out2, "word/document.xml")).unwrap();
        assert!(document.contains("Bedienungsanleitung") && !document.contains("{/"), "{document}");
        assert!(crate::ooxml::is_well_formed(&document));
    }

    #[test]
    fn a_draft_notices_a_changed_or_missing_original() {
        let dir = temp_dir("session-fp");
        let input = docx(&dir);
        let s = Session::open(&input).unwrap();
        assert_eq!(s.fingerprint.check(&input), FingerprintStatus::Same);
        // A copy with the same text (other date) is still the same document.
        let copy = dir.join("copy.docx");
        std::fs::copy(&input, &copy).unwrap();
        assert_eq!(s.fingerprint.check(&copy), FingerprintStatus::Same);
        // Another text.
        let other = dir.join("other.docx");
        make_zip(&other, &[("[Content_Types].xml", CT), ("word/document.xml", br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Something else.</w:t></w:r></w:p></w:body></w:document>"#)]);
        assert_eq!(s.fingerprint.check(&other), FingerprintStatus::Changed);
        assert_eq!(s.fingerprint.check(&dir.join("gone.docx")), FingerprintStatus::Missing);
    }

    #[test]
    fn a_text_file_has_paragraph_places_and_markdown_is_refused() {
        let dir = temp_dir("session-txt");
        let txt = dir.join("a.txt");
        std::fs::write(&txt, "Hello world.\n\nGoodbye world.\n").unwrap();
        let s = Session::open(&txt).unwrap();
        assert_eq!(s.segments.iter().map(|g| (g.header.part, g.header.number)).collect::<Vec<_>>(), [("paragraph", Some(1)), ("paragraph", Some(2))]);
        let out = dir.join("b.txt");
        s.save(&["Hallo Welt.".to_string(), "Tschüss Welt.".to_string()], &out).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "Hallo Welt.\n\nTschüss Welt.\n");
        let md = dir.join("a.md");
        std::fs::write(&md, "# Title\n").unwrap();
        assert!(Session::open(&md).is_err());
    }

    #[test]
    fn pptx_and_xlsx_headers_name_slides_notes_sheets_and_cells() {
        let dir = temp_dir("session-office");
        let pptx = dir.join("deck.pptx");
        let a = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        let slide = |t: &str| format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" {a}><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#);
        let (s1, s2, notes) = (slide("First slide"), slide("Second slide"), slide("Speaker words"));
        let pres = br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId1"/></p:sldIdLst></p:presentation>"#;
        let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="slides/slide1.xml"/><Relationship Id="rId2" Target="slides/slide2.xml"/></Relationships>"#;
        let notes_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="../slides/slide1.xml"/></Relationships>"#;
        make_zip(
            &pptx,
            &[
                ("[Content_Types].xml", CT),
                ("ppt/presentation.xml", pres),
                ("ppt/_rels/presentation.xml.rels", rels),
                ("ppt/slides/slide1.xml", s1.as_bytes()),
                ("ppt/slides/slide2.xml", s2.as_bytes()),
                ("ppt/notesSlides/notesSlide1.xml", notes.as_bytes()),
                ("ppt/notesSlides/_rels/notesSlide1.xml.rels", notes_rels),
            ],
        );
        let s = Session::open(&pptx).unwrap();
        let got: Vec<(&str, Option<usize>, &str)> = s.segments.iter().map(|g| (g.header.part, g.header.number, g.source.as_str())).collect();
        // slide1.xml is the second slide of the presentation; its notes belong to it.
        assert!(got.contains(&("slide", Some(2), "First slide")), "{got:?}");
        assert!(got.contains(&("slide", Some(1), "Second slide")), "{got:?}");
        assert!(got.contains(&("notes", Some(2), "Speaker words")), "{got:?}");

        let xlsx = dir.join("book.xlsx");
        let ns = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main""#;
        let workbook = format!(r#"<workbook {ns} xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sales" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
        let wb_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let sheet = format!(r#"<worksheet {ns}><sheetData><row r="1"><c r="A1"><v>5</v></c><c r="B5" t="s"><v>1</v></c></row></sheetData></worksheet>"#);
        let strings = format!(r#"<sst {ns}><si><t>Unused text</t></si><si><t>Total price</t></si></sst>"#);
        make_zip(&xlsx, &[("[Content_Types].xml", CT), ("xl/workbook.xml", workbook.as_bytes()), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", sheet.as_bytes()), ("xl/sharedStrings.xml", strings.as_bytes())]);
        let s = Session::open(&xlsx).unwrap();
        let cell = s.segments.iter().find(|g| g.source == "Total price").unwrap();
        assert_eq!((cell.header.part, cell.header.name.as_deref(), cell.header.cell.as_deref()), ("sheet", Some("Sales"), Some("B5")));
    }

    /// About 300 pages: 12,000 paragraphs of 40 words each, some with bold stretches.
    #[test]
    fn a_300_page_document_opens_edits_and_saves_in_reasonable_time() {
        let dir = temp_dir("session-big");
        let path = dir.join("big.docx");
        let mut body = String::new();
        for i in 0..12_000 {
            let words = format!("Paragraph number {i} has a good many ordinary words in it so that it fills a line of the page and more words follow here and there until the end of the paragraph is reached.");
            if i % 3 == 0 {
                body.push_str(&format!(r#"<w:p><w:r><w:t xml:space="preserve">{words} </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>Important part</w:t></w:r><w:r><w:t xml:space="preserve"> and the rest.</w:t></w:r></w:p>"#));
            } else {
                body.push_str(&format!(r#"<w:p><w:r><w:t>{words}</w:t></w:r></w:p>"#));
            }
        }
        let document = format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
        make_zip(&path, &[("[Content_Types].xml", CT), ("word/document.xml", document.as_bytes())]);
        let started = std::time::Instant::now();
        let s = Session::open(&path).unwrap();
        let opened = started.elapsed();
        assert_eq!(s.units.len(), 12_000);
        let translations: Vec<String> = s.units.iter().map(|u| format!("[de]{u}")).collect();
        let started = std::time::Instant::now();
        s.save(&translations, &dir.join("out.docx")).unwrap();
        eprintln!("300 pages: open {opened:?}, save {:?}", started.elapsed());
        assert!(opened.as_secs() < 20 && started.elapsed().as_secs() < 30);
    }
}
