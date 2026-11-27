//! Meeting captions (Teams, Zoom, Meet): the caption area shows an avatar with initials, a speaker name and one or two lines
//! of text for each dialog, and the newest dialog keeps growing while someone speaks, pushing the older ones up. Plain
//! subtitle logic (wait for the text to settle) lags behind that. This module
//!
//! * reads the layout: [`parse_meeting`] splits the recognized lines into blocks of one speaker each, leaving out the
//!   initials of the avatar, and keeps the speaker name apart from the text (a name is never translated or mixed into a
//!   sentence);
//! * follows what each speaker says: [`MeetingTracker`] gives every dialog (utterance) an id, recognizes it by its words
//!   wherever it moves (see [`crate::track`]), grows its text as the caption grows (also when the first words scroll out
//!   of the caption box), and reports an update at once for every change, so the translation can follow the speaker word
//!   by word. An utterance is finished when it moves up, another one follows, or after a pause; a later correction of its
//!   words updates it in place.

use crate::lines::split_lines;
use crate::track::{align, common_words, corrected, key, keys, same_word, CaptionUpdate, Item, HISTORY};

/// One speaker's caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub speaker: Option<String>,
    pub lines: Vec<String>,
}

impl Block {
    pub fn text(&self) -> String {
        self.lines.join(" ")
    }
}

/// Words that start a sentence but are never the first word of a name.
const NOT_A_NAME: [&str; 40] = [
    "good", "thank", "thanks", "yes", "no", "okay", "ok", "hello", "hi", "hallo", "guten", "danke", "ja", "nein", "so", "well", "also", "the", "der", "die", "das", "and", "und", "but", "aber", "this", "that", "what", "how", "why", "when", "where", "if", "we", "wir", "i", "ich", "please", "bitte", "now",
];

/// Lowercase words inside names ("Ludwig van Beethoven", "Ahmed bin Ali").
const PARTICLES: [&str; 14] = ["van", "von", "de", "der", "den", "la", "le", "di", "da", "bin", "al", "el", "ten", "ter"];

const LABELS: [&str; 6] = ["speaker", "sprecher", "guest", "gast", "unknown", "unbekannt"];

fn is_capitalized_word(w: &str) -> bool {
    let mut chars = w.chars();
    let Some(first) = chars.next() else { return false };
    let count = w.chars().count();
    if !first.is_uppercase() || !(2..=20).contains(&count) {
        return false;
    }
    // "NASA" is not a name; "Mc", "O'Neil" and "Anne-Marie" are.
    w.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'' || c == '\u{2019}') && w.chars().skip(1).any(|c| c.is_lowercase())
}

/// "Speaker 1", "Sprecher 2", "Guest".
fn is_label(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let Some(first) = parts.next() else { return false };
    LABELS.contains(&first.to_lowercase().as_str()) && parts.clone().count() <= 1 && parts.all(|p| p.chars().all(|c| c.is_ascii_digit()))
}

/// Could this line be a speaker name? Two to four words, each capitalized (or a particle between them), no punctuation at the
/// end, or a label such as "Speaker 1". A single capitalized word is only a name when `single_ok` (initials stood before it).
fn looks_like_name(line: &str, single_ok: bool) -> bool {
    let line = line.trim();
    if line.is_empty() || line.chars().count() > 40 {
        return false;
    }
    if is_label(line) {
        return true;
    }
    if line.ends_with(['.', '!', '?', ',', ';', ':', '\u{2026}', '"', '\u{201d}']) || line.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() > 4 || words.is_empty() || (words.len() == 1 && !single_ok) {
        return false;
    }
    if NOT_A_NAME.contains(&words[0].to_lowercase().as_str()) {
        return false;
    }
    let ok = words.iter().enumerate().all(|(i, w)| is_capitalized_word(w) || (i > 0 && i + 1 < words.len() && PARTICLES.contains(w)));
    ok
}

