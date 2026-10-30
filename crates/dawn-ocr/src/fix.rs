//! Conservative repairs of the slips OCR makes on small text, and a way to pick the cleaner of two
//! readings of the same image.
//!
//! No dictionary is available, so a repair needs evidence from the text itself: the repaired word must
//! occur elsewhere in the text, or the slip must be a shape that real text does not have (a letter
//! glued to a closing bracket). A word that is already fine is never touched.

/// Letters that small, blurry commas and full stops are most often read as.
const COMMA_LOOKALIKES: &[char] = &['t', 'r', 'l', 'i', 'j'];

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | ':' | '!' | '?' | '"')).filter(|t| !t.is_empty())
}

fn strip_edges(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Words of the text as they stand alone (lowercase), for evidence.
fn vocabulary(text: &str) -> std::collections::HashSet<String> {
    tokens(text).map(strip_edges).filter(|t| t.chars().count() >= 3 && t.chars().all(char::is_alphabetic)).map(str::to_lowercase).collect()
}

/// Fix what can be fixed with confidence.
///
/// * A letter glued to a closing bracket (`int8)r and`) was a comma: `int8), and`.
/// * A word ending in a comma lookalike whose base word is in the text without it, followed by
///   another lowercase word (`detectiont and` with `detection` elsewhere): `detection, and`.
/// * A capitalized word in the middle of a sentence whose lowercase form is in the text (`With` after
///   a lowercase word, `with` elsewhere): lowercased.
pub fn fix_slips(text: &str) -> String {
    let vocab = vocabulary(text);
    text.split('\n').map(|line| fix_line(line, &vocab, text)).collect::<Vec<_>>().join("\n")
}

fn fix_line(line: &str, vocab: &std::collections::HashSet<String>, whole: &str) -> String {
    let words: Vec<&str> = line.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, word) in words.iter().enumerate() {
        let next = words.get(i + 1).copied();
        let previous = out.last().map(String::as_str);
        out.push(fix_word(word, previous, next, vocab, whole));
    }
    out.join(" ")
}

fn fix_word(word: &str, previous: Option<&str>, next: Option<&str>, vocab: &std::collections::HashSet<String>, whole: &str) -> String {
    let chars: Vec<char> = word.chars().collect();

    // `)r`, `]t`: a bracket followed by one stray lowercase letter and then the end of the word.
    if chars.len() >= 3 {
        let (last, before) = (chars[chars.len() - 1], chars[chars.len() - 2]);
        if matches!(before, ')' | ']') && COMMA_LOOKALIKES.contains(&last) && chars[..chars.len() - 2].iter().any(|c| c.is_alphanumeric()) {
            return format!("{},", chars[..chars.len() - 1].iter().collect::<String>());
        }
    }

    // `detectiont` -> `detection,`
    if chars.len() >= 5 && chars.iter().all(|c| c.is_alphabetic()) && chars.last().is_some_and(|c| COMMA_LOOKALIKES.contains(c)) {
        let base: String = chars[..chars.len() - 1].iter().collect();
        let continues = next.is_some_and(|n| n.chars().next().is_some_and(char::is_lowercase));
        let base_known = vocab.contains(&base.to_lowercase());
        // Evidence that the glued form is not a word of this text: it occurs once, the base elsewhere.
        let once = whole.matches(word).count() == 1;
        if continues && base_known && once {
            return format!("{base},");
        }
    }

    // `With` in the middle of a sentence
    if chars.len() >= 3 && chars[0].is_uppercase() && chars[1..].iter().all(|c| c.is_lowercase()) {
        let lower = word.to_lowercase();
        let mid_sentence = previous.is_some_and(|p| p.chars().last().is_some_and(|c| c.is_lowercase() || c == ','));
        let seen_lowercase = whole.split(|c: char| !c.is_alphabetic()).any(|t| t == lower);
        if mid_sentence && seen_lowercase {
            return lower;
        }
    }
    word.to_string()
}

/// How many words of `reading` look like a slip that `other` did not make: a word of `reading` that
/// is the same as a word of `other` plus a stray last letter, where `other` has punctuation instead.
pub fn slips_against(reading: &str, other: &str) -> usize {
    let theirs: std::collections::HashSet<&str> = tokens_keep_punctuation(other).collect();
    tokens_keep_punctuation(reading)
        .filter(|t| !theirs.contains(t))
        .filter(|t| {
            let n = t.chars().count();
            if n < 4 {
                return false;
            }
            let (head, last) = (&t[..t.len() - t.chars().last().map_or(0, char::len_utf8)], t.chars().last().unwrap_or(' '));
            COMMA_LOOKALIKES.contains(&last) && (theirs.contains(format!("{head},").as_str()) || theirs.contains(format!("{head}.").as_str()) || theirs.contains(head))
        })
        .count()
}

