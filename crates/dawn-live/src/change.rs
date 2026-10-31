//! Frame change detection and black-region detection.

/// A coarse picture of a frame: the average brightness of a grid of blocks. Comparing two of them is far cheaper than
/// comparing the frames, and a word appearing in a caption line changes a few blocks a lot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    cells: Vec<u8>,
}

const GRID_W: usize = 48;
const GRID_H: usize = 12;
/// At most this many pixels per row and column are looked at in a block: a block of 20x20 pixels needs 25 reads, not 400.
const SAMPLES: usize = 5;

fn luma(px: &[u8]) -> u32 {
    (px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8
}

/// The signature of an RGBA frame.
pub fn signature(rgba: &[u8], width: u32, height: u32) -> Signature {
    let (w, h) = (width as usize, height as usize);
    let mut cells = vec![0u8; GRID_W * GRID_H];
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Signature { cells };
    }
    for gy in 0..GRID_H {
        let (y0, y1) = (gy * h / GRID_H, ((gy + 1) * h / GRID_H).max(gy * h / GRID_H + 1).min(h));
        for gx in 0..GRID_W {
            let (x0, x1) = (gx * w / GRID_W, ((gx + 1) * w / GRID_W).max(gx * w / GRID_W + 1).min(w));
            let (mut sum, mut n) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                let y = y0 + sy * (y1 - y0) / SAMPLES;
                for sx in 0..SAMPLES {
                    let x = x0 + sx * (x1 - x0) / SAMPLES;
                    let i = (y.min(h - 1) * w + x.min(w - 1)) * 4;
                    sum += luma(&rgba[i..i + 3]);
                    n += 1;
                }
            }
            cells[gy * GRID_W + gx] = (sum / n.max(1)) as u8;
        }
    }
    Signature { cells }
}

impl Signature {
    /// How many blocks differ by at least `delta` brightness levels.
    pub fn differing_cells(&self, other: &Signature, delta: u8) -> usize {
        self.cells.iter().zip(&other.cells).filter(|(a, b)| a.abs_diff(**b) >= delta).count()
    }
}

/// Decides whether a frame is different enough from the last one that mattered to be worth reading.
#[derive(Debug, Clone)]
pub struct ChangeDetector {
    last: Option<Signature>,
    /// Brightness difference of a block that counts.
    pub cell_delta: u8,
    /// How many blocks must differ.
    pub min_cells: usize,
}

impl Default for ChangeDetector {
    fn default() -> Self {
        // Video compression noise moves single blocks by a few levels; a new word moves several blocks by tens.
        Self { last: None, cell_delta: 24, min_cells: 2 }
    }
}

impl ChangeDetector {
    /// True when this frame differs from the last frame that counted as a change (the first frame always does).
    /// The reference only moves on a change, so a slow fade is noticed once it adds up.
    pub fn changed(&mut self, sig: Signature) -> bool {
        let changed = match &self.last {
            None => true,
            Some(last) => last.differing_cells(&sig, self.cell_delta) >= self.min_cells,
        };
        if changed {
            self.last = Some(sig);
        }
        changed
    }

    /// Forget the reference: the next frame is read (after a pause, or when the region was moved).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Is every sampled pixel (nearly) black? Protected video comes out of screen capture as pure black.
pub fn is_black(rgba: &[u8], width: u32, height: u32) -> bool {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return false;
    }
    let (step_x, step_y) = ((w / 40).max(1), (h / 20).max(1));
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let i = (y * w + x) * 4;
            if rgba[i].max(rgba[i + 1]).max(rgba[i + 2]) > 3 {
                return false;
            }
            x += step_x;
        }
        y += step_y;
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlackEvent {
    /// The region has been black long enough to say so.
    Started,
    /// It shows something again.
    Ended,
}

/// Reports a region that stays black. Only said when no text was ever read in it: black between two subtitles (a letterbox
/// bar) is normal, a region that was never anything else is probably protected content.
#[derive(Debug, Clone, Default)]
pub struct BlackWatch {
    since: Option<u64>,
    announced: bool,
}

/// How long the region must stay black before it is reported.
pub const BLACK_AFTER_MS: u64 = 4000;