/// The initials of a name: "Anna Keller" is "AK".
fn initials_of(name: &str) -> (String, String) {
    let caps: Vec<char> = name.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_uppercase)).filter_map(|w| w.chars().next()).collect();
    let all: String = caps.iter().collect();
    let ends: String = match (caps.first(), caps.last()) {
        (Some(a), Some(b)) if caps.len() > 1 => [*a, *b].iter().collect(),
        _ => all.clone(),
    };
    (all, ends)
}

fn matches_initials(token: &str, name: &str) -> bool {
    let (all, ends) = initials_of(name);
    token == all || token == ends
}

/// Avatar initials on a line of their own: one to three capital letters.
fn is_initials(line: &str) -> bool {
    let n = line.chars().count();
    (1..=3).contains(&n) && line.chars().all(|c| c.is_alphabetic() && c.is_uppercase())
}

/// An icon read as stray characters ("@", "&", "Q").
fn is_noise(line: &str) -> bool {
    let letters = line.chars().filter(|c| c.is_alphanumeric()).count();
    line.chars().count() <= 3 && letters < 2 && !is_initials(line)
}

/// "AK Anna Keller": the avatar's initials and the name on one line. Returns the name.
fn name_after_initials(line: &str) -> Option<&str> {
    let (token, rest) = line.split_once(' ')?;
    (is_initials(token) && looks_like_name(rest, false) && matches_initials(token, rest)).then_some(rest.trim())
}

fn ends_sentence(line: &str) -> bool {
    line.trim_end().ends_with(['.', '!', '?', '\u{2026}', ':', '"', '\u{201d}', ')', '\u{3002}', '\u{ff01}', '\u{ff1f}'])
}

/// Split the recognized lines (one per screen line, top to bottom) into one block per speaker.
pub fn parse_meeting(lines: &[String]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current = Block { speaker: None, lines: Vec::new() };
    let mut after_initials = false;
    let flush = |blocks: &mut Vec<Block>, current: &mut Block| {
        if current.speaker.is_some() || !current.lines.is_empty() {
            blocks.push(std::mem::replace(current, Block { speaker: None, lines: Vec::new() }));
        }
    };
    // The names on screen: their initials are avatars, wherever the reading order put them.
    let names: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| looks_like_name(l, false) && !is_label(l)).collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() || is_noise(line) {
            continue;
        }
        // Avatar initials and the name on one line.
        if let Some(name) = name_after_initials(line) {
            flush(&mut blocks, &mut current);
            current.speaker = Some(name.to_string());
            after_initials = false;
            continue;
        }
        if is_initials(line) {
            let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
            // The initials before a name, or the initials of the speaker whose name was just read.
            if looks_like_name(next, true) && !next.is_empty() {
                after_initials = true;
                continue;
            }
            if line.chars().count() > 1 && (current.speaker.as_deref().is_some_and(|n| matches_initials(line, n)) || names.iter().any(|n| matches_initials(line, n))) {
                continue;
            }
        }
        let previous_done = current.lines.last().is_none_or(|l| ends_sentence(l));
        let has_text_after = i + 1 < lines.len();
        if has_text_after && looks_like_name(line, after_initials) && (previous_done || after_initials || current.speaker.is_none() || looks_like_name(line, false)) {
            flush(&mut blocks, &mut current);
            current.speaker = Some(line.to_string());
            after_initials = false;
            continue;
        }
        after_initials = false;
        // The avatar's initials in front of the first line of text ("HH Some words ...").
        let text = match line.split_once(' ') {
            Some((token, rest)) if is_initials(token) && current.speaker.as_deref().is_some_and(|n| matches_initials(token, n)) => rest.trim(),
            _ => line,
        };
        if !text.is_empty() {
            current.lines.push(text.to_string());
        }
    }
    flush(&mut blocks, &mut current);
    blocks
}

/// Convenience: parse recognized text.
pub fn parse_meeting_text(text: &str) -> Vec<Block> {
    parse_meeting(&split_lines(text))
}

// --- following what is said --------------------------------------------------------

#[derive(Debug, Clone)]
struct Utterance {
    id: u64,
    speaker: Option<String>,
    words: Vec<String>,
    /// `words` as they are compared.
    keys: Vec<String>,
    last_change_ms: u64,
    finished: bool,
    /// It was the bottom block of the last read: the one being spoken.
    at_bottom: bool,
    /// Another reading of it that waits to be seen once more before it is believed.
    pending: Option<Vec<String>>,
}

