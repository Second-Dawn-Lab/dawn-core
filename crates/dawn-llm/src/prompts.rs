//! The fixed prompts of Quality mode. One per task, tested; there is no free-form chat. Every prompt ends the same way:
//! "only output the text", and the answer is checked again by [`crate::validate`].

use crate::tone::{tone_instruction, Tone};

#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub user: String,
    /// Upper bound for the answer, in tokens, from the length of the text.
    pub max_tokens: u32,
}

/// A glossary entry as the model is told about it: the term in the source language, how it stands in the sentence
/// ("owls" for the term "owl") and the required translation.
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub source: String,
    pub found: String,
    pub target: String,
}

impl Term {
    /// A term that stands in the sentence as the glossary lists it.
    pub fn exact(source: &str, target: &str) -> Self {
        Self { source: source.into(), found: source.into(), target: target.into() }
    }
}

/// English name of a language code, for the prompts.
pub fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "pl" => "Polish",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "cs" => "Czech",
        "tr" => "Turkish",
        "ar" => "Arabic",
        "fa" => "Persian",
        "he" => "Hebrew",
        "hi" => "Hindi",
        "ur" => "Urdu",
        "bn" => "Bengali",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        "el" => "Greek",
        "sv" => "Swedish",
        "da" => "Danish",
        "fi" => "Finnish",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        other => other,
    }
}

/// Room for the answer: a few times the text's length (tokens of non-Latin scripts are short), within bounds.
fn answer_budget(text: &str, factor: usize) -> u32 {
    (text.chars().count() * factor).clamp(64, 1024) as u32
}

fn glossary_rule(terms: &[Term], to: &str) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let list: Vec<String> = terms
        .iter()
        .map(|t| {
            if t.found.eq_ignore_ascii_case(&t.source) && t.found == t.source {
                format!("\"{}\" must be translated as \"{}\"", t.source, t.target)
            } else {
                format!("\"{}\" (a form of the glossary term \"{}\") must be translated with \"{}\"", t.found, t.source, t.target)
            }
        })
        .collect();
    format!(
        " Glossary: {}. Use each required {} term, and inflect it correctly for the number, case, gender and article it needs in the sentence (a plural in the source needs a plural).",
        list.join("; "),
        language_name(to)
    )
}

/// Polish one sentence: the fast translation `draft` is rewritten so that it reads naturally. `tone` and the glossary
/// are applied in the same pass.
pub fn polish(from: &str, to: &str, source: &str, draft: &str, tone: Tone, terms: &[Term]) -> Prompt {
    let (f, t) = (language_name(from), language_name(to));
    let tone = tone_instruction(to, tone).map(|s| format!(" {s}")).unwrap_or_default();
    Prompt {
        user: format!(
            "Here is a text in {f} and a draft translation into {t}. Rewrite the draft so that it is accurate, natural and idiomatic {t}. \
             Keep the meaning, names and numbers.{tone}{glossary} Only output the improved {t} text, nothing else.\n\n{f}: {source}\nDraft: {draft}",
            glossary = glossary_rule(terms, to)
        ),
        max_tokens: answer_budget(draft, 3),
    }
}

/// What the writer meant, not what was typed: the model translates the source itself. The source may contain typos, missing
/// words, words run together or speech-recognition errors; the quick machine translation is given only as a hint (and left
/// out when it is known to be broken). Tone and glossary rules are part of the same prompt.
pub fn translate_intended(from: &str, to: &str, source: &str, hint: Option<&str>, tone: Tone, terms: &[Term]) -> Prompt {
    let (f, t) = (language_name(from), language_name(to));
    let tone = tone_instruction(to, tone).map(|s| format!(" {s}")).unwrap_or_default();
    let hint_line = hint.map(|h| format!("\n\nQuick machine translation (a hint only; it may be wrong where the {f} text is garbled): {h}")).unwrap_or_default();
    Prompt {
        user: format!(
            "Translate the following {f} text into {t}. The text may contain typos, missing words, words run together or speech-recognition errors: \
             translate the meaning the writer intended, naturally and correctly, and do not add information that is not in the text. \
             A typo in the {f} text is not a word of {t}, even when it looks like one: never copy it through \
             (for example, English \"im Ali\" means \"I'm Ali\", so the German is \"Ich bin Ali\", not \"Im Ali\"). \
             Keep names and numbers.{tone}{glossary} Only output the {t} translation, nothing else.\n\n{f}: {source}{hint_line}",
            glossary = glossary_rule(terms, to)
        ),
        max_tokens: answer_budget(source, 4),
    }
}

