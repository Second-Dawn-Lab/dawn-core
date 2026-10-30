//! Word (.docx), PowerPoint (.pptx) and Excel (.xlsx) files: zips of XML parts.
//!
//! Only the text of the parts that carry content is touched. Every other entry of the zip (images,
//! styles, themes, relationships, embedded files) is copied as stored, bit for bit.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::ooxml::{self, Dialect, Scan, Unit};
use crate::{DocError, Kind, Loaded, Writer};

/// The first bytes of a legacy Office / encrypted OOXML container (an OLE compound file).
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

struct PartPlan {
    name: String,
    xml: String,
    scan: Scan,
    units: Vec<Unit>,
    /// Index of this part's first unit in the document's unit list.
    first: usize,
}

struct OfficeWriter {
    input: PathBuf,
    kind: Kind,
    parts: Vec<PartPlan>,
    /// Shared strings that are table column names: Excel needs them to stay as they are.
    keep_original: HashSet<String>,
    note_formulas: bool,
}

fn matches_pattern(name: &str, dir: &str, prefix: &str) -> bool {
    name.strip_prefix(dir).is_some_and(|rest| rest.starts_with(prefix) && rest.ends_with(".xml") && !rest.contains('/'))
}

fn dialect_for(kind: Kind, name: &str) -> Option<&'static Dialect> {
    match kind {
        Kind::Docx => {
            let wanted = name == "word/document.xml" || matches_pattern(name, "word/", "header") || matches_pattern(name, "word/", "footer") || name == "word/footnotes.xml" || name == "word/endnotes.xml" || name == "word/comments.xml";
            wanted.then_some(&ooxml::WORD)
        }
        Kind::Pptx => {
            let wanted = matches_pattern(name, "ppt/slides/", "slide") || matches_pattern(name, "ppt/notesSlides/", "notesSlide");
            wanted.then_some(&ooxml::DRAWING)
        }
        Kind::Xlsx => {
            if name == "xl/sharedStrings.xml" || matches_pattern(name, "xl/worksheets/", "sheet") || matches_pattern(name, "xl/", "comments") {
                Some(&ooxml::EXCEL_STRINGS)
            } else if matches_pattern(name, "xl/drawings/", "drawing") {
                Some(&ooxml::DRAWING)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>, DocError> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 8];
    let n = file.read(&mut magic)?;
    if n == 8 && magic == OLE_MAGIC {
        // Password-protected OOXML files are stored in this container; so are old .doc/.xls files.
        return Err(DocError::Encrypted);
    }
    drop(file);
    ZipArchive::new(std::fs::File::open(path)?).map_err(|e| DocError::Corrupt(e.to_string()))
}

pub(crate) fn load(path: &Path, kind: Kind) -> Result<Loaded, DocError> {
    let mut zip = open_zip(path)?;
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let has = |needle: &str| names.iter().any(|n| n.starts_with(needle));
    let content_part = match kind {
        Kind::Docx => "word/document.xml",
        Kind::Pptx => "ppt/presentation.xml",
        _ => "xl/workbook.xml",
    };
    if !names.iter().any(|n| n == content_part) {
        return Err(DocError::Corrupt(format!("{content_part} is missing")));
    }

    let mut skipped = Vec::new();
    let mut parts = Vec::new();
    let mut units_text: Vec<String> = Vec::new();
    let mut ordered: Vec<&String> = names.iter().collect();
    ordered.sort_by_key(|n| natural_key(n));
    for name in ordered {
        let Some(dialect) = dialect_for(kind, name) else { continue };
        let mut data = Vec::new();
        zip.by_name(name).map_err(|e| DocError::Corrupt(e.to_string()))?.read_to_end(&mut data)?;
        let Ok(xml) = String::from_utf8(data) else {
            skipped.push(format!("{name}: not UTF-8, left untranslated"));
            continue;
        };
        let scan = match ooxml::scan(&xml, dialect) {
            Ok(s) => s,
            Err(why) => {
                skipped.push(format!("{name}: could not be read ({why}), left untranslated"));
                continue;
            }
        };
        let units = ooxml::plan(&scan);
        let first = units_text.len();
        units_text.extend(units.iter().map(|u| u.text.clone()));
        parts.push(PartPlan { name: name.clone(), xml, scan, units, first });
    }

    // Excel: table header cells must keep matching the table definition.
    let mut keep_original = HashSet::new();
    if kind == Kind::Xlsx {
        for name in names.iter().filter(|n| n.starts_with("xl/tables/") && n.ends_with(".xml")) {
            let mut xml = String::new();
            if zip.by_name(name).ok().and_then(|mut f| f.read_to_string(&mut xml).ok()).is_some() {
                keep_original.extend(table_column_names(&xml));
            }
        }
        if !keep_original.is_empty() {
            skipped.push("table headers were kept as they are (Excel ties them to the table definition)".into());
        }
    }
    let note_formulas = kind == Kind::Xlsx && parts.iter().any(|p| p.name.starts_with("xl/worksheets/") && (p.xml.contains("<f>") || p.xml.contains("<f ")));
    for (needle, what) in [
        ("word/charts/", "charts"),
        ("word/diagrams/", "SmartArt diagrams"),
        ("ppt/charts/", "charts"),
        ("ppt/diagrams/", "SmartArt diagrams"),
        ("xl/charts/", "charts"),
        ("xl/pivotTables/", "pivot tables"),
    ] {
        if has(needle) {
            skipped.push(format!("{what} were not translated (their text stays in the original language)"));
        }
    }

    let count = |dir: &str, prefix: &str| names.iter().filter(|n| matches_pattern(n, dir, prefix)).count();
    let parts_info = match kind {
        Kind::Pptx => Some((count("ppt/slides/", "slide"), "slides")),
        Kind::Xlsx => Some((count("xl/worksheets/", "sheet"), "sheets")),
        _ => None,
    };
    Ok(Loaded { units: units_text, parts: parts_info, skipped, writer: Box::new(OfficeWriter { input: path.to_path_buf(), kind, parts, keep_original, note_formulas }) })
}

/// `slide2.xml` before `slide10.xml`.
fn natural_key(name: &str) -> (String, usize, String) {
    let digits: String = name.chars().filter(char::is_ascii_digit).collect();
    let stem: String = name.chars().filter(|c| !c.is_ascii_digit()).collect();
    (stem, digits.parse().unwrap_or(0), name.to_string())
}

fn table_column_names(xml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<tableColumn ") {
        let tag_end = rest[start..].find('>').map_or(rest.len(), |i| start + i);
        let tag = &rest[start..tag_end];
        if let Some(at) = tag.find(" name=\"") {
            let value = &tag[at + 7..];
            if let Some(end) = value.find('"') {
                if let Ok(text) = quick_xml::escape::unescape(&value[..end]) {
                    names.push(text.into_owned());
                }
            }
        }
        rest = &rest[tag_end..];
    }
    names
}

