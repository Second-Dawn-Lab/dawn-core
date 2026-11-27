//! Plain subtitles: which captions on screen are new. Captions scroll (a line moves up as a new one appears), grow (a
//! line gets its last words), wrap (one sentence over two screen lines), get corrected and repeat ("Yes." twice). Lines
//! are recognized by their words (see [`crate::track`]), and the lines of one sentence make one caption, which is
//! translated as a whole: a new one, or the same one with other text.

use crate::track::{align, corrected, keys, CaptionUpdate, Item, HISTORY};

/// Ids of plain captions start here, so they never meet the ids of meeting utterances of the same session.
const FIRST_ID: u64 = 1 << 32;

/// The most screen lines joined into one caption (captions without punctuation would otherwise never end).
const MAX_LINES_PER_CAPTION: usize = 3;

/// Split recognized text into its lines (empty ones dropped).
pub fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect()
}

fn ends_sentence(line: &str) -> bool {
    line.trim_end().ends_with(['.', '!', '?', '\u{2026}', ':', '"', '\u{201d}', '\u{bb}', ')', '\u{3002}', '\u{ff01}', '\u{ff1f}', '\u{61f}', '\u{964}'])
}

/// One screen line of the conversation.
#[derive(Debug, Clone)]
struct Line {
    caption: u64,
    text: String,
    keys: Vec<String>,
}

/// Follows plain captions. Feed it the lines of every stable read; it says which captions are new or changed.
#[derive(Debug, Clone)]
pub struct LineTracker {
    /// The conversation so far, oldest first (the latest [`HISTORY`] lines).
    said: Vec<Line>,
    /// Lines before this index were on screen before the captions went away.
    floor: usize,
    next_id: u64,
    /// The caption that is not finished yet (the newest one) and when it last changed.
    open: Option<(u64, u64)>,
    /// A caption with no change for this long is finished.
    pub pause_ms: u64,
}

impl Default for LineTracker {
    fn default() -> Self {
        Self { said: Vec::new(), floor: 0, next_id: FIRST_ID, open: None, pause_ms: 1800 }
    }
}

impl LineTracker {
    fn update(&self, caption: u64, finished: bool) -> CaptionUpdate {
        let text: Vec<&str> = self.said.iter().filter(|l| l.caption == caption).map(|l| l.text.as_str()).collect();
        CaptionUpdate { id: caption, speaker: None, text: text.join(" "), finished }
    }

    /// The lines of a stable read, top to bottom; returns the captions that are new or have other text now.
    pub fn push(&mut self, now_ms: u64, lines: &[String]) -> Vec<CaptionUpdate> {
        let seen: Vec<(&String, Vec<String>)> = lines.iter().map(|l| (l, keys(&l.split_whitespace().collect::<Vec<_>>()))).filter(|(_, k)| !k.is_empty()).collect();
        if seen.is_empty() {
            return Vec::new();
        }
        let places = {
            let known: Vec<Item> = self.said.iter().map(|l| Item { speaker: None, words: &l.keys }).collect();
            let items: Vec<Item> = seen.iter().map(|(_, k)| Item { speaker: None, words: k }).collect();
            align(&known, &items, self.floor, self.floor < self.said.len())
        };
        let newest = self.said.len().wrapping_sub(1);
        let mut changed = std::collections::BTreeSet::new();
        for (n, place) in places.iter().enumerate() {
            let Some(j) = *place else { continue };
            let (text, k) = &seen[n];
            let line = &mut self.said[j];
            if *k == line.keys {
                continue;
            }
            // The line that was the newest takes what is read now (it grows, also while it moves up); an older one only a
            // real correction, and never a line that is half gone at the top.
            let was_newest = j == newest && k.len() >= line.keys.len();
            let cut = n == 0 && (k.len() < line.keys.len() || k.first() != line.keys.first());
            if was_newest || (!cut && corrected(&line.keys, k)) {
                line.text = (*text).clone();
                line.keys = k.clone();
                changed.insert(line.caption);
            }
        }
        // New lines are the unknown ones below everything that is known.
        let first_new = places.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        // The line above the first new one on screen is the newest line of the conversation.
        let mut follows = first_new > 0 && places[first_new - 1] == Some(newest);
        for (text, k) in seen.iter().skip(first_new) {
            let continued = match self.said.last() {
                Some(last) if follows && !ends_sentence(&last.text) && self.said.iter().filter(|l| l.caption == last.caption).count() < MAX_LINES_PER_CAPTION => Some(last.caption),
                _ => None,
            };
            let caption = continued.unwrap_or_else(|| {
                self.next_id += 1;
                self.next_id
            });
            self.said.push(Line { caption, text: (*text).clone(), keys: k.clone() });
            changed.insert(caption);
            follows = true;
        }
        let newest_caption = self.said.last().map(|l| l.caption);
        let mut out = Vec::new();
        // Another caption follows the running one: it is over.
        if let Some((id, _)) = self.open.filter(|(id, _)| Some(*id) != newest_caption) {
            self.open = None;
            changed.insert(id);
        }
        for id in changed {
            let live = Some(id) == newest_caption;
            if live {
                self.open = Some((id, now_ms));
            }
            out.push(self.update(id, !live));
        }
        if self.said.len() > HISTORY {
            let gone = self.said.len() - HISTORY;
            self.said.drain(..gone);
            self.floor = self.floor.saturating_sub(gone);
        }
        out
    }

