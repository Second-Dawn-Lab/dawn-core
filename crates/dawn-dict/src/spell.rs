//! "Did you mean ...?" for the source text, built only on the headword and form lists of an installed dictionary pack (no new
//! data source). It is conservative: a word is only corrected when it is not in the dictionary and exactly one headword is
//! within one or two edits, and names, numbers, links, short words and words with capitals inside are left alone.

use crate::format::Dict;
use crate::lookup::distance;

/// One suggested correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    /// Byte range of the word in the text.
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub replacement: String,
}

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '\u{2019}' || c == '-'
}

/// Chunks (between spaces) that look like links, addresses, paths, numbers or file names are never touched.
fn skip_chunk(chunk: &str) -> bool {
    chunk.chars().any(|c| c.is_ascii_digit() || matches!(c, '@' | '/' | '\\' | '#' | '_' | '~'))
        || chunk.contains("://")
        || chunk.to_lowercase().starts_with("www.")
        || chunk.trim_matches(|c: char| !c.is_alphanumeric()).contains('.')
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + chars.as_str())
}

/// Corrections for the words of `text` that the dictionary does not know. Empty when there is nothing safe to suggest.
pub fn spell_fixes(dict: &Dict, text: &str) -> Vec<Fix> {
    let mut fixes = Vec::new();
    let mut sentence_start = true;
    let chars_at: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars_at.len() {
        let (pos, c) = chars_at[i];
        if !is_word_char(c) {
            if matches!(c, '.' | '!' | '?' | '\u{2026}') {
                sentence_start = true;
            }
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars_at.len() && is_word_char(chars_at[j].1) {
            j += 1;
        }
        let end = chars_at.get(j).map_or(text.len(), |b| b.0);
        let word = &text[pos..end];
        let at_start = std::mem::replace(&mut sentence_start, false);
        i = j;
        // The space-delimited chunk the word belongs to.
        let chunk_start = text[..pos].rfind(char::is_whitespace).map_or(0, |p| p + text[p..].chars().next().map_or(1, char::len_utf8));
        let chunk_end = text[end..].find(char::is_whitespace).map_or(text.len(), |e| end + e);
        if skip_chunk(&text[chunk_start..chunk_end]) || word.chars().count() < 4 || word.contains(['\'', '\u{2019}', '-']) {
            continue;
        }
        let chars: Vec<char> = word.chars().collect();
        let upper = chars.iter().filter(|c| c.is_uppercase()).count();
        // Names (a capital in the middle of a sentence), ALL CAPS and camelCase are left alone.
        if upper > 1 || (upper == 1 && !chars[0].is_uppercase()) || (chars[0].is_uppercase() && !at_start) {
            continue;
        }
        if !dict.postings(word).is_empty() {
            continue;
        }
        let Ok(candidates) = dict.suggest(word) else { continue };
        let lower: Vec<char> = word.to_lowercase().chars().collect();
        let limit = if lower.len() > 5 { 2 } else { 1 };
        let mut scored: Vec<(usize, String)> = candidates
            .into_iter()
            .filter(|c| !c.contains(' ') && !c.contains('-'))
            .map(|c| c.to_lowercase())
            .filter(|c| c.chars().next() == lower.first().copied())
            .map(|c| (distance(&lower, &c.chars().collect::<Vec<_>>(), limit), c))
            .filter(|(d, _)| *d >= 1 && *d <= limit)
            .collect();
        scored.sort();
        scored.dedup();
        let Some((best, replacement)) = scored.first().cloned() else { continue };
        // Two equally close headwords: not sure which one was meant.
        if scored.iter().filter(|(d, _)| *d == best).count() > 1 {
            continue;
        }
        let replacement = if chars[0].is_uppercase() { capitalize(&replacement) } else { replacement };
        fixes.push(Fix { start: pos, end, word: word.to_string(), replacement });
    }
    fixes
}

/// The text with the fixes applied.
pub fn apply_fixes(text: &str, fixes: &[Fix]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in fixes {
        if f.start < at || f.end > text.len() {
            continue;
        }
        out.push_str(&text[at..f.start]);
        out.push_str(&f.replacement);
        at = f.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;
    use crate::format::Writer;

    fn dict() -> Dict {
        let mut w = Writer::new("en", "de");
        for head in ["software", "developer", "hello", "world", "invoice", "before", "friday", "name", "where", "please", "send", "how", "are", "you", "work", "and", "the", "month", "bark", "park", "team", "fault", "version"] {
            let id = w.add_entry(&Entry { head: head.into(), pos: "noun".into(), ..Default::default() });
            if head == "developer" {
                w.add_key("developers", id, true);
            }
        }
        Dict::from_bytes(w.finish()).unwrap()
    }

    fn fix(text: &str) -> String {
        apply_fixes(text, &spell_fixes(&dict(), text))
    }

    #[test]
    fn misspelled_words_get_the_one_headword_that_is_close() {
        assert_eq!(fix("hi how are you ? i m softwar developer"), "hi how are you ? i m software developer");
        assert_eq!(fix("pls sned me the invoce befor frday"), "pls sned me the invoice before friday", "short words and unclear ones stay");
        assert_eq!(fix("work as softwar devloper"), "work as software developer");
    }

    #[test]
    fn known_words_forms_and_correct_text_are_untouched() {
        assert_eq!(fix("Hello world, the developers work."), "Hello world, the developers work.");
    }

    #[test]
    fn names_numbers_links_and_odd_capitals_are_left_alone() {
        for text in [
            "Please send it to Softwar Corp.",
            "mail ali@softwar.com now",
            "see www.softwar.org please",
            "open https://softwar.example/x",
            "the SOFTWAR team",
            "the sOftwar team",
            "version softwar2 here",
            "the soft-war team",
            "that softwar's fault",
        ] {
            assert_eq!(fix(text), text, "{text}");
        }
    }

    #[test]
    fn a_capitalized_word_at_the_start_of_a_sentence_can_be_fixed_and_keeps_its_capital() {
        assert_eq!(fix("Softwar is nice. Devloper too."), "Software is nice. Developer too.");
    }

    #[test]
    fn two_equally_close_headwords_are_not_guessed() {
        // One edit from both "bark" and "park" would be a tie, but only "bark" shares the first letter.
        assert_eq!(fix("the bxrk"), "the bark");
        assert_eq!(fix("the xark"), "the xark");
    }

    #[test]
    fn nothing_is_suggested_for_empty_text_or_another_script() {
        assert_eq!(fix(""), "");
        assert_eq!(fix("日本語のテキスト"), "日本語のテキスト");
    }
}
