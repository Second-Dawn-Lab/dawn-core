//! The model's answer is never trusted: it is cleaned (only the text, nothing around it) and checked. A rejected answer
//! means the caller keeps the fast translation.

/// What the answer is checked against.
pub struct Expect<'a> {
    /// The language the answer must be in (`de`, `ja`, ...).
    pub lang: &'a str,
    /// The text the answer replaces (the fast translation, or the text being rewritten); sets the allowed length.
    pub reference: &'a str,
    /// Glossary targets that must be present (as written or inflected).
    pub terms: &'a [String],
    /// Bounds of the answer's length compared to the reference (a "Shorter" rewrite allows much less).
    pub min_ratio: f32,
    pub max_ratio: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reject {
    Empty,
    TooLong,
    TooShort,
    WrongLanguage,
    MissingTerm(String),
    /// A repetition loop, or far longer than the text it answers.
    Degenerate,
    /// A rewrite that must keep the meaning changed it (a noun became singular, content words came or went).
    ChangedMeaning,
}

impl Reject {
    /// A stable code for the UI ("too-long"): what went wrong decides what the message says.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "rejected-empty",
            Self::TooLong => "rejected-too-long",
            Self::TooShort => "rejected-too-short",
            Self::WrongLanguage => "rejected-wrong-language",
            Self::MissingTerm(_) => "rejected-missing-term",
            Self::Degenerate => "rejected-degenerate",
            Self::ChangedMeaning => "rejected-changed-meaning",
        }
    }
}

/// Texts of fewer words than this are short: a correct answer can be a large multiple of them, and statistics cannot
/// tell their language.
pub const SHORT_WORDS: usize = 8;

/// Extra characters an answer may have beyond the ratio, for texts that rewrite to something longer ("i and me go zoo" is
/// 15 characters, "He and I go to the zoo." is 23). About six words.
const EXTRA_CHARS: f32 = 50.0;

const ENGLISH_WORDS: [&str; 34] = [
    "the", "and", "is", "are", "was", "were", "of", "to", "in", "on", "at", "for", "with", "that", "this", "it", "as", "he", "she", "they", "we", "i", "you", "me", "him", "her", "my", "be", "have", "has", "do", "go", "not", "will",
];

/// Does a short text read as English (many of its words are English function words)? Short texts cannot be told apart by
/// statistics, but a sentence that is mostly "the", "and", "to" is obviously English.
fn looks_english(text: &str) -> bool {
    let words: Vec<String> = text.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let hits = words.iter().filter(|w| ENGLISH_WORDS.contains(&w.as_str())).count();
    hits >= 2 && hits * 100 >= words.len() * 30
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

const LABELS: [&str; 11] = ["translation", "translated", "improved", "polished", "revised", "rewritten", "result", "output", "übersetzung", "traduction", "traducción"];

/// Keep only the text: no reasoning block, code fence, label ("Translation:"), quotes around everything, or trailing
/// notes. With `single_paragraph` only the first paragraph is kept (the sentence being polished is one paragraph).
pub fn clean_output(raw: &str, single_paragraph: bool) -> String {
    let mut s = raw.trim().to_string();
    // A reasoning block some models write first.
    if let Some(end) = s.find("</think>") {
        s = s[end + "</think>".len()..].trim().to_string();
    }
    if s.starts_with("```") {
        let inner = s.trim_start_matches('`');
        let inner = inner.split_once('\n').map_or(inner, |(_, rest)| rest);
        s = inner.split("```").next().unwrap_or("").trim().to_string();
    }
    // A label on the first line: "Translation:", "Improved German text:", "Polished:".
    if let Some((label, rest)) = s.split_once(':') {
        let lower = label.to_lowercase();
        let looks_like_label = label.split_whitespace().count() <= 4 && !label.contains(['.', '!', '?', '\n']) && LABELS.iter().any(|k| lower.contains(k));
        if looks_like_label && !rest.trim().is_empty() {
            s = rest.trim().to_string();
        }
    }
    if single_paragraph {
        if let Some((first, _)) = s.split_once("\n\n") {
            s = first.trim().to_string();
        }
    }
    // Notes and explanations after the text.
    let kept: Vec<&str> = s
        .lines()
        .take_while(|l| {
            let t = l.trim_start().to_lowercase();
            !(t.starts_with("note:") || t.starts_with("(note") || t.starts_with("explanation") || t.starts_with("changes:") || t.starts_with("*note"))
        })
        .collect();
    s = kept.join("\n").trim().to_string();
    // Quotes around the whole answer.
    for (open, close) in [('"', '"'), ('“', '”'), ('«', '»'), ('„', '“'), ('「', '」')] {
        let inner_start = open.len_utf8();
        if s.chars().count() > 2 && s.starts_with(open) && s.ends_with(close) && s.len() >= inner_start + close.len_utf8() {
            let inner = &s[inner_start..s.len() - close.len_utf8()];
            if !inner.contains(open) {
                s = inner.trim().to_string();
                break;
            }
        }
    }
    s
}

/// Does the glossary target appear in the answer, as written or inflected ("Eule" in "Eulen", "Eulen" for "Eule")?
/// A short or non-letter term must appear exactly; a longer one may lose its last letter.
pub fn term_present(output: &str, term: &str) -> bool {
    let out = output.to_lowercase();
    let t = term.trim().to_lowercase();
    if t.is_empty() || out.contains(&t) {
        return true;
    }
    let chars: Vec<char> = t.chars().collect();
    if chars.len() < 5 || !chars.iter().all(|c| c.is_alphabetic() || c.is_whitespace() || *c == '-') {
        return false;
    }
    let stem: String = chars[..chars.len() - 1].iter().collect();
    out.contains(&stem)
}

/// 0 to 1: how alike two texts are (edit distance of the lowercased text).
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f32 / longest as f32
}

