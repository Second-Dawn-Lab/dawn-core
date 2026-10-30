//! PDF files with a text layer. The layout of a PDF (columns, positions, fonts) cannot be rebuilt
//! faithfully from translated text, so the result is a Word document: the text of every page in
//! reading order, paragraphs re-formed from the lines, a page break between pages. A PDF without a
//! text layer (a scan) is refused with a clear message; reading those needs OCR, which comes later.

use std::io::Write;
use std::path::Path;

use lopdf::Document;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::layout::Header;
use crate::{SegmentInfo, DocError, Loaded, Writer};

struct PdfWriter {
    /// Per page: the number of paragraphs on it (a page break follows each page but the last).
    pages: Vec<usize>,
}

/// Fewer letters than this per page on average: treated as a scan (a text layer with page numbers only).
const MIN_LETTERS_PER_PAGE: usize = 5;

pub(crate) fn load(path: &Path) -> Result<Loaded, DocError> {
    let doc = Document::load(path).map_err(|e| {
        let why = e.to_string();
        if why.to_lowercase().contains("encrypt") || why.to_lowercase().contains("password") {
            DocError::Encrypted
        } else {
            DocError::Corrupt(why)
        }
    })?;
    if doc.is_encrypted() {
        return Err(DocError::Encrypted);
    }
    let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    if page_numbers.is_empty() {
        return Err(DocError::Corrupt("the PDF has no pages".into()));
    }

    let mut units = Vec::new();
    let mut pages = Vec::new();
    let mut skipped = Vec::new();
    let mut letters = 0usize;
    for number in &page_numbers {
        let raw = match doc.extract_text(&[*number]) {
            Ok(text) => text,
            Err(err) => {
                skipped.push(format!("page {number}: the text could not be read ({err})"));
                pages.push(0);
                continue;
            }
        };
        letters += raw.chars().filter(|c| c.is_alphabetic()).count();
        let paragraphs: Vec<String> = paragraphs_of(&raw).into_iter().filter(|p| crate::is_translatable(p)).collect();
        pages.push(paragraphs.len());
        units.extend(paragraphs);
    }
    if letters < MIN_LETTERS_PER_PAGE * page_numbers.len() || units.is_empty() {
        return Err(DocError::Scanned);
    }
    skipped.push("the layout of the PDF is not kept: the translation is a Word document with the text of each page".into());
    let mut segments = Vec::new();
    let mut unit = 0;
    for (page, count) in pages.iter().enumerate() {
        for _ in 0..*count {
            segments.push(SegmentInfo { header: Header { part: "page", number: Some(page + 1), ..Default::default() }, unit: Some(unit), source: units[unit].clone(), styles: Vec::new(), plain: true });
            unit += 1;
        }
    }
    Ok(Loaded { units, layout: Some(segments), restorers: Vec::new(), parts: Some((page_numbers.len(), "pages")), skipped, writer: Box::new(PdfWriter { pages }) })
}