impl BlackWatch {
    pub fn update(&mut self, now_ms: u64, black: bool, text_seen_before: bool) -> Option<BlackEvent> {
        if !black {
            self.since = None;
            return std::mem::take(&mut self.announced).then_some(BlackEvent::Ended);
        }
        let since = *self.since.get_or_insert(now_ms);
        if !self.announced && !text_seen_before && now_ms.saturating_sub(since) >= BLACK_AFTER_MS {
            self.announced = true;
            return Some(BlackEvent::Started);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey frame with an optional bright block (a word).
    fn frame(w: u32, h: u32, grey: u8, block: Option<(u32, u32, u32, u32)>) -> Vec<u8> {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let on = block.is_some_and(|(bx, by, bw, bh)| x >= bx && x < bx + bw && y >= by && y < by + bh);
                let v = if on { 255 } else { grey };
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        px
    }

    #[test]
    fn the_same_frame_is_not_a_change_and_the_first_one_always_is() {
        let f = frame(400, 80, 40, None);
        let mut d = ChangeDetector::default();
        assert!(d.changed(signature(&f, 400, 80)));
        assert!(!d.changed(signature(&f, 400, 80)));
    }

    #[test]
    fn a_new_word_is_a_change_and_small_noise_is_not() {
        let mut d = ChangeDetector::default();
        let base = frame(400, 80, 40, None);
        d.changed(signature(&base, 400, 80));
        // Compression noise: the whole frame a few levels brighter.
        assert!(!d.changed(signature(&frame(400, 80, 46, None), 400, 80)));
        // A word: a bright block of 60x20 pixels.
        assert!(d.changed(signature(&frame(400, 80, 40, Some((100, 30, 60, 20))), 400, 80)));
    }

    #[test]
    fn a_slow_fade_adds_up_because_the_reference_only_moves_on_a_change() {
        let mut d = ChangeDetector::default();
        d.changed(signature(&frame(200, 40, 10, None), 200, 40));
        let mut changed_at = None;
        for step in 1..=12u8 {
            if d.changed(signature(&frame(200, 40, 10 + step * 4, None), 200, 40)) {
                changed_at = Some(step);
                break;
            }
        }
        assert!(changed_at.is_some_and(|s| s <= 7), "{changed_at:?}");
    }

    #[test]
    fn signatures_cope_with_tiny_and_empty_frames() {
        let _ = signature(&[], 0, 0);
        let tiny = frame(3, 2, 100, None);
        let _ = signature(&tiny, 3, 2);
        assert!(!is_black(&[], 0, 0));
    }

    #[test]
    fn a_black_frame_is_black_and_one_with_text_is_not() {
        assert!(is_black(&frame(300, 60, 0, None), 300, 60));
        assert!(!is_black(&frame(300, 60, 0, Some((0, 0, 300, 60))), 300, 60));
        assert!(!is_black(&frame(300, 60, 12, None), 300, 60), "dark grey is not protected video");
    }

    #[test]
    fn a_region_that_stays_black_is_reported_once_after_a_few_seconds_and_only_if_no_text_was_seen() {
        let mut w = BlackWatch::default();
        assert_eq!(w.update(0, true, false), None);
        assert_eq!(w.update(3000, true, false), None);
        assert_eq!(w.update(4100, true, false), Some(BlackEvent::Started));
        assert_eq!(w.update(5000, true, false), None, "said once");
        assert_eq!(w.update(5500, false, false), Some(BlackEvent::Ended));
        // A letterbox bar between subtitles of a video that did have text is silent.
        let mut quiet = BlackWatch::default();
        for t in (0..10).map(|i| i * 1000) {
            assert_eq!(quiet.update(t, true, true), None);
        }
        // A flicker of non-black resets the timer.
        let mut flicker = BlackWatch::default();
        flicker.update(0, true, false);
        flicker.update(3000, false, false);
        assert_eq!(flicker.update(3500, true, false), None);
        assert_eq!(flicker.update(7000, true, false), None);
        assert_eq!(flicker.update(7600, true, false), Some(BlackEvent::Started));
    }
}