/// A new translation of one sentence with tone and glossary, used when there is no usable draft.
pub fn translate(from: &str, to: &str, source: &str, tone: Tone, terms: &[Term]) -> Prompt {
    let (f, t) = (language_name(from), language_name(to));
    let tone = tone_instruction(to, tone).map(|s| format!(" {s}")).unwrap_or_default();
    Prompt {
        user: format!(
            "Translate the following {f} text into {t}.{tone}{glossary} Only output the {t} translation, nothing else.\n\n{source}",
            glossary = glossary_rule(terms, to)
        ),
        max_tokens: answer_budget(source, 4),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteKind {
    Improve,
    Shorter,
    MoreFormal,
    MoreCasual,
    FixGrammar,
}

impl WriteKind {
    fn instruction(self) -> &'static str {
        match self {
            Self::Improve => "Improve the text: make it clearer and more fluent, and fix mistakes, without changing its meaning",
            Self::Shorter => "Make the text shorter and more concise without losing its main information",
            Self::MoreFormal => "Rewrite the text in a more formal, professional register without changing its meaning",
            Self::MoreCasual => "Rewrite the text in a more casual, friendly register without changing its meaning",
            Self::FixGrammar => "Fix every grammar, spelling and punctuation mistake: the case of pronouns (\"me and him went\" becomes \"he and I went\", \"between you and I\" becomes \"between you and me\"), subject-verb agreement, plurals, verb forms and tenses, articles and prepositions. Make the smallest change that fixes each mistake: never change a noun, a number, a name, the tense or the content, and do not add or remove information (\"The owls is sleeping\" becomes \"The owls are sleeping\", not \"The owl is sleeping\"). Do not change the wording or the style otherwise",
        }
    }

    /// The shortest and longest acceptable answer compared to the text.
    pub fn length_bounds(self) -> (f32, f32) {
        match self {
            Self::Shorter => (0.1, 0.95),
            Self::FixGrammar => (0.6, 1.5),
            _ => (0.3, 2.0),
        }
    }
}

/// The Write tab: rewrite `text` in its own language `lang`.
pub fn write(kind: WriteKind, lang: &str, text: &str) -> Prompt {
    let l = language_name(lang);
    Prompt {
        user: format!(
            "{}. The text is in {l}; the answer must be in {l} too. Keep line breaks. Only output the revised text, nothing else.\n\n{text}",
            kind.instruction()
        ),
        max_tokens: answer_budget(text, 3),
    }
}

/// The second try of "Fix grammar" after an answer that changed the meaning: even stricter.
pub fn write_strict_fix(lang: &str, text: &str) -> Prompt {
    let l = language_name(lang);
    Prompt {
        user: format!(
            "Correct only the grammar mistakes of this {l} text. Change as few words as possible. Never change a noun, its singular or plural, a name, a number, the tense or any content word; only fix verb agreement and verb forms, the case of pronouns, spelling and punctuation. \
             Keep line breaks. The answer must be in {l}. Only output the corrected text, nothing else.\n\n{text}"
        ),
        max_tokens: answer_budget(text, 3),
    }
}

/// Other words for one position of a translation (the click-a-word list). The model sees the source sentence and the
/// translation, and lists replacements for the word only.
pub fn alternatives(from: &str, to: &str, source: &str, sentence: &str, word: &str, count: usize) -> Prompt {
    let (f, t) = (language_name(from), language_name(to));
    Prompt {
        user: format!(
            "{f} sentence: {source}\n{t} translation: {sentence}\n\nGive {count} different {t} words or short phrases that could replace \"{word}\" in the translation \
             and keep the sentence correct and natural. They must fit the grammar of the sentence. One per line, no numbers, no explanations."
        ),
        max_tokens: 96,
    }
}

/// Other whole-sentence translations, in the same tone and with the same glossary rules as a polish.
pub fn sentence_alternatives(from: &str, to: &str, source: &str, tone: Tone, terms: &[Term], count: usize) -> Prompt {
    let (f, t) = (language_name(from), language_name(to));
    let tone = tone_instruction(to, tone).map(|s| format!(" {s}")).unwrap_or_default();
    Prompt {
        user: format!(
            "Give {count} different, correct and natural translations of the following {f} text into {t}. Fix any typos of the {f} text silently and translate every word; keep names and numbers.{tone}{glossary} \
             Put each translation on its own line, without numbers, quotes or explanations.\n\n{source}",
            glossary = glossary_rule(terms, to)
        ),
        max_tokens: answer_budget(source, 4) * count as u32,
    }
}

/// The lines of an answer to [`sentence_alternatives`]: bullets, numbers and quotes removed, empty lines dropped.
pub fn parse_sentences(answer: &str) -> Vec<String> {
    answer
        .lines()
        .map(|line| {
            let s = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | '\u{2022}' | ' '));
            s.trim().trim_matches(|c: char| matches!(c, '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{ab}' | '\u{bb}' | '`')).trim().to_string()
        })
        .filter(|s| s.chars().count() >= 3)
        .collect()
}

