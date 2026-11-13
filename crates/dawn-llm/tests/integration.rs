//! Tests that need a real model. They are `#[ignore]`d; run them with a runtime and a model on disk:
//!
//! ```text
//! set DAWN_LLM_RUNTIME=C:\path\to\llama-server.exe
//! set DAWN_LLM_MODEL=C:\path\to\model.gguf
//! cargo test -p dawn-llm --test integration -- --ignored --nocapture --test-threads=1
//! ```
//!
//! They check what the prompts and the output checks are for, not the exact words of the model: a tone shows in the
//! pronoun, a glossary term comes back inflected, a rewrite keeps the language.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

use dawn_llm::tasks::{self, PolishRequest};
use dawn_llm::{prompts, Choice, Config, Llm, SidecarSpec, Term, Tone, WriteKind};

fn config() -> Config {
    let exe = std::env::var("DAWN_LLM_RUNTIME").expect("set DAWN_LLM_RUNTIME to llama-server.exe");
    let model = std::env::var("DAWN_LLM_MODEL").expect("set DAWN_LLM_MODEL to a .gguf file");
    Config {
        choice: Choice::Builtin,
        builtin: Some(SidecarSpec { exe: PathBuf::from(exe), model: PathBuf::from(model), threads: 0, context: 4096 }),
        builtin_ram: 0, // the test machine is the judge
        ollama_endpoint: dawn_llm::OLLAMA_DEFAULT.into(),
        ollama_model: String::new(),
    }
}

/// One model for all tests: it loads once.
fn llm() -> &'static Llm {
    static LLM: OnceLock<Llm> = OnceLock::new();
    LLM.get_or_init(Llm::new)
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

fn polish(from: &str, to: &str, source: &str, draft: &str, tone: Tone, terms: &[Term]) -> Result<String, dawn_llm::LlmError> {
    tasks::polish(llm(), &config(), &PolishRequest { from, to, source, draft, draft_unclean: false, tone, terms }, &no_cancel())
}

#[test]
#[ignore = "needs a model"]
fn polish_makes_a_stiff_translation_natural_and_keeps_the_language() {
    let out = polish("en", "de", "I'm astonished.", "Ich bin mich erstaunt.", Tone::Automatic, &[]).unwrap();
    println!("polished: {out}");
    assert!(!out.contains("mich erstaunt"), "{out}");
    assert!(out.starts_with("Ich bin") || out.starts_with("Ich"), "{out}");
}

#[test]
#[ignore = "needs a model"]
fn messy_input_is_translated_by_its_intended_meaning_not_word_by_word() {
    // The drafts are what the fast engine really answered for these sources.
    let cases = [
        ("hi how are you ? i m softwar devllper", "Hallo, wie geht's dir? iem Weichwarn-Devllper", ["hallo", "entwickler"]),
        ("im ali and i work as softwar devloper", "im ali und ich arbeite als Softwar-Devloper", ["ali", "entwickler"]),
    ];
    for (source, draft, must) in cases {
        let started = std::time::Instant::now();
        let out = polish("en", "de", source, draft, Tone::Automatic, &[]).unwrap();
        println!("{source:?} -> {out:?} ({} ms)", started.elapsed().as_millis());
        let lower = out.to_lowercase();
        for word in must {
            assert!(lower.contains(word), "{out} lacks {word}");
        }
        assert!(!lower.contains("devllper") && !lower.contains("devloper") && !lower.contains("softwar-"), "{out}");
        assert!(dawn_core::degenerate::find_degeneration(source, &out).is_none());
    }
    // The same without any hint (the fast engine looped on it).
    let out = tasks::polish(llm(), &config(), &PolishRequest { from: "en", to: "de", source: cases[0].0, draft: "", draft_unclean: true, tone: Tone::Automatic, terms: &[] }, &no_cancel()).unwrap();
    println!("without a hint: {out}");
    assert!(out.to_lowercase().contains("entwickler"), "{out}");
}

#[test]
#[ignore = "needs a model"]
fn a_source_typo_that_is_a_german_word_is_not_copied_through() {
    let source = "im ali and i work as softwar devloper";
    // The draft the fast engine gave before the source was cleaned up: "Im" is German for "in the".
    let out = polish("en", "de", source, "Im Ali und ich arbeite als Softwareentwickler.", Tone::Automatic, &[]).unwrap();
    println!("typo draft -> {out:?}");
    let lower = out.to_lowercase();
    assert!(lower.starts_with("ich bin ali") || lower.starts_with("ich heiße ali") || lower.starts_with("ich heisse ali"), "{out}");
    assert!(lower.contains("entwickler"), "{out}");
    // The normalized source, with the draft the fast engine gives for it.
    let clean = dawn_core::normalize::normalize_source("en", source, &[]);
    assert_eq!(clean, "I'm ali and I work as softwar devloper");
    let out = polish("en", "de", &clean, "Ich bin Ali und ich arbeite als Softwareentwickler.", Tone::Automatic, &[]).unwrap();
    println!("clean source -> {out:?}");
    let lower = out.to_lowercase();
    assert!(lower.starts_with("ich bin ali") || lower.starts_with("ich heiße ali") || lower.starts_with("ich heisse ali"), "{out}");
    assert!(lower.contains("entwickler"), "{out}");
}