impl Utterance {
    fn update(&self) -> CaptionUpdate {
        CaptionUpdate { id: self.id, speaker: self.speaker.clone(), text: self.words.join(" "), finished: self.finished }
    }

    fn set(&mut self, mut words: Vec<String>, now_ms: u64) {
        if words.len() > MAX_WORDS {
            words.drain(..words.len() - MAX_WORDS);
        }
        self.keys = keys(&words);
        self.words = words;
        self.last_change_ms = now_ms;
        self.pending = None;
    }

    /// True the second time in a row the same other reading is seen.
    fn confirmed(&mut self, words: &[String]) -> bool {
        if self.pending.as_deref().is_some_and(|p| keys(p) == keys(words)) {
            return true;
        }
        self.pending = Some(words.to_vec());
        false
    }
}

/// The most words of an utterance that are kept.
const MAX_WORDS: usize = 300;

/// The utterance after the caption showed `new` for it. In the top block of the area the first words may have scrolled
/// out: the old words before the place where `new` begins are kept.
fn merge(old: &[String], new: &[String], top: bool) -> Vec<String> {
    let mut merged = if top { old[..scrolled_out(old, new)].to_vec() } else { Vec::new() };
    merged.extend_from_slice(new);
    merged
}

/// How many words at the start of `old` are above what `new` shows.
fn scrolled_out(old: &[String], new: &[String]) -> usize {
    let lower = |w: &[String]| w.iter().map(|w| key(w)).collect::<Vec<_>>();
    let (old, new) = (lower(old), lower(new));
    // The caption goes on where the end of the old words is the start of the new ones.
    for k in (old.len().min(2)..=old.len().min(new.len())).rev() {
        if k > 0 && old[old.len() - k..].iter().zip(&new[..k]).all(|(o, n)| same_word(o, n)) {
            return old.len() - k;
        }
    }
    // Some words were corrected: the distance most of the shared words have moved.
    let mut moved = std::collections::BTreeMap::new();
    for (i, j) in common_words(&old, &new) {
        *moved.entry(i as isize - j as isize).or_insert(0usize) += 1;
    }
    moved.into_iter().max_by_key(|&(_, count)| count).filter(|&(distance, count)| distance > 0 && count >= 2).map_or(0, |(distance, _)| distance as usize)
}

/// `new` is `old` with words missing.
fn shrunk(old: &[String], new: &[String]) -> bool {
    new.len() < old.len() && common_words(old, new).len() == new.len()
}

/// Follows the utterances of a meeting. Feed it the blocks of every read; it says what changed.
#[derive(Debug, Clone)]
pub struct MeetingTracker {
    /// The conversation so far, oldest first (the latest [`HISTORY`] utterances).
    said: Vec<Utterance>,
    /// Utterances before this index were on screen before the captions went away.
    floor: usize,
    next_id: u64,
    empty_reads: u32,
    /// An utterance with no change for this long is finished.
    pub pause_ms: u64,
}

impl Default for MeetingTracker {
    fn default() -> Self {
        Self { said: Vec::new(), floor: 0, next_id: 1, empty_reads: 0, pause_ms: 1800 }
    }
}

impl MeetingTracker {
    pub fn reset(&mut self) {
        self.said.clear();
        self.floor = 0;
        self.empty_reads = 0;
    }

