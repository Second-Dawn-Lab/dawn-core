//! The steps of one look at the region, in one place, so the whole chain is tested (and measured) without a screen:
//! change detection, black region, reading only when something changed, waiting for stable text, finding the new lines.

use crate::change::{is_black, signature, BlackEvent, BlackWatch, ChangeDetector};
use crate::lines::{split_lines, Delta, LineTracker};
use crate::meeting::{parse_meeting, MeetingTracker, MeetingUpdate};
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
    /// Meeting captions: the utterances that changed (translate them at once, the text is still growing), or ended.
    pub updates: Vec<MeetingUpdate>,
}

/// Whether the region holds meeting captions (a speaker name above the text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeetingMode {
    /// Never: every region is plain subtitles.
    Off,
    /// The first speaker name that is read switches the region to meeting captions.
    #[default]
    Auto,
    /// Always: the text is followed word by word, with or without speaker names.
    On,
}

impl MeetingMode {
    pub fn parse(text: &str) -> Self {
        match text {
            "off" => Self::Off,
            "on" => Self::On,
            _ => Self::Auto,
        }
    }
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
    meeting: MeetingTracker,
    mode: MeetingMode,
    /// A speaker name was read: this region shows meeting captions.
    meeting_active: bool,
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
        self.meeting.reset();
        self.meeting_active = false;
        self.last_text.clear();
    }

    pub fn set_meeting_mode(&mut self, mode: MeetingMode) {
        self.mode = mode;
    }

    /// The captions are followed as a meeting (speaker names, word by word).
    pub fn meeting_active(&self) -> bool {
        self.mode == MeetingMode::On || (self.mode == MeetingMode::Auto && self.meeting_active)
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
        if self.mode != MeetingMode::Off && did_read {
            // The first speaker name read in the region switches it to meeting captions (until the region is reset).
            if parse_meeting(&split_lines(&self.last_text)).iter().any(|b| b.speaker.is_some()) {
                self.meeting_active = true;
            }
        }
        if self.meeting_active() {
            // No waiting for the text to settle: every change of a growing line is passed on at once.
            let text = self.last_text.trim().to_string();
            if text.is_empty() {
                step.updates = self.meeting.push_empty(now_ms);
                step.cleared = !step.updates.is_empty();
            } else {
                step.updates = self.meeting.push(now_ms, &parse_meeting(&split_lines(&text)));
            }
            step.updates.extend(self.meeting.tick(now_ms));
            return step;
        }
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

    // --- meeting captions ---

    fn meeting_play(mode: MeetingMode, sequence: &[(usize, &str)], every_ms: u64) -> Vec<(u64, Vec<MeetingUpdate>)> {
        let mut pipeline = Pipeline::new();
        pipeline.set_meeting_mode(mode);
        sequence
            .iter()
            .enumerate()
            .map(|(i, (picture, text))| {
                let now = i as u64 * every_ms;
                let step = pipeline.step(now, &frame(*picture), W, H, &mut || Some(text.to_string()));
                (now, step.updates)
            })
            .collect()
    }

    #[test]
    fn a_growing_meeting_caption_is_passed_on_at_every_change_without_waiting_for_a_second_read() {
        let seq = [(1, "AK
Anna Keller
We should"), (2, "AK
Anna Keller
We should look at"), (3, "AK
Anna Keller
We should look at the numbers.")];
        let out = meeting_play(MeetingMode::Auto, &seq, 500);
        // The very first read already gives the text (plain subtitles would wait for a second, equal read).
        assert_eq!(out[0].1.len(), 1);
        assert_eq!((out[0].1[0].speaker.as_deref(), out[0].1[0].text.as_str()), (Some("Anna Keller"), "We should"));
        assert_eq!(out[1].1[0].text, "We should look at");
        assert_eq!(out[2].1[0].text, "We should look at the numbers.");
        assert!(out.iter().all(|(_, u)| u.iter().all(|x| x.id == 1 && !x.finished)));
    }

    #[test]
    fn the_utterance_ends_after_a_pause_without_reading_again() {
        let seq = [(1, "AK
Anna Keller
Yes."), (1, "x"), (1, "x"), (1, "x"), (1, "x")];
        let out = meeting_play(MeetingMode::Auto, &seq, 500);
        let finished: Vec<u64> = out.iter().filter(|(_, u)| u.iter().any(|x| x.finished)).map(|(t, _)| *t).collect();
        assert_eq!(finished, [2000], "1800 ms after the last change, at the next look");
    }

    #[test]
    fn plain_subtitles_are_not_taken_for_a_meeting_in_auto_mode_and_off_never_is() {
        let seq = [(1, "We will begin with
the quarterly report."), (1, "x"), (1, "x")];
        assert!(meeting_play(MeetingMode::Auto, &seq, 400).iter().all(|(_, u)| u.is_empty()));
        let named = [(1, "AK
Anna Keller
We start now.")];
        assert!(meeting_play(MeetingMode::Off, &named, 400).iter().all(|(_, u)| u.is_empty()));
        assert_eq!(meeting_play(MeetingMode::On, &seq[..1], 400)[0].1.len(), 1, "On follows any text word by word");
    }

    #[test]
    fn meeting_captions_that_go_away_end_the_utterance() {
        let seq = [(1, "AK
Anna Keller
That is all."), (0, ""), (0, ""), (0, "")];
        let out = meeting_play(MeetingMode::Auto, &seq, 500);
        assert!(out[2].1.iter().any(|u| u.finished), "{out:?}");
    }
}
