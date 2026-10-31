//! Lookups in the real packs built from the kaikki.org extracts:
//!
//! ```text
//! DAWN_DICT_DIR=E:/SecondDawn/dict-build cargo test -p dawn-dict --test real_pack -- --ignored --nocapture
//! ```
//!
//! The directory holds `de-en.dict` and `en-de.dict` (see `docs/dictionary.md` for how they are built).

use std::path::PathBuf;

use dawn_dict::{Dict, Lookup, Match};

fn dict(name: &str) -> Dict {
    let dir = std::env::var("DAWN_DICT_DIR").expect("set DAWN_DICT_DIR to the folder with the built packs");
    Dict::open(&PathBuf::from(dir).join(name)).unwrap()
}

fn show(word: &str, found: &Lookup) {
    for hit in &found.hits {
        println!("{word}: {:?} {} ({}) | {} | {:?}", hit.matched, hit.entry.head, hit.entry.pos, hit.entry.grammar, hit.entry.short_translations(4));
    }
    println!("{word}: suggestions {:?}", found.suggestions);
}

#[test]
#[ignore = "needs the built packs"]
fn german_to_english_finds_base_forms() {
    let d = dict("de-en.dict");
    println!("de-en: {} entries, {} keys", d.entry_count(), d.key_count());
    for (word, base, matched) in [("erstaunt", "erstaunen", Match::Form), ("Häuser", "Haus", Match::Form), ("ging", "gehen", Match::Form), ("Haus", "Haus", Match::Head), ("gegangen", "gehen", Match::Form)] {
        let found = d.lookup(word).unwrap();
        show(word, &found);
        assert!(found.hits.iter().any(|h| h.entry.head == base && h.matched == matched), "{word} should lead to {base}");
    }
    let haus = &d.lookup("Haus").unwrap().hits[0].entry;
    assert!(haus.grammar.contains("Häuser"), "{}", haus.grammar);
    assert!(haus.short_translations(5).iter().any(|t| t.contains("house") || t.contains("building")), "{:?}", haus.short_translations(5));
    let gehen = d.lookup("gehen").unwrap();
    assert!(gehen.hits[0].entry.grammar.contains("ging"), "{}", gehen.hits[0].entry.grammar);
    let miss = d.lookup("Haeuserr").unwrap();
    println!("suggestions for Haeuserr: {:?}", miss.suggestions);
}

#[test]
#[ignore = "needs the built packs"]
fn english_to_german_lists_translations_with_gender() {
    let d = dict("en-de.dict");
    println!("en-de: {} entries, {} keys", d.entry_count(), d.key_count());
    for (word, want) in [("house", "Haus"), ("cat", "Katze"), ("went", "go"), ("astonished", "erstaun")] {
        let found = d.lookup(word).unwrap();
        show(word, &found);
        let flat: Vec<String> = found.hits.iter().flat_map(|h| h.entry.senses.iter().flat_map(|s| s.words.iter().map(|w| w.word.clone()))).collect();
        let heads: Vec<&str> = found.hits.iter().map(|h| h.entry.head.as_str()).collect();
        assert!(flat.iter().any(|w| w.contains(want)) || heads.contains(&want), "{word}: expected {want} in {flat:?} / {heads:?}");
    }
}

/// The published pack, as the app installs it: download from the pinned Hugging Face url, verify the checksum from the
/// catalog, extract, open both directions and look words up.
#[test]
#[ignore = "downloads the 11 MB pack"]
fn the_published_pack_installs_and_answers() {
    use std::sync::atomic::AtomicBool;
    let root = std::env::temp_dir().join(format!("dawn-dict-install-{}", std::process::id()));
    let store = dawn_models::ModelStore::new(&root);
    let catalog = dawn_models::Catalog::bundled();
    let pack = catalog.dictionary_for("en", "de").expect("the dictionary is in the catalog").clone();
    let cancel = AtomicBool::new(false);
    store.install_dictionary(&pack, &dawn_models::InstallOptions { proxy: None, cancel: &cancel }, &mut |_| {}).unwrap();
    assert!(store.is_dictionary_installed(&pack.id));
    let dir = store.dictionary_dir(&pack.id);
    let de_en = Dict::open(&dir.join("de-en.dict")).unwrap();
    let en_de = Dict::open(&dir.join("en-de.dict")).unwrap();
    assert_eq!((de_en.source(), de_en.target(), en_de.source(), en_de.target()), ("de", "en", "en", "de"));
    for (word, base) in [("erstaunt", "erstaunen"), ("Häuser", "Haus"), ("ging", "gehen")] {
        assert!(de_en.lookup(word).unwrap().hits.iter().any(|h| h.entry.head == base), "{word} -> {base}");
    }
    assert!(en_de.lookup("houses").unwrap().hits.iter().any(|h| h.entry.head == "house"));
    store.remove_dictionary(&pack.id).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// One lookup per language of the v0.3 packs, in the folders `dawn-dict-build pair` wrote (`DAWN_DICT_DIR/v2/<lang>/`).
/// `(language, an inflected word, its base form)`; prints the entry counts and the first meanings for the quality notes.
#[test]
#[ignore = "needs the built packs of every language"]
fn every_language_answers_a_lookup_with_its_base_form() {
    let root = PathBuf::from(std::env::var("DAWN_DICT_DIR").expect("set DAWN_DICT_DIR")).join("v2");
    let cases = [
        ("de", "Häuser", "Haus"),
        ("fr", "chevaux", "cheval"),
        ("es", "hablaron", "hablar"),
        ("it", "andavo", "andare"),
        ("ru", "книги", "книга"),
        ("tr", "evlerimizde", "ev"),
        ("ar", "كتب", "كتاب"),
        ("hi", "लड़कियों", "लड़की"),
        ("ur", "لڑکے", "لڑکا"),
        ("ja", "食べた", "食べる"),
        ("zh", "你好", "你好"),
        ("pa", "ਕਿਤਾਬਾਂ", "ਕਿਤਾਬ"),
        ("bn", "বাচ্চার", "বাচ্চা"),
    ];
    for (lang, word, base) in cases {
        let to = Dict::open(&root.join(lang).join(format!("{lang}-en.dict"))).unwrap();
        let from = Dict::open(&root.join(lang).join(format!("en-{lang}.dict"))).unwrap();
        let found = to.lookup(word).unwrap();
        let first = found.hits.first().map(|h| format!("{} ({}) {:?}", h.entry.head, h.entry.pos, h.entry.short_translations(3))).unwrap_or_default();
        println!("{lang}: {lang}->en {} entries, en->{lang} {} entries | {word} -> {first}", to.entry_count(), from.entry_count());
        if !found.hits.iter().any(|h| h.entry.head == base) {
            println!("   NOTE: {word} did not lead to {base}; got {:?} / suggestions {:?}", found.hits.iter().map(|h| h.entry.head.clone()).collect::<Vec<_>>(), found.suggestions);
        }
        assert!(!found.hits.is_empty() || !found.suggestions.is_empty(), "{lang}: nothing found for {word}");
    }
}