fn script_share(text: &str, pred: impl Fn(char) -> bool) -> f32 {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return 0.0;
    }
    letters.iter().filter(|c| pred(**c)).count() as f32 / letters.len() as f32
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}

/// Is the text in the language `lang`? Judged by script for non-Latin languages and by a language detector for Latin
/// ones (only a confident "this is another language" is a rejection; short text is not judged).
pub fn is_language(text: &str, lang: &str) -> bool {
    language_ok(text, lang, false)
}

/// `short`: the text is too short for statistics, so a Latin-script answer is accepted for a Latin-script language (the
/// script is still checked, and so is every non-Latin language).
pub fn language_ok(text: &str, lang: &str, short: bool) -> bool {
    match lang {
        "ru" | "uk" | "bg" | "sr" => script_share(text, |c| matches!(c as u32, 0x0400..=0x052F)) > 0.8,
        "zh" => script_share(text, |c| is_cjk(c) && !matches!(c as u32, 0x3040..=0x30FF)) > 0.6,
        "ja" => script_share(text, is_cjk) > 0.6,
        "ko" => script_share(text, |c| matches!(c as u32, 0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F)) > 0.6,
        "ar" | "ur" | "fa" => script_share(text, |c| matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)) > 0.8,
        "hi" | "mr" => script_share(text, |c| matches!(c as u32, 0x0900..=0x097F)) > 0.8,
        "he" => script_share(text, |c| matches!(c as u32, 0x0590..=0x05FF)) > 0.8,
        "el" => script_share(text, |c| matches!(c as u32, 0x0370..=0x03FF)) > 0.8,
        "th" => script_share(text, |c| matches!(c as u32, 0x0E00..=0x0E7F)) > 0.8,
        _ => {
            if script_share(text, |c| c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c)) < 0.85 {
                return false;
            }
            let Some(want) = latin_lang(lang) else { return true };
            if short || text.chars().filter(|c| c.is_alphabetic()).count() < 25 {
                return true;
            }
            match whatlang::detect(text) {
                Some(info) if info.confidence() >= 0.5 && info.lang() != want => false,
                _ => true,
            }
        }
    }
}

fn latin_lang(code: &str) -> Option<whatlang::Lang> {
    use whatlang::Lang::*;
    Some(match code {
        "en" => Eng,
        "de" => Deu,
        "fr" => Fra,
        "es" => Spa,
        "it" => Ita,
        "pt" => Por,
        "nl" => Nld,
        "pl" => Pol,
        "tr" => Tur,
        "cs" => Ces,
        "sv" => Swe,
        "da" => Dan,
        "fi" => Fin,
        "ro" => Ron,
        "hu" => Hun,
        "id" => Ind,
        "vi" => Vie,
        _ => return None,
    })
}

