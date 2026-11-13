//! The tasks of Quality mode: a fixed prompt, one request, and the answer checked. A rejected or failed answer is an
//! `Err`; the caller keeps what it had (the fast translation, the user's own text).

use std::sync::atomic::AtomicBool;

use crate::error::{LlmError, Result};
use crate::prompts::{self, Prompt, Term, WriteKind};
use crate::service::{Config, Llm};
use crate::tone::Tone;
use crate::tone_check;
use crate::validate::{self, Expect};

pub struct PolishRequest<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub source: &'a str,
    /// The fast translation.
    pub draft: &'a str,
    /// The fast engine could not translate this sentence cleanly (it looped): it is not shown to the model.
    pub draft_unclean: bool,
    pub tone: Tone,
    /// Glossary entries whose source term occurs in `source`.
    pub terms: &'a [Term],
}

/// Rewrite the fast translation of one sentence so it reads naturally, in the chosen tone, with the glossary terms
/// inflected. Every glossary target must still be in the answer.
///
/// The tone is applied only when the text addresses someone as "you" ([`tone_check::tone_applies`]), and the answer is
/// compared with the fast translation ([`tone_check::check_tone_change`]): when the tone changed who speaks, the kind of
/// sentence or the content, the answer is dropped and the sentence is polished once more without a tone.
pub fn polish(llm: &Llm, cfg: &Config, req: &PolishRequest, cancel: &AtomicBool) -> Result<String> {
    let tone = effective_tone(req);
    let hint = usable_hint(req);
    if tone != Tone::Automatic {
        let prompt = prompts::translate_intended(req.from, req.to, req.source, hint, tone, req.terms);
        if let Ok(out) = check_polish(&llm.complete(cfg, &prompt, cancel)?, req) {
            // The tone answer is compared with the fast translation when that is trustworthy, else only by sentence type.
            let kept = match hint {
                Some(draft) => tone_check::check_tone_change(req.to, draft, &out),
                None => tone_check::check_tone_light(req.source, &out),
            };
            if kept.is_ok() {
                return Ok(out);
            }
        }
    }
    let prompt = prompts::translate_intended(req.from, req.to, req.source, hint, Tone::Automatic, req.terms);
    check_polish(&llm.complete(cfg, &prompt, cancel)?, req)
}

/// The fast translation as a hint for the model: left out when the engine could not translate the sentence cleanly, or when
/// it is mostly the source's own words carried over (what a model does with typos and unknown words: "im ali und ich arbeite
/// als Softwar-Devloper"), since such a hint only misleads.
pub fn usable_hint<'a>(req: &PolishRequest<'a>) -> Option<&'a str> {
    let d = req.draft.trim();
    if req.draft_unclean || d.is_empty() || crate::validate::similarity(d, req.source) >= 0.75 {
        return None;
    }
    let source_words = crate::tone_check::words(req.source);
    let draft_words: Vec<String> = crate::tone_check::words(d).into_iter().filter(|w| w.chars().count() >= 3).collect();
    let copied = draft_words.iter().filter(|w| source_words.iter().any(|s| s.chars().count() >= 3 && crate::validate::similarity(w, s) >= 0.75)).count();
    (draft_words.is_empty() || copied * 10 < draft_words.len() * 3).then_some(req.draft)
}

/// The tone that is really used for this sentence: none when nobody is addressed.
pub fn effective_tone(req: &PolishRequest) -> Tone {
    if req.tone != Tone::Automatic && tone_check::tone_applies(req.from, req.to, req.source, req.draft) {
        req.tone
    } else {
        Tone::Automatic
    }
}

/// The checks after the model answered (separate so they can be tested without a model).
pub fn check_polish(raw: &str, req: &PolishRequest) -> Result<String> {
    let targets: Vec<String> = req.terms.iter().map(|t| t.target.clone()).collect();
    // The length of the answer is judged against the fast translation, or against the source when that is not usable.
    let reference = usable_hint(req).unwrap_or(req.source);
    let expect = Expect { lang: req.to, reference, terms: &targets, min_ratio: 0.4, max_ratio: 2.2 };
    validate::check(raw, &expect, true).map_err(LlmError::Rejected)
}

/// Up to `count` other good translations of one sentence, in the same tone and with the same glossary rules as the
/// polished one. Each is checked like a polish; ones that are the current text, copies of the source, or that break the
/// tone rules are left out.
pub fn sentence_alternatives(llm: &Llm, cfg: &Config, req: &PolishRequest, current: &str, count: usize, cancel: &AtomicBool) -> Result<Vec<String>> {
    let tone = effective_tone(req);
    let prompt = prompts::sentence_alternatives(req.from, req.to, req.source, tone, req.terms, count + 1);
    let answer = llm.complete(cfg, &prompt, cancel)?;
    Ok(filter_sentence_alternatives(&answer, req, tone, current, count))
}