impl Writer for OfficeWriter {
    fn write(&self, translations: &[String], out: &Path) -> Result<Vec<String>, DocError> {
        let mut notes = Vec::new();
        let mut replaced: HashMap<String, String> = HashMap::new();
        for part in &self.parts {
            let mut edits: HashMap<usize, String> = HashMap::new();
            for (i, unit) in part.units.iter().enumerate() {
                if self.kind == Kind::Xlsx && unit.spans() == 1 && self.keep_original.contains(unit.original()) {
                    continue;
                }
                for (node, text) in unit.apply(&translations[part.first + i]) {
                    edits.insert(node, text);
                }
            }
            if edits.is_empty() {
                continue;
            }
            let xml = ooxml::rewrite(&part.xml, &part.scan, &edits);
            if ooxml::is_well_formed(&xml) {
                replaced.insert(part.name.clone(), xml);
            } else {
                notes.push(format!("{}: the result was not valid, so this part was left untranslated", part.name));
            }
        }

        let mut zip = ZipArchive::new(std::fs::File::open(&self.input)?).map_err(|e| DocError::Corrupt(e.to_string()))?;
        let mut writer = ZipWriter::new(std::fs::File::create(out)?);
        for i in 0..zip.len() {
            let entry = zip.by_index_raw(i).map_err(|e| DocError::Corrupt(e.to_string()))?;
            let name = entry.name().to_string();
            match replaced.get(&name) {
                Some(xml) => {
                    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(entry.last_modified().unwrap_or_default());
                    drop(entry);
                    writer.start_file(name, options).map_err(|e| DocError::Corrupt(e.to_string()))?;
                    writer.write_all(xml.as_bytes())?;
                }
                None => writer.raw_copy_file(entry).map_err(|e| DocError::Corrupt(e.to_string()))?,
            }
        }
        writer.finish().map_err(|e| DocError::Corrupt(e.to_string()))?;
        if self.note_formulas {
            notes.push("formulas were not changed; a formula that compares a cell with fixed text (for example =IF(A1=\"Yes\", ...)) may need adjusting".into());
        }
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{translate_document, Translator};

    const CONTENT_TYPES: &[u8] = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;
    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    const STYLES: &[u8] = br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:styleId="Normal"/></w:styles>"#;

    fn docx(dir: &Path) -> PathBuf {
        let path = dir.join("letter.docx");
        let document = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Dear customer,</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Please read the </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>user manual</w:t></w:r><w:r><w:t xml:space="preserve"> carefully.</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2024</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        let header = br#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Company report</w:t></w:r></w:p></w:hdr>"#;
        let footnotes = br#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnote w:id="1"><w:p><w:r><w:t>See the appendix.</w:t></w:r></w:p></w:footnote></w:footnotes>"#;
        make_zip(
            &path,
            &[
                ("[Content_Types].xml", CONTENT_TYPES),
                ("word/document.xml", document),
                ("word/header1.xml", header),
                ("word/footnotes.xml", footnotes),
                ("word/styles.xml", STYLES),
                ("word/media/image1.png", PNG),
            ],
        );
        path
    }

    #[test]
    fn a_docx_is_translated_in_every_text_part_and_everything_else_is_byte_identical() {
        let dir = temp_dir("docx");
        let input = docx(&dir);
        let output = dir.join("letter (DE).docx");
        let report = translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        assert!(report.units >= 5, "{report:?}");

        let document = String::from_utf8(read_part(&output, "word/document.xml")).unwrap();
        assert!(document.contains("<w:t>[de]Dear customer,</w:t>"), "{document}");
        assert!(document.contains("<w:t>[de]Item</w:t>"));
        assert!(document.contains("<w:t>2024</w:t>"), "numbers stay");
        assert!(document.contains("<w:rPr><w:b/></w:rPr>"), "formatting stays");
        assert!(String::from_utf8(read_part(&output, "word/header1.xml")).unwrap().contains("[de]Company report"));
        assert!(String::from_utf8(read_part(&output, "word/footnotes.xml")).unwrap().contains("[de]See the appendix."));

        for untouched in ["[Content_Types].xml", "word/styles.xml", "word/media/image1.png"] {
            assert_eq!(read_part(&output, untouched), read_part(&input, untouched), "{untouched}");
        }
    }

    #[test]
    fn untouched_entries_are_copied_as_stored_not_recompressed() {
        let dir = temp_dir("raw");
        let input = docx(&dir);
        let output = dir.join("out.docx");
        translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        let (mut a, mut b) = (ZipArchive::new(std::fs::File::open(&input).unwrap()).unwrap(), ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap());
        let (ea, eb) = (a.by_name("word/media/image1.png").unwrap(), b.by_name("word/media/image1.png").unwrap());
        assert_eq!((ea.crc32(), ea.compressed_size(), ea.compression()), (eb.crc32(), eb.compressed_size(), eb.compression()));
    }

    #[test]
    fn inline_formatting_is_sent_with_markers_and_comes_back_in_the_right_runs() {
        struct German;
        impl Translator for German {
            fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, DocError> {
                Ok(texts
                    .iter()
                    .map(|t| match t.as_str() {
                        "{1}Please read the {/1}{2}user manual{/2}{3} carefully.{/3}" => "{1}Bitte lesen Sie die {/1}{2}Bedienungsanleitung{/2}{3} sorgfältig.{/3}".to_string(),
                        other => other.to_string(),
                    })
                    .collect())
            }
        }
        let dir = temp_dir("inline");
        let input = docx(&dir);
        let output = dir.join("out.docx");
        translate_document(&input, &output, &mut German, &mut |_| {}).unwrap();
        let document = String::from_utf8(read_part(&output, "word/document.xml")).unwrap();
        assert!(document.contains(r#"<w:t xml:space="preserve">Bitte lesen Sie die </w:t>"#), "{document}");
        assert!(document.contains("<w:rPr><w:b/></w:rPr><w:t>Bedienungsanleitung</w:t>"), "the bold run holds the bold words");
        assert!(document.contains(r#"<w:t xml:space="preserve"> sorgfältig.</w:t>"#));
    }

    #[test]
    fn a_translator_that_breaks_the_markers_still_gives_a_valid_document() {
        struct Sloppy;
        impl Translator for Sloppy {
            fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, DocError> {
                Ok(texts.iter().map(|t| t.replace("{2}", "").replace("{/3}", "").replace('{', "<<")).collect())
            }
        }
        let dir = temp_dir("sloppy");
        let input = docx(&dir);
        let output = dir.join("out.docx");
        translate_document(&input, &output, &mut Sloppy, &mut |_| {}).unwrap();
        assert!(ooxml::is_well_formed(&String::from_utf8(read_part(&output, "word/document.xml")).unwrap()));
    }

    #[test]
    fn a_pptx_translates_slides_and_notes_and_counts_slides() {
        let dir = temp_dir("pptx");
        let input = dir.join("deck.pptx");
        let slide = |text: &str| format!(r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp><p:graphicFrame><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Cell text</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></p:graphicFrame></p:spTree></p:cSld></p:sld>"#);
        let (s1, s2, s10) = (slide("First slide"), slide("Second slide"), slide("Tenth slide"));
        let notes = br#"<p:notes xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Speaker notes here</a:t></a:r></a:p></p:notes>"#;
        make_zip(
            &input,
            &[
                ("[Content_Types].xml", CONTENT_TYPES),
                ("ppt/presentation.xml", b"<p:presentation xmlns:p=\"p\"/>"),
                ("ppt/slides/slide1.xml", s1.as_bytes()),
                ("ppt/slides/slide2.xml", s2.as_bytes()),
                ("ppt/slides/slide10.xml", s10.as_bytes()),
                ("ppt/notesSlides/notesSlide1.xml", notes),
                ("ppt/charts/chart1.xml", b"<c:chart xmlns:c=\"c\"/>"),
                ("ppt/media/image1.png", PNG),
            ],
        );
        let output = dir.join("deck (DE).pptx");
        let report = translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        assert_eq!(report.parts, Some((3, "slides")));
        assert!(report.skipped.iter().any(|s| s.contains("charts")));
        assert!(String::from_utf8(read_part(&output, "ppt/slides/slide1.xml")).unwrap().contains("<a:t>[de]First slide</a:t>"));
        assert!(String::from_utf8(read_part(&output, "ppt/slides/slide1.xml")).unwrap().contains("<a:t>[de]Cell text</a:t>"));
        assert!(String::from_utf8(read_part(&output, "ppt/notesSlides/notesSlide1.xml")).unwrap().contains("[de]Speaker notes here"));
        assert_eq!(read_part(&output, "ppt/media/image1.png"), PNG);
    }

    fn xlsx(dir: &Path, with_table: bool) -> PathBuf {
        let path = dir.join(if with_table { "table.xlsx" } else { "sheet.xlsx" });
        let shared = br#"<sst xmlns="x" count="4" uniqueCount="4"><si><t>Name</t></si><si><t>Total price</t></si><si><t>Paid</t></si><si><t>2024-05-01</t></si></sst>"#;
        let sheet = br#"<worksheet xmlns="x"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><f>SUM(C2:C9)</f><v>42</v></c><c r="C2" t="inlineStr"><is><t>Inline text</t></is></c></row></sheetData></worksheet>"#;
        let table = br#"<table xmlns="x" name="T1"><tableColumns count="1"><tableColumn id="1" name="Total price"/></tableColumns></table>"#;
        let mut parts: Vec<(&str, &[u8])> = vec![
            ("[Content_Types].xml", CONTENT_TYPES),
            ("xl/workbook.xml", b"<workbook xmlns=\"x\"><sheets><sheet name=\"Data\" sheetId=\"1\"/></sheets></workbook>"),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet),
        ];
        if with_table {
            parts.push(("xl/tables/table1.xml", table));
        }
        make_zip(&path, &parts);
        path
    }

    #[test]
    fn an_xlsx_translates_text_cells_and_leaves_formulas_numbers_dates_and_sheet_names_alone() {
        let dir = temp_dir("xlsx");
        let input = xlsx(&dir, false);
        let output = dir.join("out.xlsx");
        let report = translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        let shared = String::from_utf8(read_part(&output, "xl/sharedStrings.xml")).unwrap();
        assert!(shared.contains("<t>[de]Name</t>") && shared.contains("<t>[de]Paid</t>"));
        assert!(shared.contains("<t>2024-05-01</t>"), "dates stay");
        let sheet = String::from_utf8(read_part(&output, "xl/worksheets/sheet1.xml")).unwrap();
        assert!(sheet.contains("<f>SUM(C2:C9)</f><v>42</v>"), "formulas and cached values stay");
        assert!(sheet.contains("<t>[de]Inline text</t>"));
        assert_eq!(read_part(&output, "xl/workbook.xml"), read_part(&input, "xl/workbook.xml"), "sheet names stay");
        assert_eq!(report.parts, Some((1, "sheets")));
        assert!(report.skipped.iter().any(|s| s.contains("formula")));
    }

    #[test]
    fn excel_table_headers_are_kept_so_the_table_stays_valid() {
        let dir = temp_dir("xlsx-table");
        let input = xlsx(&dir, true);
        let output = dir.join("out.xlsx");
        let report = translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        let shared = String::from_utf8(read_part(&output, "xl/sharedStrings.xml")).unwrap();
        assert!(shared.contains("<t>Total price</t>"), "the header named in the table is not translated");
        assert!(shared.contains("<t>[de]Name</t>"));
        assert_eq!(read_part(&output, "xl/tables/table1.xml"), read_part(&input, "xl/tables/table1.xml"));
        assert!(report.skipped.iter().any(|s| s.contains("table headers")));
    }

    #[test]
    fn a_password_protected_file_is_reported_as_encrypted() {
        let dir = temp_dir("encrypted");
        let path = dir.join("secret.docx");
        let mut data = OLE_MAGIC.to_vec();
        data.extend_from_slice(&[0u8; 512]);
        std::fs::write(&path, data).unwrap();
        let err = translate_document(&path, &dir.join("o.docx"), &mut Tag("de"), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "encrypted");
    }

    #[test]
    fn a_file_that_is_not_a_zip_or_misses_its_main_part_is_corrupt() {
        let dir = temp_dir("corrupt");
        let path = dir.join("bad.docx");
        std::fs::write(&path, b"this is not a zip file at all").unwrap();
        assert_eq!(translate_document(&path, &dir.join("o.docx"), &mut Tag("de"), &mut |_| {}).unwrap_err().code(), "corrupt");
        let empty = dir.join("empty.docx");
        make_zip(&empty, &[("[Content_Types].xml", CONTENT_TYPES)]);
        assert_eq!(translate_document(&empty, &dir.join("o2.docx"), &mut Tag("de"), &mut |_| {}).unwrap_err().code(), "corrupt");
    }

    #[test]
    fn a_part_that_cannot_be_read_is_left_untranslated_and_listed() {
        let dir = temp_dir("badpart");
        let path = dir.join("odd.docx");
        make_zip(
            &path,
            &[
                ("[Content_Types].xml", CONTENT_TYPES),
                ("word/document.xml", br#"<w:document xmlns:w="w"><w:p><w:r><w:t>Fine text</w:t></w:r></w:p></w:document>"#),
                ("word/header1.xml", b"<w:hdr><w:p><w:r><w:t>broken</w:p></w:hdr>"),
            ],
        );
        let output = dir.join("out.docx");
        let report = translate_document(&path, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        assert!(report.skipped.iter().any(|s| s.contains("word/header1.xml")));
        assert_eq!(read_part(&output, "word/header1.xml"), read_part(&path, "word/header1.xml"));
        assert!(String::from_utf8(read_part(&output, "word/document.xml")).unwrap().contains("[de]Fine text"));
    }

    #[test]
    fn units_are_numbered_in_reading_order_across_numbered_slides() {
        assert!(natural_key("ppt/slides/slide2.xml") < natural_key("ppt/slides/slide10.xml"));
    }
}