    /// The blocks of one read, top to bottom. The last block is the one being spoken; the ones above it are over.
    pub fn push(&mut self, now_ms: u64, blocks: &[Block]) -> Vec<CaptionUpdate> {
        self.empty_reads = 0;
        let blocks: Vec<(Option<&str>, Vec<String>, Vec<String>)> = blocks
            .iter()
            .map(|b| {
                let words: Vec<String> = b.text().split_whitespace().map(str::to_string).collect();
                (b.speaker.as_deref(), keys(&words), words)
            })
            .filter(|(_, k, _)| !k.is_empty())
            .collect();
        if blocks.is_empty() {
            return Vec::new();
        }
        let places = {
            let known: Vec<Item> = self.said.iter().map(|u| Item { speaker: u.speaker.as_deref(), words: &u.keys }).collect();
            let seen: Vec<Item> = blocks.iter().map(|(speaker, k, _)| Item { speaker: *speaker, words: k }).collect();
            align(&known, &seen, self.floor, self.said.last().is_some_and(|u| u.at_bottom))
        };
        let bottom = blocks.len() - 1;
        // New utterances are the unknown blocks below everything that is known: captions only appear at the bottom.
        let first_new = places.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        let was_at_bottom: Vec<bool> = self.said.iter().map(|u| u.at_bottom).collect();
        self.said.iter_mut().for_each(|u| u.at_bottom = false);
        let newest = self.said.len().wrapping_sub(1);
        // What changed, by position in the conversation.
        let mut changed = std::collections::BTreeSet::new();
        for (n, place) in places.iter().enumerate() {
            let Some(j) = *place else { continue };
            let (speaker, _, words) = &blocks[n];
            let u = &mut self.said[j];
            let merged = merge(&u.words, words, n == 0);
            let merged_keys = keys(&merged);
            if merged_keys == u.keys {
                u.pending = None;
            } else if n == bottom && j == newest {
                // The speaker goes on (or the app rewrote the words). Words that went missing are believed on the second look.
                if !shrunk(&u.keys, &merged_keys) || u.confirmed(&merged) {
                    u.set(merged, now_ms);
                    u.finished = false;
                    changed.insert(j);
                }
            } else {
                // Moved up. At the very top a line can be half gone: that is not a correction.
                let cut = n == 0 && (merged.len() < u.words.len() || merged_keys.first() != u.keys.first());
                // The words that arrived while it moved up are taken at once; a later correction on the second look.
                let last_words = was_at_bottom[j] && merged.len() >= u.words.len();
                // Words that are only missing were not read, not corrected.
                if !cut && (last_words || (!shrunk(&u.keys, &merged_keys) && corrected(&u.words, &merged) && u.confirmed(&merged))) {
                    u.set(merged, now_ms);
                    changed.insert(j);
                }
            }
            if u.speaker.is_none() {
                u.speaker = speaker.map(str::to_string);
            }
            u.at_bottom = n == bottom;
            if n != bottom && !u.finished {
                u.finished = true;
                changed.insert(j);
            }
        }
        for (n, (speaker, k, words)) in blocks.iter().enumerate().skip(first_new) {
            // Left-overs of an earlier utterance at the top of the caption box (its first line cut off, no name) are not new.
            if n < bottom && speaker.is_none() {
                continue;
            }
            // Another utterance: whatever was running is over.
            for (j, u) in self.said.iter_mut().enumerate() {
                if !u.finished {
                    u.finished = true;
                    changed.insert(j);
                }
            }
            changed.insert(self.said.len());
            self.said.push(Utterance { id: self.next_id, speaker: speaker.map(str::to_string), words: words.clone(), keys: k.clone(), last_change_ms: now_ms, finished: n < bottom, at_bottom: n == bottom, pending: None });
            self.next_id += 1;
        }
        let out = changed.into_iter().map(|j| self.said[j].update()).collect();
        if self.said.len() > HISTORY {
            let gone = self.said.len() - HISTORY;
            self.said.drain(..gone);
            self.floor = self.floor.saturating_sub(gone);
        }
        out
    }

    /// A read with no caption text. Two in a row end the running utterance; the same short words later are a new one.
    pub fn push_empty(&mut self, _now_ms: u64) -> Vec<CaptionUpdate> {
        self.empty_reads += 1;
        let mut out = Vec::new();
        if self.empty_reads >= 2 {
            self.finish_all(&mut out);
            self.floor = self.said.len();
            self.said.iter_mut().for_each(|u| u.at_bottom = false);
        }
        out
    }

