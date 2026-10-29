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

fn code_known(code: &str) -> bool {
    LANGS.iter().any(|(c, _)| *c == code)
}

/// Detect with what we know about the user: the active languages and the last source language.
///
/// * Long text: as [`detect_language`] (all languages; the caller reports "not installed").
/// * Short text: only the active languages plus English are candidates, a short list of common
///   words helps, and a weak guess falls back to `last` (the last detected or chosen source).
///   Short text is never "unsure"; only text without letters is [`Detection::NoText`].
pub fn detect_in_context(text: &str, active: &[String], last: Option<&str>) -> Detection {
    if !text.chars().any(char::is_alphabetic) {
        return Detection::NoText;
    }
    if !is_short(text) {
        return detect_language(text);
    }
    let mut allowed: Vec<String> = active.iter().filter(|c| code_known(c)).cloned().collect();
    if !allowed.iter().any(|c| c == "en") {
        allowed.push("en".into());
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
    fn long_text_keeps_the_full_detection() {
        let text = "Die Sonne scheint heute, und wir gehen am Nachmittag gemeinsam in den Park spazieren.";
        assert_eq!(detect_in_context(text, &active(&["en"]), Some("en")), lang("de"));
        assert!(!is_short(text));
        assert!(is_short("Guten Tag zusammen"));
        assert!(is_short("Aufgabe"));
    }
}
