//! Word alternatives and the glossary with real models. They use the models that are already installed
//! in the shared store (English to German and back), nothing is downloaded:
//!
//! ```text
//! cargo test -p dawn-translate --test features -- --ignored --nocapture --test-threads=1
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use dawn_models::{Catalog, ModelStore};
use dawn_translate::{Ct2Engine, Ct2Options, Glossary, GlossaryEntry};

fn engine() -> &'static Ct2Engine {
    static ENGINE: OnceLock<Ct2Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let store = ModelStore::open_default();
        let catalog = Arc::new(Catalog::bundled());
        assert!(store.is_language_installed(&catalog, "de"), "install German first (Owl-L, Language packs)");
        Ct2Engine::new(store, catalog, &Ct2Options::default())
    })
}

fn translate(text: &str, from: &str, to: &str) -> dawn_translate::Translation {
    engine().translate_detailed(text, from, to, &AtomicBool::new(false)).unwrap()
}

fn entry(source: &str, target: &str, from: &str, to: &str) -> GlossaryEntry {
    GlossaryEntry { id: 1, from: from.into(), to: to.into(), source: source.into(), target: target.into(), match_case: false, whole_word: true, enabled: true }
}

/// The byte-exact start of word number `n` (counted from 0) of `text`, as UTF-16 units.
fn word_offset(text: &str, n: usize) -> (usize, String) {
    let words = dawn_translate::word_bounds_all(text);
    let (start, end) = words[n];
    let units: Vec<u16> = text.encode_utf16().collect();
    (start, String::from_utf16_lossy(&units[start..end]))
}

fn show(e: &Ct2Engine, source: &str, word: usize) -> Vec<dawn_translate::Alternative> {
    let out = translate(source, "en", "de");
    let seg = &out.segments[0];
    let (start, current) = word_offset(&seg.target, word);
    let started = Instant::now();
    let alts = e.alternatives("en", "de", &seg.input, &seg.target, start, &current, 10).unwrap();
    println!("\n{source:?} -> {:?}, word {word} ({current:?}): {} alternatives in {} ms", seg.target, alts.len(), started.elapsed().as_millis());
    for a in &alts {
        println!("   {}{:<14} {:>6.2}  {}", if a.current { "*" } else { " " }, a.word, a.score, a.preview);
    }
    alts
}

#[test]
#[ignore = "needs the German pack"]
fn alternatives_at_a_position_and_their_latency() {
    let e = engine();
    let alts = show(e, "The cat sleeps on the sofa because it is tired.", 1);
    assert!(alts.first().is_some_and(|a| a.current && a.word == "Katze"));
    // A warm call.
    let started = Instant::now();
    show(e, "The cat sleeps on the sofa because it is tired.", 0);
    println!("warm call incl. translation: {} ms", started.elapsed().as_millis());
}

/// Regression: "I'm astonished" -> "Ich bin erstaunt." Clicking "erstaunt" must list whole words for that
/// position, current word first, no fragments, no duplicates, no ungrammatical forced words ("ich", "mich").
#[test]
#[ignore = "needs the German pack"]
fn regression_astonished() {
    let e = engine();
    let alts = show(e, "I'm astonished.", 2);
    let words: Vec<&str> = alts.iter().map(|a| a.word.as_str()).collect();
    assert_eq!(words[0], "erstaunt");
    assert!(alts[0].current);
    for junk in ["ich"] {
        assert!(!words.contains(&junk), "{junk} is not an alternative here: {words:?}");
    }
    assert!(words.iter().any(|w| ["überrascht", "verblüfft"].contains(w)), "{words:?}");
    assert!(alts.iter().all(|a| a.text.starts_with("Ich bin ")), "every alternative keeps the prefix");
    assert!(alts.iter().all(|a| !a.word.is_empty() && !a.word.ends_with(['.', ',', '!', '?'])), "whole words without punctuation: {words:?}");
    let mut keys: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), words.len(), "no duplicates: {words:?}");
    assert!(alts.len() <= 10);
}

/// A word that the model writes as several pieces, and a word after punctuation.
#[test]
#[ignore = "needs the German pack"]
fn regression_multi_piece_word_and_punctuation() {
    let e = engine();
    // "Unbelievably" is split into several SentencePiece pieces in German ("Unglaublicherweise").
    let alts = show(e, "Unbelievably, the weather is nice today.", 0);
    assert!(alts[0].current);
    assert!(alts.iter().all(|a| !a.word.contains(' ')), "single words only");
    // The word after a comma: the prefix must contain the comma.
    let out = translate("Yes, the weather is nice today.", "en", "de");
    let seg = &out.segments[0];
    let (start, current) = word_offset(&seg.target, 1);
    let alts = e.alternatives("en", "de", &seg.input, &seg.target, start, &current, 10).unwrap();
    println!("\nafter the comma ({current:?}): {:?}", alts.iter().map(|a| a.word.as_str()).collect::<Vec<_>>());
    let prefix = &seg.target[..seg.target.find(&current).unwrap()];
    assert!(alts.iter().all(|a| a.text.starts_with(prefix.trim_end())), "prefix {prefix:?} kept: {alts:?}");
}

