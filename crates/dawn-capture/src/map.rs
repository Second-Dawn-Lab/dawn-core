//! Coordinate mapping between what the user drags on an overlay and the stored screenshot.
//!
//! An overlay covers one monitor exactly. The page inside it measures in CSS pixels (physical
//! pixels divided by the monitor's DPI scale); the screenshot is in physical pixels. Instead of
//! trusting a scale factor, the mapping uses the ratio of the screenshot size to the page's
//! viewport size, which stays right when the two disagree by a pixel through rounding.

/// A screenshot: 8-bit RGBA, rows top to bottom.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// A drag shorter than this (CSS pixels, either side) is an accident and cancels.
pub const MIN_DRAG: f64 = 8.0;

/// The drag is too small to be a selection.
pub fn is_tiny(w: f64, h: f64) -> bool {
    w.abs() < MIN_DRAG || h.abs() < MIN_DRAG
}

/// The rectangle between two points, whichever way the user dragged, kept inside the viewport.
pub fn drag_rect(start: (f64, f64), end: (f64, f64), viewport: (f64, f64)) -> (f64, f64, f64, f64) {
    let clamp = |v: f64, max: f64| v.clamp(0.0, max);
    let (x0, y0) = (clamp(start.0, viewport.0), clamp(start.1, viewport.1));
    let (x1, y1) = (clamp(end.0, viewport.0), clamp(end.1, viewport.1));
    (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
}

/// Map a selection in CSS pixels of a viewport to pixels of the screenshot. `None` when the
/// result would be empty.
pub fn to_image_rect(sel: (f64, f64, f64, f64), viewport: (f64, f64), image: (u32, u32)) -> Option<PixelRect> {
    if viewport.0 <= 0.0 || viewport.1 <= 0.0 {
        return None;
    }
    let (sx, sy) = (image.0 as f64 / viewport.0, image.1 as f64 / viewport.1);
    let x0 = (sel.0 * sx).round().clamp(0.0, image.0 as f64);
    let y0 = (sel.1 * sy).round().clamp(0.0, image.1 as f64);
    let x1 = ((sel.0 + sel.2) * sx).round().clamp(0.0, image.0 as f64);
    let y1 = ((sel.1 + sel.3) * sy).round().clamp(0.0, image.1 as f64);
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    (w > 0 && h > 0).then_some(PixelRect { x: x0 as i32, y: y0 as i32, w, h })
}

/// A rectangle of one monitor's screenshot in virtual-desktop pixels (where the popup goes).
pub fn to_desktop(monitor_origin: (i32, i32), rect: PixelRect) -> PixelRect {
    PixelRect { x: monitor_origin.0 + rect.x, y: monitor_origin.1 + rect.y, ..rect }
}

/// Copy a rectangle out of a screenshot. The rectangle is clipped to the image.
pub fn crop(frame: &Frame, rect: PixelRect) -> Option<Frame> {
    let x0 = rect.x.max(0) as u32;
    let y0 = rect.y.max(0) as u32;
    let x1 = (rect.x.max(0) as u32 + rect.w).min(frame.width);
    let y1 = (rect.y.max(0) as u32 + rect.h).min(frame.height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in y0..y1 {
        let start = (row as usize * frame.width as usize + x0 as usize) * 4;
        rgba.extend_from_slice(&frame.rgba[start..start + w * 4]);
    }
    Some(Frame { width: w as u32, height: h as u32, rgba })
}

/// The screenshot as an uncompressed 32-bit BMP (top-down), for the overlay to display.
/// No compression keeps this instant even for a 4K monitor.
pub fn to_bmp(frame: &Frame) -> Vec<u8> {
    let pixels = frame.width as usize * frame.height as usize * 4;
    let mut out = Vec::with_capacity(54 + pixels);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixels) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(frame.width as i32).to_le_bytes());
    out.extend_from_slice(&(-(frame.height as i32)).to_le_bytes()); // negative: rows top to bottom
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // BI_RGB
    out.extend_from_slice(&(pixels as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for px in frame.rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> Frame {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        Frame { width: w, height: h, rgba }
    }

    #[test]
    fn a_100_percent_monitor_maps_one_to_one() {
        let r = to_image_rect((100.0, 50.0, 200.0, 80.0), (1920.0, 1080.0), (1920, 1080)).unwrap();
        assert_eq!(r, PixelRect { x: 100, y: 50, w: 200, h: 80 });
    }

    #[test]
    fn a_150_percent_monitor_scales_css_pixels_up() {
        // 2880x1620 physical shows as 1920x1080 CSS pixels at 150 %.
        let r = to_image_rect((100.0, 50.0, 200.0, 80.0), (1920.0, 1080.0), (2880, 1620)).unwrap();
        assert_eq!(r, PixelRect { x: 150, y: 75, w: 300, h: 120 });
    }

    #[test]
    fn a_200_percent_monitor_next_to_a_100_percent_one() {
        // Each overlay maps with its own monitor, so mixed DPI needs no special case.
        let left = to_image_rect((10.0, 10.0, 100.0, 40.0), (1920.0, 1080.0), (1920, 1080)).unwrap();
        let right = to_image_rect((10.0, 10.0, 100.0, 40.0), (1920.0, 1080.0), (3840, 2160)).unwrap();
        assert_eq!((left.w, left.h), (100, 40));
        assert_eq!((right.x, right.y, right.w, right.h), (20, 20, 200, 80));
    }

    #[test]
    fn rounding_differences_between_viewport_and_image_are_absorbed() {
        // 125 % of 1366 css px is 1707.5: the image is 1708 wide, the viewport reports 1366.
        let r = to_image_rect((0.0, 0.0, 1366.0, 768.0), (1366.0, 768.0), (1708, 960)).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 1708, 960));
    }

    #[test]
    fn selections_outside_the_screen_are_clamped_and_empty_ones_rejected() {
        let r = to_image_rect((1900.0, 1060.0, 100.0, 100.0), (1920.0, 1080.0), (1920, 1080)).unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (1900, 1060, 20, 20));
        assert_eq!(to_image_rect((2000.0, 0.0, 10.0, 10.0), (1920.0, 1080.0), (1920, 1080)), None);
        assert_eq!(to_image_rect((0.0, 0.0, 0.0, 10.0), (1920.0, 1080.0), (1920, 1080)), None);
        assert_eq!(to_image_rect((0.0, 0.0, 10.0, 10.0), (0.0, 0.0), (1920, 1080)), None);
    }

    #[test]
    fn dragging_in_any_direction_gives_the_same_rectangle() {
        let v = (800.0, 600.0);
        let a = drag_rect((100.0, 100.0), (300.0, 250.0), v);
        assert_eq!(a, (100.0, 100.0, 200.0, 150.0));
        assert_eq!(drag_rect((300.0, 250.0), (100.0, 100.0), v), a);
        assert_eq!(drag_rect((300.0, 100.0), (100.0, 250.0), v), a);
        // Dragging past the edge stops at the edge.
        assert_eq!(drag_rect((700.0, 500.0), (900.0, 700.0), v), (700.0, 500.0, 100.0, 100.0));
        assert_eq!(drag_rect((100.0, 100.0), (-50.0, 20.0), v), (0.0, 20.0, 100.0, 80.0));
    }

    #[test]
    fn a_tiny_drag_cancels() {
        assert!(is_tiny(3.0, 200.0));
        assert!(is_tiny(200.0, 7.9));
        assert!(is_tiny(0.0, 0.0));
        assert!(!is_tiny(8.0, 8.0));
        assert!(!is_tiny(120.0, 40.0));
    }

    #[test]
    fn the_selection_is_placed_on_the_virtual_desktop() {
        // A monitor left of the primary one has a negative origin.
        let r = to_desktop((-1920, 100), PixelRect { x: 50, y: 20, w: 300, h: 100 });
        assert_eq!(r, PixelRect { x: -1870, y: 120, w: 300, h: 100 });
    }

    #[test]
    fn cropping_copies_exactly_the_rectangle() {
        let f = frame(10, 8);
        let c = crop(&f, PixelRect { x: 2, y: 3, w: 4, h: 2 }).unwrap();
        assert_eq!((c.width, c.height, c.rgba.len()), (4, 2, 4 * 2 * 4));
        assert_eq!(&c.rgba[0..2], &[2, 3], "top-left pixel is (2,3)");
        assert_eq!(&c.rgba[c.rgba.len() - 4..c.rgba.len() - 2], &[5, 4], "bottom-right pixel is (5,4)");
    }

    #[test]
    fn cropping_clips_to_the_image() {
        let f = frame(10, 8);
        let c = crop(&f, PixelRect { x: 8, y: 6, w: 10, h: 10 }).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert!(crop(&f, PixelRect { x: 10, y: 0, w: 5, h: 5 }).is_none());
        assert!(crop(&f, PixelRect { x: 0, y: 0, w: 0, h: 5 }).is_none());
    }

    #[test]
    fn the_bmp_has_a_valid_header_and_bgra_pixels() {
        let f = Frame { width: 2, height: 1, rgba: vec![10, 20, 30, 0, 40, 50, 60, 0] };
        let bmp = to_bmp(&f);
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize, bmp.len());
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -1, "top-down");
        assert_eq!(&bmp[54..62], &[30, 20, 10, 255, 60, 50, 40, 255]);
    }
}
