//! Join recognized lines into text for translation.
//!
//! OCR returns one string per visual line. Translating line by line breaks sentences, so lines
//! that belong together are joined with a space; paragraph breaks (a larger gap, or a line that
//! ends a sentence short) stay as line breaks. A word hyphenated at the end of a line is
//! stitched back together.

use crate::OcrLine;

/// A gap taller than this many line heights starts a new paragraph.
const PARAGRAPH_GAP: f32 = 0.8;

fn ends_sentence(text: &str) -> bool {
    text.trim_end().chars().last().is_some_and(|c| matches!(c, '.' | '!' | '?' | ':' | ';' | '…' | '。' | '！' | '？' | '۔' | '؟'))
}

fn starts_new_item(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    match chars.next() {
        Some('-' | '•' | '*' | '–' | '·') => true,
        Some(c) if c.is_ascii_digit() => matches!(chars.find(|c| !c.is_ascii_digit()), Some('.' | ')')) && t.len() < 80,
        _ => false,
    }
}

/// Text from lines in reading order.
pub fn join_lines(lines: &[OcrLine]) -> String {
    let mut out = String::new();
    let mut previous: Option<&OcrLine> = None;
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(prev) = previous {
            let gap = line.top - (prev.top + prev.height);
            let paragraph = gap > prev.height.max(line.height) * PARAGRAPH_GAP;
            let hyphenated = out.ends_with('-') && out.chars().rev().nth(1).is_some_and(char::is_alphabetic) && text.chars().next().is_some_and(char::is_lowercase);
            if hyphenated {
                out.pop();
            } else if paragraph || ends_sentence(&out) && starts_new_item(text) || starts_new_item(text) {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        out.push_str(text);
        previous = Some(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, top: f32) -> OcrLine {
        OcrLine { text: text.into(), top, height: 20.0 }
    }

    #[test]
    fn lines_of_one_sentence_are_joined() {
        let lines = [line("The weather is lovely today, so we", 0.0), line("are going to the park.", 24.0)];
        assert_eq!(join_lines(&lines), "The weather is lovely today, so we are going to the park.");
    }

    #[test]
    fn a_big_gap_starts_a_new_paragraph() {
        let lines = [line("First paragraph ends here.", 0.0), line("Second one starts here.", 60.0)];
        assert_eq!(join_lines(&lines), "First paragraph ends here.\nSecond one starts here.");
    }

    #[test]
    fn hyphenated_words_are_stitched_together() {
        let lines = [line("Das ist eine Heraus-", 0.0), line("forderung für alle.", 24.0)];
        assert_eq!(join_lines(&lines), "Das ist eine Herausforderung für alle.");
        // A dash before a capital letter or a digit is kept.
        let lines = [line("Baden-", 0.0), line("Württemberg", 24.0)];
        assert_eq!(join_lines(&lines), "Baden- Württemberg");
    }

    #[test]
    fn list_items_stay_on_their_own_lines() {
        let lines = [line("Shopping list", 0.0), line("- milk", 24.0), line("- bread", 48.0), line("1. eggs", 72.0)];
        assert_eq!(join_lines(&lines), "Shopping list\n- milk\n- bread\n1. eggs");
    }

    #[test]
    fn empty_lines_are_skipped_and_nothing_gives_nothing() {
        assert_eq!(join_lines(&[]), "");
        assert_eq!(join_lines(&[line("   ", 0.0), line("", 24.0)]), "");
        assert_eq!(join_lines(&[line("  hello ", 0.0)]), "hello");
    }

    #[test]
    fn a_title_followed_by_text_with_a_normal_gap_is_joined_like_any_line() {
        // Without layout hints we cannot tell a title from a sentence start; the gap decides.
        let lines = [line("Title", 0.0), line("Body text follows.", 24.0)];
        assert_eq!(join_lines(&lines), "Title Body text follows.");
    }
}
