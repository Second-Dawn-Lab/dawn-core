//! The steps of one look at the region, in one place, so the whole chain is tested (and measured) without a screen:
//! change detection, black region, reading only when something changed, waiting for stable text, finding the new lines.

use crate::change::{is_black, signature, BlackEvent, BlackWatch, ChangeDetector};
use crate::lines::{split_lines, Delta, LineTracker};
use crate::stable::{Stabilizer, Verdict};

/// What one look produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// The region became black for long enough, or stopped being black.
    pub black: Option<BlackEvent>,
    /// The frame differed from the last one and was read (the expensive part ran).
    pub read: bool,
    /// The captions went away.
    pub cleared: bool,
    /// New stable lines to translate, if any.
    pub delta: Delta,
    /// How long nothing changed in the region, for the update rate.
    pub idle_ms: u64,
}

#[derive(Debug, Default)]
pub struct Pipeline {
    change: ChangeDetector,
    stabilizer: Stabilizer,
    tracker: LineTracker,
    black: BlackWatch,
    text_seen: bool,
    last_text: String,
    last_change_ms: u64,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything read so far (the region or a language changed).
    pub fn reset(&mut self) {
        self.change.reset();
        self.stabilizer.reset();
        self.tracker.reset();
        self.last_text.clear();
    }

    /// Forget only the pixels (after a pause): the next frame is read even if it looks like the last one.
    pub fn forget_frame(&mut self) {
        self.change.reset();
    }

    /// One look. `read` recognizes the text of the frame; it is only called when the frame changed and is not black.
    /// `None` from it means the text could not be read (treated as no text).
    pub fn step(&mut self, now_ms: u64, rgba: &[u8], width: u32, height: u32, read: &mut dyn FnMut() -> Option<String>) -> Step {
        let dark = is_black(rgba, width, height);
        let black = self.black.update(now_ms, dark, self.text_seen);
        let changed = self.change.changed(signature(rgba, width, height));
        if changed {
            self.last_change_ms = now_ms;
        }
        let mut did_read = false;
        if dark {
            self.last_text.clear();
        } else if changed {
            did_read = true;
            self.last_text = read().unwrap_or_default();
            if !self.last_text.trim().is_empty() {
                self.text_seen = true;
            }
        }
        // A frame that did not change is the same text again: that is how a caption becomes stable without reading it twice.
        let mut step = Step { black, read: did_read, idle_ms: now_ms.saturating_sub(self.last_change_ms), ..Default::default() };
        match self.stabilizer.observe(now_ms, &self.last_text) {
            Verdict::Stable(text) => step.delta = self.tracker.push(&split_lines(&text)),
            Verdict::Cleared => {
                self.tracker.reset();
                step.cleared = true;
            }
            Verdict::Wait => {}
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 300;
    const H: u32 = 60;

    /// A frame whose picture identifies a caption: block k is bright for caption number k (0 = nothing).
    fn frame(caption: usize) -> Vec<u8> {
        let mut px = vec![0u8; (W * H * 4) as usize];
        for p in px.chunks_mut(4) {
            p.copy_from_slice(&[30, 30, 30, 255]);
        }
        if caption > 0 {
            // Each caption lights a different 40 px wide strip, so two captions always differ in several blocks.
            let x0 = (caption as u32 * 45) % (W - 45);
            for y in 20..40 {
                for x in x0..x0 + 40 {
                    let i = ((y * W + x) * 4) as usize;
                    px[i..i + 3].copy_from_slice(&[240, 240, 240]);
                }
            }
        }
        px
    }

    /// Plays a sequence of (caption picture, text the reader would return) and collects what the pipeline asks to translate.
    fn play(sequence: &[(usize, &str)], every_ms: u64) -> (Vec<String>, usize) {
        let mut pipeline = Pipeline::new();
        let (mut translated, mut reads) = (Vec::new(), 0);
        for (i, (picture, text)) in sequence.iter().enumerate() {
            let rgba = frame(*picture);
            let mut read = || {
                reads += 1;
                Some(text.to_string())
            };
            let step = pipeline.step(i as u64 * every_ms, &rgba, W, H, &mut read);
            for line in step.delta.lines {
                if step.delta.replace_last {
                    translated.pop();
                }
                translated.push(line);
            }
        }
        (translated, reads)
    }

    #[test]
    fn a_caption_that_builds_up_word_by_word_is_translated_once_when_it_is_complete() {
        let seq = [(1, "The"), (2, "The meeting"), (3, "The meeting starts"), (4, "The meeting starts at nine."), (4, ""), (4, ""), (4, "")];
        // The picture stays the same after the last word: only the changed frames are read.
        let (lines, reads) = play(&[seq[0], seq[1], seq[2], seq[3], (4, "x"), (4, "x"), (4, "x")], 400);
        assert_eq!(lines, ["The meeting starts at nine."]);
        assert_eq!(reads, 4, "unchanged frames are not read");
    }

    #[test]
    fn scrolling_captions_give_each_new_line_once() {
        let seq = [
            (1, "We will begin with\nthe quarterly report."),
            (1, "x"),
            (1, "x"),
            (2, "the quarterly report.\nPlease open page four."),
            (2, "x"),
            (2, "x"),
            (3, "Please open page four.\nQuestions are welcome."),
            (3, "x"),
            (3, "x"),
        ];
        let (lines, reads) = play(&seq, 400);
        assert_eq!(lines, ["We will begin with", "the quarterly report.", "Please open page four.", "Questions are welcome."]);
        assert_eq!(reads, 3);
    }

    #[test]
    fn the_same_words_again_after_the_captions_cleared_are_new_but_a_static_caption_is_not_repeated() {
        let seq = [(1, "Yes."), (1, "x"), (1, "x"), (1, "x"), (0, ""), (0, ""), (0, ""), (1, "Yes."), (1, "x"), (1, "x")];
        let (lines, _) = play(&seq, 400);
        assert_eq!(lines, ["Yes.", "Yes."]);
    }

    #[test]
    fn a_black_region_is_never_read_and_is_reported_after_a_few_seconds() {
        let mut pipeline = Pipeline::new();
        let black = vec![0u8; (W * H * 4) as usize];
        let mut reads = 0;
        let mut event = None;
        for i in 0..14 {
            let step = pipeline.step(i * 500, &black, W, H, &mut || {
                reads += 1;
                Some("never".into())
            });
            event = event.or(step.black);
        }
        assert_eq!((reads, event), (0, Some(BlackEvent::Started)));
    }

    #[test]
    fn the_idle_time_counts_from_the_last_change() {
        let mut pipeline = Pipeline::new();
        let rgba = frame(1);
        pipeline.step(0, &rgba, W, H, &mut || Some("A".into()));
        let step = pipeline.step(7000, &rgba, W, H, &mut || Some("A".into()));
        assert_eq!(step.idle_ms, 7000);
        assert!(!step.read);
    }

    #[test]
    fn a_failed_read_is_no_text_and_a_reset_reads_the_same_frame_again() {
        let mut pipeline = Pipeline::new();
        let rgba = frame(1);
        let step = pipeline.step(0, &rgba, W, H, &mut || None);
        assert!(step.read && step.delta.is_empty());
        pipeline.reset();
        let step = pipeline.step(500, &rgba, W, H, &mut || Some("Hello there".into()));
        assert!(step.read);
    }
}
