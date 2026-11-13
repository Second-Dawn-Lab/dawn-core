//! A conservative clean-up of English text before it is sent to a translation engine ("im ali and i work as ..."). Engines
//! read what is there: a typo that is a real word of the target language ("Im" is "in the" in German) is carried through. The
//! user's text on screen is never changed; only what is sent to the engines is.
//!
//! Only English, only whole lowercase words (and a few at the start of a sentence), and never inside links, code, names,
//! abbreviations or glossary terms.

/// Contractions typed without the apostrophe.
const CONTRACTIONS: [(&str, &str); 8] = [
    ("dont", "don't"),
    ("doesnt", "doesn't"),
    ("cant", "can't"),
    ("wont", "won't"),
    ("isnt", "isn't"),
    ("arent", "aren't"),
    ("didnt", "didn't"),
    ("wasnt", "wasn't"),
];

/// Words before which "ur" is "your" and cannot be "you're" ("ur name", never "ur nice").
const POSSESSED: [&str; 40] = [
    "name", "email", "mail", "phone", "address", "house", "home", "job", "work", "team", "friend", "friends", "family", "mom", "dad", "mother", "father", "wife", "husband", "life", "day", "time", "help", "order", "account", "number", "car",
    "company", "project", "idea", "question", "message", "request", "support", "birthday", "age", "country", "city", "school", "office",
];

/// Words before which a lowercase "im" is the start of a self-introduction even when a capitalized word follows ("hi im Ali").
const LEAD_IN: [&str; 9] = ["hi", "hello", "hey", "and", "but", "so", "well", "ok", "okay"];

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '\u{2019}'
}

/// Chunks (between spaces) that look like links, code, addresses, paths, numbers or abbreviations ("u.s.") are left alone.
fn skip_chunk(chunk: &str) -> bool {
    chunk.chars().any(|c| c.is_ascii_digit() || matches!(c, '@' | '/' | '\\' | '#' | '_' | '~' | '`' | '=' | '<' | '>' | '{' | '}' | '[' | ']' | '(' | ')'))
        || chunk.contains("://")
        || chunk.to_lowercase().starts_with("www.")
        || chunk.trim_matches(|c: char| !c.is_alphanumeric()).contains(['.', '-'])
}

struct Word {
    start: usize,
    end: usize,
}

/// Byte ranges of the occurrences of the glossary terms in `text` (ignoring case): words inside them are never changed.
fn protected_ranges(text: &str, terms: &[String]) -> Vec<(usize, usize)> {
    let lower = text.to_lowercase();
    // `to_lowercase` can change the byte length of some characters; positions are only trusted when it did not.
    if lower.len() != text.len() {
        return vec![(0, text.len())];
    }
    let mut out = Vec::new();
    for term in terms.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()) {
        let mut from = 0;
        while let Some(at) = lower[from..].find(&term) {
            let (a, b) = (from + at, from + at + term.len());
            // Whole words only: the term "u" does not protect the "u" inside "just".
            let before_ok = lower[..a].chars().next_back().is_none_or(|c| !c.is_alphabetic());
            let after_ok = lower[b..].chars().next().is_none_or(|c| !c.is_alphabetic());
            if before_ok && after_ok {
                out.push((a, b));
            }
            from = b;
        }
    }
    out
}

