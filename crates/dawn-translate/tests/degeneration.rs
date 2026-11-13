//! Misspelled input made the fast engine loop ("Hallo, wie geht's dir? n n n n n ..."). Uses the German pack that is
//! installed in the shared model store (no download):
//!
//! ```text
//! cargo test -p dawn-translate --test degeneration -- --ignored --nocapture
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use dawn_core::degenerate::find_degeneration;
use dawn_models::{Catalog, ModelStore};
use dawn_translate::{Ct2Engine, Ct2Options};

fn engine() -> Ct2Engine {
    let store = ModelStore::open_default();
    let catalog = Arc::new(Catalog::bundled());
    assert!(store.is_language_installed(&catalog, "de"), "install German first (Settings, Language packs)");
    Ct2Engine::new(store, catalog, &Ct2Options::default())
}

#[test]
#[ignore = "needs the German pack in the shared store"]
fn the_reported_misspelled_sentence_never_comes_back_as_a_loop() {
    let engine = engine();
    for source in [
        "hi how are you ? i m softwar devllper",
        "im ali and i work as softwar devloper",
        "wat is ur nam and wher do u liv",
        "pls sned me teh invoce befor frday",
        "hi how are you ? i m softwar devllper hi how are you ? i m softwar devllper",
    ] {
        let out = engine.translate_detailed(source, "en", "de", &AtomicBool::new(false)).unwrap();
        println!("{source:?} => {:?} (unclean: {:?})", out.text, out.segments.iter().map(|s| s.unclean).collect::<Vec<_>>());
        assert!(!out.text.trim().is_empty());
        assert_eq!(find_degeneration(source, &out.text), None, "a loop was shown: {}", out.text);
        assert!(!out.text.contains("n n n"), "{}", out.text);
    }
}