#[test]
#[ignore = "needs a model"]
fn polish_latency_for_a_short_sentence_before_and_after() {
    let (source, draft) = ("Please send me the invoice before the end of the month.", "Bitte senden Sie mir die Rechnung vor dem Ende des Monats.");
    let cfg = config();
    let old = prompts::polish("en", "de", source, draft, Tone::Automatic, &[]);
    let new = prompts::translate_intended("en", "de", source, Some(draft), Tone::Automatic, &[]);
    let _ = llm().complete(&cfg, &old, &no_cancel()).unwrap(); // loads the model
    let time = |p: &prompts::Prompt| {
        let mut t = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let _ = llm().complete(&cfg, p, &no_cancel()).unwrap();
            t.push(started.elapsed().as_millis());
        }
        t
    };
    println!("before (rewrite the draft): {:?} ms", time(&old));
    println!("after (translate the intended meaning, draft as a hint): {:?} ms", time(&new));
}

#[test]
#[ignore = "needs a model"]
fn tone_german_sie_and_du() {
    let source = "Could you please tell me where you live?";
    let draft = "Könnten Sie mir bitte sagen, wo Sie wohnen?";
    let formal = polish("en", "de", source, draft, Tone::Formal, &[]).unwrap();
    let informal = polish("en", "de", source, draft, Tone::Informal, &[]).unwrap();
    println!("formal:   {formal}\ninformal: {informal}");
    assert!(formal.contains("Sie"), "{formal}");
    assert!(!informal.contains("Sie ") && (informal.contains(" du") || informal.contains("du ") || informal.contains("dein")), "{informal}");
}

#[test]
#[ignore = "needs a model"]
fn tone_never_changes_who_speaks_or_the_kind_of_sentence() {
    // The reported bug: "Formal" turned "I work" into "you work", "Informal" made a question of the statement.
    let source = "my name is ali and i work as software developer";
    let draft = "Mein Name ist Ali und ich arbeite als Softwareentwickler";
    for tone in [Tone::Formal, Tone::Informal, Tone::Automatic] {
        let out = polish("en", "de", source, draft, tone, &[]).unwrap();
        println!("{tone:?}: {out}");
        let lower = out.to_lowercase();
        assert!(lower.contains("ich") || lower.contains("mein"), "still the first person: {out}");
        assert!(!out.contains('?'), "still a statement: {out}");
        assert!(!out.contains("Sie ") && !out.contains("Ihr ") && !lower.contains(" du "), "nobody is addressed: {out}");
    }
}

#[test]
#[ignore = "needs a model"]
fn the_model_loads_again_after_it_was_freed() {
    let llm = Llm::new();
    let cfg = config();
    let prompt = prompts::translate("en", "de", "Good morning.", Tone::Automatic, &[]);
    assert!(llm.complete(&cfg, &prompt, &no_cancel()).is_ok());
    llm.unload();
    assert!(!llm.is_loaded());
    assert!(llm.complete(&cfg, &prompt, &no_cancel()).is_ok(), "the next request starts the runtime again");
    assert!(llm.is_loaded());
    llm.unload();
}

#[test]
#[ignore = "needs a model"]
fn glossary_term_is_inflected_owl_becomes_eulen() {
    // The glossary lists "owl"; the sentence has "owls". The fast translation has a wrong or uninflected word.
    let terms = vec![Term { source: "owl".into(), found: "owls".into(), target: "Eule".into() }];
    let out = polish("en", "de", "The owls sleep during the day.", "Die Käuze schlafen am Tag.", Tone::Automatic, &terms).unwrap();
    println!("glossary: {out}");
    assert!(out.contains("Eulen"), "{out}");
    // Singular with the right article.
    let single = vec![Term::exact("owl", "Eule")];
    let one = polish("en", "de", "The children are afraid of the owl.", "Die Kinder haben Angst vor dem Kauz.", Tone::Automatic, &single).unwrap();
    assert!(one.contains("Eule"), "{one}");
}

#[test]
#[ignore = "needs a model"]
fn write_fix_grammar_keeps_the_language_and_fixes_the_mistakes() {
    let text = "Yesterday me and him goes to the shop and buyed three apple.";
    let out = tasks::write(llm(), &config(), WriteKind::FixGrammar, "en", text, &no_cancel()).unwrap();
    println!("fixed: {out}");
    assert!(out.contains("went") || out.contains("go"), "{out}");
    assert!(out.contains("bought"), "{out}");
    let pieces = dawn_llm::diff::diff_words(text, &out);
    assert!(dawn_llm::diff::change_count(&pieces) >= 2);
    assert_eq!(dawn_llm::diff::apply(&pieces, &vec![true; dawn_llm::diff::change_count(&pieces)]), out);
}

