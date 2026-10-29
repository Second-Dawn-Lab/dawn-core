//! Needs Windows OCR with German and English installed: `cargo test -p dawn-ocr -- --ignored`.
//! The test image is drawn by PowerShell (System.Drawing) into the temp folder.

#![cfg(windows)]

use std::process::Command;

use dawn_ocr::{choose, join, scale, OcrEngine, OcrImage, WindowsOcr};

fn render(text: &str, name: &str) -> OcrImage {
    let path = std::env::temp_dir().join(format!("dawn-ocr-test-{name}.png"));
    let script = format!(
        "Add-Type -AssemblyName System.Drawing; \
         $bmp = New-Object System.Drawing.Bitmap 900, 140; \
         $g = [System.Drawing.Graphics]::FromImage($bmp); \
         $g.Clear([System.Drawing.Color]::White); \
         $g.TextRenderingHint = 'ClearTypeGridFit'; \
         $font = New-Object System.Drawing.Font 'Segoe UI', 15; \
         $g.DrawString('{}', $font, [System.Drawing.Brushes]::Black, 10, 10); \
         $bmp.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png)",
        text.replace('\'', "''"),
        path.display()
    );
    let status = Command::new("powershell").args(["-NoProfile", "-Command", &script]).status().expect("powershell");
    assert!(status.success());
    let img = image::open(&path).expect("test image").to_rgba8();
    let _ = std::fs::remove_file(&path);
    OcrImage { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

/// Share of the expected words that were recognized.
fn word_overlap(expected: &str, got: &str) -> f32 {
    let norm = |s: &str| s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let want = norm(expected);
    let have = norm(got);
    want.iter().filter(|w| have.contains(w)).count() as f32 / want.len() as f32
}

fn recognize(engine: &WindowsOcr, image: &OcrImage, code: &str) -> String {
    let tags = engine.available_languages();
    let tag = choose::tag_for(code, &tags).unwrap_or_else(|| panic!("Windows OCR language {code} is not installed (have {tags:?})"));
    let factor = scale::upscale_factor(image.width, image.height);
    let lines = engine.recognize(&scale::upscale(image, factor), &tag).expect("recognize");
    join::join_lines(&lines)
}

#[test]
#[ignore = "needs Windows OCR with German installed"]
fn recognizes_german_text() {
    let expected = "Die Sonne scheint heute, und wir gehen am Nachmittag spazieren.";
    let got = recognize(&WindowsOcr::new(), &render(expected, "de"), "de");
    assert!(word_overlap(expected, &got) >= 0.8, "recognized: {got:?}");
}

#[test]
#[ignore = "needs Windows OCR with English installed"]
fn recognizes_english_text() {
    let expected = "The weather is lovely today, so we are going for a walk.";
    let got = recognize(&WindowsOcr::new(), &render(expected, "en"), "en");
    assert!(word_overlap(expected, &got) >= 0.8, "recognized: {got:?}");
}

#[test]
#[ignore = "needs Windows OCR"]
fn an_image_without_text_gives_no_text() {
    let blank = OcrImage { width: 300, height: 100, rgba: vec![255; 300 * 100 * 4] };
    let engine = WindowsOcr::new();
    let tag = engine.available_languages().into_iter().next().expect("some OCR language");
    let lines = engine.recognize(&blank, &tag).expect("recognize");
    assert!(join::join_lines(&lines).trim().is_empty());
}

#[test]
#[ignore = "needs Windows OCR"]
fn lists_the_installed_languages() {
    let tags = WindowsOcr::new().available_languages();
    println!("Windows OCR languages here: {tags:?}");
    assert!(!tags.is_empty());
}