/// The lines of an answer to [`alternatives`], cleaned: no bullets or numbers, no quotes, no duplicates, not the word itself.
pub fn parse_alternatives(answer: &str, word: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in answer.lines() {
        let mut s = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | '*' | '•' | ' ')).trim();
        s = s.trim_matches(|c: char| matches!(c, '"' | '“' | '”' | '„' | '«' | '»' | '`'));
        let s = s.trim_end_matches(['.', ',', ';']).trim();
        if s.is_empty() || s.chars().count() > 40 || s.to_lowercase() == word.to_lowercase() || out.iter().any(|o| o.to_lowercase() == s.to_lowercase()) {
            continue;
        }
        out.push(s.to_string());
        if out.len() == max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> Vec<Term> {
        vec![Term::exact("owl", "Eule")]
    }

    #[test]
    fn the_polish_prompt_names_languages_tone_and_glossary_and_asks_for_only_the_text() {
        let p = polish("en", "de", "The owls sleep.", "Die Eule schlafen.", Tone::Formal, &terms());
        assert!(p.user.contains("English") && p.user.contains("German"));
        assert!(p.user.contains("Sie"), "formal German names Sie");
        assert!(p.user.contains("\"owl\" must be translated as \"Eule\""));
        assert!(p.user.contains("inflect it correctly"));
        let plural = polish("en", "de", "The owls sleep.", "Die Eule schlafen.", Tone::Automatic, &[Term { source: "owl".into(), found: "owls".into(), target: "Eule".into() }]);
        assert!(plural.user.contains("\"owls\" (a form of the glossary term \"owl\") must be translated with \"Eule\""));
        assert!(plural.user.contains("a plural in the source needs a plural"));
        assert!(p.user.contains("Only output the improved German text, nothing else."));
        assert!(p.user.ends_with("English: The owls sleep.\nDraft: Die Eule schlafen."));
    }

    #[test]
    fn the_intended_meaning_prompt_tells_the_model_the_text_may_be_garbled_and_the_hint_is_only_a_hint() {
        let p = translate_intended("en", "de", "hi how are you ? i m softwar devllper", Some("Hallo, wie geht's dir? iem Weichwarn-Devllper"), Tone::Automatic, &[]);
        assert!(p.user.contains("typos") && p.user.contains("speech-recognition errors") && p.user.contains("intended"));
        assert!(p.user.contains("do not add information"));
        assert!(p.user.contains("never copy it through") && p.user.contains("Ich bin Ali"));
        assert!(p.user.contains("English: hi how are you ? i m softwar devllper"));
        assert!(p.user.contains("a hint only") && p.user.contains("iem Weichwarn-Devllper"));
        assert!(p.user.contains("Only output the German translation, nothing else."));
        // Without a usable hint the line is not there at all.
        let none = translate_intended("en", "de", "im ali and i work as softwar devloper", None, Tone::Formal, &[Term::exact("ali", "Ali")]);
        assert!(!none.user.contains("Quick machine translation") && none.user.contains("Sie") && none.user.contains("\"ali\" must be translated as \"Ali\""));
    }

    #[test]
    fn automatic_tone_and_no_glossary_add_nothing() {
        let p = polish("en", "de", "Hi.", "Hallo.", Tone::Automatic, &[]);
        assert!(!p.user.contains("address") && !p.user.contains("Glossary"));
    }

    #[test]
    fn tone_is_ignored_for_a_language_without_it() {
        let p = polish("de", "en", "Hallo.", "Hello.", Tone::Formal, &[]);
        assert!(!p.user.contains("formal register"));
    }

    #[test]
    fn the_answer_budget_is_bounded() {
        assert_eq!(polish("en", "de", "a", "b", Tone::Automatic, &[]).max_tokens, 64);
        assert_eq!(polish("en", "de", "a", &"x".repeat(5000), Tone::Automatic, &[]).max_tokens, 1024);
    }

    #[test]
    fn write_prompts_keep_the_language_and_ask_for_only_the_text() {
        for kind in [WriteKind::Improve, WriteKind::Shorter, WriteKind::MoreFormal, WriteKind::MoreCasual, WriteKind::FixGrammar] {
            let p = write(kind, "de", "Ich habe ein Buch gelesen.");
            assert!(p.user.contains("German") && p.user.contains("Only output the revised text"));
            let (lo, hi) = kind.length_bounds();
            assert!(lo < hi);
        }
        let fix = write(WriteKind::FixGrammar, "en", "x").user;
        assert!(fix.contains("the case of pronouns") && fix.contains("he and I") && fix.contains("subject-verb agreement") && fix.contains("plurals") && fix.contains("tenses"));
    }

    #[test]
    fn sentence_alternatives_are_requested_with_tone_and_glossary_and_parsed_from_lines() {
        let p = sentence_alternatives("en", "de", "The owls sleep.", Tone::Informal, &[Term { source: "owl".into(), found: "owls".into(), target: "Eule".into() }], 3);
        assert!(p.user.contains("3 different") && p.user.contains("Eule") && p.user.contains("Tone:"));
        assert_eq!(parse_sentences("1. Eins zwei\n- \"Drei vier\"\n\n* ok\n"), ["Eins zwei", "Drei vier"]);
    }

    #[test]
    fn alternatives_are_parsed_from_lines() {
        let list = parse_alternatives("1. überrascht\n- \"verblüfft\"\n* erstaunt\n\nentsetzt.\nÜberrascht\n", "erstaunt", 5);
        assert_eq!(list, vec!["überrascht", "verblüfft", "entsetzt"]);
        assert_eq!(parse_alternatives("a\nb\nc", "x", 2), vec!["a", "b"]);
        assert!(parse_alternatives("this is a very long explanation that is not a word at all really", "x", 5).is_empty());
    }
}