/// The Write tab's model: `DAWN_LLM_WRITE_MODEL` (Phi-4 mini, the writing model), else the translation model.
fn write_config() -> Config {
    let mut cfg = config();
    if let Ok(model) = std::env::var("DAWN_LLM_WRITE_MODEL") {
        if let Some(spec) = cfg.builtin.as_mut() {
            spec.model = PathBuf::from(model);
        }
    }
    cfg
}

#[test]
#[ignore = "needs the writing model (set DAWN_LLM_WRITE_MODEL to Phi-4 mini)"]
fn write_fix_grammar_fixes_the_pronoun_case_too() {
    // The reported case: "me and him goes" was only half fixed ("Yesterday me and him went ...").
    let llm = Llm::new();
    let text = "Yesterday me and him goes to the shop and buyed three apple.";
    let out = tasks::write(&llm, &write_config(), WriteKind::FixGrammar, "en", text, &no_cancel()).unwrap();
    println!("fixed with the writing model: {out}");
    let lower = out.to_lowercase();
    assert!(lower.contains("he and i went") || lower.contains("i and he went"), "the subject pronouns are fixed: {out}");
    assert!(lower.contains("went") && lower.contains("bought") && lower.contains("apples"), "{out}");
    assert!(!lower.contains("me and him") && !lower.contains("him and i"), "{out}");
    llm.unload();
}

#[test]
#[ignore = "needs the writing model (set DAWN_LLM_WRITE_MODEL to Phi-4 mini)"]
fn short_texts_are_rewritten_for_every_kind_not_rejected() {
    // The reported failure: "The answer of the model could not be used" for every action on a five-word text.
    let llm = Llm::new();
    for kind in [WriteKind::FixGrammar, WriteKind::Improve, WriteKind::MoreFormal] {
        let out = tasks::write(&llm, &write_config(), kind, "en", "i and me go zoo", &no_cancel()).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        println!("{kind:?}: {out}");
        assert!(out.to_lowercase().contains("zoo") && out.len() > 8, "{out}");
    }
    let fixed = tasks::write(&llm, &write_config(), WriteKind::FixGrammar, "en", "i and me go zoo", &no_cancel()).unwrap();
    assert!(fixed.to_lowercase().starts_with("he and i"), "{fixed}");
    llm.unload();
}

#[test]
#[ignore = "needs the writing model (set DAWN_LLM_WRITE_MODEL to Phi-4 mini)"]
fn fix_grammar_keeps_the_plural_the_verb_is_what_is_wrong() {
    let llm = Llm::new();
    let out = tasks::write(&llm, &write_config(), WriteKind::FixGrammar, "en", "The owls is sleeping in the barn.", &no_cancel()).unwrap();
    println!("fixed: {out}");
    assert_eq!(out.trim(), "The owls are sleeping in the barn.");
    llm.unload();
}

#[test]
#[ignore = "needs a model"]
fn sentence_alternatives_keep_the_whole_meaning_of_the_messy_sentence() {
    let source = "im ali and i work as softwar devloper";
    let polished = "Ich bin Ali und arbeite als Softwareentwickler";
    let req = PolishRequest { from: "en", to: "de", source, draft: "im ali und ich arbeite als Softwar-Devloper", draft_unclean: false, tone: Tone::Automatic, terms: &[] };
    let list = tasks::sentence_alternatives(llm(), &config(), &req, polished, 3, &no_cancel()).unwrap();
    println!("alternatives: {list:?}");
    for alt in &list {
        assert!(alt.contains("Ali"), "the name is kept: {alt}");
        assert!(dawn_llm::tone_check::content_preserved("de", source, polished, alt), "{alt}");
    }
    assert!(!list.iter().any(|a| a.trim() == "Ich arbeite als Softwareentwickler"));
}

#[test]
#[ignore = "needs a model"]
fn alternatives_come_back_as_a_clean_word_list() {
    let list = tasks::alternatives(llm(), &config(), "en", "de", "I'm astonished.", "Ich bin erstaunt.", "erstaunt", &no_cancel()).unwrap();
    println!("alternatives: {list:?}");
    assert!(!list.is_empty() && !list.iter().any(|w| w.eq_ignore_ascii_case("erstaunt") || w.contains('\n')));
    assert!(!list.contains(&"mich".to_string()), "{list:?}");
}

#[test]
#[ignore = "needs a model"]
fn the_prompt_for_each_task_is_what_the_model_was_tested_with() {
    // A guard against editing a prompt without running the tests above again.
    let p = prompts::polish("en", "de", "Hi.", "Hallo.", Tone::Formal, &[]);
    assert!(p.user.starts_with("Here is a text in English and a draft translation into German."));
}
