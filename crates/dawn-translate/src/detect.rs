//! Language detection with `whatlang` (statistical, tiny, no model download).
//!
//! Detection looks at every language it knows, not only the installed ones, so the app can say
//! "German detected, not installed" instead of guessing wrongly. Weak guesses are "unsure".

use whatlang::{Detector, Lang};

/// ISO 639-1 code <-> whatlang language.
const LANGS: &[(&str, Lang)] = &[
    ("en", Lang::Eng),
    ("de", Lang::Deu),
    ("ur", Lang::Urd),
    ("fr", Lang::Fra),
    ("es", Lang::Spa),
    ("it", Lang::Ita),
    ("tr", Lang::Tur),
    ("ar", Lang::Ara),
    ("ru", Lang::Rus),
    ("zh", Lang::Cmn),
    ("ja", Lang::Jpn),
    ("pt", Lang::Por),
    ("nl", Lang::Nld),
    ("pl", Lang::Pol),
    ("hi", Lang::Hin),
    ("fa", Lang::Pes),
    ("ko", Lang::Kor),
    ("sv", Lang::Swe),
    ("uk", Lang::Ukr),
    ("cs", Lang::Ces),
    ("el", Lang::Ell),
    ("he", Lang::Heb),
    ("vi", Lang::Vie),
    ("id", Lang::Ind),
    ("bn", Lang::Ben),
    ("pa", Lang::Pan),
];

/// Below this (and not flagged reliable) the guess is not trusted.
const MIN_CONFIDENCE: f64 = 0.5;

fn code_of(lang: Lang) -> Option<&'static str> {
    LANGS.iter().find(|(_, l)| *l == lang).map(|(c, _)| *c)
}

/// What the detector made of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// A language (ISO 639-1 code), whether or not it is installed.
    Language(String),
    /// Letters were found but the guess is not trustworthy (short or mixed text).
    Unsure,
    /// No letters at all.
    NoText,
}

/// Guess the language of `text` among every language the detector knows.
///
/// Detection does not depend on what is installed: a German text is German even without the
/// German pack, so the app can offer to download it. A weak guess is reported as
/// [`Detection::Unsure`] instead of being replaced by a default, so the user can pick the language.
pub fn detect_language(text: &str) -> Detection {
    if !text.chars().any(char::is_alphabetic) {
        return Detection::NoText;
    }
    let langs: Vec<Lang> = LANGS.iter().map(|(_, l)| *l).collect();
    match Detector::with_allowlist(langs).detect(text) {
        Some(info) if info.is_reliable() || info.confidence() >= MIN_CONFIDENCE => {
            code_of(info.lang()).map_or(Detection::Unsure, |c| Detection::Language(c.to_string()))
        }
        _ => Detection::Unsure,
    }
}

/// Texts with fewer words or characters than this are "short": statistics alone are unreliable.
const SHORT_WORDS: usize = 4;
const SHORT_CHARS: usize = 25;
/// Confidence needed to trust a guess on short text.
const SHORT_MIN_CONFIDENCE: f64 = 0.2;

/// Very common short words that the statistical detector gets wrong ("Hallo" looks English to it).
/// A stopgap for short text until the detector question is settled (see docs/detection.md).
/// Several languages can share a word; the first one that is active wins.
const COMMON_WORDS: &[(&str, &[&str])] = &[
    ("hallo", &["de", "nl"]),
    ("danke", &["de"]),
    ("bitte", &["de"]),
    ("tschüss", &["de"]),
    ("ja", &["de"]),
    ("nein", &["de"]),
    ("guten", &["de"]),
    ("tag", &["de"]),
    ("hello", &["en"]),
    ("hi", &["en"]),
    ("thanks", &["en"]),
    ("thank", &["en"]),
    ("please", &["en"]),
    ("yes", &["en"]),
    ("bye", &["en"]),
    ("merci", &["fr"]),
    ("bonjour", &["fr"]),
    ("salut", &["fr"]),
    ("oui", &["fr"]),
    ("non", &["fr", "it"]),
    ("hola", &["es"]),
    ("gracias", &["es"]),
    ("adiós", &["es"]),
    ("sí", &["es"]),
    ("ciao", &["it"]),
    ("grazie", &["it"]),
    ("prego", &["it"]),
    ("buongiorno", &["it"]),
    ("obrigado", &["pt"]),
    ("olá", &["pt"]),
    ("merhaba", &["tr"]),
    ("teşekkürler", &["tr"]),
    ("dank", &["nl"]),
];

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Short means: fewer than ~4 words or ~25 characters.
pub fn is_short(text: &str) -> bool {
    word_count(text) < SHORT_WORDS || text.trim().chars().count() < SHORT_CHARS
}