/// Clean and check one answer. `Ok(text)` is safe to show; `Err` means: keep the fast translation.
pub fn check(raw: &str, expect: &Expect, single_paragraph: bool) -> Result<String, Reject> {
    let out = clean_output(raw, single_paragraph);
    if out.trim().is_empty() {
        return Err(Reject::Empty);
    }
    let ref_len = expect.reference.chars().count().max(1) as f32;
    let out_len = out.chars().count() as f32;
    if dawn_core::degenerate::find_degeneration(expect.reference, &out).is_some() {
        return Err(Reject::Degenerate);
    }
    // The longest answer: the ratio, or the text plus a few words, whichever is larger (not for "make it shorter").
    let max_len = if expect.max_ratio >= 1.0 { (ref_len * expect.max_ratio).max(ref_len + EXTRA_CHARS) } else { ref_len * expect.max_ratio };
    if out_len > max_len {
        return Err(Reject::TooLong);
    }
    // A very short text has no meaningful lower bound.
    if ref_len >= 40.0 && out_len < ref_len * expect.min_ratio {
        return Err(Reject::TooShort);
    }
    let short = word_count(expect.reference) < SHORT_WORDS || word_count(&out) < SHORT_WORDS;
    if !language_ok(&out, expect.lang, short) {
        return Err(Reject::WrongLanguage);
    }
    // A short English answer for a non-English text is wrong, unless the text itself was English (its language may have been
    // guessed wrong from a handful of words: the answer is then in the language the text really has).
    if short && expect.lang != "en" && looks_english(&out) && !looks_english(expect.reference) {
        return Err(Reject::WrongLanguage);
    }
    if let Some(missing) = expect.terms.iter().find(|t| !term_present(&out, t)) {
        return Err(Reject::MissingTerm(missing.clone()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect<'a>(lang: &'a str, reference: &'a str, terms: &'a [String]) -> Expect<'a> {
        Expect { lang, reference, terms, min_ratio: 0.5, max_ratio: 2.0 }
    }

    #[test]
    fn only_the_text_is_kept() {
        assert_eq!(clean_output("Translation: Die Eule schläft.", true), "Die Eule schläft.");
        assert_eq!(clean_output("\"Die Eule schläft.\"", true), "Die Eule schläft.");
        assert_eq!(clean_output("```\nDie Eule schläft.\n```", true), "Die Eule schläft.");
        assert_eq!(clean_output("<think>hmm</think>\nDie Eule schläft.", true), "Die Eule schläft.");
        assert_eq!(clean_output("Die Eule schläft.\n\nNote: I used the formal form.", true), "Die Eule schläft.");
        assert_eq!(clean_output("Die Eule schläft.\nNote: formal.", false), "Die Eule schläft.");
        assert_eq!(clean_output("„Die Eule schläft.“", true), "Die Eule schläft.");
        // A colon in the sentence itself is not a label.
        assert_eq!(clean_output("Achtung: die Eule schläft.", true), "Achtung: die Eule schläft.");
    }

    #[test]
    fn paragraphs_are_kept_for_documents() {
        assert_eq!(clean_output("Eins.\n\nZwei.", false), "Eins.\n\nZwei.");
    }

    #[test]
    fn empty_too_long_and_too_short_answers_are_rejected() {
        let e = expect("de", "Die Eule schläft am Tag.", &[]);
        assert_eq!(check("  ", &e, true), Err(Reject::Empty));
        assert_eq!(check("Die Eule schläft am Tag und jagt in der Nacht, weil sie ein nachtaktives Tier ist.", &e, true), Err(Reject::TooLong));
        // A tiny text has no lower bound; a longer one does.
        assert!(check("Eule.", &e, true).is_ok());
        let longer = expect("de", "Die Eule schläft am Tag und jagt in der Nacht im großen Wald hinter dem Haus.", &[]);
        assert_eq!(check("Eule.", &longer, true), Err(Reject::TooShort));
        assert_eq!(check("Die Eule schläft tagsüber.", &e, true), Ok("Die Eule schläft tagsüber.".into()));
    }

    #[test]
    fn an_answer_in_the_wrong_language_is_rejected() {
        let e = expect("de", "Die Eule schläft am Tag und jagt nachts.", &[]);
        assert_eq!(check("The owl sleeps during the day and hunts at night.", &e, true), Err(Reject::WrongLanguage));
        assert!(check("Die Eule schläft am Tag und jagt in der Nacht.", &e, true).is_ok());
        let ru = expect("ru", "Сова спит днём.", &[]);
        assert_eq!(check("Die Eule schläft.", &ru, true), Err(Reject::WrongLanguage));
        assert!(check("Сова спит днём.", &ru, true).is_ok());
        let ja = expect("ja", "フクロウは昼間に眠ります。", &[]);
        assert_eq!(check("The owl sleeps.", &ja, true), Err(Reject::WrongLanguage));
        assert!(check("フクロウは昼は寝ます。", &ja, true).is_ok());
        let zh = expect("zh", "猫头鹰白天睡觉。", &[]);
        assert!(check("猫头鹰白天睡觉。", &zh, true).is_ok());
        let hi = expect("hi", "उल्लू दिन में सोते हैं।", &[]);
        assert!(check("उल्लू दिन में सोता है।", &hi, true).is_ok());
        assert_eq!(check("The owls sleep during the day.", &hi, true), Err(Reject::WrongLanguage));
    }

    #[test]
    fn a_repetition_loop_is_rejected_before_anything_else() {
        let e = expect("de", "Hallo, wie geht's dir? Ich bin Softwareentwickler.", &[]);
        assert_eq!(check(&format!("Hallo, wie geht's dir? {}", "n ".repeat(30)), &e, true), Err(Reject::Degenerate));
        assert_eq!(check("Ich bin bin bin bin bin Softwareentwickler hier.", &e, true), Err(Reject::Degenerate));
        assert!(check("Hallo, wie geht's? Ich bin Softwareentwickler.", &e, true).is_ok());
    }

    #[test]
    fn a_short_text_may_become_much_longer_by_adding_a_few_words() {
        // The reported case: "i and me go zoo" (15 characters) fixed to "He and I go to the zoo." (23).
        let e = Expect { lang: "en", reference: "i and me go zoo", terms: &[], min_ratio: 0.6, max_ratio: 1.5 };
        assert_eq!(check("He and I go to the zoo.", &e, true), Ok("He and I go to the zoo.".into()));
        assert!(check("He and I are going to the big zoo today.", &e, true).is_ok(), "six extra words are fine");
        assert!(check("He and I are going to the big zoo today, together with all of our friends and family.", &e, true).is_err(), "far more than a few words");
        // A long text keeps the ratio.
        let long = "I has a dog and a cat and they plays in the garden every day of the week.";
        let e = Expect { lang: "en", reference: long, terms: &[], min_ratio: 0.6, max_ratio: 1.5 };
        assert_eq!(check(&format!("{long} {long} {long}"), &e, true), Err(Reject::TooLong), "a long text keeps the ratio");
    }

    #[test]
    fn make_it_shorter_never_allows_a_longer_answer_and_tiny_texts_have_no_lower_bound() {
        let shorter = Expect { lang: "en", reference: "I just wanted to quickly let you know that I cannot come.", terms: &[], min_ratio: 0.1, max_ratio: 0.95 };
        assert_eq!(check("I just wanted to quickly let you know that I cannot come today.", &shorter, true), Err(Reject::TooLong));
        let tiny = Expect { lang: "en", reference: "Hi there you", terms: &[], min_ratio: 0.6, max_ratio: 1.5 };
        assert!(check("Hi.", &tiny, true).is_ok());
    }

    #[test]
    fn a_short_answer_is_not_judged_by_the_language_detector_only_by_its_script() {
        // The language of a short text may itself have been guessed wrong (a "German" label on English words).
        let e = Expect { lang: "de", reference: "i and me go zoo", terms: &[], min_ratio: 0.6, max_ratio: 2.0 };
        assert!(check("He and I go to the zoo.", &e, true).is_ok());
        // A clear script difference still counts.
        assert_eq!(check("Он и я идем в зоопарк.", &e, true), Err(Reject::WrongLanguage));
        // A long answer in the wrong language is still found.
        let long = Expect { lang: "de", reference: "Die Eulen schlafen am Tag und jagen nachts im Wald und in der Scheune.", terms: &[], min_ratio: 0.5, max_ratio: 2.0 };
        assert_eq!(check("The owls sleep during the day and hunt at night in the forest and in the barn.", &long, true), Err(Reject::WrongLanguage));
    }

    #[test]
    fn every_rejection_has_its_own_code() {
        let codes: Vec<&str> = [Reject::Empty, Reject::TooLong, Reject::TooShort, Reject::WrongLanguage, Reject::MissingTerm("x".into()), Reject::Degenerate, Reject::ChangedMeaning].iter().map(Reject::code).collect();
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len());
        assert!(codes.iter().all(|c| c.starts_with("rejected-")));
    }

    #[test]
    fn short_text_is_not_judged_by_the_detector() {
        let e = expect("de", "Ja.", &[]);
        assert!(check("Ja!", &e, true).is_ok());
    }

    #[test]
    fn a_dropped_glossary_term_is_rejected_but_an_inflected_one_passes() {
        let terms = vec!["Eule".to_string()];
        let e = expect("de", "Die Eulen schlafen am Tag.", &terms);
        assert!(check("Die Eulen schlafen tagsüber.", &e, true).is_ok(), "plural of the term");
        assert!(check("Die Eule schläft tagsüber.", &e, true).is_ok());
        assert_eq!(check("Die Käuze schlafen tagsüber.", &e, true), Err(Reject::MissingTerm("Eule".into())));
    }

    #[test]
    fn term_matching_rules() {
        assert!(term_present("Wir sahen zwei Eulen.", "Eule"));
        assert!(term_present("Ich sehe die Eule", "eule"));
        assert!(!term_present("Der Hund bellt", "Katze"));
        // Short terms must match exactly: "Kind" is not found in "Kin".
        assert!(!term_present("Das Kin", "Kind"));
        assert!(term_present("Die Kinder", "Kind"));
        assert!(term_present("プロジェクト", "プロジェクト"));
        assert!(!term_present("プロジェ", "プロジェクト"));
    }
}
