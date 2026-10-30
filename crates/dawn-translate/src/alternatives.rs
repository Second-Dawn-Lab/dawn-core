//! Word alternatives: "what else could stand here?".
//!
//! CTranslate2 can be asked to continue a translation from a given prefix and to return several
//! different first tokens (`return_alternatives`), each with the rest of the sentence generated
//! from it. So the alternatives for a clicked word are the continuations after the translation up
//! to that word, and choosing one gives a grammatical sentence: the model rewrites everything after
//! the new word instead of only swapping one word.
//!
//! The raw candidates are model tokens, so they are cleaned before they are shown: a candidate is a
//! whole word (all pieces up to the next word start, without trailing punctuation), duplicates and
//! fragments are dropped (a forced "ich" after "Ich bin" gives an ungrammatical sentence
//! with a much lower score is not a signal: scores are not comparable across candidates), duplicates and
//! fragments are dropped, a word that only repeats one of the two words in front of it is dropped ("Ich bin
//! ich ..." after "Ich bin"), and the current word comes first.

/// One alternative for a position.
#[derive(Debug, Clone, PartialEq)]
pub struct Alternative {
    /// The whole word at the position, as it would be written (without trailing punctuation).
    pub word: String,
    /// The whole sentence when this word is chosen: the prefix, the word and the generated rest.
    pub text: String,
    /// How the sentence continues from the word on ("erstaunt. ..."), to see what the choice does.
    pub preview: String,
    /// Length-normalized log probability of the sentence (higher is better).
    pub score: f32,
    /// This is the word that stands there now.
    pub current: bool,
}

/// SentencePiece marks the first piece of a word with this character.
pub const WORD_START: &str = "\u{2581}";

/// The pieces of the first word of a continuation: up to the next piece that starts a word.
pub fn first_word_pieces(continuation: &[String]) -> &[String] {
    let end = continuation.iter().skip(1).position(|p| p.starts_with(WORD_START)).map_or(continuation.len(), |i| i + 1);
    &continuation[..end]
}

fn is_closing_punctuation(c: char) -> bool {
    !c.is_alphanumeric() && !matches!(c, '\'' | '\u{2019}' | '-' | '_')
}

/// A word without the punctuation that SentencePiece glues to its last piece ("erstaunt." -> "erstaunt").
pub fn clean_word(word: &str) -> String {
    word.trim().trim_matches(is_closing_punctuation).to_string()
}

/// What two candidates have in common when they "only differ in case or punctuation".
pub fn word_key(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The start of a continuation for the list: the first few words, with an ellipsis when there is more.
pub fn preview_of(continuation: &str, max_chars: usize) -> String {
    let text = continuation.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    // End at a word boundary.
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max_chars / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{} \u{2026}", cut.trim_end())
}

/// A raw candidate straight from the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The pieces after the prefix.
    pub pieces: Vec<String>,
    /// Decoded word (first word of the continuation, with its punctuation).
    pub word: String,
    /// Decoded continuation.
    pub continuation: String,
    /// The whole sentence.
    pub text: String,
    pub score: f32,
}

/// Turn raw candidates into the list shown to the user.
///
/// * The current word (`current_word`, from the clicked text) is always first and marked; if the model did
///   not return it, it is added from the sentence as it stands (`sentence`, `current_continuation`).
/// * Candidates that glue onto the previous word (their first piece does not start a word, but the prefix is
///   not empty) are fragments and dropped, as are punctuation-only words and words that only differ from one
///   already listed in case or punctuation.
/// * A word equal to one of the two words right before the position is dropped: the model offers "ich" after
///   "Ich bin" with a good-looking continuation, but it is a repeat, not an alternative.
pub fn clean_candidates(candidates: Vec<Candidate>, current_word: &str, sentence: &str, current_continuation: &str, prefix_text: &str, limit: usize) -> Vec<Alternative> {
    let current_key = word_key(current_word);
    let prefix_is_empty = prefix_text.trim().is_empty();
    let recent: Vec<String> = prefix_text.split_whitespace().rev().take(2).map(word_key).collect();
    let best = candidates.iter().map(|c| c.score).fold(f32::NEG_INFINITY, f32::max);
    let mut out: Vec<Alternative> = Vec::new();
    let mut seen: Vec<String> = vec![current_key.clone()];
    let mut current: Option<Alternative> = None;
    for c in candidates {
        let word = clean_word(&c.word);
        let key = word_key(&word);
        let starts_word = prefix_is_empty || c.pieces.first().is_some_and(|p| p.starts_with(WORD_START));
        if key.is_empty() || !starts_word {
            continue;
        }
        if key == current_key {
            if current.is_none() {
                current = Some(Alternative { word: word.clone(), text: c.text.clone(), preview: preview_of(&c.continuation, 32), score: c.score, current: true });
            }
            continue;
        }
        if seen.contains(&key) || recent.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(Alternative { word, text: c.text, preview: preview_of(&c.continuation, 32), score: c.score, current: false });
    }
    out.truncate(limit.saturating_sub(1));
    let current = current.unwrap_or_else(|| Alternative { word: clean_word(current_word), text: sentence.to_string(), preview: preview_of(current_continuation, 32), score: best, current: true });
    let mut list = vec![current];
    list.extend(out);
    list
}

