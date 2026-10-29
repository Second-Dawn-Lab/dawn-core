//! Timing of the capture path on the real screen: `cargo test -p dawn-ocr --test screen -- --ignored --nocapture`.
//! Prints sizes and timings only, never the recognized text.

#![cfg(windows)]

use std::time::Instant;

use dawn_capture::map::{crop, PixelRect};
use dawn_ocr::{choose, join, scale, OcrEngine, OcrImage, WindowsOcr};

#[test]
#[ignore = "uses the real screen and Windows OCR"]
fn capture_and_recognize_timing() {
    let t = Instant::now();
    let screens = dawn_capture::capture_all().expect("capture");
    println!("capture_all: {} ms for {} monitor(s)", t.elapsed().as_millis(), screens.len());
    for s in &screens {
        println!("  monitor at ({}, {}) {}x{} scale {} primary {}", s.x, s.y, s.width, s.height, s.scale, s.frame.width == s.width && s.frame.height == s.height);
    }
    let screen = &screens[0];
    let t = Instant::now();
    let bmp = dawn_capture::map::to_bmp(&screen.frame);
    println!("to_bmp: {} ms ({} MB)", t.elapsed().as_millis(), bmp.len() / 1_000_000);

    let rect = PixelRect { x: 0, y: 0, w: screen.frame.width.min(900), h: screen.frame.height.min(300) };
    let frame = crop(&screen.frame, rect).expect("crop");
    let image = OcrImage { width: frame.width, height: frame.height, rgba: frame.rgba };
    let engine = WindowsOcr::new();
    let tags = engine.available_languages();
    let tag = choose::tag_for("en", &tags).or_else(|| tags.first().cloned()).expect("an OCR language");
    let t = Instant::now();
    let big = scale::upscale(&image, scale::upscale_factor(image.width, image.height));
    println!("upscale {}x{} -> {}x{}: {} ms", image.width, image.height, big.width, big.height, t.elapsed().as_millis());
    for round in 0..2 {
        let t = Instant::now();
        let lines = engine.recognize(&big, &tag).expect("recognize");
        let text = join::join_lines(&lines);
        println!("recognize #{round} ({tag}): {} ms, {} lines, {} characters", t.elapsed().as_millis(), lines.len(), text.chars().count());
    }
}