/// The usable lines of an answer to [`prompts::sentence_alternatives`].
pub fn filter_sentence_alternatives(answer: &str, req: &PolishRequest, tone: Tone, current: &str, count: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in prompts::parse_sentences(answer) {
        let Ok(line) = check_polish(&line, req) else { continue };
        if line.to_lowercase() == current.to_lowercase() || out.iter().any(|o| o.to_lowercase() == line.to_lowercase()) {
            continue;
        }
        // A copy of the source (untranslated words, typos kept) is not a translation.
        if crate::validate::similarity(&line, req.source) > 0.8 {
            continue;
        }
        if tone != Tone::Automatic && tone_check::check_tone_change(req.to, req.draft, &line).is_err() {
            continue;
        }
        // Another wording is fine, a lost part of the meaning (a dropped name or clause) is not.
        let reference = if current.trim().is_empty() { req.draft } else { current };
        if !tone_check::content_preserved(req.to, req.source, reference, &line) {
            continue;
        }
        out.push(line);
        if out.len() == count {
            break;
        }
    }
    out
}

/// The Write tab: rewrite `text` in its own language.
pub fn write(llm: &Llm, cfg: &Config, kind: WriteKind, lang: &str, text: &str, cancel: &AtomicBool) -> Result<String> {
    let prompt = prompts::write(kind, lang, text);
    let first = check_write(&llm.complete(cfg, &prompt, cancel)?, kind, lang, text);
    // "Fix grammar" must make the smallest change: when the answer changed the meaning, ask once more, stricter.
    if kind == WriteKind::FixGrammar && matches!(first, Err(LlmError::Rejected(validate::Reject::ChangedMeaning))) {
        let strict = prompts::write_strict_fix(lang, text);
        return check_write(&llm.complete(cfg, &strict, cancel)?, kind, lang, text);
    }
    first
}

pub fn check_write(raw: &str, kind: WriteKind, lang: &str, text: &str) -> Result<String> {
    let (min_ratio, max_ratio) = kind.length_bounds();
    let expect = Expect { lang, reference: text, terms: &[], min_ratio, max_ratio };
    let out = validate::check(raw, &expect, !text.contains("\n\n")).map_err(LlmError::Rejected)?;
    if kind == WriteKind::FixGrammar && !tone_check::is_minimal_fix(text, &out) {
        return Err(LlmError::Rejected(validate::Reject::ChangedMeaning));
    }
    Ok(out)
}