    /// Time passes: an utterance that has not changed for `pause_ms` is finished.
    pub fn tick(&mut self, now_ms: u64) -> Vec<CaptionUpdate> {
        let mut out = Vec::new();
        for u in &mut self.said {
            if !u.finished && now_ms.saturating_sub(u.last_change_ms) >= self.pause_ms {
                u.finished = true;
                out.push(u.update());
            }
        }
        out
    }

    fn finish_all(&mut self, out: &mut Vec<CaptionUpdate>) {
        for u in &mut self.said {
            if !u.finished {
                u.finished = true;
                out.push(u.update());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // --- the layout ---

    #[test]
    fn a_speaker_name_is_kept_apart_from_the_text_and_the_initials_are_dropped() {
        let blocks = parse_meeting(&lines(&["AK", "Anna Keller", "Good morning everyone, thanks for joining", "the call today."]));
        assert_eq!(blocks, [Block { speaker: Some("Anna Keller".into()), lines: lines(&["Good morning everyone, thanks for joining", "the call today."]) }]);
        assert_eq!(blocks[0].text(), "Good morning everyone, thanks for joining the call today.");
    }

    #[test]
    fn the_initials_and_the_name_on_one_line_or_after_the_name() {
        let b = parse_meeting(&lines(&["AK Anna Keller", "We start with the budget."]));
        assert_eq!(b[0].speaker.as_deref(), Some("Anna Keller"));
        assert_eq!(b[0].lines, ["We start with the budget."]);
        let b = parse_meeting(&lines(&["Anna Keller", "AK", "We start with the budget."]));
        assert_eq!((b[0].speaker.as_deref(), b[0].lines.clone()), (Some("Anna Keller"), lines(&["We start with the budget."])));
    }

    #[test]
    fn a_stray_icon_character_is_ignored() {
        let b = parse_meeting(&lines(&["@", "Bo Lindqvist", "Can everyone see my screen?"]));
        assert_eq!((b[0].speaker.as_deref(), b[0].lines.clone()), (Some("Bo Lindqvist"), lines(&["Can everyone see my screen?"])));
    }

    #[test]
    fn two_speakers_make_two_blocks_also_when_the_first_sentence_is_not_finished() {
        let b = parse_meeting(&lines(&["AK", "Anna Keller", "and then we should look at", "BL", "Bo Lindqvist", "Sorry to interrupt."]));
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].speaker.as_deref(), b[0].text().as_str()), (Some("Anna Keller"), "and then we should look at"));
        assert_eq!((b[1].speaker.as_deref(), b[1].text().as_str()), (Some("Bo Lindqvist"), "Sorry to interrupt."));
    }

    #[test]
    fn the_initials_in_front_of_the_first_line_of_text_are_dropped() {
        let b = parse_meeting(&lines(&["Anna Keller", "AK So the plan for next quarter is", "to move the team."]));
        assert_eq!((b[0].speaker.as_deref(), b[0].text().as_str()), (Some("Anna Keller"), "So the plan for next quarter is to move the team."));
        // A word in capitals that is not the speaker's initials stays.
        let b = parse_meeting(&lines(&["Anna Keller", "IT will be ready soon."]));
        assert_eq!(b[0].text(), "IT will be ready soon.");
    }

    #[test]
    fn the_initials_of_another_speaker_read_between_the_lines_are_not_text() {
        let b = parse_meeting(&lines(&["Anna Keller", "We start with the budget.", "BL", "AK", "Bo Lindqvist", "Fine with me."]));
        assert_eq!((b[0].text().as_str(), b[1].text().as_str()), ("We start with the budget.", "Fine with me."), "{b:?}");
        let b = parse_meeting(&lines(&["Anna Keller", "We start with the budget.", "Bo Lindqvist", "Fine with me.", "AK", "BL"]));
        assert_eq!(b[1].text(), "Fine with me.", "{b:?}");
    }

