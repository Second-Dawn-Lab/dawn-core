//! Word alternatives: "what else could stand here?".
//!
//! CTranslate2 can be asked to continue a translation from a given prefix and to return several
//! different first tokens (`return_alternatives`), each with the rest of the sentence generated
//! from it. So the alternatives for a clicked word are the continuations after the translation up
//! to that word, and choosing one gives a grammatical sentence: the model rewrites everything after
//! the new word instead of only swapping one word.

/// One alternative for a position.
#[derive(Debug, Clone, PartialEq)]
pub struct Alternative {
    /// The word (or word piece group) at the position, as it would be written.
    pub word: String,
    /// The whole sentence when this word is chosen: the prefix, the word and the generated rest.
    pub text: String,
}

/// SentencePiece marks the first piece of a word with this character.
const WORD_START: &str = "\u{2581}";

/// The pieces of the first word of a continuation: up to the next piece that starts a word.
pub fn first_word_pieces(continuation: &[String]) -> &[String] {
    let end = continuation.iter().skip(1).position(|p| p.starts_with(WORD_START)).map_or(continuation.len(), |i| i + 1);
    &continuation[..end]
}

/// Keep the first alternative per distinct word, drop the one equal to the current word, and cap the list.
pub fn dedupe(alternatives: Vec<Alternative>, current_word: &str, count: usize) -> Vec<Alternative> {
    let current = current_word.trim().to_lowercase();
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for alt in alternatives {
        let key = alt.word.trim().to_lowercase();
        if key.is_empty() || key == current || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(alt);
        if out.len() >= count {
            break;
        }
    }
    out
}

/// The word that contains UTF-16 offset `at` in `text` (what a click in the page gives), as
/// `(start, end)` UTF-16 offsets, or `None` when the click is on whitespace or punctuation.
pub fn word_bounds(text: &str, at: usize) -> Option<(usize, usize)> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if at >= units.len() {
        return None;
    }
    let is_word = |u: u16| char::from_u32(u as u32).is_some_and(|c| c.is_alphanumeric() || c == '\'' || c == '\u{2019}' || c == '-' || c == '_');
    if !is_word(units[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && is_word(units[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end < units.len() && is_word(units[end]) {
        end += 1;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_first_word_ends_before_the_next_word_start() {
        let p = pieces(&["\u{2581}Be", "gr\u{fc}\u{df}ung", "\u{2581}ist", "\u{2581}toll"]);
        assert_eq!(first_word_pieces(&p), &p[..2]);
        let single = pieces(&["\u{2581}Hallo"]);
        assert_eq!(first_word_pieces(&single), &single[..]);
        assert!(first_word_pieces(&[]).is_empty());
    }

    #[test]
    fn a_continuation_that_does_not_start_a_word_keeps_its_first_piece() {
        // After a prefix that ends inside a word, the first piece has no word-start mark.
        let p = pieces(&["ung", "\u{2581}ist"]);
        assert_eq!(first_word_pieces(&p), &p[..1]);
    }

    fn alt(word: &str) -> Alternative {
        Alternative { word: word.into(), text: format!("Die {word}.") }
    }

    #[test]
    fn duplicates_and_the_current_word_are_dropped_and_the_list_is_capped() {
        let list = vec![alt("Katze"), alt("katze"), alt("Kater"), alt("Tier"), alt("Hund"), alt("Maus")];
        let out = dedupe(list, "Kater", 3);
        assert_eq!(out.iter().map(|a| a.word.as_str()).collect::<Vec<_>>(), ["Katze", "Tier", "Hund"]);
    }

    #[test]
    fn words_are_found_around_a_click() {
        let text = "Die Katze, die d\u{e4}mmert.";
        assert_eq!(word_bounds(text, 5), Some((4, 9)));
        assert_eq!(word_bounds(text, 4), Some((4, 9)));
        assert_eq!(word_bounds(text, 8), Some((4, 9)));
        assert_eq!(word_bounds(text, 9), None, "the comma");
        assert_eq!(word_bounds(text, 3), None, "the space");
        assert_eq!(word_bounds(text, 100), None);
        assert_eq!(word_bounds("don't stop", 2), Some((0, 5)));
    }

    #[test]
    fn offsets_count_utf16_units_not_characters() {
        // The emoji is two UTF-16 units: the word after it starts at 3, not 2.
        let text = "\u{1F989} Eule";
        assert_eq!(word_bounds(text, 4), Some((3, 7)));
    }
}
