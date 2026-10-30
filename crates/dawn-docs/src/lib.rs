//! Document translation: Word, PowerPoint, Excel, text, Markdown and PDF files.
//!
//! The crate knows nothing about translation engines. Every format is read into a list of text
//! *units* (a paragraph, a table cell, a slide title), a [`Translator`] turns the units into their
//! translations, and the format writes them back. Anything that is not text (styles, images,
//! formulas, numbers) is copied from the original file untouched.
//!
//! Safety rules, in short:
//!
//! * the input file is never modified; the output is written to a temporary file and moved into
//!   place only when it is complete;
//! * every XML part is checked to be well formed before it is written, otherwise the part is kept as
//!   it was and listed in [`Report::skipped`];
//! * parts that cannot be handled (charts, SmartArt, pivot tables) stay in the file untranslated and
//!   are listed too.

mod layout;
mod markdown;
mod office;
mod ooxml;
mod pdf;
mod protect;
mod session;
mod text;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub use layout::{style_from_key, Header, SpanStyle};
pub use session::{Fingerprint, FingerprintStatus, SegmentInfo, Session};
pub use ooxml::{plan, plan_all, rewrite, scan, split_markers, split_proportionally, Planned, Unit};
pub use protect::is_translatable;

/// Larger files are refused: they are read into memory.
pub const MAX_FILE_BYTES: u64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Docx,
    Pptx,
    Xlsx,
    Txt,
    Markdown,
    Pdf,
}

impl Kind {
    pub fn from_path(path: &Path) -> Option<Kind> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "docx" => Some(Kind::Docx),
            "pptx" => Some(Kind::Pptx),
            "xlsx" => Some(Kind::Xlsx),
            "txt" => Some(Kind::Txt),
            "md" | "markdown" => Some(Kind::Markdown),
            "pdf" => Some(Kind::Pdf),
            _ => None,
        }
    }

    /// The extension of the translated file. A PDF becomes a Word document.
    pub fn output_extension(self) -> &'static str {
        match self {
            Kind::Docx | Kind::Pdf => "docx",
            Kind::Pptx => "pptx",
            Kind::Xlsx => "xlsx",
            Kind::Txt => "txt",
            Kind::Markdown => "md",
        }
    }
}

/// Why a document could not be translated. `code()` is stable for the UI.
#[derive(Debug)]
pub enum DocError {
    Unsupported(String),
    /// A password-protected file.
    Encrypted,
    Corrupt(String),
    /// A PDF without a text layer (a scan).
    Scanned,
    TooLarge(u64),
    Cancelled,
    Io(std::io::Error),
    /// The translator failed; the message comes from it.
    Translate(String),
}

