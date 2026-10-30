//! Documents translated with the real engine and the German pack that is already installed:
//!
//! ```text
//! cargo test -p dawn-docs --test with_models -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dawn_docs::{translate_document, DocError, Translator};
use dawn_models::{Catalog, ModelStore};
use dawn_translate::{Ct2Engine, Ct2Options, Glossary, GlossaryEntry};

struct Engine(Ct2Engine);

impl Translator for Engine {
    fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, DocError> {
        texts.iter().map(|t| self.0.translate_detailed(t, "en", "de", &AtomicBool::new(false)).map(|r| r.text).map_err(|e| DocError::Translate(e.to_string()))).collect()
    }
}

fn engine() -> Engine {
    let store = ModelStore::open_default();
    let catalog = Arc::new(Catalog::bundled());
    assert!(store.is_language_installed(&catalog, "de"), "install German first");
    Engine(Ct2Engine::new(store, catalog, &Ct2Options::default()))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dawn-docs-it-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_zip(path: &Path, parts: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn part(path: &Path, name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut s = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn docx(path: &Path) {
    let paragraphs = [
        r#"<w:p><w:r><w:t xml:space="preserve">Please read the </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>user manual</w:t></w:r><w:r><w:t xml:space="preserve"> before you start the machine.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">The </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>quick</w:t></w:r><w:r><w:t xml:space="preserve"> brown fox jumps over the </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>lazy dog</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>We offer free shipping on all orders over 50 euros.</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Product</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Price</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>The owl sleeps.</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>12.50</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    ];
    let document = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{}</w:body></w:document>"#, paragraphs.concat());
    make_zip(path, &[("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#), ("word/document.xml", &document)]);
}

#[test]
#[ignore = "needs the German pack"]
fn docx_translates_and_keeps_bold_and_italic_runs() {
    let dir = temp("docx");
    let (input, output) = (dir.join("a.docx"), dir.join("a (DE).docx"));
    docx(&input);
    let report = translate_document(&input, &output, &mut engine(), &mut |_| {}).unwrap();
    let xml = part(&output, "word/document.xml");
    println!("{report:?}\n{xml}");
    // The result is a well-formed document that still has its bold and italic runs and no marker leaks.
    assert!(!xml.contains("{1}") && !xml.contains("{/"), "markers must not leak");
    assert_eq!(xml.matches("<w:b/>").count(), 2);
    assert_eq!(xml.matches("<w:i/>").count(), 1);
    assert!(xml.contains("Bedienungsanleitung") || xml.contains("Handbuch") || xml.contains("Benutzerhandbuch"), "{xml}");
    assert!(xml.contains("12.50"), "numbers are untouched");
}

#[test]
#[ignore = "needs the German pack"]
fn glossary_terms_appear_in_a_translated_docx() {
    let dir = temp("glossary");
    let (input, output) = (dir.join("a.docx"), dir.join("a (DE).docx"));
    docx(&input);
    let mut eng = engine();
    let mut g = Glossary::default();
    g.entries = vec![GlossaryEntry { id: 1, from: "en".into(), to: "de".into(), source: "owl".into(), target: "Eule".into(), match_case: false, whole_word: true, enabled: true }];
    eng.0.set_glossary(g);
    translate_document(&input, &output, &mut eng, &mut |_| {}).unwrap();
    let xml = part(&output, "word/document.xml");
    assert!(xml.contains("Eule"), "{xml}");
}

#[test]
#[ignore = "needs the German pack"]
fn pptx_and_xlsx_translate_and_stay_valid() {
    let dir = temp("office");
    let pptx = dir.join("deck.pptx");
    make_zip(
        &pptx,
        &[
            ("ppt/presentation.xml", "<p:presentation xmlns:p=\"p\"/>"),
            ("ppt/slides/slide1.xml", "<p:sld xmlns:p=\"p\" xmlns:a=\"a\"><a:p><a:r><a:t>Welcome to our quarterly meeting</a:t></a:r></a:p><a:p><a:r><a:t>Sales grew by 12 percent</a:t></a:r></a:p></p:sld>"),
        ],
    );
    let out = dir.join("deck (DE).pptx");
    translate_document(&pptx, &out, &mut engine(), &mut |_| {}).unwrap();
    let slide = part(&out, "ppt/slides/slide1.xml");
    println!("{slide}");
    assert!(slide.contains("Willkommen"), "{slide}");

    let xlsx = dir.join("book.xlsx");
    make_zip(
        &xlsx,
        &[
            ("xl/workbook.xml", "<workbook xmlns=\"x\"/>"),
            ("xl/sharedStrings.xml", "<sst xmlns=\"x\"><si><t>Total amount</t></si><si><t>Customer name</t></si></sst>"),
            ("xl/worksheets/sheet1.xml", "<worksheet xmlns=\"x\"><sheetData><row><c t=\"s\"><v>0</v></c><c><f>SUM(B2:B9)</f><v>10</v></c></row></sheetData></worksheet>"),
        ],
    );
    let out = dir.join("book (DE).xlsx");
    translate_document(&xlsx, &out, &mut engine(), &mut |_| {}).unwrap();
    let shared = part(&out, "xl/sharedStrings.xml");
    println!("{shared}");
    assert!(!shared.contains("Total amount"));
    assert!(part(&out, "xl/worksheets/sheet1.xml").contains("<f>SUM(B2:B9)</f><v>10</v>"));
}
