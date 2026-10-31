//! Which of the lines on screen are new. Captions scroll (the oldest line goes, a new one appears), grow (a line gets
//! its last words) and repeat ("Yes." twice); only what is new is translated.

use crate::stable::nearly_same;

/// What to show after a stable read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delta {
    /// The first of `lines` is the last line shown before, now with more words: it replaces it.
    pub replace_last: bool,
    /// The lines to translate and add.
    pub lines: Vec<String>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Split recognized text into its lines (empty ones dropped).
pub fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect()
}

/// New lines grouped into the pieces to translate: a line that does not end a sentence belongs with the next one ("We will
/// begin with" + "the quarterly report."), so a caption split over two screen lines is translated as one sentence.
pub fn group_sentences(lines: &[String]) -> Vec<String> {
    let ends = |l: &str| l.trim_end().ends_with(['.', '!', '?', '\u{2026}', ':', '"', '\u{201d}', '\u{bb}', ')', '\u{3002}', '\u{ff01}', '\u{ff1f}', '\u{61f}', '\u{964}']);
    let mut groups: Vec<String> = Vec::new();
    let mut open = false;
    for line in lines {
        match groups.last_mut() {
            Some(last) if open => {
                last.push(' ');
                last.push_str(line);
            }
            _ => groups.push(line.clone()),
        }
        open = !ends(line);
    }
    groups
}

fn same(a: &str, b: &str) -> bool {
    nearly_same(a, b)
}

/// `longer` is `shorter` with more words after it.
fn extends(longer: &str, shorter: &str) -> bool {
    let (l, s) = (longer.to_lowercase(), shorter.to_lowercase());
    l.len() > s.len() + 2 && l.starts_with(s.trim_end_matches(['.', ',', '\u{2026}', ' '])) && !same(&l, &s)
}

#[derive(Debug, Clone, Default)]
pub struct LineTracker {
    prev: Vec<String>,
}

impl LineTracker {
    /// The lines of a stable read; returns what is new compared with the last read.
    pub fn push(&mut self, now: &[String]) -> Delta {
        let prev = std::mem::replace(&mut self.prev, now.to_vec());
        if now.is_empty() {
            return Delta::default();
        }
        if prev.is_empty() {
            return Delta { replace_last: false, lines: now.to_vec() };
        }
        // The longest stretch where the end of the old lines is the start of the new ones (scrolling).
        for k in (1..=prev.len().min(now.len())).rev() {
            if prev[prev.len() - k..].iter().zip(&now[..k]).all(|(a, b)| same(a, b)) {
                return Delta { replace_last: false, lines: now[k..].to_vec() };
            }
        }
        // The last line got longer.
        if let Some(last) = prev.last() {
            if let Some(j) = now.iter().position(|l| extends(l, last)) {
                return Delta { replace_last: true, lines: now[j..].to_vec() };
            }
        }
        Delta { replace_last: false, lines: now.to_vec() }
    }

    pub fn reset(&mut self) {
        self.prev.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn text_is_split_into_clean_lines() {
        assert_eq!(split_lines("  Hello   there \n\n How are you?\n"), ["Hello there", "How are you?"]);
        assert!(split_lines(" \n ").is_empty());
    }

    #[test]
    fn lines_of_one_sentence_are_translated_together() {
        assert_eq!(group_sentences(&lines(&["We will begin with", "the quarterly report.", "Please open page four."])), ["We will begin with the quarterly report.", "Please open page four."]);
        assert_eq!(group_sentences(&lines(&["One.", "Two!", "Three?"])), ["One.", "Two!", "Three?"]);
        assert_eq!(group_sentences(&lines(&["no end here"])), ["no end here"]);
        assert!(group_sentences(&[]).is_empty());
    }

    #[test]
    fn the_first_read_is_all_new_and_the_same_read_again_is_nothing() {
        let mut t = LineTracker::default();
        assert_eq!(t.push(&lines(&["A one", "B two"])).lines, ["A one", "B two"]);
        assert!(t.push(&lines(&["A one", "B two"])).is_empty());
        assert!(t.push(&lines(&["A one", "B twu"])).is_empty(), "an OCR slip is the same line");
    }

    #[test]
    fn scrolling_captions_only_give_the_new_line() {
        let mut t = LineTracker::default();
        t.push(&lines(&["Line one here", "Line two here"]));
        let d = t.push(&lines(&["Line two here", "Line three here"]));
        assert_eq!((d.replace_last, d.lines), (false, lines(&["Line three here"])));
        let d = t.push(&lines(&["Line three here", "Line four here"]));
        assert_eq!(d.lines, ["Line four here"]);
        // Several lines at once.
        let d = t.push(&lines(&["Line four here", "Line five here", "Line six here"]));
        assert_eq!(d.lines, ["Line five here", "Line six here"]);
    }

    #[test]
    fn a_caption_that_gets_longer_replaces_its_earlier_form() {
        let mut t = LineTracker::default();
        t.push(&lines(&["Earlier line stays", "We are going"]));
        let d = t.push(&lines(&["Earlier line stays", "We are going to the market today"]));
        assert_eq!((d.replace_last, d.lines), (true, lines(&["We are going to the market today"])));
    }

    #[test]
    fn a_completely_new_caption_is_all_new() {
        let mut t = LineTracker::default();
        t.push(&lines(&["Good morning everyone"]));
        let d = t.push(&lines(&["Thanks for joining us today"]));
        assert_eq!((d.replace_last, d.lines.len()), (false, 1));
    }

    #[test]
    fn a_repeated_line_after_the_screen_cleared_is_new_again_but_not_while_it_stays() {
        let mut t = LineTracker::default();
        assert_eq!(t.push(&lines(&["Yes."])).lines, ["Yes."]);
        assert!(t.push(&lines(&["Yes."])).is_empty());
        t.reset(); // the captions went away (Stabilizer: Cleared)
        assert_eq!(t.push(&lines(&["Yes."])).lines, ["Yes."]);
    }

    #[test]
    fn a_line_that_shrinks_or_scrolls_out_gives_nothing() {
        let mut t = LineTracker::default();
        t.push(&lines(&["Alpha beta gamma", "Delta epsilon zeta"]));
        assert!(t.push(&lines(&["Delta epsilon zeta"])).is_empty());
        assert!(t.push(&[]).is_empty());
    }
}
