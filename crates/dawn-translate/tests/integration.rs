//! End-to-end tests with real models. They need the models converted by
//! `tools/convert-models` (de, ur and hi) and the matching `file://` urls in `models/catalog.json`:
//!
//! ```text
//! cargo test -p dawn-translate --test integration -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use dawn_models::{Catalog, InstallOptions, ModelStore};
use dawn_translate::{Ct2Engine, Ct2Options, TranslationEngine};

/// A throwaway store with German and Urdu installed from the catalog's `file://` urls.
fn engine() -> &'static Ct2Engine {
    static ENGINE: OnceLock<Ct2Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let root: PathBuf = std::env::temp_dir().join(format!("dawn-it-store-{}", std::process::id()));
        let store = ModelStore::new(root);
        let catalog = Arc::new(Catalog::bundled());
        let cancel = AtomicBool::new(false);
        for code in ["de", "ur", "hi"] {
            store
                .install_language(&catalog, code, &InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {})
                .unwrap_or_else(|e| panic!("could not install {code}: {e}"));
        }
        Ct2Engine::new(store, catalog, &Ct2Options::default())
    })
}

fn translate(text: &str, from: &str, to: &str) -> (String, bool) {
    let out = engine().translate_detailed(text, from, to, &AtomicBool::new(false)).unwrap();
    println!("{from}->{to}: {text:?} => {:?} (pivot: {})", out.text, out.pivoted);
    assert!(!out.text.trim().is_empty());
    assert_ne!(out.text, text);
    (out.text, out.pivoted)
}

#[test]
#[ignore = "needs converted models"]
fn english_to_german() {
    let (text, pivoted) = translate("Hello, how are you?", "en", "de");
    assert!(!pivoted);
    assert!(text.contains("Hallo") || text.contains("geht"), "unexpected German: {text}");
}

#[test]
#[ignore = "needs converted models"]
fn german_to_english() {
    let (text, pivoted) = translate("Hallo, wie geht es dir?", "de", "en");
    assert!(!pivoted);
    assert!(text.to_lowercase().contains("hello") || text.to_lowercase().contains("how"), "unexpected English: {text}");
}

#[test]
#[ignore = "needs converted models"]
fn english_to_urdu_and_back() {
    let (urdu, _) = translate("Hello, how are you?", "en", "ur");
    assert!(urdu.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)), "expected Arabic-script Urdu: {urdu}");
    translate(&urdu, "ur", "en");
}

#[test]
#[ignore = "needs converted models"]
fn german_to_urdu_is_direct_and_urdu_to_german_pivots() {
    // The multilingual -inc model reads German, so de->ur needs no pivot ...
    let (_, pivoted) = translate("Hallo, wie geht es dir?", "de", "ur");
    assert!(!pivoted);
    let (_, pivoted) = translate("Hallo, wie geht es dir?", "de", "hi");
    assert!(!pivoted);
    // ... but the reverse model only writes English, so ur->de goes through English.
    let (_, pivoted) = translate("آپ کیسے ہیں؟", "ur", "de");
    assert!(pivoted);
}

#[test]
#[ignore = "needs converted models; prints samples"]
fn urdu_and_hindi_samples() {
    let sentences = [
        "Good morning! How did you sleep last night?",
        "The meeting has been moved to Thursday at 3:30 p.m., so please update your calendar.",
        "I can't find my keys anywhere, and I'm already late for work.",
        "If it rains tomorrow, we will cancel the picnic and stay at home instead.",
        "The new software update fixes several security problems and improves battery life by about 15 percent.",
        "Could you tell me how to get to the nearest train station?",
    ];
    for target in ["ur", "hi"] {
        for sentence in sentences {
            engine().clear_cache();
            let to = engine().translate_detailed(sentence, "en", target, &AtomicBool::new(false)).unwrap();
            let back = engine().translate_detailed(&to.text, target, "en", &AtomicBool::new(false)).unwrap();
            println!("SAMPLE|en-{target}|{sentence}|{}|back to English: {}", to.text, back.text);
        }
    }
    for sentence in ["Hallo, wie geht es dir heute?", "Das neue Software-Update behebt mehrere Sicherheitsprobleme."] {
        for target in ["ur", "hi"] {
            let out = engine().translate_detailed(sentence, "de", target, &AtomicBool::new(false)).unwrap();
            println!("SAMPLE|de-{target} (direct: {})|{sentence}|{}", !out.pivoted, out.text);
        }
    }
}

