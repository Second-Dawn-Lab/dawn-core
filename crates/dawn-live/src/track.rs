//! Recognizing captions by their text. A caption area scrolls: a finished dialog moves up while the new speech grows at the
//! bottom. Where a line is on screen says nothing about what it is, so every read is matched against the conversation so
//! far by its words, in order: what is on screen is a stretch of the conversation (the newest at the bottom), possibly with
//! new captions after it.
//!
//! * [`align`] matches the captions on screen with the captions known so far. It keeps their order, tolerates OCR slips,
//!   growth, a first line cut off at the top and words the caption app corrected afterwards.
//! * The trackers ([`crate::meeting::MeetingTracker`] for speaker blocks, [`crate::lines::LineTracker`] for plain
//!   subtitles) decide what changed.

use crate::stable::nearly_same;

/// A change of one caption (an utterance of a speaker, or a sentence of plain subtitles), to translate and show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionUpdate {
    /// Ids grow in the order the captions were spoken; a known id is the same caption with other text.
    pub id: u64,
    pub speaker: Option<String>,
    pub text: String,
    /// The caption is over (it moved up, another one follows, or nothing changed for a while): its translation can be
    /// polished. A finished caption can still be corrected, and then comes again.
    pub finished: bool,
}

/// The most captions remembered for matching.
pub(crate) const HISTORY: usize = 64;

/// A caption from before the captions went away is only recognized again when this many words agree: the same short words
/// after a gap ("Yes.") are new.
const WORDS_ACROSS_A_GAP: usize = 4;

