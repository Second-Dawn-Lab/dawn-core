//! Quality spot-check and CPU-feature report, run once per backend build and diffed:
//!
//! ```text
//! cargo test -p dawn-translate --test quality -- --ignored --nocapture --test-threads=1 > out.txt
//! ```
//!
//! Needs the converted de, fr, ur and ja models and their `file://` urls in `models/catalog.json`.
//! Every `Q|` line is `pair | sentence number | source | translation`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use dawn_models::{Catalog, InstallOptions, ModelStore};
use dawn_translate::{Ct2Engine, Ct2Options};

const ENGLISH: [&str; 10] = [
    "Good morning! How did you sleep last night?",
    "The meeting has been moved to Thursday at 3:30 p.m., so please update your calendar.",
    "I can't find my keys anywhere, and I'm already late for work.",
    "Please don't forget to turn off the lights when you leave the office.",
    "She has been studying medicine for six years, but she still isn't sure it's the right career.",
    "If it rains tomorrow, we will cancel the picnic and stay at home instead.",
    "The new software update fixes several security problems and improves battery life by about 15 percent.",
    "Could you tell me how to get to the nearest train station?",
    "He said he would call me back, but he never did.",
    "Learning a new language takes patience, practice, and a lot of curiosity.",
];

const GERMAN: [&str; 10] = [
    "Guten Morgen! Wie hast du letzte Nacht geschlafen?",
    "Das Treffen wurde auf Donnerstag um 15:30 Uhr verschoben, bitte aktualisiere deinen Kalender.",
    "Ich finde meine Schlüssel nirgendwo und bin schon zu spät zur Arbeit.",
    "Vergiss bitte nicht, das Licht auszuschalten, wenn du das Büro verlässt.",
    "Sie studiert seit sechs Jahren Medizin, ist sich aber immer noch nicht sicher, ob es der richtige Beruf ist.",
    "Wenn es morgen regnet, sagen wir das Picknick ab und bleiben stattdessen zu Hause.",
    "Das neue Software-Update behebt mehrere Sicherheitsprobleme und verbessert die Akkulaufzeit um etwa 15 Prozent.",
    "Könnten Sie mir sagen, wie ich zum nächsten Bahnhof komme?",
    "Er sagte, er würde mich zurückrufen, aber er hat es nie getan.",
    "Eine neue Sprache zu lernen erfordert Geduld, Übung und viel Neugier.",
];

fn engine(languages: &[&str]) -> Ct2Engine {
    let root: PathBuf = std::env::temp_dir().join(format!("dawn-quality-store-{}", std::process::id()));
    let store = ModelStore::new(root);
    let catalog = Arc::new(Catalog::bundled());
    let cancel = AtomicBool::new(false);
    for code in languages {
        store.install_language(&catalog, code, &InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {}).unwrap_or_else(|e| panic!("install {code}: {e}"));
    }
    Ct2Engine::new(store, catalog, &Ct2Options::default())
}

#[test]
#[ignore = "needs converted models; prints translations"]
fn quality_dump() {
    let engine = engine(&["de", "fr", "ur", "ja"]);
    let cancel = AtomicBool::new(false);
    let jobs: [(&str, &str, &str, &[&str; 10]); 6] = [
        ("en-de", "en", "de", &ENGLISH),
        ("de-en", "de", "en", &GERMAN),
        ("en-fr", "en", "fr", &ENGLISH),
        ("en-ur", "en", "ur", &ENGLISH),
        ("en-ja", "en", "ja", &ENGLISH),
        ("de-ur (via English)", "de", "ur", &GERMAN),
    ];
    for (label, from, to, sentences) in jobs {
        for (i, sentence) in sentences.iter().enumerate() {
            // One sentence per request, so batching can never explain a difference.
            engine.clear_cache();
            let out = engine.translate_detailed(sentence, from, to, &cancel).unwrap();
            println!("Q|{label}|{:02}|{sentence}|{}", i + 1, out.text);
        }
    }
}

/// Which instruction sets this CPU has and what CTranslate2 / oneDNN chose.
#[test]
#[ignore = "needs converted models; prints the CPU report"]
fn cpu_report() {
    #[cfg(target_arch = "x86_64")]
    {
        println!("CPU sse4.1={} avx={} avx2={} fma={} avx512f={} avx512bw={} avx512vl={} avx512vnni={}",
            is_x86_feature_detected!("sse4.1"), is_x86_feature_detected!("avx"), is_x86_feature_detected!("avx2"), is_x86_feature_detected!("fma"),
            is_x86_feature_detected!("avx512f"), is_x86_feature_detected!("avx512bw"), is_x86_feature_detected!("avx512vl"), is_x86_feature_detected!("avx512vnni"));
    }
    println!("CT2_FORCE_CPU_ISA = {:?}", std::env::var("CT2_FORCE_CPU_ISA").ok());
    // CTranslate2 logs the ISA it selected at info level when the first model loads.
    ct2rs::set_log_level(ct2rs::LogLevel::Info);
    let engine = engine(&["de"]);
    let cancel = AtomicBool::new(false);
    let t = Instant::now();
    let out = engine.translate_detailed("Good morning! How did you sleep last night?", "en", "de", &cancel).unwrap();
    println!("SMOKE|{}|{} ms", out.text, t.elapsed().as_millis());
    let out = engine.translate_detailed("Das neue Software-Update behebt mehrere Sicherheitsprobleme.", "de", "en", &cancel).unwrap();
    println!("SMOKE|{}", out.text);
}