#[test]
#[ignore = "needs the German pack"]
fn a_picked_alternative_sticks_through_retranslation_until_the_source_changes() {
    let e = engine();
    let sentence = "The cat sleeps on the sofa.";
    let first = translate(sentence, "en", "de");
    let seg = &first.segments[0];
    let alts = e.alternatives("en", "de", &seg.input, &seg.target, 0, "Die", 10).unwrap();
    let pick = alts.iter().find(|a| a.text != seg.target).expect("a different alternative").clone();
    e.pin("en", "de", &seg.source, &seg.input, &pick.text);

    let again = translate(sentence, "en", "de");
    assert_eq!(again.text, pick.text, "the pick wins over the model");
    assert!(again.segments[0].pinned);
    // Another sentence of the same text is unaffected.
    let two = translate("The cat sleeps on the sofa. The dog barks.", "en", "de");
    assert!(two.text.starts_with(&pick.text));
    assert!(!two.segments[1].pinned);
    // A changed source sentence is translated normally.
    let changed = translate("The cat sleeps on the chair.", "en", "de");
    assert!(!changed.segments[0].pinned);
}

/// Which placeholder shape does the model copy through unchanged? Prints a table.
#[test]
#[ignore = "needs the German pack; prints an evaluation"]
fn which_placeholder_survives_translation() {
    let shapes: [(&str, fn(usize) -> String); 6] = [
        ("Zq1x", |i| format!("Zq{i}x")),
        ("[[1]]", |i| format!("[[{i}]]")),
        ("XYZ1", |i| format!("XYZ{i}")),
        ("Zorbax", |i| ["Zorbax", "Quillen", "Brindor"][i - 1].to_string()),
        ("<1>", |i| format!("<{i}>")),
        ("§1§", |i| format!("\u{a7}{i}\u{a7}")),
    ];
    let templates = [
        "The {} sleeps on the sofa.",
        "I saw two {}s in the garden.",
        "Look at the {}!",
        "She gave the {} a name.",
        "The {} and the {} are friends.",
        "Is the {} hungry?",
    ];
    for (from, to, label) in [("en", "de", "en->de")] {
        println!("\n{label}");
        for (name, shape) in &shapes {
            let mut kept = 0;
            let mut total = 0;
            let mut samples = Vec::new();
            for t in templates {
                let tokens: Vec<String> = (1..=t.matches("{}").count()).map(shape).collect();
                let mut text = t.to_string();
                for token in &tokens {
                    text = text.replacen("{}", token, 1);
                }
                let out = translate(&text, from, to).text;
                for token in &tokens {
                    total += 1;
                    if out.matches(token.as_str()).count() == 1 {
                        kept += 1;
                    }
                }
                if samples.len() < 3 {
                    samples.push(out);
                }
            }
            println!("  {name:<8} kept {kept}/{total}   e.g. {:?}", samples);
        }
    }
}

#[test]
#[ignore = "needs the German pack"]
fn glossary_terms_appear_in_the_output_and_the_report_counts_them() {
    let e = engine();
    let mut g = Glossary::default();
    g.entries = vec![entry("owl", "Eule", "en", "de"), entry("sofa", "Couch", "en", "de")];
    e.set_glossary(g);
    let cases = [
        "The owl sleeps on the sofa.",
        "I saw two owls in the garden.",
        "She gave the owl a name.",
        "Owl pellets are interesting.",
        "The owls and the sofa are friends.",
        "Look at the owl!",
        "We moved the sofa to the owl's tree.",
    ];
    let mut ok = 0;
    for text in cases {
        let out = translate(text, "en", "de");
        let has_owl = !text.to_lowercase().contains("owl") || out.text.contains("Eule");
        println!("{text:<44} -> {:<52} hits {} {}", out.text, out.glossary_hits, if has_owl { "" } else { "(MISSING)" });
        if has_owl {
            ok += 1;
        }
    }
    println!("owl term present in {ok}/{}", cases.len());
    e.set_glossary(Glossary::default());
    assert!(ok >= cases.len() - 2, "the placeholder technique should work for most sentences");
}

/// Placeholder (Zq1x, restored afterwards) against the target term written inline in the English
/// sentence ("The Eule sleeps."): which one agrees better with the articles? Prints a table.
#[test]
#[ignore = "needs the German pack; prints an evaluation"]
fn placeholder_against_inline_target_term() {
    let terms = [("owl", "Eule"), ("dog", "Hund"), ("child", "Kind"), ("sofa", "Couch")];
    let templates = ["The {} sleeps in the garden.", "I gave the {} a name.", "Look at the {}!", "The {} is bigger than the house.", "We talk about the {} every day."];
    for (source, target) in terms {
        println!("\n{source} -> {target}");
        for t in templates {
            let placeholder = translate(&t.replace("{}", "Zq1x"), "en", "de").text.replace("Zq1x", target);
            let inline = translate(&t.replace("{}", target), "en", "de").text;
            let plain = translate(&t.replace("{}", source), "en", "de").text;
            println!("  placeholder: {placeholder:<44} inline: {inline:<44} plain: {plain}");
        }
    }
}