    #[test]
    fn several_blocks_of_the_same_speaker_one_below_the_other_are_separate_utterances() {
        let screen = lines(&["Mit dem Test dann wohl da.", "Anna Keller", "AK Das erste Stück.", "Anna Keller", "AK Das zweite.", "Anna Keller", "AK Das dritte"]);
        let blocks = parse_meeting(&screen);
        assert_eq!(blocks.len(), 4, "{blocks:?}");
        assert_eq!(blocks[0].speaker, None, "the cut-off first line of an earlier utterance has no name");
        assert_eq!(blocks[3].text(), "Das dritte");
        let mut t = MeetingTracker::default();
        let first = t.push(0, &blocks);
        // The last state of every utterance: the ones above are over, only the last block is being spoken.
        let mut last: std::collections::BTreeMap<u64, &CaptionUpdate> = std::collections::BTreeMap::new();
        for u in &first {
            last.insert(u.id, u);
        }
        let live: Vec<_> = last.values().filter(|u| !u.finished).collect();
        assert_eq!(live.len(), 1, "only the last block is being spoken: {first:?}");
        assert_eq!(live[0].text, "Das dritte");
        // The next read: the box scrolled, the last block grew, and the old ones are the same: only one update.
        let mut next = blocks.clone();
        next[3].lines = lines(&["Das dritte Stück kommt gleich."]);
        let u = t.push(400, &next);
        assert_eq!((u.len(), u[0].text.as_str()), (1, "Das dritte Stück kommt gleich."));
    }

    #[test]
    fn labels_such_as_speaker_1_are_names() {
        let b = parse_meeting(&lines(&["Speaker 1", "Welcome to the meeting."]));
        assert_eq!(b[0].speaker.as_deref(), Some("Speaker 1"));
        let b = parse_meeting(&lines(&["Sprecher 2", "Willkommen zum Treffen."]));
        assert_eq!(b[0].speaker.as_deref(), Some("Sprecher 2"));
    }

    #[test]
    fn names_with_particles_and_hyphens() {
        let b = parse_meeting(&lines(&["Jan van der Meer", "Thanks, I will share the notes."]));
        assert_eq!(b[0].speaker.as_deref(), Some("Jan van der Meer"));
        let b = parse_meeting(&lines(&["Anne-Marie Dupont", "Merci à tous."]));
        assert_eq!(b[0].speaker.as_deref(), Some("Anne-Marie Dupont"));
    }

    #[test]
    fn ordinary_sentences_are_not_taken_for_names() {
        for text in [lines(&["Good Morning", "everyone and welcome."]), lines(&["Thank You", "all for coming."]), lines(&["We will begin with", "the quarterly report."]), lines(&["Hello", "world."])] {
            let b = parse_meeting(&text);
            assert!(b.iter().all(|b| b.speaker.is_none()), "{b:?}");
        }
        // A name line is only a name when text follows.
        assert!(parse_meeting(&lines(&["Anna Keller"]))[0].speaker.is_none());
    }

    #[test]
    fn text_without_a_name_is_one_anonymous_block() {
        let b = parse_meeting(&lines(&["the budget for next year", "is almost ready."]));
        assert_eq!((b.len(), b[0].speaker.clone()), (1, None));
    }

    // --- following the speaker ---

    fn block(speaker: &str, text: &str) -> Vec<Block> {
        vec![Block { speaker: Some(speaker.into()), lines: vec![text.into()] }]
    }

    #[test]
    fn a_growing_line_is_one_utterance_updated_at_every_change_without_waiting() {
        let mut t = MeetingTracker::default();
        let first = t.push(0, &block("Anna Keller", "We should"));
        assert_eq!(first, [CaptionUpdate { id: 1, speaker: Some("Anna Keller".into()), text: "We should".into(), finished: false }]);
        let second = t.push(400, &block("Anna Keller", "We should look at the"));
        assert_eq!((second.len(), second[0].id, second[0].text.as_str()), (1, 1, "We should look at the"));
        assert!(t.push(500, &block("Anna Keller", "We should look at the")).is_empty(), "no change, no update");
        let third = t.push(900, &block("Anna Keller", "We should look at the numbers first."));
        assert_eq!(third[0].text, "We should look at the numbers first.");
    }