#[test]
#[ignore = "needs converted models"]
fn multi_line_text_keeps_its_lines() {
    let (text, _) = translate("Good morning.\n\nSee you tomorrow. Have a nice day!", "en", "de");
    assert_eq!(text.lines().count(), 3, "{text:?}");
    assert!(text.lines().nth(1).unwrap().is_empty());
}

#[test]
#[ignore = "needs converted models"]
fn engine_reports_supported_pairs() {
    let pairs = engine().supported_pairs();
    assert_eq!(pairs.len(), 12, "{pairs:?}");
}

/// Arabic needs the `>>ara<<` token; Turkish uses the transformer-big `opus-mt-tc-big-en-tr`.
#[test]
#[ignore = "needs converted models"]
fn arabic_and_turkish() {
    let root: PathBuf = std::env::temp_dir().join(format!("dawn-it-store-art-{}", std::process::id()));
    let store = ModelStore::new(root);
    let catalog = Arc::new(Catalog::bundled());
    let cancel = AtomicBool::new(false);
    for code in ["ar", "tr"] {
        store.install_language(&catalog, code, &InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {}).unwrap();
    }
    let engine = Ct2Engine::new(store, catalog, &Ct2Options::default());
    let go = |text: &str, from: &str, to: &str| {
        let out = engine.translate_detailed(text, from, to, &AtomicBool::new(false)).unwrap();
        println!("{from}->{to}: {text:?} => {:?} (pivot: {})", out.text, out.pivoted);
        assert!(!out.text.trim().is_empty() && out.text != text);
        out
    };
    let ar = go("Hello, how are you today? I hope you are well.", "en", "ar");
    assert!(ar.text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)), "expected Arabic script: {}", ar.text);
    go(&ar.text, "ar", "en");
    let tr = go("Hello, how are you today? I hope you are well.", "en", "tr");
    assert!(tr.text.to_lowercase().contains("merhaba") || tr.text.to_lowercase().contains("nasıl"), "unexpected Turkish: {}", tr.text);
    go(&tr.text, "tr", "en");
    let pivot = go("Merhaba, bugün nasılsın?", "tr", "ar");
    assert!(pivot.pivoted);
}

/// Prints timings; run with `--nocapture`.
#[test]
#[ignore = "needs converted models; prints timings"]
fn bench_speed_and_memory() {
    let short = "Hello, how are you today?";
    let paragraph = "The weather was lovely this morning, so we decided to walk to the market. \
        There were fresh vegetables, warm bread and a small band playing music near the fountain. \
        After a while, my sister found a book she had wanted for years, and we sat down for coffee. \
        It was one of those quiet days that you remember for a long time.";

    let run = |label: &str, text: &str, from: &str, to: &str| {
        // First call loads the models; report it separately from warm calls.
        let t = Instant::now();
        engine().translate_detailed(text, from, to, &AtomicBool::new(false)).unwrap();
        let cold = t.elapsed();
        let runs = 5;
        let t = Instant::now();
        for _ in 0..runs {
            engine().translate_detailed(text, from, to, &AtomicBool::new(false)).unwrap();
        }
        let warm = t.elapsed() / runs;
        println!("BENCH {label:<22} cold {:>6} ms   warm {:>6} ms", cold.as_millis(), warm.as_millis());
    };

    println!("BENCH threads per model: {}", dawn_translate::default_threads());
    println!("BENCH memory before any model is loaded: {} MB", working_set_mb());
    run("en->de short", short, "en", "de");
    run("en->de paragraph", paragraph, "en", "de");
    run("de->ur short (pivot)", "Hallo, wie geht es dir heute?", "de", "ur");
    run("de->ur para (pivot)", "Das Wetter war heute Morgen wunderbar, also gingen wir zum Markt. Es gab frisches Gemüse und warmes Brot. Danach tranken wir Kaffee.", "de", "ur");
    println!("BENCH models loaded: {}", engine().loaded_models());
    println!("BENCH memory with those models loaded: {} MB", working_set_mb());

    // Exactly two models in memory: en->de and de->en.
    engine().unload_all();
    println!("BENCH memory after unloading: {} MB", working_set_mb());
    engine().translate_detailed(short, "en", "de", &AtomicBool::new(false)).unwrap();
    engine().translate_detailed("Hallo, wie geht es dir?", "de", "en", &AtomicBool::new(false)).unwrap();
    println!("BENCH models loaded: {}  memory with 2 models: {} MB", engine().loaded_models(), working_set_mb());
}

/// Resident memory of this test process, in MB (Windows; asks PowerShell).
fn working_set_mb() -> u64 {
    let script = format!("(Get-Process -Id {}).WorkingSet64", std::process::id());
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().unwrap_or(0) / 1_000_000
}
