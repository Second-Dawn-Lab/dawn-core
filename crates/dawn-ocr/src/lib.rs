//! OCR engines behind a single trait, plus the pure text logic around them.
//!
//! * [`join`]: turn recognized lines into text with sentences that continue across lines joined.
//! * [`choose`]: pick the engine and OCR language for the user's capture languages.
//! * [`scale`]: upscale small images before recognition.
//! * `winocr` (Windows): the built-in Windows OCR engine.

use anyhow::Result;

pub mod choose;
pub mod fix;
pub mod join;
pub mod scale;
#[cfg(windows)]
mod winocr;

#[cfg(windows)]
pub use winocr::WindowsOcr;

/// An image in memory: 8-bit RGBA, rows top to bottom. Screenshots are never written to disk.
#[derive(Debug, Clone)]
pub struct OcrImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One recognized line with its place in the image (pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub top: f32,
    pub height: f32,
}

/// An OCR backend.
pub trait OcrEngine {
    /// Short name for messages ("windows").
    fn id(&self) -> &'static str;

    /// BCP-47 tags of the languages this engine can recognize right now ("de-DE").
    fn available_languages(&self) -> Vec<String>;

    /// Recognize text in an image in one language, lines in reading order.
    fn recognize(&self, image: &OcrImage, language_tag: &str) -> Result<Vec<OcrLine>>;
}