/// Re-form paragraphs from the lines of a page: lines are joined with a space (a hyphen at the end
/// of a line joins the word), and a paragraph ends at a blank line or after a short line that ends a sentence.
pub(crate) fn paragraphs_of(raw: &str) -> Vec<String> {
    let lines: Vec<&str> = raw.lines().map(str::trim).collect();
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut previous_len = 0usize;
    for line in lines {
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            let ended_sentence = current.ends_with(['.', '!', '?', ':', '\u{3002}']);
            let short_line = previous_len * 4 < longest * 3;
            if ended_sentence && short_line {
                paragraphs.push(std::mem::take(&mut current));
            }
        }
        if current.is_empty() {
            current.push_str(line);
        } else if current.ends_with('-') && line.starts_with(char::is_lowercase) {
            current.pop();
            current.push_str(line);
        } else {
            current.push(' ');
            current.push_str(line);
        }
        previous_len = line.chars().count();
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A minimal, valid Word document with these paragraphs and page breaks.
pub(crate) fn docx_bytes(pages: &[Vec<&str>]) -> Result<Vec<u8>, DocError> {
    let mut body = String::new();
    for (i, page) in pages.iter().enumerate() {
        for paragraph in page {
            body.push_str(&format!("<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", escape(paragraph)));
        }
        if i + 1 < pages.len() {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>"
    );
    const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";
    const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";

    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in [("[Content_Types].xml", CONTENT_TYPES), ("_rels/.rels", RELS), ("word/document.xml", document.as_str())] {
        zip.start_file(name, options).map_err(|e| DocError::Corrupt(e.to_string()))?;
        zip.write_all(data.as_bytes())?;
    }
    Ok(zip.finish().map_err(|e| DocError::Corrupt(e.to_string()))?.into_inner())
}

impl Writer for PdfWriter {
    fn write(&self, translations: &[String], out: &Path) -> Result<Vec<String>, DocError> {
        let mut cursor = 0;
        let mut pages: Vec<Vec<&str>> = Vec::new();
        for &count in &self.pages {
            pages.push(translations[cursor..cursor + count].iter().map(String::as_str).collect());
            cursor += count;
        }
        std::fs::write(out, docx_bytes(&pages)?)?;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{translate_document, DocError};
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Object, Stream};

    /// A PDF whose pages hold the given lines of text (built with lopdf, no copyrighted file).
    fn make_pdf(path: &Path, pages: &[&[&str]]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let mut kids = Vec::new();
        for lines in pages {
            let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 12.into()]), Operation::new("TL", vec![14.into()]), Operation::new("Td", vec![72.into(), 720.into()])];
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    ops.push(Operation::new("T*", vec![]));
                }
                ops.push(Operation::new("Tj", vec![Object::string_literal(*line)]));
            }
            ops.push(Operation::new("ET", vec![]));
            let content = Content { operations: ops };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()] });
            kids.push(Object::Reference(page));
        }
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.save(path).unwrap();
    }

    #[test]
    fn lines_are_joined_into_paragraphs() {
        let raw = "This is the first line of a long paragraph that keeps\ngoing on the second line and ends here.\n\nShort heading.\nNext paragraph starts here and has hy-\nphenated words in it.\n";
        let paragraphs = paragraphs_of(raw);
        assert_eq!(paragraphs[0], "This is the first line of a long paragraph that keeps going on the second line and ends here.");
        assert_eq!(paragraphs[1], "Short heading.");
        assert_eq!(paragraphs[2], "Next paragraph starts here and has hyphenated words in it.");
    }

    #[test]
    fn a_text_pdf_becomes_a_valid_docx_with_page_breaks() {
        let dir = temp_dir("pdf");
        let input = dir.join("paper.pdf");
        make_pdf(&input, &[&["Introduction to the topic of owls and their habits."], &["The second page talks about food."]]);
        let output = dir.join("paper (DE).docx");
        let report = translate_document(&input, &output, &mut Tag("de"), &mut |_| {}).unwrap();
        assert_eq!(report.parts, Some((2, "pages")));
        assert!(report.skipped.iter().any(|s| s.contains("layout")));
        let document = String::from_utf8(read_part(&output, "word/document.xml")).unwrap();
        assert!(document.contains("[de]Introduction to the topic of owls and their habits."), "{document}");
        assert!(document.contains("[de]The second page talks about food."));
        assert_eq!(document.matches("w:type=\"page\"").count(), 1);
        assert!(crate::ooxml::is_well_formed(&document));
    }

    #[test]
    fn a_pdf_without_text_is_reported_as_scanned() {
        let dir = temp_dir("scanned");
        let input = dir.join("scan.pdf");
        make_pdf(&input, &[&[""], &[""]]);
        let err = translate_document(&input, &dir.join("o.docx"), &mut Tag("de"), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "scanned");
    }

    #[test]
    fn a_file_that_is_not_a_pdf_is_corrupt() {
        let dir = temp_dir("badpdf");
        let input = dir.join("bad.pdf");
        std::fs::write(&input, b"not a pdf").unwrap();
        let err = translate_document(&input, &dir.join("o.docx"), &mut Tag("de"), &mut |_| {}).unwrap_err();
        assert!(matches!(err, DocError::Corrupt(_)), "{err}");
    }

    #[test]
    fn the_written_docx_scans_back_to_the_same_paragraphs() {
        let bytes = docx_bytes(&[vec!["One & two", "Three <four>"], vec!["Five"]]).unwrap();
        let dir = temp_dir("docxback");
        let path = dir.join("a.docx");
        std::fs::write(&path, bytes).unwrap();
        let xml = String::from_utf8(read_part(&path, "word/document.xml")).unwrap();
        let scan = crate::ooxml::scan(&xml, &crate::ooxml::WORD).unwrap();
        let texts: Vec<&str> = scan.nodes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, ["One & two", "Three <four>", "Five"]);
    }
}