    #[test]
    fn when_the_caption_box_scrolls_the_utterance_keeps_all_its_words() {
        let mut t = MeetingTracker::default();
        t.push(0, &[Block { speaker: Some("Anna Keller".into()), lines: lines(&["We should look at the numbers", "for the second quarter and then"]) }]);
        let u = t.push(400, &[Block { speaker: Some("Anna Keller".into()), lines: lines(&["for the second quarter and then", "decide about the new hires."]) }]);
        assert_eq!(u[0].text, "We should look at the numbers for the second quarter and then decide about the new hires.");
        assert_eq!(u[0].id, 1);
    }

    #[test]
    fn a_word_that_was_read_cut_off_is_completed() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "The budget is ready for revi"));
        let u = t.push(400, &block("Anna Keller", "The budget is ready for review today"));
        assert_eq!(u[0].text, "The budget is ready for review today");
    }

    #[test]
    fn another_speaker_finishes_the_first_utterance_and_starts_a_new_one() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "That is all from me."));
        let u = t.push(500, &block("Bo Lindqvist", "Thanks Anna."));
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].id, u[0].finished), (1, true));
        assert_eq!((u[1].id, u[1].speaker.as_deref(), u[1].finished), (2, Some("Bo Lindqvist"), false));
    }

    #[test]
    fn two_blocks_on_screen_do_not_start_the_old_one_again() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "That is all from me."));
        let both = vec![Block { speaker: Some("Anna Keller".into()), lines: lines(&["That is all from me."]) }, Block { speaker: Some("Bo Lindqvist".into()), lines: lines(&["Thanks Anna."]) }];
        let u = t.push(500, &both);
        assert!(u.iter().any(|x| x.id == 1 && x.finished) && u.iter().any(|x| x.id == 2 && !x.finished), "{u:?}");
        // The next read has the same two blocks: nothing new.
        assert!(t.push(900, &both).is_empty());
        // Bo goes on.
        let more = vec![both[0].clone(), Block { speaker: Some("Bo Lindqvist".into()), lines: lines(&["Thanks Anna. Let me share."]) }];
        let u = t.push(1300, &more);
        assert_eq!((u.len(), u[0].id, u[0].text.as_str()), (1, 2, "Thanks Anna. Let me share."));
    }

    #[test]
    fn a_pause_finishes_the_utterance_and_a_cleared_screen_makes_the_same_words_new() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "Yes."));
        assert!(t.tick(1000).is_empty());
        let done = t.tick(1900);
        assert_eq!((done.len(), done[0].finished), (1, true));
        assert!(t.tick(5000).is_empty(), "once");
        assert!(t.push_empty(6000).is_empty());
        t.push_empty(6500);
        let again = t.push(7000, &block("Anna Keller", "Yes."));
        assert_eq!((again[0].id, again[0].finished), (2, false), "after the captions cleared, the same words are a new utterance");
    }

    #[test]
    fn an_ocr_slip_in_the_name_or_the_text_does_not_start_a_new_utterance() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "We should look at the numbers first"));
        let u = t.push(400, &block("Anna Kel1er", "We should look at the nurnbers first today"));
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].id, 1);
    }

    #[test]
    fn a_continuation_without_a_visible_name_belongs_to_the_running_utterance() {
        let mut t = MeetingTracker::default();
        t.push(0, &block("Anna Keller", "We should look at the numbers first"));
        let u = t.push(400, &[Block { speaker: None, lines: lines(&["look at the numbers first and then decide."]) }]);
        assert_eq!((u[0].id, u[0].speaker.as_deref()), (1, Some("Anna Keller")));
    }

    #[test]
    fn a_very_long_utterance_keeps_only_the_latest_words() {
        let mut t = MeetingTracker::default();
        let long: Vec<String> = (0..350).map(|i| format!("word{i}")).collect();
        let u = t.push(0, &[Block { speaker: None, lines: vec![long.join(" ")] }]);
        assert_eq!(u[0].text.split_whitespace().count(), 350, "a first read is taken as it is");
        let more = format!("{} word350 word351", long[long.len() - 10..].join(" "));
        let u = t.push(400, &[Block { speaker: None, lines: vec![more] }]);
        assert!(u[0].text.split_whitespace().count() <= MAX_WORDS);
        assert!(u[0].text.ends_with("word351"));
    }
}
