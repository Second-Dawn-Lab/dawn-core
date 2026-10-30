//! Upscaling small images before recognition. Both Windows OCR and PaddleOCR read text that is
//! about 30 pixels tall much better than the 12 to 16 pixels of normal screen text.

use crate::OcrImage;

/// Images up to this size are enlarged; bigger ones already have room.
const SMALL_WIDTH: u32 = 1600;
const SMALL_HEIGHT: u32 = 1000;
/// Windows OCR refuses images larger than this on either side.
pub const MAX_SIDE: u32 = 10_000;

/// By how much to enlarge an image (1 = leave as is).
pub fn upscale_factor(width: u32, height: u32) -> u32 {
    let factor = if width <= SMALL_WIDTH && height <= SMALL_HEIGHT { if height < 300 { 3 } else { 2 } } else { 1 };
    // Never produce an image the engine cannot take.
    let mut f = factor;
    while f > 1 && (width * f > MAX_SIDE || height * f > MAX_SIDE) {
        f -= 1;
    }
    f
}

/// Shrink an image whose sides exceed what the engines accept.
pub fn fit_max_side(image: OcrImage) -> OcrImage {
    let longest = image.width.max(image.height);
    if longest <= MAX_SIDE {
        return image;
    }
    let ratio = MAX_SIDE as f32 / longest as f32;
    resize(&image, ((image.width as f32 * ratio) as u32).max(1), ((image.height as f32 * ratio) as u32).max(1))
}

/// Enlarge by `factor` (bilinear).
pub fn upscale(image: &OcrImage, factor: u32) -> OcrImage {
    if factor <= 1 {
        return image.clone();
    }
    resize(image, image.width * factor, image.height * factor)
}

/// Bilinear resize of an RGBA image.
pub fn resize(src: &OcrImage, width: u32, height: u32) -> OcrImage {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let x_ratio = sw as f32 / w as f32;
    let y_ratio = sh as f32 / h as f32;
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * y_ratio - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, wy) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * x_ratio - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, wx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..4 {
                let p = |xx: usize, yy: usize| src.rgba[(yy * sw + xx) * 4 + c] as f32;
                let top = p(x0, y0) * (1.0 - wx) + p(x1, y0) * wx;
                let bottom = p(x0, y1) * (1.0 - wx) + p(x1, y1) * wx;
                rgba[(y * w + x) * 4 + c] = (top * (1.0 - wy) + bottom * wy).round() as u8;
            }
        }
    }
    OcrImage { width, height, rgba }
}

/// Selections this short hold small text (a line or two of 9 to 12 pixel letters). Measured on generated
/// text images: Windows OCR reads nothing at 1x, and 2x reads 9 and 10 pixel text with about 40 % fewer
/// errors than 3x, which is too much for the thin strokes of tiny text; larger factors (4x to 6x) and light
/// or strong sharpening were not better. So small text is read at 2x and 3x and the cleaner reading wins.
const SMALL_TEXT_HEIGHT: u32 = 120;

/// The enlargement factors to try, first the usual one.
pub fn scales(width: u32, height: u32) -> Vec<u32> {
    if height >= SMALL_TEXT_HEIGHT || width > SMALL_WIDTH {
        return vec![upscale_factor(width, height)];
    }
    [2u32, 3].into_iter().filter(|f| width * f <= MAX_SIDE && height * f <= MAX_SIDE).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_text_is_read_at_two_scales_and_big_images_at_one() {
        assert_eq!(scales(400, 60), vec![2, 3]);
        assert_eq!(scales(800, 119), vec![2, 3]);
        assert_eq!(scales(800, 120), vec![3]);
        assert_eq!(scales(800, 300), vec![2]);
        assert_eq!(scales(1920, 80), vec![1]);
        assert_eq!(scales(3300, 100), vec![1]);
        assert_eq!(scales(1500, 90), vec![2, 3]);
    }

    #[test]
    fn small_selections_are_enlarged_more() {
        assert_eq!(upscale_factor(400, 80), 3);
        assert_eq!(upscale_factor(800, 600), 2);
        assert_eq!(upscale_factor(1600, 1000), 2);
        assert_eq!(upscale_factor(1920, 1080), 1);
        assert_eq!(upscale_factor(3000, 200), 1);
    }

    #[test]
    fn the_enlarged_image_stays_within_the_engine_limit() {
        assert!(upscale_factor(1600, 100) * 1600 <= MAX_SIDE);
        assert_eq!(upscale_factor(5000, 100), 1);
    }

    #[test]
    fn resizing_keeps_colors_and_size() {
        let img = OcrImage { width: 2, height: 2, rgba: vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255] };
        let big = upscale(&img, 3);
        assert_eq!((big.width, big.height, big.rgba.len()), (6, 6, 6 * 6 * 4));
        assert!(big.rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));
        assert_eq!(upscale(&img, 1).width, 2);
    }

    #[test]
    fn oversized_images_are_shrunk() {
        let img = OcrImage { width: 20_000, height: 10, rgba: vec![0; 20_000 * 10 * 4] };
        let fit = fit_max_side(img);
        assert_eq!((fit.width, fit.height), (10_000, 5));
    }
}