impl DocError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported",
            Self::Encrypted => "encrypted",
            Self::Corrupt(_) => "corrupt",
            Self::Scanned => "scanned",
            Self::TooLarge(_) => "too_large",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io",
            Self::Translate(_) => "translate",
        }
    }
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "this file type is not supported: {what}"),
            Self::Encrypted => write!(f, "the file is password protected"),
            Self::Corrupt(why) => write!(f, "the file is damaged or not a valid document: {why}"),
            Self::Scanned => write!(f, "the PDF has no text layer (it looks like a scan)"),
            Self::TooLarge(size) => write!(f, "the file is too large ({} MB; the limit is {} MB)", size / 1_000_000, MAX_FILE_BYTES / 1_000_000),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Io(err) => write!(f, "file error: {err}"),
            Self::Translate(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for DocError {}

impl From<std::io::Error> for DocError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Turns texts into their translations. `texts` is a batch; the result has one entry per text.
pub trait Translator {
    fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, DocError>;

    /// Checked between batches; return `true` to stop the job with [`DocError::Cancelled`].
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// What a finished job did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub kind: Kind,
    /// Text units found (paragraphs, cells, ...).
    pub units: usize,
    /// Distinct texts sent to the translator (repeated texts are translated once).
    pub translated: usize,
    /// "slides", "sheets" or "pages", with how many.
    pub parts: Option<(usize, &'static str)>,
    /// Things that were left as they were, with the reason, for the summary.
    pub skipped: Vec<String>,
}

/// A file read into units, ready to be written back with translations.
pub(crate) struct Loaded {
    /// The text of every unit, in document order.
    pub units: Vec<String>,
    /// Every text segment with its place, for an editor (`None`: one unit per paragraph, no places known).
    pub layout: Option<Vec<SegmentInfo>>,
    /// Per unit: how to show its translation (address tokens written out). Empty when units carry no tokens.
    pub restorers: Vec<Option<Unit>>,
    pub parts: Option<(usize, &'static str)>,
    pub skipped: Vec<String>,
    pub writer: Box<dyn Writer>,
}

pub(crate) trait Writer: Send + Sync {
    /// Write the output file; `translations[i]` is the translation of unit `i`. Returns more notes.
    fn write(&self, translations: &[String], out: &Path) -> Result<Vec<String>, DocError>;
}

fn check_input(path: &Path) -> Result<Kind, DocError> {
    let kind = Kind::from_path(path).ok_or_else(|| DocError::Unsupported(path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()))?;
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Err(DocError::TooLarge(size));
    }
    Ok(kind)
}

fn load(path: &Path, kind: Kind) -> Result<Loaded, DocError> {
    match kind {
        Kind::Docx | Kind::Pptx | Kind::Xlsx => office::load(path, kind),
        Kind::Txt => text::load_text(path),
        Kind::Markdown => markdown::load(path),
        Kind::Pdf => pdf::load(path),
    }
}

/// The first `max_chars` characters of the document's text, to detect its language.
pub fn sample_text(path: &Path, max_chars: usize) -> Result<String, DocError> {
    let kind = check_input(path)?;
    let loaded = load(path, kind)?;
    let mut sample = String::new();
    for unit in loaded.units {
        if sample.chars().count() >= max_chars {
            break;
        }
        sample.push_str(&unit);
        sample.push('\n');
    }
    Ok(sample.chars().take(max_chars).collect())
}

const BATCH_UNITS: usize = 16;
const BATCH_CHARS: usize = 4000;

/// Translate `units` (repeated texts once, in batches), returning one translation per unit and the number of
/// distinct texts that went to the translator.
pub(crate) fn translate_texts(units: &[String], translator: &mut dyn Translator, progress: &mut dyn FnMut(Progress)) -> Result<(Vec<String>, usize), DocError> {
    // Distinct texts, in order of first appearance.
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    let mut distinct: Vec<&str> = Vec::new();
    let unit_to_distinct: Vec<usize> = units
        .iter()
        .map(|u| {
            *index_of.entry(u.as_str()).or_insert_with(|| {
                distinct.push(u.as_str());
                distinct.len() - 1
            })
        })
        .collect();

    let total = distinct.len();
    progress(Progress { done: 0, total });
    let mut translated: Vec<String> = Vec::with_capacity(total);
    let mut start = 0;
    while start < total {
        if translator.is_cancelled() {
            return Err(DocError::Cancelled);
        }
        let mut end = start;
        let mut chars = 0;
        while end < total && end - start < BATCH_UNITS && (end == start || chars + distinct[end].len() <= BATCH_CHARS) {
            chars += distinct[end].len();
            end += 1;
        }
        let batch: Vec<String> = distinct[start..end].iter().map(|s| s.to_string()).collect();
        let out = translator.translate(&batch)?;
        if out.len() != batch.len() {
            return Err(DocError::Translate("the translator returned a different number of texts".into()));
        }
        translated.extend(out);
        start = end;
        progress(Progress { done: start, total });
    }
    if translator.is_cancelled() {
        return Err(DocError::Cancelled);
    }

    let per_unit: Vec<String> = unit_to_distinct.iter().map(|&i| translated[i].clone()).collect();
    Ok((per_unit, total))
}

/// Translate `input` into `output`.
///
/// Repeated texts are translated once. `progress` is called after every batch. The output is
/// written next to its final name and renamed when complete, so a cancelled or failed job leaves no
/// partial file.
pub fn translate_document(input: &Path, output: &Path, translator: &mut dyn Translator, progress: &mut dyn FnMut(Progress)) -> Result<Report, DocError> {
    let kind = check_input(input)?;
    let loaded = load(input, kind)?;

    let (per_unit, total) = translate_texts(&loaded.units, translator, progress)?;
    let temp = temp_path(output);
    let result = loaded.writer.write(&per_unit, &temp);
    let notes = match result {
        Ok(notes) => notes,
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
    };
    if output.exists() {
        std::fs::remove_file(output)?;
    }
    std::fs::rename(&temp, output)?;

    let mut skipped = loaded.skipped;
    skipped.extend(notes);
    Ok(Report { kind, units: loaded.units.len(), translated: total, parts: loaded.parts, skipped })
}

fn temp_path(output: &Path) -> PathBuf {
    let mut name = output.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    output.with_file_name(name)
}

/// `report (DE).docx` next to the original, numbered if it exists: the original is never overwritten.
pub fn default_output_path(input: &Path, target_label: &str, dir: Option<&Path>) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let ext = Kind::from_path(input).map_or("txt", Kind::output_extension);
    let dir = dir.map(Path::to_path_buf).or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
    let label = target_label.to_uppercase();
    let first = dir.join(format!("{stem} ({label}).{ext}"));
    if !first.exists() && first != input {
        return first;
    }
    (2..1000)
        .map(|n| dir.join(format!("{stem} ({label}) {n}.{ext}")))
        .find(|p| !p.exists() && p != input)
        .unwrap_or(first)
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use zip::write::SimpleFileOptions;

    pub fn temp_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("dawn-docs-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip file with the given parts (name, content).
    pub fn make_zip(path: &Path, parts: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in parts {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    pub fn read_part(path: &Path, name: &str) -> Vec<u8> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut data = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
        data
    }

    /// Translates by tagging: `[de]text`, and records what it was asked.
    pub struct Tag(pub &'static str);

    impl super::Translator for Tag {
        fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, super::DocError> {
            Ok(texts.iter().map(|t| format!("[{}]{t}", self.0)).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn file_types_map_to_kinds_and_pdf_becomes_docx() {
        assert_eq!(Kind::from_path(Path::new("a.DOCX")), Some(Kind::Docx));
        assert_eq!(Kind::from_path(Path::new("a.markdown")), Some(Kind::Markdown));
        assert_eq!(Kind::from_path(Path::new("a.exe")), None);
        assert_eq!(Kind::Pdf.output_extension(), "docx");
    }

    #[test]
    fn unsupported_and_missing_files_are_refused() {
        let dir = temp_dir("refuse");
        let exe = dir.join("a.exe");
        std::fs::write(&exe, b"x").unwrap();
        let err = translate_document(&exe, &dir.join("out.exe"), &mut Tag("de"), &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "unsupported");
    }

    #[test]
    fn the_output_name_never_overwrites_the_original_or_another_file() {
        let dir = temp_dir("names");
        let input = dir.join("report.docx");
        std::fs::write(&input, b"x").unwrap();
        let first = default_output_path(&input, "de", None);
        assert_eq!(first.file_name().unwrap(), "report (DE).docx");
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(default_output_path(&input, "de", None).file_name().unwrap(), "report (DE) 2.docx");
        let pdf = dir.join("paper.pdf");
        assert_eq!(default_output_path(&pdf, "fr", Some(&dir)).file_name().unwrap(), "paper (FR).docx");
    }

    struct Counting {
        calls: Vec<Vec<String>>,
        cancel_after: Option<usize>,
    }
    impl Translator for Counting {
        fn translate(&mut self, texts: &[String]) -> Result<Vec<String>, DocError> {
            self.calls.push(texts.to_vec());
            Ok(texts.iter().map(|t| t.to_uppercase()).collect())
        }
        fn is_cancelled(&self) -> bool {
            self.cancel_after.is_some_and(|n| self.calls.len() >= n)
        }
    }

    #[test]
    fn repeated_texts_are_translated_once_and_progress_is_reported() {
        let dir = temp_dir("dedupe");
        let input = dir.join("a.txt");
        std::fs::write(&input, "Hello\n\nHello\n\nWorld\n").unwrap();
        let mut tr = Counting { calls: Vec::new(), cancel_after: None };
        let mut seen = Vec::new();
        let report = translate_document(&input, &dir.join("b.txt"), &mut tr, &mut |p| seen.push(p)).unwrap();
        assert_eq!(report.units, 3);
        assert_eq!(report.translated, 2);
        assert_eq!(tr.calls.concat(), vec!["Hello", "World"]);
        assert_eq!(seen.first(), Some(&Progress { done: 0, total: 2 }));
        assert_eq!(seen.last(), Some(&Progress { done: 2, total: 2 }));
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "HELLO\n\nHELLO\n\nWORLD\n");
    }

    #[test]
    fn a_cancelled_job_leaves_no_output_and_no_partial_file() {
        let dir = temp_dir("cancel");
        let input = dir.join("a.txt");
        let text: String = (0..60).map(|i| format!("Paragraph number {i} has some words.\n\n")).collect();
        std::fs::write(&input, text).unwrap();
        let mut tr = Counting { calls: Vec::new(), cancel_after: Some(1) };
        let err = translate_document(&input, &dir.join("b.txt"), &mut tr, &mut |_| {}).unwrap_err();
        assert_eq!(err.code(), "cancelled");
        assert!(!dir.join("b.txt").exists());
        assert!(!dir.join("b.txt.part").exists());
    }

    #[test]
    fn oversized_files_are_refused_with_the_limit() {
        let err = DocError::TooLarge(150_000_000);
        assert!(err.to_string().contains("100 MB"));
        assert_eq!(err.code(), "too_large");
    }

    #[test]
    fn make_zip_helper_round_trips() {
        let dir = temp_dir("zip");
        make_zip(&dir.join("a.zip"), &[("x.txt", b"hello")]);
        assert_eq!(read_part(&dir.join("a.zip"), "x.txt"), b"hello");
    }
}