/// Other words for one position of a translation, from the model.
pub fn alternatives(llm: &Llm, cfg: &Config, from: &str, to: &str, source: &str, sentence: &str, word: &str, cancel: &AtomicBool) -> Result<Vec<String>> {
    let prompt: Prompt = prompts::alternatives(from, to, source, sentence, word, 8);
    let answer = llm.complete(cfg, &prompt, cancel)?;
    let list = prompts::parse_alternatives(&validate::clean_output(&answer, false), word, 8);
    // Words in another script than the translation are not alternatives.
    Ok(list.into_iter().filter(|w| validate::is_language(w, to) || w.chars().count() < 25).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::Reject;

    fn req<'a>(terms: &'a [Term]) -> PolishRequest<'a> {
        PolishRequest { from: "en", to: "de", source: "The owls sleep during the day.", draft: "Die Eulen schlafen während des Tages.", draft_unclean: false, tone: Tone::Automatic, terms }
    }

    #[test]
    fn a_good_polish_is_accepted_and_a_term_is_inflected() {
        let terms = vec![Term::exact("owl", "Eule")];
        assert_eq!(check_polish("Die Eulen schlafen tagsüber.", &req(&terms)).unwrap(), "Die Eulen schlafen tagsüber.");
    }

    #[test]
    fn extra_text_wrong_language_and_dropped_terms_keep_the_fast_translation() {
        let terms = vec![Term::exact("owl", "Eule")];
        let r = req(&terms);
        assert_eq!(check_polish("Hier ist die Übersetzung: Die Eulen schlafen tagsüber.", &r).unwrap(), "Die Eulen schlafen tagsüber.");
        assert_eq!(check_polish("The owls sleep during the day, as they hunt at night.", &r), Err(LlmError::Rejected(Reject::WrongLanguage)));
        assert_eq!(check_polish("Die Käuze schlafen tagsüber.", &r), Err(LlmError::Rejected(Reject::MissingTerm("Eule".into()))));
        assert_eq!(check_polish("", &r), Err(LlmError::Rejected(Reject::Empty)));
        let long = "Die Eulen schlafen tagsüber ".repeat(10);
        assert_eq!(check_polish(&long, &r), Err(LlmError::Rejected(Reject::Degenerate)), "a text that repeats itself is a loop");
    }

    fn polish_req<'a>(source: &'a str, draft: &'a str, tone: Tone) -> PolishRequest<'a> {
        PolishRequest { from: "en", to: "de", source, draft, draft_unclean: false, tone, terms: &[] }
    }

    #[test]
    fn a_hint_that_is_mostly_the_source_carried_over_or_unclean_is_not_given_to_the_model() {
        let r = polish_req("im ali and i work as softwar devloper", "im ali und ich arbeite als Softwar-Devloper", Tone::Automatic);
        assert_eq!(usable_hint(&r), None, "the typos were carried through");
        let r = polish_req("hi how are you ? i m softwar devllper", "Hallo, wie geht's dir? iem Weichwarn-Devllper", Tone::Automatic);
        assert!(usable_hint(&r).is_some(), "one carried-over word is still a translation");
        let clean = polish_req("Please send me the invoice before the end of the month.", "Bitte senden Sie mir die Rechnung vor dem Ende des Monats.", Tone::Automatic);
        assert!(usable_hint(&clean).is_some());
        let unclean = PolishRequest { draft_unclean: true, ..polish_req("a b c", "Hallo, wie geht's dir?", Tone::Automatic) };
        assert_eq!(usable_hint(&unclean), None);
        assert_eq!(usable_hint(&polish_req("Hello world today", "Hello world today", Tone::Automatic)), None, "a copy is not a translation");
    }

    #[test]
    fn the_tone_is_dropped_for_a_sentence_that_addresses_nobody() {
        let r = polish_req("my name is ali and i work as software developer", "Mein Name ist Ali und ich arbeite als Softwareentwickler", Tone::Formal);
        assert_eq!(effective_tone(&r), Tone::Automatic);
        let r = polish_req("Could you tell me where you live?", "Könnten Sie mir sagen, wo Sie wohnen?", Tone::Informal);
        assert_eq!(effective_tone(&r), Tone::Informal);
        assert_eq!(effective_tone(&polish_req("Could you help?", "Könnten Sie helfen?", Tone::Automatic)), Tone::Automatic);
    }

    #[test]
    fn sentence_alternatives_drop_copies_the_current_text_and_what_breaks_the_tone_rules() {
        let r = polish_req("Could you tell me where you live?", "Könnten Sie mir sagen, wo Sie wohnen?", Tone::Informal);
        let answer = "1. Könnten Sie mir sagen, wo Sie wohnen?\n2. Kannst du mir sagen, wo du wohnst?\n3. Wo wohnst du eigentlich, sagst du es mir?\n4. Could you tell me where you live?\n5. Sag mir, wo du wohnst.";
        let got = filter_sentence_alternatives(answer, &r, Tone::Informal, "Könnten Sie mir sagen, wo Sie wohnen?", 3);
        assert_eq!(got[0], "Kannst du mir sagen, wo du wohnst?");
        assert!(!got.iter().any(|g| g.contains("Could you") || g.contains("Könnten Sie")), "{got:?}");
    }

    #[test]
    fn sentence_alternatives_must_keep_the_whole_meaning() {
        let r = polish_req("im ali and i work as softwar devloper", "Ich bin Ali und arbeite als Softwareentwickler", Tone::Automatic);
        let answer = "Ich arbeite als Softwareentwickler\nIch heiße Ali und arbeite als Softwareentwickler\nIch bin Peter und arbeite als Softwareentwickler";
        let got = filter_sentence_alternatives(answer, &r, Tone::Automatic, "Ich bin Ali und arbeite als Softwareentwickler", 3);
        assert_eq!(got, ["Ich heiße Ali und arbeite als Softwareentwickler"]);
    }

    #[test]
    fn fix_grammar_that_changes_a_noun_number_is_rejected_by_the_checks() {
        let text = "The owls is sleeping in the barn.";
        assert_eq!(check_write("The owl is sleeping in the barn.", WriteKind::FixGrammar, "en", text), Err(LlmError::Rejected(Reject::ChangedMeaning)));
        assert_eq!(check_write("The owls are sleeping in the barn.", WriteKind::FixGrammar, "en", text).unwrap(), "The owls are sleeping in the barn.");
        // Other kinds may reword.
        assert!(check_write("The owl sleeps in the barn.", WriteKind::Improve, "en", text).is_ok());
    }

    #[test]
    fn short_texts_pass_the_write_checks_for_every_kind() {
        let text = "i and me go zoo";
        assert!(check_write("He and I go to the zoo.", WriteKind::FixGrammar, "en", text).is_ok());
        assert!(check_write("He and I are going to the zoo.", WriteKind::Improve, "en", text).is_ok());
        assert!(check_write("He and I would like to go to the zoo.", WriteKind::MoreFormal, "en", text).is_ok());
    }

    #[test]
    fn write_answers_follow_the_length_rules_of_their_kind() {
        let text = "I just wanted to quickly let you know that I will not be able to attend the meeting tomorrow morning because of an appointment.";
        assert!(check_write("I cannot attend tomorrow's morning meeting because of an appointment.", WriteKind::Shorter, "en", text).is_ok());
        assert_eq!(check_write(text, WriteKind::Shorter, "en", text), Err(LlmError::Rejected(Reject::TooLong)));
        assert_eq!(check_write("Ok.", WriteKind::FixGrammar, "en", text), Err(LlmError::Rejected(Reject::TooShort)));
    }
}
