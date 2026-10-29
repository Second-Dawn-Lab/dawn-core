//! Windows' built-in OCR (`Windows.Media.Ocr`).
//!
//! No download: the languages come from the OS (Settings, Time & language, Language & region,
//! "Optical character recognition" capability of each language). Calls are blocking and must not
//! run on a thread that already owns a single-threaded COM apartment, so they get their own
//! multithreaded one.

use anyhow::{anyhow, Result};
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::{OcrEngine, OcrImage, OcrLine};

/// The Windows OCR engine.
#[derive(Default)]
pub struct WindowsOcr;

impl WindowsOcr {
    pub fn new() -> Self {
        Self
    }
}

fn init_com() {
    thread_local! {
        static INIT: () = {
            // Already initialized by someone else is fine (S_FALSE / RPC_E_CHANGED_MODE).
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        };
    }
    INIT.with(|_| {});
}

impl OcrEngine for WindowsOcr {
    fn id(&self) -> &'static str {
        "windows"
    }

    fn available_languages(&self) -> Vec<String> {
        init_com();
        let Ok(list) = WinEngine::AvailableRecognizerLanguages() else { return Vec::new() };
        list.into_iter().filter_map(|l| l.LanguageTag().ok()).map(|t| t.to_string()).collect()
    }

    fn recognize(&self, image: &OcrImage, language_tag: &str) -> Result<Vec<OcrLine>> {
        init_com();
        let max = WinEngine::MaxImageDimension().unwrap_or(10_000);
        if image.width > max || image.height > max {
            return Err(anyhow!("image too large for Windows OCR ({}x{}, limit {max})", image.width, image.height));
        }
        // Windows wants BGRA.
        let mut bgra = image.rgba.clone();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra)?;
        let buffer = writer.DetachBuffer()?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, image.width as i32, image.height as i32)?;

        let language = Language::CreateLanguage(&HSTRING::from(language_tag))?;
        let engine = WinEngine::TryCreateFromLanguage(&language).map_err(|_| anyhow!("Windows OCR has no {language_tag}"))?;
        let result = engine.RecognizeAsync(&bitmap)?.get()?;

        let mut lines = Vec::new();
        for line in result.Lines()? {
            let text = line.Text()?.to_string();
            let (mut top, mut bottom) = (f32::MAX, 0f32);
            for word in line.Words()? {
                let r = word.BoundingRect()?;
                top = top.min(r.Y);
                bottom = bottom.max(r.Y + r.Height);
            }
            if top == f32::MAX {
                continue;
            }
            lines.push(OcrLine { text, top, height: bottom - top });
        }
        Ok(lines)
    }
}