/// A word as it is compared: letters and digits in lower case.
pub(crate) fn key(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The words of a text as they are compared (punctuation alone is no word).
pub(crate) fn keys<S: AsRef<str>>(words: &[S]) -> Vec<String> {
    words.iter().map(|w| key(w.as_ref())).filter(|k| !k.is_empty()).collect()
}

/// The same word, give or take what OCR gets wrong (longer words only: "in" and "on" differ).
pub(crate) fn same_word(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // Letters that look like one another on screen.
    let look = |w: &str| w.replace("rn", "m").replace("vv", "w");
    if look(a) == look(b) {
        return true;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    // A number that differs is another number.
    if a.chars().chain(b.chars()).any(|c| c.is_ascii_digit()) {
        return false;
    }
    la >= 4 && lb >= 4 && la.abs_diff(lb) <= (la.max(lb) / 8).max(1) && nearly_same(a, b)
}

/// The words two texts share, in order (longest common subsequence), as pairs of positions.
pub(crate) fn common_words(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let width = b.len() + 1;
    let same: Vec<bool> = a.iter().flat_map(|x| b.iter().map(move |y| same_word(x, y))).collect();
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if same[i * b.len() + j] { table[(i + 1) * width + j + 1] + 1 } else { table[(i + 1) * width + j].max(table[i * width + j + 1]) };
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        if same[i * b.len() + j] && table[i * width + j] == table[(i + 1) * width + j + 1] + 1 {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// `short` is the beginning (a caption still growing) or the end (its first words scrolled out) of `long`. A last word
/// that was read half counts.
fn head_or_tail(short: &[String], long: &[String]) -> bool {
    let n = short.len();
    if n == 0 || n > long.len() {
        return false;
    }
    let head = short.iter().zip(long).enumerate().all(|(i, (s, l))| same_word(s, l) || (i + 1 == n && l.starts_with(s.as_str())));
    head || short.iter().zip(&long[long.len() - n..]).all(|(s, l)| same_word(s, l))
}

/// A caption, as [`align`] sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Item<'a> {
    pub speaker: Option<&'a str>,
    pub words: &'a [String],
}

fn same_speaker(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => nearly_same(a, b),
        _ => true,
    }
}

/// How well a caption on screen is a known one: the number of shared words and a score, or `None` when it is another one.
fn relation(known: Item, seen: Item) -> Option<(usize, f32)> {
    if !same_speaker(known.speaker, seen.speaker) {
        return None;
    }
    let common = common_words(known.words, seen.words).len();
    let short = known.words.len().min(seen.words.len());
    let long = known.words.len().max(seen.words.len());
    // One or two words say little: they must be the beginning or the end of the other text.
    let (a, b) = if known.words.len() <= seen.words.len() { (known.words, seen.words) } else { (seen.words, known.words) };
    // One text is the other one still growing, or cut off at the top.
    let part = head_or_tail(a, b);
    // One or two words say little: they must be such a part.
    if !(part || (short > 2 && common * 10 >= short * 6)) {
        return None;
    }
    let common = common.max(1);
    // The more of both texts agrees, the more the match counts: a caption read again beats one that only shares its pattern.
    let agreement = if part { 0.8f32.max(common as f32 / long as f32) } else { common as f32 / long as f32 };
    Some((common, common.min(12) as f32 * agreement * agreement + 0.5))
}

/// Close enough to be the same caption rewritten, where the order already says which one it should be.
fn loosely(known: Item, seen: Item) -> bool {
    if !same_speaker(known.speaker, seen.speaker) || known.words.is_empty() || seen.words.is_empty() {
        return false;
    }
    let common = common_words(known.words, seen.words).len();
    let short = known.words.len().min(seen.words.len());
    let first = same_word(&known.words[0], &seen.words[0]);
    // A caption of one or two words was probably read in the middle of its first word.
    let barely_begun = known.words.len() <= 2 && known.words[0].chars().next() == seen.words[0].chars().next();
    (common >= 1 && common * 3 >= short) || first || barely_begun
}

/// Which known caption (oldest first) each caption on screen (top to bottom) is; `None` for one that is not known.
/// The order is kept: a caption below another one is a later one. Captions before `floor` were on screen before the
/// captions went away. `live` says the newest known caption was the bottom one at the last read.
pub(crate) fn align(known: &[Item], seen: &[Item], floor: usize, live: bool) -> Vec<Option<usize>> {
    const SKIP: f32 = 0.6;
    let skip = |n: usize| SKIP * n.min(5) as f32;
    let (n, m) = (known.len(), seen.len());
    let mut result = vec![None; m];
    if n == 0 || m == 0 {
        return result;
    }
    // The best chain of matches ending with seen[i] = known[j]: (score, shared words, the match before it).
    let mut best: Vec<Option<(f32, usize, Option<(usize, usize)>)>> = vec![None; n * m];
    for i in 0..m {
        for j in 0..n {
            let Some((common, score)) = relation(known[j], seen[i]) else { continue };
            if j < floor && common < WORDS_ACROSS_A_GAP {
                continue;
            }
            let mut chain = (score, common, None);
            for pi in 0..i {
                for pj in 0..j {
                    if let Some((s, c, _)) = best[pi * n + pj] {
                        let total = s + score - skip(j - pj - 1) - SKIP * (i - pi - 1) as f32;
                        if total > chain.0 {
                            chain = (total, c + common, Some((pi, pj)));
                        }
                    }
                }
            }
            best[i * n + j] = Some(chain);
        }
    }
    // The chain that explains most, preferring one that reaches the newest captions.
    let mut end: Option<(f32, usize, usize)> = None;
    for i in 0..m {
        for j in 0..n {
            let Some((score, common, _)) = best[i * n + j] else { continue };
            // A few words alone do not pull the conversation back to an older caption.
            if j + 1 < n && common < 3 {
                continue;
            }
            let total = score - skip(n - 1 - j);
            if total > 0.0 && end.is_none_or(|(t, ..)| total >= t) {
                end = Some((total, i, j));
            }
        }
    }
    let mut at = end.map(|(_, i, j)| (i, j));
    while let Some((i, j)) = at {
        result[i] = Some(j);
        at = best[i * n + j].and_then(|(_, _, before)| before);
    }
    pair_rewritten(known, seen, live, &mut result);
    result
}

/// Captions that did not match by their words but can only be one known caption: between two matches there are as many
/// unknown ones on screen as known ones not seen, or the one below the last match was the growing one.
fn pair_rewritten(known: &[Item], seen: &[Item], live: bool, result: &mut [Option<usize>]) {
    let anchors: Vec<(usize, usize)> = result.iter().enumerate().filter_map(|(i, j)| j.map(|j| (i, j))).collect();
    for pair in anchors.windows(2) {
        let ((i0, j0), (i1, j1)) = (pair[0], pair[1]);
        if i1 - i0 == j1 - j0 {
            for k in 1..i1 - i0 {
                if loosely(known[j0 + k], seen[i0 + k]) {
                    result[i0 + k] = Some(j0 + k);
                }
            }
        }
    }
    if !live {
        return;
    }
    let newest = known.len() - 1;
    let slot = match anchors.last() {
        Some(&(i, j)) if j + 1 == newest => i + 1,
        None if seen.len() == 1 => 0,
        _ => return,
    };
    if slot < seen.len() && loosely(known[newest], seen[slot]) {
        result[slot] = Some(newest);
    }
}

/// A difference that is more than the usual slips of OCR (a misread letter, two words read as one): worth translating a
/// finished caption again.
pub(crate) fn corrected(old: &[String], new: &[String]) -> bool {
    let (a, b) = (keys(old), keys(new));
    let slips = a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| same_word(x, y));
    !slips && a.concat() != b.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        keys(&text.split_whitespace().collect::<Vec<_>>())
    }

    /// Aligns texts without speakers.
    fn matched(known: &[&str], seen: &[&str]) -> Vec<Option<usize>> {
        let k: Vec<_> = known.iter().map(|t| words(t)).collect();
        let s: Vec<_> = seen.iter().map(|t| words(t)).collect();
        let k: Vec<Item> = k.iter().map(|w| Item { speaker: None, words: w }).collect();
        let s: Vec<Item> = s.iter().map(|w| Item { speaker: None, words: w }).collect();
        align(&k, &s, 0, true)
    }

    #[test]
    fn captions_that_moved_up_are_the_same_and_the_one_below_is_new() {
        let known = ["we start with the budget", "then the new hires", "and the office move"];
        assert_eq!(matched(&known, &["then the new hires", "and the office move", "any questions so far"]), [Some(1), Some(2), None]);
    }

    #[test]
    fn many_visible_captions_all_keep_their_place() {
        let known: Vec<String> = (0..12).map(|i| format!("point number {i} of the long agenda item{i}")).collect();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        assert_eq!(matched(&known, &known[3..]), (3..12).map(Some).collect::<Vec<_>>());
    }

    #[test]
    fn equal_short_captions_are_told_apart_by_their_order() {
        let known = ["yes", "we will send the plan on friday", "yes"];
        assert_eq!(matched(&known, &["yes", "we will send the plan on friday", "yes"]), [Some(0), Some(1), Some(2)]);
        assert_eq!(matched(&known[..2], &["yes", "we will send the plan on friday", "yes"]), [Some(0), Some(1), None]);
    }

    #[test]
    fn a_new_caption_that_starts_like_the_one_above_is_not_taken_for_it() {
        let known = ["and then we look at the numbers"];
        assert_eq!(matched(&known, &["and then we look at the numbers", "and then"]), [Some(0), None]);
    }

    #[test]
    fn ocr_slips_a_cut_first_line_and_a_correction_are_the_same_caption() {
        let known = ["we should look at the numbers for the second quarter"];
        assert_eq!(matched(&known, &["we should look at the nurnbers for the second quarter"]), [Some(0)]);
        assert_eq!(matched(&known, &["for the second quarter"]), [Some(0)]);
        assert_eq!(matched(&["the best and fine"], &["the test is fine today"]), [Some(0)]);
    }

    #[test]
    fn a_changed_word_is_a_correction_and_a_misread_letter_is_not() {
        let said = words("we will bring ten chairs to the large room on the ground floor");
        assert!(corrected(&said, &words("we will bring two chairs to the large room on the ground floor")));
        assert!(!corrected(&said, &words("we will bring ten chairs to the large roorn on the ground floor")));
        assert!(!corrected(&said, &words("we will bring ten chairs tothe large room on the ground floor")));
    }

    #[test]
    fn a_short_caption_that_comes_again_does_not_reach_back_to_an_old_one() {
        assert_eq!(matched(&["yes", "that is right"], &["yes"]), [None]);
    }
}