/// Whole-sentence alternatives: the model's n-best translations without the current one, repeats, empty
/// ones and ones that only differ in case, spacing or punctuation. The input order (best first) is kept.
pub fn dedupe_sentences(candidates: Vec<String>, current: &str, count: usize) -> Vec<String> {
    let mut seen = vec![word_key(current)];
    let mut out = Vec::new();
    for text in candidates {
        let text = text.trim().to_string();
        let key = word_key(&text);
        if key.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(text);
        if out.len() >= count {
            break;
        }
    }
    out
}

/// The pieces that come before the word starting at byte `word_start` of the sentence. `spans` are the
/// byte ranges the pieces cover in the sentence (what SentencePiece reports). A piece is in the prefix when
/// it ends at or before the word: whatever it is (a whole word, one piece of a long word, punctuation). The
/// end is used, not the start, because SentencePiece counts the space in front of a word to the word's own
/// piece ("▁bin" covers the space before "bin"), so the piece of the clicked word *starts* before it.
pub fn prefix_piece_count(spans: &[(usize, usize)], word_start: usize) -> usize {
    spans.iter().take_while(|(_, end)| *end <= word_start).count()
}

/// The byte offset of UTF-16 offset `at` in `text` (a click in the page counts in UTF-16 units).
pub fn byte_offset_of_utf16(text: &str, at: usize) -> usize {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units >= at {
            return byte;
        }
        units += c.len_utf16();
    }
    text.len()
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