fn tokens_keep_punctuation(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
}

/// Words that no text has: a symbol between letters (`inst:ll`, `int8%`) or a capital at the end of a
/// lowercase word (`modeL`). They are what a misread letter often looks like.
pub fn oddities(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| {
            let chars: Vec<char> = word.trim_start_matches(|c: char| !c.is_alphanumeric()).chars().collect();
            let inner_symbol = chars.windows(3).any(|w| w[0].is_alphabetic() && matches!(w[1], ':' | '%' | '|' | '^' | '~' | '@' | '#') && w[2].is_alphabetic());
            let capital_tail = chars.len() >= 4 && chars[..chars.len() - 1].iter().all(|c| c.is_lowercase()) && chars[chars.len() - 1].is_uppercase();
            let trailing_symbol = chars.len() >= 3 && chars[0].is_alphabetic() && matches!(chars[chars.len() - 1], '%' | '|' | '^' | '~') ;
            inner_symbol || capital_tail || trailing_symbol
        })
        .count()
}

/// The index of the cleaner reading: the one with fewer slips (compared with each other) and fewer
/// odd words. On a tie the earlier one wins.
pub fn cleaner(readings: &[String]) -> usize {
    let mut best = 0;
    let mut best_cost = usize::MAX;
    for (i, reading) in readings.iter().enumerate() {
        let slips: usize = readings.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, other)| slips_against(reading, other)).sum();
        let cost = slips + oddities(reading);
        if cost < best_cost {
            best = i;
            best_cost = cost;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_glued_to_a_closing_bracket_was_a_comma() {
        assert_eq!(fix_slips("Packs are converted to CTranslate2 (int8)r and stored"), "Packs are converted to CTranslate2 (int8), and stored");
        assert_eq!(fix_slips("see [1]t and more"), "see [1], and more");
    }

    #[test]
    fn a_word_with_a_glued_comma_lookalike_is_fixed_when_the_text_has_the_base_word() {
        let text = "Language detection is local. We run detectiont and then translate";
        assert_eq!(fix_slips(text), "Language detection is local. We run detection, and then translate");
    }

    #[test]
    fn real_words_that_end_in_those_letters_are_never_touched() {
        for text in ["The result is that a hub transfers text to the part", "Print the report and insert the list", "Hello world it works"] {
            assert_eq!(fix_slips(text), text);
        }
        // `part` exists in the text without the final letter ("par") only by accident: no evidence, no change.
        assert_eq!(fix_slips("par and part continue here"), "par and part continue here");
    }

    #[test]
    fn a_capital_in_the_middle_of_a_sentence_is_lowercased_only_with_evidence() {
        assert_eq!(fix_slips("Install the app with care and With the app you can start"), "Install the app with care and with the app you can start");
        // Names and no lowercase twin in the text: unchanged.
        assert_eq!(fix_slips("Ask Anna about Berlin"), "Ask Anna about Berlin");
        // After a full stop a capital is right.
        assert_eq!(fix_slips("It works. With care it lasts, and with luck too"), "It works. With care it lasts, and with luck too");
    }

    #[test]
    fn line_breaks_and_spacing_are_kept() {
        let text = "First line (int8)r and\n\nsecond   line";
        assert_eq!(fix_slips(text), "First line (int8), and\n\nsecond   line");
    }

    #[test]
    fn the_reading_without_slips_is_the_cleaner_one() {
        let low = "We run detectiont and then stop".to_string();
        let high = "We run detection, and then stop".to_string();
        assert_eq!(cleaner(&[low.clone(), high.clone()]), 1);
        assert_eq!(cleaner(&[high, low]), 0);
    }

    #[test]
    fn odd_words_count_against_a_reading() {
        assert_eq!(oddities("Pscks are inst:ll and (int8% with the modeL"), 3);
        assert_eq!(oddities("Packs are installed (int8), with the model and CTranslate2"), 0);
        let noisy = "Download the psck inst:ll it".to_string();
        let clean = "Download the pack install it".to_string();
        assert_eq!(cleaner(&[noisy.clone(), clean.clone()]), 1);
        assert_eq!(cleaner(&[clean, noisy]), 0);
    }

    #[test]
    fn identical_readings_keep_the_first() {
        let a = "Same text here".to_string();
        assert_eq!(cleaner(&[a.clone(), a]), 0);
    }
}