/// The English text as it should be sent to an engine. `protect`: glossary terms (their words stay as typed). Text in another
/// language is returned unchanged.
pub fn normalize_source(lang: &str, text: &str, protect: &[String]) -> String {
    if lang != "en" || text.is_empty() {
        return text.to_string();
    }
    let protected = protected_ranges(text, protect);
    let mut words: Vec<Word> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match (is_word_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                words.push(Word { start: s, end: i });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push(Word { start: s, end: text.len() });
    }

    let mut out = String::with_capacity(text.len() + 8);
    let mut at = 0;
    let mut k = 0;
    let mut sentence_start = true;
    let mut prev_word = String::new();
    while k < words.len() {
        let w = &words[k];
        let word = &text[w.start..w.end];
        // Sentence start: only terminal punctuation between the previous word and this one (or the start of the text).
        let gap = &text[words.get(k.wrapping_sub(1)).filter(|_| k > 0).map_or(0, |p| p.end)..w.start];
        if k > 0 {
            sentence_start = gap.contains(['.', '!', '?', '\n', '\u{2026}']);
        }
        let chunk_start = text[..w.start].rfind(char::is_whitespace).map_or(0, |p| p + text[p..].chars().next().map_or(1, char::len_utf8));
        let chunk_end = text[w.end..].find(char::is_whitespace).map_or(text.len(), |e| w.end + e);
        let skip = skip_chunk(&text[chunk_start..chunk_end]) || protected.iter().any(|(a, b)| w.start < *b && w.end > *a);
        let next = words.get(k + 1).filter(|n| text[w.end..n.start].chars().all(|c| c == ' '));
        let next_word = next.map(|n| &text[n.start..n.end]);
        let next_capitalized = next_word.and_then(|n| n.chars().next()).is_some_and(char::is_uppercase);
        let mut replacement: Option<(String, usize)> = None; // (text, number of words consumed)
        if !skip {
            let lower = word.to_lowercase();
            if word == "i" {
                // "i m" (the apostrophe forgotten and a space typed instead) is "I'm"; a lone "i" is "I".
                if next_word == Some("m") && next.is_some_and(|n| !skip_chunk(&text[n.start..text[n.end..].find(char::is_whitespace).map_or(text.len(), |e| n.end + e)])) {
                    replacement = Some(("I'm".into(), 2));
                } else {
                    replacement = Some(("I".into(), 1));
                }
            } else if matches!(word, "i'm" | "i'll" | "i've" | "i'd") {
                replacement = Some((format!("I{}", &word[1..]), 1));
            } else if word == "im" && next_word.is_some() && (!next_capitalized || sentence_start || prev_word.is_empty() || LEAD_IN.contains(&prev_word.as_str())) {
                replacement = Some(("I'm".into(), 1));
            } else if word == "Im" && sentence_start && next_word.is_some() && !next_capitalized {
                replacement = Some(("I'm".into(), 1));
            } else if let Some((_, with)) = CONTRACTIONS.iter().find(|(plain, _)| *plain == lower) {
                // "dont" and "Dont", but not "DONT" (shouting is left alone).
                if word == lower || (word.chars().next().is_some_and(char::is_uppercase) && word[1..] == lower[1..]) {
                    let mut fixed = (*with).to_string();
                    if word != lower {
                        fixed = fixed[..1].to_uppercase() + &fixed[1..];
                    }
                    replacement = Some((fixed, 1));
                }
            } else if word == "u" && words_are_plain(text, w) {
                replacement = Some(("you".into(), 1));
            } else if word == "ur" && next_word.is_some_and(|n| POSSESSED.contains(&n)) {
                replacement = Some(("your".into(), 1));
            }
        }
        match replacement {
            Some((fixed, consumed)) => {
                let last = &words[k + consumed - 1];
                out.push_str(&text[at..w.start]);
                out.push_str(&fixed);
                at = last.end;
                prev_word = fixed.to_lowercase();
                k += consumed;
            }
            None => {
                prev_word = word.to_lowercase();
                k += 1;
            }
        }
    }
    out.push_str(&text[at..]);
    out
}

