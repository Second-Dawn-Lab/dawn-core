//! Waiting for text to settle. Subtitles often appear word by word, and OCR of the same picture can differ by a letter;
//! translating every read would make the caption bar flicker. Text is passed on when two reads in a row agree, or when
//! it has been there a while.

fn norm(s: &str) -> Vec<char> {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().chars().take(300).collect()
}

/// The edit distance of two texts, ignoring case and spacing differences (long texts: their first 300 characters), and the
/// length of the longer one.
fn edits(a: &str, b: &str) -> (usize, usize) {
    let (a, b) = (norm(a), norm(b));
    let longest = a.len().max(b.len());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    (prev[b.len()], longest)
}

/// Similarity of two strings from 0 (nothing alike) to 1 (equal): `1 - edit distance / length`.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (distance, longest) = edits(a, b);
    if longest == 0 {
        1.0
    } else {
        1.0 - distance as f32 / longest as f32
    }
}

/// Are these the same text, give or take what OCR gets wrong? Up to one letter in eight may differ (at least one letter).
pub fn nearly_same(a: &str, b: &str) -> bool {
    let (distance, longest) = edits(a, b);
    distance <= (longest / 8).max(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not stable yet (or nothing new).
    Wait,
    /// The text to translate now.
    Stable(String),
    /// The captions went away: what was shown is over.
    Cleared,
}

#[derive(Debug, Clone)]
pub struct Stabilizer {
    last_read: Option<String>,
    /// How many reads in a row were the same as `last_read`.
    same_reads: u32,
    /// When the text last became different, for the timeout.
    changed_at: u64,
    /// The text passed on last (until the captions clear).
    emitted: Option<String>,
    /// Pass text on after this long even if it keeps changing a little.
    pub timeout_ms: u64,
    /// Reads of empty text in a row (one empty read can be a missed frame).
    empty_reads: u32,
}

impl Default for Stabilizer {
    fn default() -> Self {
        Self { last_read: None, same_reads: 0, changed_at: 0, emitted: None, timeout_ms: 1500, empty_reads: 0 }
    }
}

impl Stabilizer {
    pub fn observe(&mut self, now_ms: u64, text: &str) -> Verdict {
        let text = text.trim();
        if text.is_empty() {
            self.empty_reads += 1;
            self.last_read = None;
            self.same_reads = 0;
            if self.empty_reads >= 2 && self.emitted.take().is_some() {
                return Verdict::Cleared;
            }
            return Verdict::Wait;
        }
        self.empty_reads = 0;
        match &self.last_read {
            Some(prev) if nearly_same(prev, text) => self.same_reads += 1,
            _ => {
                self.same_reads = 1;
                self.changed_at = now_ms;
            }
        }
        self.last_read = Some(text.to_string());

        let settled = self.same_reads >= 2 || now_ms.saturating_sub(self.changed_at) >= self.timeout_ms;
        if !settled {
            return Verdict::Wait;
        }
        match &self.emitted {
            Some(done) if nearly_same(done, text) => Verdict::Wait,
            _ => {
                self.emitted = Some(text.to_string());
                Verdict::Stable(text.to_string())
            }
        }
    }

    /// Start over (the region moved, the language changed).
    pub fn reset(&mut self) {
        *self = Self { timeout_ms: self.timeout_ms, ..Self::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(reads: &[&str], step: u64) -> Vec<Verdict> {
        let mut s = Stabilizer::default();
        reads.iter().enumerate().map(|(i, r)| s.observe(i as u64 * step, r)).collect()
    }

    #[test]
    fn similar_texts_are_nearly_one_and_different_ones_are_not() {
        assert_eq!(similarity("Hello world", "hello  world"), 1.0);
        assert!(similarity("Hello world", "Hello wor1d") > 0.9);
        assert!(similarity("Hello world", "Goodbye") < 0.5);
        assert_eq!(similarity("", ""), 1.0);
        assert!(similarity("a", "a very long caption that is nothing like it") < 0.2);
    }

    #[test]
    fn text_that_builds_up_word_by_word_is_passed_on_once_it_stops_changing() {
        let out = run(&["The", "The quick", "The quick brown", "The quick brown fox", "The quick brown fox", "The quick brown fox"], 400);
        assert_eq!(out[..4], [Verdict::Wait, Verdict::Wait, Verdict::Wait, Verdict::Wait]);
        assert_eq!(out[4], Verdict::Stable("The quick brown fox".into()));
        assert_eq!(out[5], Verdict::Wait, "once only");
    }

    #[test]
    fn a_one_letter_ocr_difference_does_not_count_as_a_change() {
        let out = run(&["We went to the market", "We went to the rnarket", "We went to the market"], 400);
        assert!(matches!(out[1], Verdict::Stable(_)), "{out:?}");
        assert_eq!(out[2], Verdict::Wait);
    }

    #[test]
    fn text_that_never_settles_is_passed_on_after_the_timeout() {
        // Moving video behind the captions makes every read a bit different.
        let reads = ["Coming up next", "Coming up nex", "Coming up next t", "Coming up next", "Coming up next"];
        let mut s = Stabilizer::default();
        let mut got = None;
        for (i, r) in reads.iter().enumerate() {
            if let Verdict::Stable(t) = s.observe(i as u64 * 500, r) {
                got = Some((i, t));
                break;
            }
        }
        assert!(got.is_some(), "the timeout must pass the text on");
    }

    #[test]
    fn a_new_caption_after_the_old_one_is_stable_again_and_a_cleared_screen_is_reported() {
        let mut s = Stabilizer::default();
        let mut at = 0;
        let mut feed = |s: &mut Stabilizer, t: &str| {
            at += 500;
            s.observe(at, t)
        };
        feed(&mut s, "First line");
        assert_eq!(feed(&mut s, "First line"), Verdict::Stable("First line".into()));
        assert_eq!(feed(&mut s, "First line"), Verdict::Wait);
        assert_eq!(feed(&mut s, ""), Verdict::Wait, "one empty read can be a missed frame");
        assert_eq!(feed(&mut s, ""), Verdict::Cleared);
        assert_eq!(feed(&mut s, ""), Verdict::Wait, "reported once");
        feed(&mut s, "First line");
        assert_eq!(feed(&mut s, "First line"), Verdict::Stable("First line".into()), "the same words again after a gap are new");
    }
}