/// Which paired marker survives translation, so bold/italic words inside a sentence can be put back?
#[test]
#[ignore = "needs the German pack; prints an evaluation"]
fn which_formatting_markers_survive() {
    let shapes: [(&str, fn(usize, bool) -> String); 6] = [
        ("[[1]]..[[/1]]", |i, close| if close { format!("[[/{i}]]") } else { format!("[[{i}]]") }),
        ("<1>..</1>", |i, close| if close { format!("</{i}>") } else { format!("<{i}>") }),
        ("{1}..{/1}", |i, close| if close { format!("{{/{i}}}") } else { format!("{{{i}}}") }),
        ("<b1>..</b1>", |i, close| if close { format!("</b{i}>") } else { format!("<b{i}>") }),
        ("[1]..[/1]", |i, close| if close { format!("[/{i}]") } else { format!("[{i}]") }),
        ("*1*..*1*", |_, _| "*".to_string()),
    ];
    let cases = [
        "This is a {bold} sentence about owls.",
        "Please read the {user manual} before you start the {machine}.",
        "The {quick} brown fox jumps over the {lazy dog}.",
        "{Note:} the file is saved automatically.",
        "We offer {free shipping} on all orders over 50 euros.",
    ];
    for (name, shape) in &shapes {
        let (mut balanced, mut total) = (0, 0);
        let mut sample = String::new();
        for case in cases {
            let mut text = String::new();
            let mut n = 0;
            let mut chars = case.chars();
            while let Some(c) = chars.next() {
                match c {
                    '{' => {
                        n += 1;
                        text.push_str(&shape(n, false));
                    }
                    '}' => text.push_str(&shape(n, true)),
                    _ => text.push(c),
                }
            }
            let out = translate(&text, "en", "de").text;
            // Intact: every marker occurs exactly once (the * shape: an even number of stars) and open comes before close.
            let ok = (1..=n).all(|i| {
                let (open, close) = (shape(i, false), shape(i, true));
                if open == close {
                    out.matches(open.as_str()).count() == 2
                } else {
                    out.matches(open.as_str()).count() == 1 && out.matches(close.as_str()).count() == 1 && out.find(open.as_str()) < out.find(close.as_str())
                }
            });
            total += 1;
            balanced += ok as usize;
            if sample.is_empty() {
                sample = out;
            }
        }
        println!("{name:<16} intact {balanced}/{total}   e.g. {sample:?}");
    }
}

/// Regression: "My Words" with `astonished -> xsdcv`; a sentence picked from the alternatives before the term was added
/// must not hide the glossary, and a made-up word must come out as written.
#[test]
#[ignore = "needs the German pack"]
fn a_glossary_term_wins_over_an_earlier_pick_and_a_made_up_word_survives() {
    let e = engine();
    e.set_glossary(Glossary::default());
    let first = translate("I'm astonished.", "en", "de");
    let seg = &first.segments[0];
    let alts = e.alternatives("en", "de", &seg.input, &seg.target, dawn_translate::word_bounds_all(&seg.target)[2].0, "erstaunt", 10).unwrap();
    let pick = alts.iter().find(|a| !a.current && a.word == "überrascht").expect("überrascht is offered");
    e.pin("en", "de", &seg.source, &seg.input, &pick.text);
    assert_eq!(translate("I'm astonished.", "en", "de").text, pick.text);

    let mut g = Glossary::default();
    g.entries = vec![entry("astonished", "xsdcv", "en", "de")];
    e.set_glossary(g);
    let out = translate("I'm astonished.", "en", "de");
    println!("with the glossary: {:?} (hits {}, terms {:?})", out.text, out.glossary_hits, out.glossary_terms);
    assert!(out.text.contains("xsdcv"), "{}", out.text);
    assert_eq!(out.glossary_hits, 1);
    assert!(!out.segments[0].pinned);
    e.set_glossary(Glossary::default());
}

#[test]
#[ignore = "needs the German pack"]
fn sentence_alternatives_are_different_whole_sentences() {
    let e = engine();
    let out = translate("I'm astonished.", "en", "de");
    let seg = &out.segments[0];
    let alts = e.sentence_alternatives("en", "de", &seg.input, &seg.target, 3).unwrap();
    println!("{:?} -> {alts:?}", seg.target);
    assert!(!alts.is_empty() && alts.len() <= 3);
    assert!(alts.iter().all(|a| a != &seg.target));
}