/// A lone "u": not part of "u-turn", "u.s." or similar (the characters next to it are spaces or sentence punctuation).
fn words_are_plain(text: &str, w: &Word) -> bool {
    let before = text[..w.start].chars().next_back();
    let after = text[w.end..].chars().next();
    let ok = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '"' | '\u{2019}' | ')'));
    // "u." at the end of a text is fine; "u.s" is not (a letter follows the dot).
    ok(before) && ok(after) && !(after == Some('.') && text[w.end + 1..].chars().next().is_some_and(char::is_alphabetic))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(text: &str) -> String {
        normalize_source("en", text, &[])
    }

    #[test]
    fn the_reported_sentences() {
        assert_eq!(n("im ali and i work as softwar devloper"), "I'm ali and I work as softwar devloper");
        assert_eq!(n("hi how are you ? i m softwar devllper"), "hi how are you ? I'm softwar devllper");
        assert_eq!(n("Im ali and i work"), "I'm ali and I work");
    }

    #[test]
    fn im_before_a_name_after_a_greeting_or_at_the_start() {
        assert_eq!(n("hi im Ali"), "hi I'm Ali");
        assert_eq!(n("im Ali"), "I'm Ali");
        assert_eq!(n("ok. im tired"), "ok. I'm tired");
        assert_eq!(n("so im Ali and u?"), "so I'm Ali and you?");
    }

    #[test]
    fn a_lone_i_and_the_i_contractions() {
        assert_eq!(n("i think i can. i'll try, i've done it and i'd go"), "I think I can. I'll try, I've done it and I'd go");
    }

    #[test]
    fn contractions_without_the_apostrophe() {
        assert_eq!(n("i dont know, he doesnt care, we cant, you wont, it isnt, they arent, she didnt, it wasnt"), "I don't know, he doesn't care, we can't, you won't, it isn't, they aren't, she didn't, it wasn't");
        assert_eq!(n("Dont go. Cant wait"), "Don't go. Can't wait");
        assert_eq!(n("DONT SHOUT"), "DONT SHOUT");
    }

    #[test]
    fn u_and_ur() {
        assert_eq!(n("how are u? thank u"), "how are you? thank you");
        assert_eq!(n("what is ur name and ur email"), "what is your name and your email");
        // "ur" is also "you're": left alone unless the next word makes it "your".
        assert_eq!(n("ur nice"), "ur nice");
        assert_eq!(n("ur welcome"), "ur welcome");
        assert_eq!(n("ur going home"), "ur going home");
    }

    #[test]
    fn things_that_must_not_change() {
        for text in [
            "Im Wald",
            "Wir gehen im Wald spazieren",
            "the IM client and an IM abbreviation",
            "Im Chang-min is a surname",
            "we live in the u.s. now",
            "a u-turn at the u.s. border",
            "visit www.im.example or https://example.com/i/u",
            "mail im@example.com about u",
            "code: `i = u` and x[i]",
            "Alice and Bob",
            "I'm already fine, Ali.",
            "es lebt im Wald",
        ] {
            let got = n(text);
            // Only the lone "u" of the mail line is a plain word; everything else is unchanged.
            let expected = if text == "mail im@example.com about u" { "mail im@example.com about you" } else { text };
            assert_eq!(got, expected, "{text}");
        }
    }

    #[test]
    fn german_and_other_languages_are_never_touched() {
        for lang in ["de", "fr", "es", "ur", "ja"] {
            assert_eq!(normalize_source(lang, "Im Wald dont i u ur", &[]), "Im Wald dont i u ur");
        }
    }

    #[test]
    fn glossary_terms_keep_their_words() {
        let protect = vec!["u".to_string(), "i m".to_string(), "dont panic".to_string()];
        assert_eq!(normalize_source("en", "the u ship and i m here and dont panic or i go", &protect), "the u ship and i m here and dont panic or I go");
        // Another word in the same sentence is still fixed.
        assert_eq!(normalize_source("en", "dont panic, i dont", &["dont panic".to_string()]), "dont panic, I don't");
    }

    #[test]
    fn spacing_punctuation_and_lines_are_kept_exactly() {
        assert_eq!(n("i  am\n\ni dont know ,  u?"), "I  am\n\nI don't know ,  you?");
        assert_eq!(n(""), "");
        assert_eq!(n("   "), "   ");
        assert_eq!(n("Ünïcode i ok"), "Ünïcode I ok");
    }

    #[test]
    fn a_glossary_term_protects_whole_words_only() {
        // The term "u" is the word u, not the letter inside "just" or "you".
        assert_eq!(normalize_source("en", "i just u", &["u".to_string()]), "I just u");
        assert_eq!(normalize_source("en", "i just u", &[]), "I just you");
    }

    #[test]
    fn it_is_idempotent() {
        let once = n("im ali and i dont know, how are u? i m fine");
        assert_eq!(n(&once), once);
    }
}
