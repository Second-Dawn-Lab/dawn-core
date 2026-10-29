//! Speed / quality measurements on two long German passages (not part of the normal test run):
//!
//! ```text
//! cargo test -p dawn-translate --test perf -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Needs the converted German model and its `file://` url in `models/catalog.json`. The backend is chosen by CPU
//! architecture (oneDNN on x86-64); to compare with ruy, temporarily change the feature in
//! `crates/dawn-translate/Cargo.toml`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dawn_models::{Catalog, InstallOptions, ModelStore};
use dawn_translate::{split_sentences, Ct2Engine, Ct2Options, TranslateOptions};
use ct2rs::tokenizers::sentencepiece::Tokenizer;
use ct2rs::{Config, TranslationOptions, Translator};

/// A stand-in written to contain the usual German traps for a sentence splitter.
const STANDIN: &str = "Die Stadt Freiburg im Breisgau gehört zu den sonnigsten Orten Deutschlands und zieht jedes Jahr mehr als 1.000.000 Besucher an. Dr. Müller, der seit 2015 an der Universität lehrt, erklärt, dass sich die Altstadt z. B. wegen der Bächle, der kleinen Wasserkanäle, von anderen Städten unterscheidet. Viele Gäste kommen im Sommer, bzw. im Frühherbst, um die Weinberge zu besuchen, die ca. 20 Minuten vom Zentrum entfernt liegen. Prof. Schmidt weist darauf hin, dass die Preise für Unterkünfte, Restaurants usw. in dieser Zeit um etwa 15 % steigen können. Am 3. Oktober feiern die Einwohner den Tag der Deutschen Einheit; an diesem Tag sind die Geschäfte geschlossen, aber die Cafés bleiben geöffnet. Wer das Münster besichtigen möchte, sollte früh kommen, denn gegen Mittag stehen oft mehr als 200 Personen vor dem Eingang. Außerdem lohnt sich ein Ausflug auf den Schlossberg, von dem man bei klarem Wetter bis zu den Vogesen sehen kann. Die Stadtverwaltung bittet alle Besucher, den Müll zu trennen und, wenn möglich, mit dem Fahrrad oder der Straßenbahn zu fahren. Das Netz der Radwege ist über 400 km lang, und die Stadt investiert jedes Jahr rund 2,5 Mio. Euro in dessen Ausbau. Wer Fragen hat, kann sich im Besucherzentrum (Tel. 0761 / 12345) informieren; dort arbeiten freundliche Mitarbeiter, die mehrere Sprachen sprechen.";

/// The real test passage supplied by the user (four paragraphs).
const USER: &str = "Die Digitalisierung hat unseren Alltag in den letzten Jahren erheblich verändert. Ob beim Einkaufen, bei der Arbeit oder in der Freizeit – digitale Technologien sind mittlerweile kaum noch wegzudenken. Viele Menschen erledigen ihre Bankgeschäfte online, buchen Reisen über das Internet und kommunizieren regelmäßig über verschiedene Plattformen.\n\nDennoch bringt diese Entwicklung nicht nur Vorteile mit sich. Besonders ältere Menschen haben manchmal Schwierigkeiten, mit den ständigen technischen Veränderungen Schritt zu halten. Gleichzeitig wächst die Sorge um den Schutz persönlicher Daten, da immer mehr Informationen auf digitalen Plattformen gespeichert und verarbeitet werden.\n\nEin Beispiel dafür ist die zunehmende Nutzung künstlicher Intelligenz in Unternehmen. Moderne Programme können große Datenmengen innerhalb weniger Sekunden analysieren und dadurch Entscheidungen unterstützen. Allerdings sollten solche Systeme nicht ohne menschliche Kontrolle eingesetzt werden, insbesondere wenn ihre Ergebnisse erhebliche Auswirkungen auf das Leben anderer Menschen haben.\n\nExperten betonen deshalb, dass technischer Fortschritt stets mit gesellschaftlicher Verantwortung verbunden sein muss. Entscheidend ist nicht allein, welche Möglichkeiten eine neue Technologie bietet, sondern auch, wie verantwortungsvoll wir damit umgehen. Nur wenn Innovation, Datenschutz und Bildung gemeinsam berücksichtigt werden, können möglichst viele Menschen langfristig von der Digitalisierung profitieren.";

fn passages() -> [(&'static str, &'static str); 2] {
    [("stand-in", STANDIN), ("user passage", USER)]
}

// --- helpers ----------------------------------------------------------------------------------