    /// Time passes: the running caption is finished when it has not changed for `pause_ms`.
    pub fn tick(&mut self, now_ms: u64) -> Vec<CaptionUpdate> {
        match self.open {
            Some((_, since)) if now_ms.saturating_sub(since) >= self.pause_ms => self.finish(),
            _ => Vec::new(),
        }
    }

    /// The captions went away: the running one is over, and the same short words later are new.
    pub fn clear(&mut self) -> Vec<CaptionUpdate> {
        self.floor = self.said.len();
        self.finish()
    }

    /// The running caption is over.
    pub fn finish(&mut self) -> Vec<CaptionUpdate> {
        self.open.take().map(|(id, _)| self.update(id, true)).into_iter().collect()
    }

    /// Forget the conversation (ids keep counting).
    pub fn reset(&mut self) {
        self.said.clear();
        self.floor = 0;
        self.open = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The updates as (number of the caption, text, finished).
    fn push(t: &mut LineTracker, now: u64, items: &[&str]) -> Vec<(u64, String, bool)> {
        t.push(now, &lines(items)).into_iter().map(|u| (u.id - FIRST_ID, u.text, u.finished)).collect()
    }

    fn live(id: u64, text: &str) -> (u64, String, bool) {
        (id, text.to_string(), false)
    }

    fn done(id: u64, text: &str) -> (u64, String, bool) {
        (id, text.to_string(), true)
    }

    #[test]
    fn text_is_split_into_clean_lines() {
        assert_eq!(split_lines("  Hello   there \n\n How are you?\n"), ["Hello there", "How are you?"]);
        assert!(split_lines(" \n ").is_empty());
    }

    #[test]
    fn a_sentence_on_two_screen_lines_is_one_caption() {
        let mut t = LineTracker::default();
        assert_eq!(push(&mut t, 0, &["We will begin with", "the quarterly report."]), [live(1, "We will begin with the quarterly report.")]);
        // The second line arrives later: the same caption with more text.
        let mut t = LineTracker::default();
        push(&mut t, 0, &["We will begin with"]);
        assert_eq!(push(&mut t, 500, &["We will begin with", "the quarterly report."]), [live(1, "We will begin with the quarterly report.")]);
    }

    #[test]
    fn scrolling_captions_give_only_the_new_line_and_finish_the_one_above() {
        let mut t = LineTracker::default();
        push(&mut t, 0, &["The first point is the budget.", "The second point is hiring."]);
        let u = push(&mut t, 500, &["The second point is hiring.", "The third point is the office."]);
        assert_eq!(u, [done(2, "The second point is hiring."), live(3, "The third point is the office.")]);
        assert!(push(&mut t, 900, &["The second point is hiring.", "The third point is the office."]).is_empty());
        // The top line leaves the area: nothing changes.
        assert!(push(&mut t, 1300, &["The third point is the office."]).is_empty());
    }

    #[test]
    fn a_growing_line_that_moved_up_with_its_last_words_is_updated_not_repeated() {
        let mut t = LineTracker::default();
        push(&mut t, 0, &["Earlier sentence stays here.", "We are going to"]);
        let u = push(&mut t, 500, &["We are going to the market today.", "Bring your own bag."]);
        assert_eq!(u, [done(2, "We are going to the market today."), live(3, "Bring your own bag.")]);
    }

    #[test]
    fn ocr_slips_and_a_cut_top_line_change_nothing_but_a_correction_does() {
        let mut t = LineTracker::default();
        push(&mut t, 0, &["We went to the market early.", "The apples were very good."]);
        assert!(push(&mut t, 500, &["We went to the rnarket early.", "The apples were very good."]).is_empty());
        assert!(push(&mut t, 900, &["the market early.", "The apples were very good."]).is_empty());
        let u = push(&mut t, 1300, &["We went to the market early.", "The apples were rather good."]);
        assert_eq!(u, [live(2, "The apples were rather good.")]);
    }

    #[test]
    fn a_repeated_line_after_the_screen_cleared_is_new_again_but_not_while_it_stays() {
        let mut t = LineTracker::default();
        assert_eq!(push(&mut t, 0, &["Yes."]), [live(1, "Yes.")]);
        assert!(push(&mut t, 500, &["Yes."]).is_empty());
        assert_eq!(t.clear().len(), 1);
        assert_eq!(push(&mut t, 900, &["Yes."]), [live(2, "Yes.")]);
    }

    #[test]
    fn a_pause_finishes_the_running_caption_once_and_a_reset_keeps_counting() {
        let mut t = LineTracker::default();
        push(&mut t, 0, &["Hello there, everyone."]);
        assert!(t.tick(1000).is_empty());
        let u = t.tick(2000);
        assert_eq!((u.len(), u[0].finished), (1, true));
        assert!(t.tick(5000).is_empty());
        t.reset();
        assert_eq!(push(&mut t, 6000, &["Hello there, everyone."]), [live(2, "Hello there, everyone.")]);
    }

    #[test]
    fn captions_without_punctuation_do_not_grow_without_end() {
        let mut t = LineTracker::default();
        let all = ["so the first thing", "we want to talk about", "is the plan for", "the coming spring season"];
        push(&mut t, 0, &all[..1]);
        push(&mut t, 400, &all[..2]);
        push(&mut t, 800, &all[1..3]);
        let u = push(&mut t, 1200, &all[2..]);
        assert_eq!(u.last().unwrap(), &live(2, "the coming spring season"));
        assert_eq!(u.first().unwrap(), &done(1, "so the first thing we want to talk about is the plan for"));
    }
}