/// Language of a short text from the common-word list, among the allowed codes.
fn from_common_words(text: &str, allowed: &[String]) -> Option<String> {
    let mut votes: Vec<(String, usize)> = Vec::new();
    for word in text.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        let word = word.to_lowercase();
        let Some((_, langs)) = COMMON_WORDS.iter().find(|(w, _)| *w == word) else { continue };
        if let Some(lang) = langs.iter().find(|l| allowed.iter().any(|a| a == **l)) {
            match votes.iter_mut().find(|(l, _)| l == lang) {
                Some(v) => v.1 += 1,
                None => votes.push((lang.to_string(), 1)),
            }
        }
    }
    votes.sort_by_key(|v| std::cmp::Reverse(v.1));
    match votes.as_slice() {
        [(lang, _)] => Some(lang.clone()),
        [(lang, n), (_, m), ..] if n > m => Some(lang.clone()),
        _ => None,
    }
}

/// Text as the detector should see it. OCR output is full of symbols, bullets, stray bars and digits
/// that carry no language but disturb the letter statistics, so everything that is not a letter,
/// an apostrophe or a hyphen becomes a space, and runs of whitespace (line breaks too) collapse.
fn clean_for_detection(text: &str) -> String {
    let spaced: String = text.chars().map(|c| if c.is_alphabetic() || matches!(c, '\'' | '\u{2019}' | '-') { c } else { ' ' }).collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Frequent function words that belong (almost) only to one language. The statistical detector
/// mistakes plain English such as "Free software under the MIT license." for German, but any text
/// with "the" and "under" is English. Words that exist in several languages are left out.
const STOP_WORDS: &[(&str, &[&str])] = &[
    ("en", &["the", "and", "of", "to", "is", "are", "you", "your", "this", "that", "with", "for", "under", "what", "how", "from", "have", "not", "can", "will", "never", "was", "were", "our"]),
    ("de", &["der", "die", "das", "und", "ist", "nicht", "ein", "eine", "ich", "wir", "mit", "für", "auf", "zu", "den", "dem", "sie", "auch", "nur", "wie"]),
    ("fr", &["le", "la", "les", "des", "est", "une", "pas", "pour", "vous", "nous", "dans", "avec", "sur", "je", "mais"]),
    ("es", &["el", "los", "las", "una", "es", "por", "para", "con", "del", "está", "pero", "como", "yo"]),
    ("it", &["il", "che", "non", "per", "sono", "della", "gli", "questo", "ma", "anche"]),
    ("pt", &["os", "uma", "não", "você", "está", "mais", "são"]),
    ("nl", &["het", "een", "niet", "van", "ik", "je", "zijn", "voor", "maar"]),
];

/// The language of `allowed` whose function words occur most, if one clearly leads.
fn from_stop_words(text: &str, allowed: &[String]) -> Option<String> {
    let mut votes: Vec<(&str, usize)> = Vec::new();
    for (code, words) in STOP_WORDS {
        if !allowed.iter().any(|a| a == code) {
            continue;
        }
        let n = text.split_whitespace().filter(|w| words.contains(&w.to_lowercase().as_str())).count();
        votes.push((code, n));
    }
    votes.sort_by_key(|v| std::cmp::Reverse(v.1));
    match votes.as_slice() {
        [(code, n)] if *n > 0 => Some(code.to_string()),
        [(code, n), (_, m), ..] if n > m => Some(code.to_string()),
        _ => None,
    }
}

/// The best guess among `allowed` (the active languages plus English), when it can be trusted.
fn restricted_guess(text: &str, allowed: &[String]) -> Option<String> {
    if let Some(code) = from_stop_words(text, allowed) {
        return Some(code);
    }
    let langs: Vec<Lang> = allowed.iter().filter_map(|c| LANGS.iter().find(|(k, _)| k == c).map(|(_, l)| *l)).collect();
    let info = Detector::with_allowlist(langs).detect(text)?;
    let trusted = info.is_reliable() || info.confidence() >= SHORT_MIN_CONFIDENCE || info.script() != whatlang::Script::Latin;
    if trusted {
        code_of(info.lang()).map(str::to_string)
    } else {
        None
    }
}

fn code_known(code: &str) -> bool {
    LANGS.iter().any(|(c, _)| *c == code)
}

/// Detect with what we know about the user: the active languages and the last source language.
///
/// The text is cleaned first (symbols, digits and line breaks from OCR or copied web pages).
///
/// * Long text: as [`detect_language`] (all languages; the caller reports "not installed"). When
///   that is unsure, the active languages plus English get a second chance: a plain English
///   sentence often stays below the all-language confidence bar.
/// * Short text: only the active languages plus English are candidates, a short list of common
///   words helps, and a weak guess falls back to `last` (the last detected or chosen source).
///   Short text is never "unsure"; only text without letters is [`Detection::NoText`].
pub fn detect_in_context(text: &str, active: &[String], last: Option<&str>) -> Detection {
    if !text.chars().any(char::is_alphabetic) {
        return Detection::NoText;
    }
    let cleaned = clean_for_detection(text);
    let text = cleaned.as_str();
    let mut allowed: Vec<String> = active.iter().filter(|c| code_known(c)).cloned().collect();
    if !allowed.iter().any(|c| c == "en") {
        allowed.push("en".into());
    }
    if !is_short(text) {
        return match detect_language(text) {
            Detection::Unsure => restricted_guess(text, &allowed).map_or(Detection::Unsure, Detection::Language),
            found => found,
        };
    }
    if allowed.len() == 1 {
        return Detection::Language(allowed.remove(0));
    }
    if let Some(code) = from_common_words(text, &allowed) {
        return Detection::Language(code);
    }
    let langs: Vec<Lang> = allowed.iter().filter_map(|c| LANGS.iter().find(|(k, _)| k == c).map(|(_, l)| *l)).collect();
    let info = Detector::with_allowlist(langs).detect(text);
    if let Some(info) = &info {
        let trusted = info.is_reliable() || info.confidence() >= SHORT_MIN_CONFIDENCE || info.script() != whatlang::Script::Latin;
        if let (true, Some(code)) = (trusted, code_of(info.lang())) {
            return Detection::Language(code.to_string());
        }
    }
    // A weak guess: stay with the language the user was just working in.
    if let Some(last) = last.filter(|l| allowed.iter().any(|a| a == l)) {
        return Detection::Language(last.to_string());
    }
    Detection::Language(info.and_then(|i| code_of(i.lang())).unwrap_or("en").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(code: &str) -> Detection {
        Detection::Language(code.to_string())
    }

    #[test]
    fn detects_without_looking_at_what_is_installed() {
        assert_eq!(detect_language("Hello, how are you today? I hope everything is fine."), lang("en"));
        assert_eq!(detect_language("Hallo, wie geht es dir heute? Ich hoffe, alles ist in Ordnung."), lang("de"));
        assert_eq!(detect_language("Bonjour, comment allez-vous aujourd'hui ? J'espère que tout va bien."), lang("fr"));
        assert_eq!(detect_language("آپ کا کیا حال ہے؟ مجھے امید ہے کہ سب ٹھیک ہے۔"), lang("ur"));
    }

    #[test]
    fn german_is_not_mistaken_for_english() {
        // The bug this guards against: with only English known, German came back as English.
        let text = "Die Sonne scheint heute, und wir gehen am Nachmittag gemeinsam in den Park spazieren.";
        assert_eq!(detect_language(text), lang("de"));
    }

    #[test]
    fn weak_guesses_are_reported_as_unsure() {
        for text in ["ok", "a", "x1", "OK?!"] {
            assert_eq!(detect_language(text), Detection::Unsure, "{text:?}");
        }
    }

    #[test]
    fn no_letters_means_no_text() {
        assert_eq!(detect_language("  123 !?"), Detection::NoText);
        assert_eq!(detect_language(""), Detection::NoText);
    }

    #[test]
    fn every_known_language_round_trips() {
        for (code, l) in LANGS {
            assert_eq!(code_of(*l), Some(*code));
        }
    }

    fn active(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn short_words_use_the_active_languages() {
        let de = active(&["en", "de"]);
        for word in ["Hallo", "Aufgabe", "Danke"] {
            assert_eq!(detect_in_context(word, &de, Some("en")), lang("de"), "{word}");
        }
        assert_eq!(detect_in_context("Merci", &active(&["en", "de", "fr"]), None), lang("fr"));
        assert_eq!(detect_in_context("hola", &active(&["en", "es", "it"]), None), lang("es"));
        assert_eq!(detect_in_context("hello", &de, Some("de")), lang("en"));
    }

    #[test]
    fn a_word_in_an_inactive_language_does_not_win() {
        let found = detect_in_context("Merci", &active(&["en", "de"]), Some("de"));
        assert!(found == lang("de") || found == lang("en"));
        assert_eq!(detect_in_context("hola", &active(&["en", "de"]), Some("de")), lang("de"));
    }

    #[test]
    fn a_weak_short_guess_keeps_the_last_language() {
        let set = active(&["en", "de"]);
        assert_eq!(detect_in_context("ok", &set, Some("de")), lang("de"));
        assert_eq!(detect_in_context("ok", &set, Some("en")), lang("en"));
        assert!(matches!(detect_in_context("ok", &set, Some("fr")), Detection::Language(_)));
    }

    #[test]
    fn short_text_is_never_unsure() {
        for text in ["ok", "a", "x1", "OK?!", "Hallo Welt", "Danke schön"] {
            assert!(matches!(detect_in_context(text, &active(&["en", "de"]), None), Detection::Language(_)), "{text:?}");
        }
        assert_eq!(detect_in_context("123 ?!", &active(&["en", "de"]), None), Detection::NoText);
    }

    #[test]
    fn other_scripts_are_recognized_among_the_active_languages() {
        assert_eq!(detect_in_context("شکریہ", &active(&["en", "ur"]), Some("en")), lang("ur"));
        assert_eq!(detect_in_context("شكرا", &active(&["en", "ar"]), Some("en")), lang("ar"));
    }

    #[test]
    fn plain_english_sentences_are_not_unsure() {
        // These stay below the detector's confidence bar when all 26 languages compete.
        for text in [
            "Hello, how are you today?",
            "The quick brown fox jumps over the lazy dog.",
            "Your text never leaves this computer.",
            "Free software under the MIT license.",
            "Language packs download only what you need",
            "I can't wait to see you again soon",
        ] {
            assert_eq!(detect_in_context(text, &active(&["en", "de"]), Some("de")), lang("en"), "{text:?}");
            assert_eq!(detect_in_context(text, &active(&["en"]), None), lang("en"), "{text:?} with only English");
        }
    }

    /// What Windows OCR hands over for a clear English paragraph: a line per line, bullets, a stray bar.
    #[test]
    fn an_english_ocr_result_with_noise_is_detected() {
        let ocr = "\u{2022} Your text never leaves\nthis computer. |\n\n  \u{00a9} 2026   -  Free software under\nthe MIT license.  12";
        assert_eq!(detect_in_context(ocr, &active(&["en", "de"]), Some("de")), lang("en"));
        assert_eq!(detect_in_context(ocr, &[], None), lang("en"));
    }

    #[test]
    fn cleaning_keeps_letters_and_drops_symbols_digits_and_line_breaks() {
        assert_eq!(clean_for_detection("\u{2022} Don't   stop |\n 3 well-known \u{00a9}"), "Don't stop well-known");
        assert_eq!(clean_for_detection("123 ?!"), "");
    }

    #[test]
    fn german_text_is_still_german_when_only_english_is_installed() {
        let text = "Wir gehen morgen früh zusammen in den Park.";
        assert_eq!(detect_in_context(text, &active(&["en"]), Some("en")), lang("de"));
        let ocr = "\u{2022} Die Sonne scheint heute,\nund wir gehen am Nachmittag\ngemeinsam in den Park. |";
        assert_eq!(detect_in_context(ocr, &active(&["en"]), Some("en")), lang("de"));
    }

    #[test]
    fn long_text_keeps_the_full_detection() {
        let text = "Die Sonne scheint heute, und wir gehen am Nachmittag gemeinsam in den Park spazieren.";
        assert_eq!(detect_in_context(text, &active(&["en"]), Some("en")), lang("de"));
        assert!(!is_short(text));
        assert!(is_short("Guten Tag zusammen"));
        assert!(is_short("Aufgabe"));
    }
}