fn store_with_german() -> (ModelStore, Arc<Catalog>, PathBuf) {
    let root: PathBuf = std::env::temp_dir().join(format!("dawn-perf-store-{}", std::process::id()));
    let store = ModelStore::new(&root);
    let catalog = Arc::new(Catalog::bundled());
    let cancel = AtomicBool::new(false);
    store.install_language(&catalog, "de", &InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {}).expect("install de");
    let (_, dir) = store.find_model(&catalog, "de", "en").expect("de->en installed");
    (store, catalog, dir)
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

fn median(mut runs: Vec<Duration>) -> Duration {
    runs.sort();
    runs[runs.len() / 2]
}

fn load(dir: &PathBuf, threads: usize) -> Translator<Tokenizer> {
    let config = Config { num_threads_per_replica: threads, ..Config::default() };
    Translator::with_tokenizer(dir, Tokenizer::new(dir).unwrap(), &config).unwrap()
}

fn translate_all(tr: &Translator<Tokenizer>, sentences: &[String], beam: usize) -> Vec<String> {
    let options = TranslationOptions::<String, String> { beam_size: beam, max_decoding_length: 512, ..Default::default() };
    tr.translate_batch(sentences, &options, None).unwrap().into_iter().map(|(t, _)| t.trim().to_string()).collect()
}

fn sentences_of(text: &str) -> Vec<String> {
    text.split('\n').flat_map(|line| split_sentences(line.trim(), "de")).collect()
}

fn overlap(a: &str, b: &str) -> f64 {
    let words = |s: &str| s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let (wa, mut wb) = (words(a), words(b));
    let total_b = wb.len();
    let mut hit = 0;
    for w in &wa {
        if let Some(i) = wb.iter().position(|x| x == w) {
            wb.swap_remove(i);
            hit += 1;
        }
    }
    if hit == 0 || wa.is_empty() || total_b == 0 {
        return 0.0;
    }
    let (p, r) = (hit as f64 / wa.len() as f64, hit as f64 / total_b as f64);
    2.0 * p * r / (p + r)
}

// --- measurements -----------------------------------------------------------------------------

/// What the user sees: time to first sentences, to a complete draft, and to the final text.
#[test]
#[ignore = "needs converted models; prints measurements"]
fn pipeline_report() {
    let (store, catalog, _) = store_with_german();
    let engine = Ct2Engine::new(store, catalog, &Ct2Options::default());
    let cancel = AtomicBool::new(false);
    println!("threads per model: {}  (logical cores: {})", engine.threads(), std::thread::available_parallelism().unwrap());

    // Load the model once so every number below is warm; the cold load is reported separately.
    let t = Instant::now();
    engine.translate_detailed("Hallo.", "de", "en", &cancel).unwrap();
    println!("model load + first tiny request (cold): {} ms", ms(t.elapsed()));

    for (name, text) in passages() {
        let sentences = sentences_of(text);
        let t = Instant::now();
        for _ in 0..2000 {
            std::hint::black_box(sentences_of(text));
        }
        println!("\n===== {name}: {} chars, {} sentences (splitting: {} us) =====", text.chars().count(), sentences.len(), (t.elapsed() / 2000).as_micros());

        // Before: one quality pass, batches of 16, nothing shown until the end.
        engine.clear_cache();
        let mut runs = Vec::new();
        for _ in 0..3 {
            engine.clear_cache();
            let t = Instant::now();
            engine.translate_detailed(text, "de", "en", &cancel).unwrap();
            runs.push(t.elapsed());
        }
        println!("single pass beam 2, nothing until done : {:>6} ms", ms(median(runs)));

        // Streamed in batches of four: the default (one pass) and the optional draft-first mode.
        for (label, options) in [("single pass (default)", TranslateOptions::default()), ("draft first (setting)", TranslateOptions::with_draft())] {
            let mut firsts = Vec::new();
            let mut drafts = Vec::new();
            let mut finals = Vec::new();
            for _ in 0..3 {
                engine.clear_cache();
                let t = Instant::now();
                let (mut first, mut draft) = (None, None);
                engine
                    .translate_streaming(text, "de", "en", &cancel, &options, &mut |u| {
                        first.get_or_insert(t.elapsed());
                        if u.done == u.total {
                            draft.get_or_insert(t.elapsed());
                        }
                    })
                    .unwrap();
                firsts.push(first.unwrap());
                drafts.push(draft.unwrap());
                finals.push(t.elapsed());
            }
            println!("streamed, {label}: first 4 sentences {:>6} ms, all sentences shown {:>6} ms, final {:>6} ms", ms(median(firsts)), ms(median(drafts)), ms(median(finals)));
        }

        let options = TranslateOptions::default();
        // The same text again (everything cached), and one edited sentence.
        let t = Instant::now();
        let first_run = engine.translate_streaming(text, "de", "en", &cancel, &options, &mut |_| {}).unwrap();
        println!("identical text again (cache)            : {:>6} ms", ms(t.elapsed()));
        let edited = text.replacen("Menschen", "Leute", 1).replacen("Besucher", "Gäste", 1);
        let t = Instant::now();
        let again = engine.translate_streaming(&edited, "de", "en", &cancel, &options, &mut |_| {}).unwrap();
        println!("one word edited (only that sentence)    : {:>6} ms   (edit changed the result: {})", ms(t.elapsed()), again.text != first_run.text);

        println!("Owl-L English (final, beam 2):\n{}", first_run.text);
    }
}

/// Quality/speed of the beam sizes on the production sentence splits (one batch, 4 threads).
#[test]
#[ignore = "needs converted models; prints measurements"]
fn beam_report() {
    let (_, _, dir) = store_with_german();
    let tr = load(&dir, 4);
    for (name, text) in passages() {
        let sentences = sentences_of(text);
        let mut results = Vec::new();
        for beam in [1, 2, 4] {
            translate_all(&tr, &sentences, beam); // warm-up
            let mut runs = Vec::new();
            let mut out = Vec::new();
            for _ in 0..3 {
                let t = Instant::now();
                out = translate_all(&tr, &sentences, beam);
                runs.push(t.elapsed());
            }
            results.push((beam, median(runs), out));
        }
        let reference = results.last().unwrap().2.clone();
        println!("\n== {name}: {} sentences ==", sentences.len());
        for (beam, d, out) in &results {
            let differing = out.iter().zip(&reference).filter(|(a, b)| a != b).count();
            println!("beam {beam}: {:>6} ms   differs from beam 4 in {differing}/{} sentences   unigram F1 vs beam 4: {:.3}", ms(*d), out.len(), overlap(&out.join(" "), &reference.join(" ")));
        }
    }
}

/// Speed by thread count at beam 2: the number that decides ruy vs oneDNN.
#[test]
#[ignore = "needs converted models; prints measurements"]
fn threads_report() {
    let (_, _, dir) = store_with_german();
    for (name, text) in passages() {
        let sentences = sentences_of(text);
        println!("\n== {name}: {} sentences, beam 2, whole text in one call ==", sentences.len());
        for threads in [1, 2, 3, 4, 6] {
            let tr = load(&dir, threads);
            translate_all(&tr, &sentences, 2); // warm-up
            let mut runs = Vec::new();
            for _ in 0..3 {
                let t = Instant::now();
                translate_all(&tr, &sentences, 2);
                runs.push(t.elapsed());
            }
            println!("{threads} threads: {:>6} ms", ms(median(runs)));
        }
    }
}

/// How batch size and the two-pass option trade first-text latency against total time.
#[test]
#[ignore = "needs converted models; prints measurements"]
fn streaming_variants() {
    let (store, catalog, _) = store_with_german();
    let engine = Ct2Engine::new(store, catalog, &Ct2Options::default());
    let cancel = AtomicBool::new(false);
    engine.translate_detailed("Hallo.", "de", "en", &cancel).unwrap(); // load the model
    let variants: [(&str, TranslateOptions); 7] = [
        ("single pass, batch 2 ", TranslateOptions { batch_size: 2, fast_beam: None, beam: 2 }),
        ("single pass, batch 4 ", TranslateOptions { batch_size: 4, fast_beam: None, beam: 2 }),
        ("single pass, batch 8 ", TranslateOptions { batch_size: 8, fast_beam: None, beam: 2 }),
        ("single pass, batch 16", TranslateOptions { batch_size: 16, fast_beam: None, beam: 2 }),
        ("beam 1 only, batch 4 ", TranslateOptions { batch_size: 4, fast_beam: None, beam: 1 }),
        ("two-pass, batch 4    ", TranslateOptions { batch_size: 4, fast_beam: Some(1), beam: 2 }),
        ("two-pass, batch 8    ", TranslateOptions { batch_size: 8, fast_beam: Some(1), beam: 2 }),
    ];
    for (name, text) in passages() {
        println!("\n== {name} ==   first text / complete draft / final");
        for (label, options) in &variants {
            let mut firsts = Vec::new();
            let mut drafts = Vec::new();
            let mut finals = Vec::new();
            for _ in 0..2 {
                engine.clear_cache();
                let t = Instant::now();
                let (mut first, mut draft) = (None, None);
                engine
                    .translate_streaming(text, "de", "en", &cancel, options, &mut |u| {
                        first.get_or_insert(t.elapsed());
                        if u.done == u.total {
                            draft.get_or_insert(t.elapsed());
                        }
                    })
                    .unwrap();
                firsts.push(first.unwrap());
                drafts.push(draft.unwrap());
                finals.push(t.elapsed());
            }
            println!("{label}: {:>6} ms / {:>6} ms / {:>6} ms", ms(median(firsts)), ms(median(drafts)), ms(median(finals)));
        }
    }
}