/// Every word of `text` as `(start, end)` UTF-16 offsets, in order.
pub fn word_bounds_all(text: &str) -> Vec<(usize, usize)> {
    let len = text.encode_utf16().count();
    let mut out = Vec::new();
    let mut at = 0;
    while at < len {
        match word_bounds(text, at) {
            Some((start, end)) => {
                out.push((start, end));
                at = end.max(at + 1);
            }
            None => at += 1,
        }
    }
    out
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
    fn words_lose_the_punctuation_glued_to_them() {
        assert_eq!(clean_word("erstaunt."), "erstaunt");
        assert_eq!(clean_word("\u{201e}Hallo\u{201c},"), "Hallo");
        assert_eq!(clean_word("don't"), "don't");
        assert_eq!(clean_word("well-known."), "well-known");
        assert_eq!(clean_word("..."), "");
        assert_eq!(word_key("\u{dc}berrascht,"), "\u{fc}berrascht");
        assert_eq!(word_key("Erstaunt"), word_key("erstaunt."));
    }

    #[test]
    fn the_preview_shows_how_the_sentence_continues() {
        assert_eq!(preview_of("erstaunt.", 32), "erstaunt.");
        let long = preview_of("sprachlos, weil sie so lange keine Nachricht von mir bekommen hatte.", 32);
        assert!(long.ends_with('\u{2026}'), "{long}");
        assert!(long.starts_with("sprachlos, weil sie so lange"), "{long}");
        assert!(long.chars().count() <= 36);
    }

    #[test]
    fn sentence_alternatives_drop_the_current_one_repeats_and_near_duplicates() {
        let n = vec![
            "Ich bin erstaunt.".to_string(),
            "Ich bin \u{fc}berrascht.".to_string(),
            "ich bin ERSTAUNT".to_string(),
            "Ich bin \u{fc}berrascht!".to_string(),
            " ".to_string(),
            "Ich bin verbl\u{fc}fft.".to_string(),
            "Ich staune.".to_string(),
            "Mich wundert das.".to_string(),
        ];
        let out = dedupe_sentences(n, "Ich bin erstaunt.", 3);
        assert_eq!(out, vec!["Ich bin \u{fc}berrascht.", "Ich bin verbl\u{fc}fft.", "Ich staune."]);
    }

    #[test]
    fn the_prefix_is_every_piece_that_starts_before_the_word() {
        // "Ich bin erstaunt." -> pieces: Ich | bin | er | staunt | .   (byte spans of the letters, not the space mark)
        let spans = [(0, 3), (4, 7), (8, 10), (10, 16), (16, 17)];
        assert_eq!(prefix_piece_count(&spans, 8), 2, "the word starts at byte 8: two pieces before it");
        assert_eq!(prefix_piece_count(&spans, 0), 0, "the first word has no prefix");
        // SentencePiece counts the space in front of a word to that word's piece: the same answer
        assert_eq!(prefix_piece_count(&[(0, 3), (3, 7), (7, 16), (16, 17)], 8), 2);
        assert_eq!(prefix_piece_count(&[(0, 3), (3, 7), (7, 16), (16, 17)], 0), 0);
        // a word after punctuation: "Hallo, Welt" -> Hallo | , | Welt
        let spans = [(0, 5), (5, 6), (7, 11)];
        assert_eq!(prefix_piece_count(&spans, 7), 2);
        // a word split in several pieces is still one word: the clicked word is the one starting at 8
        assert_eq!(prefix_piece_count(&[(0, 3), (4, 7), (8, 10), (10, 16)], 8), 2);
    }

    #[test]
    fn utf16_offsets_become_byte_offsets() {
        assert_eq!(byte_offset_of_utf16("Ich bin erstaunt", 8), 8);
        // "\u{fc}" is two bytes but one UTF-16 unit; the emoji is four bytes and two units.
        assert_eq!(byte_offset_of_utf16("\u{fc}ber alles", 4), 5);
        assert_eq!(byte_offset_of_utf16("\u{1F989} Eule", 3), 5);
        assert_eq!(byte_offset_of_utf16("abc", 99), 3);
    }

    fn cand(word: &str, first_piece: &str, text: &str, score: f32) -> Candidate {
        Candidate { pieces: vec![first_piece.to_string()], word: word.into(), continuation: word.into(), text: text.into(), score }
    }

    #[test]
    fn candidates_are_cleaned_ranked_and_the_current_word_comes_first() {
        let w = "\u{2581}";
        let list = vec![
            cand("ich", &format!("{w}ich"), "Ich bin ich erstaunt.", -1.9),
            cand("erstaunt.", &format!("{w}erstaunt"), "Ich bin erstaunt.", -0.4),
            cand("\u{fc}berrascht.", &format!("{w}\u{fc}berrascht"), "Ich bin \u{fc}berrascht.", -0.5),
            cand("\u{dc}berrascht.", &format!("{w}\u{dc}berrascht"), "Ich bin \u{dc}berrascht.", -0.6),
            cand("verbl\u{fc}fft.", &format!("{w}verbl\u{fc}fft"), "Ich bin verbl\u{fc}fft.", -0.52),
            cand("e", "e", "Ich bine erstaunt.", -0.5),
            cand(".", &format!("{w}."), "Ich bin.", -0.55),
            cand("sehr", &format!("{w}sehr"), "Ich bin sehr erstaunt.", -0.56),
        ];
        let out = clean_candidates(list, "erstaunt", "Ich bin erstaunt.", "erstaunt.", "Ich bin", 10);
        let words: Vec<&str> = out.iter().map(|a| a.word.as_str()).collect();
        assert_eq!(words, ["erstaunt", "\u{fc}berrascht", "verbl\u{fc}fft", "sehr"], "current first, then in the model's order; no repeat of \"ich\", no fragment, no punctuation, no case duplicate");
        assert!(out[0].current && !out[1].current);
        assert_eq!(out[0].text, "Ich bin erstaunt.");
    }

    #[test]
    fn the_current_word_is_added_when_the_model_did_not_return_it() {
        let list = vec![cand("anders", "\u{2581}anders", "Ich bin anders.", -0.5)];
        let out = clean_candidates(list, "erstaunt", "Ich bin erstaunt.", "erstaunt.", "Ich bin", 10);
        assert_eq!(out[0].word, "erstaunt");
        assert!(out[0].current);
        assert_eq!(out[0].text, "Ich bin erstaunt.");
        assert_eq!(out[1].word, "anders");
    }

    #[test]
    fn the_list_is_capped() {
        let w = "\u{2581}";
        let list: Vec<Candidate> = (0..20).map(|i| cand(&format!("wort{}", "x".repeat(i + 1)), &format!("{w}w{i}"), "t", -0.3 - i as f32 * 0.001)).collect();
        assert_eq!(clean_candidates(list, "zzz", "s", "zzz", "x", 10).len(), 10);
    }

    #[test]
    fn at_the_start_of_a_sentence_the_first_piece_may_lack_the_word_mark() {
        let list = vec![cand("Die", "Die", "Die Katze.", -0.3)];
        assert_eq!(clean_candidates(list, "Der", "Der Kater.", "Der", "", 10).len(), 2);
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
    fn all_words_are_listed_in_order() {
        assert_eq!(word_bounds_all("Ich bin erstaunt."), vec![(0, 3), (4, 7), (8, 16)]);
        assert!(word_bounds_all(" ,. ").is_empty());
    }

    #[test]
    fn offsets_count_utf16_units_not_characters() {
        let text = "\u{1F989} Eule";
        assert_eq!(word_bounds(text, 4), Some((3, 7)));
    }
}
